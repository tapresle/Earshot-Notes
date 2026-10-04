use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::{Store, StoreError, now_ms};

#[derive(Debug, Clone, PartialEq)]
pub struct NewSummary {
    pub meeting_id: i64,
    pub upto_ms: i64,
    pub is_final: bool,
    pub body_markdown: String,
    pub model_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: i64,
    pub meeting_id: i64,
    pub upto_ms: i64,
    pub is_final: bool,
    pub body_markdown: String,
    pub model_id: String,
    pub created_at: i64,
}

impl Store {
    pub fn insert_summary(&self, summary: NewSummary) -> Result<Summary, StoreError> {
        let conn = self.conn();
        let created_at = now_ms();
        conn.execute(
            "INSERT INTO summaries (meeting_id, upto_ms, is_final, body_markdown, model_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                summary.meeting_id,
                summary.upto_ms,
                summary.is_final,
                summary.body_markdown,
                summary.model_id,
                created_at
            ],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(f, _)
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                StoreError::MeetingNotFound(summary.meeting_id)
            }
            other => other.into(),
        })?;
        Ok(Summary {
            id: conn.last_insert_rowid(),
            meeting_id: summary.meeting_id,
            upto_ms: summary.upto_ms,
            is_final: summary.is_final,
            body_markdown: summary.body_markdown,
            model_id: summary.model_id,
            created_at,
        })
    }

    /// The most recent summary of the meeting, final or not.
    pub fn latest_summary(&self, meeting_id: i64) -> Result<Option<Summary>, StoreError> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, meeting_id, upto_ms, is_final, body_markdown, model_id, created_at
                 FROM summaries WHERE meeting_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
                [meeting_id],
                |row| {
                    Ok(Summary {
                        id: row.get(0)?,
                        meeting_id: row.get(1)?,
                        upto_ms: row.get(2)?,
                        is_final: row.get(3)?,
                        body_markdown: row.get(4)?,
                        model_id: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                },
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(meeting_id: i64, body: &str, is_final: bool) -> NewSummary {
        NewSummary {
            meeting_id,
            upto_ms: 60_000,
            is_final,
            body_markdown: body.into(),
            model_id: "gemma".into(),
        }
    }

    #[test]
    fn returns_the_latest_summary() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        store
            .insert_summary(summary(meeting.id, "so far", false))
            .unwrap();
        let last = store
            .insert_summary(summary(meeting.id, "final", true))
            .unwrap();

        assert_eq!(store.latest_summary(meeting.id).unwrap(), Some(last));
    }

    #[test]
    fn returns_none_before_any_summary() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();

        assert_eq!(store.latest_summary(meeting.id).unwrap(), None);
    }

    #[test]
    fn rejects_a_summary_for_a_missing_meeting() {
        let store = Store::open_in_memory().unwrap();

        assert!(matches!(
            store.insert_summary(summary(9, "x", true)),
            Err(StoreError::MeetingNotFound(9))
        ));
    }

    #[test]
    fn deleting_a_meeting_deletes_its_summaries() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        store
            .insert_summary(summary(meeting.id, "x", true))
            .unwrap();

        store.delete_meeting(meeting.id).unwrap();

        assert_eq!(store.latest_summary(meeting.id).unwrap(), None);
    }
}
