use rusqlite::types::Value;
use rusqlite::{OptionalExtension, Row, params, params_from_iter};
use serde::{Deserialize, Serialize};

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

/// The most meetings one page can hold.
pub const MAX_PAGE_SIZE: u32 = 200;

/// The position of the last meeting on a page. The next page starts after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingCursor {
    pub created_at: i64,
    pub id: i64,
}

impl MeetingCursor {
    /// Before every meeting, so the first page starts at the newest one.
    const START: Self = Self {
        created_at: i64::MAX,
        id: i64::MAX,
    };
}

/// Meetings after the cursor `(?1, ?2)` that also meet `filter`, newest first,
/// at most `?3` of them. Parameters in `filter` start at `?4`.
/// The `meetings_page` index serves both the seek and the order.
fn page_query(filter: &str) -> String {
    format!(
        "SELECT {MEETING_COLUMNS} FROM meetings m
         WHERE (m.created_at, m.id) < (?1, ?2) {filter}
         ORDER BY m.created_at DESC, m.id DESC LIMIT ?3"
    )
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingPage {
    pub meetings: Vec<Meeting>,
    /// Where the next page starts, or `None` on the last page.
    pub next: Option<MeetingCursor>,
}

impl MeetingPage {
    /// Builds a page from up to `limit + 1` rows; the extra row only signals
    /// that another page follows.
    pub(super) fn from_rows(mut meetings: Vec<Meeting>, limit: u32) -> Self {
        let more = meetings.len() > limit as usize;
        meetings.truncate(limit as usize);
        let next = match meetings.last() {
            Some(last) if more => Some(MeetingCursor {
                created_at: last.created_at,
                id: last.id,
            }),
            _ => None,
        };
        Self { meetings, next }
    }
}

pub(super) const MEETING_COLUMNS: &str = "m.id, m.title, m.created_at, m.started_at, m.ended_at";

pub(super) fn meeting_from_row(row: &Row) -> rusqlite::Result<Meeting> {
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

    /// One page of meetings, newest first, starting after `after` (or at the
    /// newest meeting). Pages are keyed on `(created_at, id)` rather than an
    /// offset, so each page is an index seek, and meetings created or deleted
    /// between pages don't shift the next one.
    pub fn list_meetings(
        &self,
        after: Option<MeetingCursor>,
        limit: u32,
    ) -> Result<MeetingPage, StoreError> {
        self.page_of_meetings("", Vec::new(), after, limit)
    }

    /// A page of the meetings that meet `filter`, an SQL condition on `m`
    /// that starts with `AND`, with `filter_params` bound from `?4`.
    pub(super) fn page_of_meetings(
        &self,
        filter: &str,
        filter_params: Vec<Value>,
        after: Option<MeetingCursor>,
        limit: u32,
    ) -> Result<MeetingPage, StoreError> {
        let limit = limit.clamp(1, MAX_PAGE_SIZE);
        let after = after.unwrap_or(MeetingCursor::START);
        // One more than asked for, to learn whether another page follows.
        let fetch = i64::from(limit) + 1;
        let params = [
            Value::Integer(after.created_at),
            Value::Integer(after.id),
            Value::Integer(fetch),
        ]
        .into_iter()
        .chain(filter_params);
        let conn = self.conn();
        let meetings = conn
            .prepare_cached(&page_query(filter))?
            .query_map(params_from_iter(params), meeting_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(MeetingPage::from_rows(meetings, limit))
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

        let page = store.list_meetings(None, 50).unwrap();

        assert_eq!(ids(&page.meetings), vec![second.id, first.id]);
        assert_eq!(page.next, None);
    }

    #[test]
    fn lists_nothing_for_a_new_database() {
        let page = store().list_meetings(None, 50).unwrap();

        assert!(page.meetings.is_empty());
        assert_eq!(page.next, None);
    }

    fn ids(meetings: &[Meeting]) -> Vec<i64> {
        meetings.iter().map(|m| m.id).collect()
    }

    /// Creates meetings with the given creation times, in that order.
    fn meetings_created_at(store: &Store, times: &[i64]) -> Vec<Meeting> {
        times
            .iter()
            .map(|&at| {
                let meeting = store.create_meeting("Meeting").unwrap();
                store
                    .conn()
                    .execute(
                        "UPDATE meetings SET created_at = ?1 WHERE id = ?2",
                        params![at, meeting.id],
                    )
                    .unwrap();
                Meeting {
                    created_at: at,
                    ..meeting
                }
            })
            .collect()
    }

    /// Follows `next` from the first page to the last.
    fn all_pages(store: &Store, limit: u32) -> Vec<MeetingPage> {
        let mut pages = vec![store.list_meetings(None, limit).unwrap()];
        while let Some(next) = pages.last().unwrap().next {
            pages.push(store.list_meetings(Some(next), limit).unwrap());
        }
        pages
    }

    #[test]
    fn pages_through_every_meeting_once_in_order() {
        let store = store();
        // Ties on created_at are broken by id, newest first.
        let made = meetings_created_at(&store, &[100, 200, 200, 200, 300, 400, 400]);
        let mut expected = made.clone();
        expected.sort_by_key(|m| std::cmp::Reverse((m.created_at, m.id)));

        let pages = all_pages(&store, 3);

        let sizes: Vec<usize> = pages.iter().map(|p| p.meetings.len()).collect();
        assert_eq!(sizes, vec![3, 3, 1]);
        let listed: Vec<Meeting> = pages.into_iter().flat_map(|p| p.meetings).collect();
        assert_eq!(listed, expected);
    }

    #[test]
    fn ends_without_an_empty_page_when_the_last_page_is_full() {
        let store = store();
        meetings_created_at(&store, &[1, 2, 3, 4]);

        let pages = all_pages(&store, 2);

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[1].meetings.len(), 2);
        assert_eq!(pages[1].next, None);
    }

    #[test]
    fn keeps_its_place_when_meetings_change_between_pages() {
        let store = store();
        let made = meetings_created_at(&store, &[1, 2, 3, 4, 5]);
        let first = store.list_meetings(None, 2).unwrap();
        assert_eq!(ids(&first.meetings), vec![made[4].id, made[3].id]);

        // A new meeting, and a deleted one that the cursor points at.
        store.create_meeting("Newer").unwrap();
        store.delete_meeting(made[3].id).unwrap();
        let second = store.list_meetings(first.next, 2).unwrap();

        assert_eq!(ids(&second.meetings), vec![made[2].id, made[1].id]);
    }

    #[test]
    fn clamps_the_page_size() {
        let store = store();
        meetings_created_at(&store, &vec![7; MAX_PAGE_SIZE as usize + 1]);

        assert_eq!(store.list_meetings(None, 0).unwrap().meetings.len(), 1);
        let largest = store.list_meetings(None, u32::MAX).unwrap();
        assert_eq!(largest.meetings.len(), MAX_PAGE_SIZE as usize);
        assert!(largest.next.is_some());
    }

    #[test]
    fn seeks_pages_through_the_creation_index() {
        let store = store();
        let conn = store.conn();
        let plan: Vec<String> = conn
            .prepare(&format!("EXPLAIN QUERY PLAN {}", page_query("")))
            .unwrap()
            .query_map(params![1, 1, 1], |row| row.get(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let plan = plan.join("; ");

        assert!(plan.contains("INDEX meetings_page"), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
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
