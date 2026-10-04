//! Forward-only schema migrations tracked with `PRAGMA user_version`.

use rusqlite::Connection;

use super::StoreError;

/// Never edit or reorder an entry once it has shipped. Append new migrations.
const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/0001_meetings_and_notes.sql"),
    include_str!("../../migrations/0002_transcript_segments.sql"),
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
