use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;

use super::{Store, StoreError, now_ms};

pub const DEFAULT_TITLE: &str = "Untitled meeting";
const MAX_TITLE_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    pub id: i64,
    pub title: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    #[serde(flatten)]
    pub meeting: Meeting,
    pub notes_markdown: String,
    pub notes_updated_at: i64,
}

const MEETING_COLUMNS: &str = "m.id, m.title, m.created_at, m.started_at, m.ended_at";

fn meeting_from_row(row: &Row) -> rusqlite::Result<Meeting> {
    Ok(Meeting {
        id: row.get(0)?,
        title: row.get(1)?,
        created_at: row.get(2)?,
        started_at: row.get(3)?,
        ended_at: row.get(4)?,
    })
}

/// Trims the title, falls back to the default when empty, and caps its length.
fn normalize_title(title: &str) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return DEFAULT_TITLE.to_owned();
    }
    trimmed.chars().take(MAX_TITLE_CHARS).collect()
}

impl Store {
    pub fn create_meeting(&self, title: &str) -> Result<Meeting, StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let now = now_ms();
        let title = normalize_title(title);
        tx.execute(
            "INSERT INTO meetings (title, created_at) VALUES (?1, ?2)",
            params![title, now],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO notes (meeting_id, updated_at) VALUES (?1, ?2)",
            params![id, now],
        )?;
        tx.commit()?;
        Ok(Meeting {
            id,
            title,
            created_at: now,
            started_at: None,
            ended_at: None,
        })
    }

    /// All meetings, newest first.
    pub fn list_meetings(&self) -> Result<Vec<Meeting>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {MEETING_COLUMNS} FROM meetings m ORDER BY m.created_at DESC, m.id DESC"
        ))?;
        let meetings = stmt
            .query_map([], meeting_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(meetings)
    }

    pub fn get_meeting(&self, id: i64) -> Result<MeetingDetail, StoreError> {
        let conn = self.conn();
        conn.query_row(
            &format!(
                "SELECT {MEETING_COLUMNS}, n.body_markdown, n.updated_at
                 FROM meetings m JOIN notes n ON n.meeting_id = m.id
                 WHERE m.id = ?1"
            ),
            [id],
            |row| {
                Ok(MeetingDetail {
                    meeting: meeting_from_row(row)?,
                    notes_markdown: row.get(5)?,
                    notes_updated_at: row.get(6)?,
                })
            },
        )
        .optional()?
        .ok_or(StoreError::MeetingNotFound(id))
    }

    /// Returns the stored title, which may differ from the input after normalization.
    pub fn rename_meeting(&self, id: i64, title: &str) -> Result<String, StoreError> {
        let title = normalize_title(title);
        let changed = self.conn().execute(
            "UPDATE meetings SET title = ?1 WHERE id = ?2",
            params![title, id],
        )?;
        if changed == 0 {
            return Err(StoreError::MeetingNotFound(id));
        }
        Ok(title)
    }

    /// Returns the new `updated_at` timestamp.
    pub fn update_notes(&self, id: i64, body_markdown: &str) -> Result<i64, StoreError> {
        let now = now_ms();
        let changed = self.conn().execute(
            "UPDATE notes SET body_markdown = ?1, updated_at = ?2 WHERE meeting_id = ?3",
            params![body_markdown, now, id],
        )?;
        if changed == 0 {
            return Err(StoreError::MeetingNotFound(id));
        }
        Ok(now)
    }

    /// Deletes the meeting and, through cascades, everything attached to it.
    pub fn delete_meeting(&self, id: i64) -> Result<(), StoreError> {
        let changed = self
            .conn()
            .execute("DELETE FROM meetings WHERE id = ?1", [id])?;
        if changed == 0 {
            return Err(StoreError::MeetingNotFound(id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn creates_a_meeting_with_empty_notes() {
        let store = store();

        let meeting = store.create_meeting("Design sync").unwrap();
        let detail = store.get_meeting(meeting.id).unwrap();

        assert_eq!(detail.meeting, meeting);
        assert_eq!(detail.meeting.title, "Design sync");
        assert_eq!(detail.notes_markdown, "");
        assert_eq!(detail.meeting.started_at, None);
    }

    #[test]
    fn normalizes_titles() {
        assert_eq!(normalize_title("  Standup  "), "Standup");
        assert_eq!(normalize_title("   "), DEFAULT_TITLE);
        assert_eq!(
            normalize_title(&"x".repeat(500)).chars().count(),
            MAX_TITLE_CHARS
        );
    }

    #[test]
    fn lists_meetings_newest_first() {
        let store = store();
        let first = store.create_meeting("First").unwrap();
        let second = store.create_meeting("Second").unwrap();

        let ids: Vec<i64> = store
            .list_meetings()
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();

        assert_eq!(ids, vec![second.id, first.id]);
    }

    #[test]
    fn lists_nothing_for_a_new_database() {
        assert!(store().list_meetings().unwrap().is_empty());
    }

    #[test]
    fn renames_a_meeting() {
        let store = store();
        let meeting = store.create_meeting("Old").unwrap();

        let stored = store.rename_meeting(meeting.id, "  New  ").unwrap();

        assert_eq!(stored, "New");
        assert_eq!(store.get_meeting(meeting.id).unwrap().meeting.title, "New");
    }

    #[test]
    fn updates_notes() {
        let store = store();
        let meeting = store.create_meeting("Notes").unwrap();

        let updated_at = store.update_notes(meeting.id, "- ship it").unwrap();
        let detail = store.get_meeting(meeting.id).unwrap();

        assert_eq!(detail.notes_markdown, "- ship it");
        assert_eq!(detail.notes_updated_at, updated_at);
    }

    #[test]
    fn deletes_a_meeting_and_its_notes() {
        let store = store();
        let meeting = store.create_meeting("Gone").unwrap();

        store.delete_meeting(meeting.id).unwrap();

        assert!(matches!(
            store.get_meeting(meeting.id),
            Err(StoreError::MeetingNotFound(_))
        ));
        let orphaned: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(orphaned, 0);
    }

    #[test]
    fn reports_missing_meetings() {
        let store = store();

        assert!(matches!(
            store.get_meeting(42),
            Err(StoreError::MeetingNotFound(42))
        ));
        assert!(matches!(
            store.rename_meeting(42, "x"),
            Err(StoreError::MeetingNotFound(42))
        ));
        assert!(matches!(
            store.update_notes(42, "x"),
            Err(StoreError::MeetingNotFound(42))
        ));
        assert!(matches!(
            store.delete_meeting(42),
            Err(StoreError::MeetingNotFound(42))
        ));
    }

    #[test]
    fn persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.db");
        let id = {
            let store = Store::open(&path).unwrap();
            let meeting = store.create_meeting("Kept").unwrap();
            store.update_notes(meeting.id, "remember this").unwrap();
            meeting.id
        };

        let reopened = Store::open(&path).unwrap();

        assert_eq!(
            reopened.get_meeting(id).unwrap().notes_markdown,
            "remember this"
        );
    }
}
