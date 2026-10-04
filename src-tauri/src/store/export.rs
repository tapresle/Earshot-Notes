use std::collections::HashMap;

use rusqlite::params;

use super::meetings::{MEETING_COLUMNS, meeting_from_row};
use super::{Meeting, Source, Store, StoreError, Summary, TranscriptSegment};

/// Everything export needs about one meeting.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportMeeting {
    pub meeting: Meeting,
    pub notes_markdown: String,
    pub segments: Vec<TranscriptSegment>,
    /// The latest summary, final or partial.
    pub summary: Option<Summary>,
}

impl Store {
    /// Loads the given meetings (or all of them when `ids` is `None`), oldest
    /// first, with one query per table no matter how many meetings there are.
    pub fn export_meetings(&self, ids: Option<&[i64]>) -> Result<Vec<ExportMeeting>, StoreError> {
        let filter = ids.map(|ids| serde_json::to_string(ids).expect("ids serialize"));
        let conn = self.conn();

        let mut stmt = conn.prepare(&format!(
            "SELECT {MEETING_COLUMNS}, n.body_markdown
             FROM meetings m JOIN notes n ON n.meeting_id = m.id
             WHERE ?1 IS NULL OR m.id IN (SELECT value FROM json_each(?1))
             ORDER BY m.created_at, m.id"
        ))?;
        let mut meetings: Vec<ExportMeeting> = stmt
            .query_map(params![filter], |row| {
                Ok(ExportMeeting {
                    meeting: meeting_from_row(row)?,
                    notes_markdown: row.get(5)?,
                    segments: Vec::new(),
                    summary: None,
                })
            })?
            .collect::<Result<_, _>>()?;
        let index: HashMap<i64, usize> = meetings
            .iter()
            .enumerate()
            .map(|(i, m)| (m.meeting.id, i))
            .collect();

        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, source, start_ms, end_ms, text, language
             FROM transcript_segments
             WHERE ?1 IS NULL OR meeting_id IN (SELECT value FROM json_each(?1))
             ORDER BY meeting_id, start_ms, id",
        )?;
        let segments = stmt.query_map(params![filter], |row| {
            let source: String = row.get(2)?;
            Ok(TranscriptSegment {
                id: row.get(0)?,
                meeting_id: row.get(1)?,
                source: if source == "them" {
                    Source::Them
                } else {
                    Source::Me
                },
                start_ms: row.get(3)?,
                end_ms: row.get(4)?,
                text: row.get(5)?,
                language: row.get(6)?,
            })
        })?;
        for segment in segments {
            let segment = segment?;
            if let Some(&i) = index.get(&segment.meeting_id) {
                meetings[i].segments.push(segment);
            }
        }

        let mut stmt = conn.prepare(
            "SELECT id, meeting_id, upto_ms, is_final, body_markdown, model_id, created_at FROM (
                 SELECT *, ROW_NUMBER() OVER (
                     PARTITION BY meeting_id ORDER BY created_at DESC, id DESC
                 ) AS newest
                 FROM summaries
                 WHERE ?1 IS NULL OR meeting_id IN (SELECT value FROM json_each(?1))
             ) WHERE newest = 1",
        )?;
        let summaries = stmt.query_map(params![filter], |row| {
            Ok(Summary {
                id: row.get(0)?,
                meeting_id: row.get(1)?,
                upto_ms: row.get(2)?,
                is_final: row.get(3)?,
                body_markdown: row.get(4)?,
                model_id: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;
        for summary in summaries {
            let summary = summary?;
            if let Some(&i) = index.get(&summary.meeting_id) {
                meetings[i].summary = Some(summary);
            }
        }
        Ok(meetings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewSegment, NewSummary};

    fn seeded() -> (Store, i64, i64) {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_meeting("Alpha").unwrap().id;
        let b = store.create_meeting("Beta").unwrap().id;
        store.update_notes(a, "- alpha notes").unwrap();
        for (meeting_id, start_ms, text) in
            [(a, 2000, "second"), (a, 1000, "first"), (b, 0, "beta line")]
        {
            store
                .insert_segment(NewSegment {
                    meeting_id,
                    source: Source::Me,
                    start_ms,
                    end_ms: start_ms + 500,
                    text: text.into(),
                    language: None,
                })
                .unwrap();
        }
        for body in ["older", "newest"] {
            store
                .insert_summary(NewSummary {
                    meeting_id: a,
                    upto_ms: 2500,
                    is_final: true,
                    body_markdown: body.into(),
                    model_id: "m".into(),
                })
                .unwrap();
        }
        (store, a, b)
    }

    #[test]
    fn loads_every_meeting_with_its_transcript_and_latest_summary() {
        let (store, a, b) = seeded();

        let meetings = store.export_meetings(None).unwrap();

        assert_eq!(meetings.len(), 2);
        let alpha = &meetings[0];
        assert_eq!(alpha.meeting.id, a);
        assert_eq!(alpha.notes_markdown, "- alpha notes");
        let texts: Vec<&str> = alpha.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, vec!["first", "second"]);
        assert_eq!(alpha.summary.as_ref().unwrap().body_markdown, "newest");
        assert_eq!(meetings[1].meeting.id, b);
        assert_eq!(meetings[1].summary, None);
    }

    #[test]
    fn loads_only_the_requested_meetings() {
        let (store, _, b) = seeded();

        let meetings = store.export_meetings(Some(&[b, 999])).unwrap();

        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].meeting.id, b);
        assert_eq!(meetings[0].segments.len(), 1);
    }

    #[test]
    fn loads_nothing_for_an_empty_selection() {
        let (store, _, _) = seeded();

        assert!(store.export_meetings(Some(&[])).unwrap().is_empty());
    }
}
