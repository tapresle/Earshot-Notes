use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{Store, StoreError, now_ms};

/// Which capture stream a segment came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Me,
    Them,
}

impl Source {
    /// The speaker label shown in transcripts, exports, and prompts.
    pub fn label(self) -> &'static str {
        match self {
            Source::Me => "Me",
            Source::Them => "Meeting Participants",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Source::Me => "me",
            Source::Them => "them",
        }
    }

    fn parse(value: &str) -> rusqlite::Result<Self> {
        match value {
            "me" => Ok(Source::Me),
            "them" => Ok(Source::Them),
            other => Err(rusqlite::Error::InvalidColumnType(
                0,
                format!("source {other}"),
                rusqlite::types::Type::Text,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewSegment {
    pub meeting_id: i64,
    pub source: Source,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub id: i64,
    pub meeting_id: i64,
    pub source: Source,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub language: Option<String>,
}

impl Store {
    /// How far into the meeting a recording starting now begins, in milliseconds.
    /// The first recording starts at 0; a later one continues the meeting's timeline.
    pub fn recording_offset(&self, meeting_id: i64) -> Result<i64, StoreError> {
        let started_at: Option<i64> = self
            .conn()
            .query_row(
                "SELECT started_at FROM meetings WHERE id = ?1",
                [meeting_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StoreError::MeetingNotFound(meeting_id))?;
        Ok(started_at.map_or(0, |started_at| now_ms() - started_at))
    }

    /// Marks the meeting as recording, setting its start time on the first recording.
    pub fn begin_recording(&self, meeting_id: i64) -> Result<(), StoreError> {
        let changed = self.conn().execute(
            "UPDATE meetings SET started_at = COALESCE(started_at, ?1), ended_at = NULL WHERE id = ?2",
            params![now_ms(), meeting_id],
        )?;
        if changed == 0 {
            return Err(StoreError::MeetingNotFound(meeting_id));
        }
        Ok(())
    }

    pub fn end_recording(&self, meeting_id: i64) -> Result<(), StoreError> {
        let changed = self.conn().execute(
            "UPDATE meetings SET ended_at = ?1 WHERE id = ?2",
            params![now_ms(), meeting_id],
        )?;
        if changed == 0 {
            return Err(StoreError::MeetingNotFound(meeting_id));
        }
        Ok(())
    }

    /// Closes recordings left open by a crash or forced quit. Call once at startup.
    pub fn close_interrupted_recordings(&self) -> Result<usize, StoreError> {
        Ok(self.conn().execute(
            "UPDATE meetings SET ended_at = ?1 WHERE started_at IS NOT NULL AND ended_at IS NULL",
            [now_ms()],
        )?)
    }

    pub fn insert_segment(&self, segment: NewSegment) -> Result<TranscriptSegment, StoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO transcript_segments (meeting_id, source, start_ms, end_ms, text, language)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                segment.meeting_id,
                segment.source.as_str(),
                segment.start_ms,
                segment.end_ms,
                segment.text,
                segment.language,
            ],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(f, _)
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                StoreError::MeetingNotFound(segment.meeting_id)
            }
            other => other.into(),
        })?;
        Ok(TranscriptSegment {
            id: conn.last_insert_rowid(),
            meeting_id: segment.meeting_id,
            source: segment.source,
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            text: segment.text,
            language: segment.language,
        })
    }

    /// The meeting's transcript in timeline order.
    pub fn list_segments(&self, meeting_id: i64) -> Result<Vec<TranscriptSegment>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, source, start_ms, end_ms, text, language
             FROM transcript_segments WHERE meeting_id = ?1 ORDER BY start_ms, id",
        )?;
        let segments = stmt
            .query_map([meeting_id], |row| {
                Ok(TranscriptSegment {
                    id: row.get(0)?,
                    meeting_id: row.get(1)?,
                    source: Source::parse(&row.get::<_, String>(2)?)?,
                    start_ms: row.get(3)?,
                    end_ms: row.get(4)?,
                    text: row.get(5)?,
                    language: row.get(6)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(segments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(meeting_id: i64, source: Source, start_ms: i64, text: &str) -> NewSegment {
        NewSegment {
            meeting_id,
            source,
            start_ms,
            end_ms: start_ms + 1000,
            text: text.to_owned(),
            language: Some("en".to_owned()),
        }
    }

    #[test]
    fn first_recording_starts_the_meeting_timeline() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();

        let offset = store.recording_offset(meeting.id).unwrap();
        store.begin_recording(meeting.id).unwrap();

        assert_eq!(offset, 0);
        let detail = store.get_meeting(meeting.id).unwrap();
        assert!(detail.meeting.started_at.is_some());
        assert_eq!(detail.meeting.ended_at, None);
    }

    #[test]
    fn resuming_continues_the_timeline_and_reopens_the_meeting() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        store.begin_recording(meeting.id).unwrap();
        store.end_recording(meeting.id).unwrap();
        store
            .conn()
            .execute(
                "UPDATE meetings SET started_at = started_at - 60000 WHERE id = ?1",
                [meeting.id],
            )
            .unwrap();

        let offset = store.recording_offset(meeting.id).unwrap();
        store.begin_recording(meeting.id).unwrap();

        assert!((60_000..61_000).contains(&offset), "{offset}");
        assert_eq!(
            store.get_meeting(meeting.id).unwrap().meeting.ended_at,
            None
        );
    }

    #[test]
    fn closes_interrupted_recordings() {
        let store = Store::open_in_memory().unwrap();
        let open = store.create_meeting("Open").unwrap();
        let never = store.create_meeting("Never recorded").unwrap();
        store.begin_recording(open.id).unwrap();

        assert_eq!(store.close_interrupted_recordings().unwrap(), 1);

        assert!(
            store
                .get_meeting(open.id)
                .unwrap()
                .meeting
                .ended_at
                .is_some()
        );
        assert_eq!(
            store.get_meeting(never.id).unwrap().meeting.started_at,
            None
        );
    }

    #[test]
    fn stores_and_lists_segments_in_timeline_order() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        let other = store.create_meeting("Other").unwrap();
        store
            .insert_segment(segment(meeting.id, Source::Them, 5000, "second"))
            .unwrap();
        store
            .insert_segment(segment(meeting.id, Source::Me, 1000, "first"))
            .unwrap();
        store
            .insert_segment(segment(other.id, Source::Me, 0, "elsewhere"))
            .unwrap();

        let segments = store.list_segments(meeting.id).unwrap();

        let texts: Vec<(&str, Source)> = segments
            .iter()
            .map(|s| (s.text.as_str(), s.source))
            .collect();
        assert_eq!(texts, vec![("first", Source::Me), ("second", Source::Them)]);
    }

    #[test]
    fn rejects_segments_for_a_missing_meeting() {
        let store = Store::open_in_memory().unwrap();

        assert!(matches!(
            store.insert_segment(segment(99, Source::Me, 0, "orphan")),
            Err(StoreError::MeetingNotFound(99))
        ));
    }

    #[test]
    fn deleting_a_meeting_deletes_its_transcript() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        store
            .insert_segment(segment(meeting.id, Source::Me, 0, "gone soon"))
            .unwrap();

        store.delete_meeting(meeting.id).unwrap();

        assert!(store.list_segments(meeting.id).unwrap().is_empty());
    }

    #[test]
    fn reports_missing_meetings_for_recording() {
        let store = Store::open_in_memory().unwrap();

        assert!(matches!(
            store.recording_offset(5),
            Err(StoreError::MeetingNotFound(5))
        ));
        assert!(matches!(
            store.begin_recording(5),
            Err(StoreError::MeetingNotFound(5))
        ));
        assert!(matches!(
            store.end_recording(5),
            Err(StoreError::MeetingNotFound(5))
        ));
    }
}
