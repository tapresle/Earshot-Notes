//! Forward-only schema migrations tracked with `PRAGMA user_version`.

use rusqlite::Connection;

use super::StoreError;

/// Never edit or reorder an entry once it has shipped. Append new migrations.
const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/0001_meetings_and_notes.sql"),
    include_str!("../../migrations/0002_transcript_segments.sql"),
    include_str!("../../migrations/0003_summaries_and_models.sql"),
    include_str!("../../migrations/0004_meetings_page_index.sql"),
    include_str!("../../migrations/0005_meeting_search.sql"),
    include_str!("../../migrations/0006_transcript_heard_text.sql"),
    include_str!("../../migrations/0007_chat.sql"),
];

pub fn apply(conn: &mut Connection) -> Result<(), StoreError> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if current as usize > MIGRATIONS.len() {
        return Err(StoreError::SchemaTooNew {
            found: current,
            supported: MIGRATIONS.len(),
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(conn: &Connection) -> usize {
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        version as usize
    }

    #[test]
    fn applies_all_migrations_to_a_new_database() {
        let mut conn = Connection::open_in_memory().unwrap();

        apply(&mut conn).unwrap();

        assert_eq!(version(&conn), MIGRATIONS.len());
    }

    #[test]
    fn is_a_no_op_when_already_current() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply(&mut conn).unwrap();

        apply(&mut conn).unwrap();

        assert_eq!(version(&conn), MIGRATIONS.len());
    }

    #[test]
    fn indexes_meetings_saved_before_search_existed() {
        let mut conn = Connection::open_in_memory().unwrap();
        // The schema as it was before 0005_meeting_search.sql.
        for sql in &MIGRATIONS[..4] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 4).unwrap();
        conn.execute_batch(
            "INSERT INTO meetings (id, title, created_at) VALUES (1, 'Kickoff', 0);
             INSERT INTO notes (meeting_id, body_markdown, updated_at) VALUES (1, 'Agenda', 0);
             INSERT INTO transcript_segments (meeting_id, source, start_ms, end_ms, text)
             VALUES (1, 'me', 0, 1000, 'hello there');",
        )
        .unwrap();

        apply(&mut conn).unwrap();

        for (table, word) in [
            ("meetings_fts", "kickoff"),
            ("notes_fts", "agenda"),
            ("transcript_fts", "hello"),
        ] {
            let rowid: i64 = conn
                .query_row(
                    &format!("SELECT rowid FROM {table} WHERE {table} MATCH ?1"),
                    [word],
                    |row| row.get(0),
                )
                .unwrap_or_else(|e| panic!("{table}: {e}"));
            assert_eq!(rowid, 1, "{table}");
        }
    }

    #[test]
    fn refuses_a_database_from_a_newer_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        let newer = MIGRATIONS.len() as i64 + 1;
        conn.pragma_update(None, "user_version", newer).unwrap();

        assert!(matches!(
            apply(&mut conn),
            Err(StoreError::SchemaTooNew { found, .. }) if found == newer
        ));
    }
}
