//! The saved vocabulary, and correcting transcripts with it.

use rusqlite::{Connection, OptionalExtension, params};

use super::settings::VOCABULARY;
use super::{Store, StoreError};
use crate::vocabulary::{Term, Vocabulary};

impl Store {
    /// The saved vocabulary, or an empty one when none is saved.
    pub fn vocabulary(&self) -> Result<Vocabulary, StoreError> {
        vocabulary(&self.conn())
    }

    /// Saves the vocabulary and corrects every stored transcript with it, in
    /// one transaction. Returns how many segments' text changed.
    pub fn set_vocabulary(&self, vocabulary: &Vocabulary) -> Result<usize, StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let json = serde_json::to_string(vocabulary.terms()).map_err(|e| {
            StoreError::UnreadableSetting {
                key: VOCABULARY,
                reason: e.to_string(),
            }
        })?;
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![VOCABULARY, json],
        )?;
        let changed = reapply(&tx, vocabulary)?;
        tx.commit()?;
        Ok(changed)
    }
}

pub(super) fn vocabulary(conn: &Connection) -> Result<Vocabulary, StoreError> {
    let saved: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [VOCABULARY],
            |row| row.get(0),
        )
        .optional()?;
    let Some(json) = saved else {
        return Ok(Vocabulary::default());
    };
    let unreadable = |reason: String| StoreError::UnreadableSetting {
        key: VOCABULARY,
        reason,
    };
    let terms: Vec<Term> = serde_json::from_str(&json).map_err(|e| unreadable(e.to_string()))?;
    Vocabulary::new(terms).map_err(|e| unreadable(e.to_string()))
}

/// Corrects every segment from what Whistle heard, writing only rows that
/// change. Text is updated only when it changes, so the search index is
/// rebuilt for those rows alone.
fn reapply(conn: &Connection, vocabulary: &Vocabulary) -> Result<usize, StoreError> {
    let mut text_changes = Vec::new();
    let mut heard_changes = Vec::new();
    {
        let mut stmt = conn.prepare("SELECT id, text, heard_text FROM transcript_segments")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let text: String = row.get(1)?;
            let heard: Option<String> = row.get(2)?;
            let original = heard.as_deref().unwrap_or(&text);
            let corrected = vocabulary.apply(original);
            let new_heard = (corrected != original).then(|| original.to_owned());
            if corrected != text {
                text_changes.push((id, corrected.into_owned(), new_heard));
            } else if new_heard != heard {
                heard_changes.push((id, new_heard));
            }
        }
    }
    let mut set_text =
        conn.prepare("UPDATE transcript_segments SET text = ?2, heard_text = ?3 WHERE id = ?1")?;
    for (id, text, heard) in &text_changes {
        set_text.execute(params![id, text, heard])?;
    }
    let mut set_heard =
        conn.prepare("UPDATE transcript_segments SET heard_text = ?2 WHERE id = ?1")?;
    for (id, heard) in &heard_changes {
        set_heard.execute(params![id, heard])?;
    }
    Ok(text_changes.len())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::store::{NewSegment, Source, TranscriptSegment};

    // Invented names only.
    fn ledger(heard_as: &[&str]) -> Vocabulary {
        Vocabulary::new(vec![Term {
            term: "Bluefin Ledger".into(),
            heard_as: heard_as.iter().map(|h| (*h).into()).collect(),
        }])
        .unwrap()
    }

    fn heard(store: &Store, meeting_id: i64, start_ms: i64, text: &str) -> TranscriptSegment {
        store
            .insert_transcribed_segment(NewSegment {
                meeting_id,
                source: Source::Them,
                start_ms,
                end_ms: start_ms + 1000,
                text: text.into(),
                language: Some("en".into()),
            })
            .unwrap()
            .segment
            .unwrap()
    }

    fn heard_text(store: &Store, id: i64) -> Option<String> {
        store
            .conn()
            .query_row(
                "SELECT heard_text FROM transcript_segments WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn texts(store: &Store, meeting_id: i64) -> Vec<String> {
        store
            .list_segments(meeting_id)
            .unwrap()
            .into_iter()
            .map(|s| s.text)
            .collect()
    }

    fn found(store: &Store, query: &str) -> Vec<i64> {
        let page = store.search_meetings(query, None, 10).unwrap();
        page.meetings.into_iter().map(|m| m.id).collect()
    }

    #[test]
    fn starts_empty_and_saves_terms() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.vocabulary().unwrap().is_empty());

        store.set_vocabulary(&ledger(&["blue fin ledger"])).unwrap();

        assert_eq!(store.vocabulary().unwrap(), ledger(&["blue fin ledger"]));
    }

    #[test]
    fn corrects_a_new_segment_and_keeps_what_was_heard() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        store.set_vocabulary(&ledger(&["blue fin ledger"])).unwrap();

        let corrected = heard(&store, meeting.id, 0, "Send it to blue fin ledger.");
        let untouched = heard(&store, meeting.id, 1000, "Nothing to fix.");

        assert_eq!(corrected.text, "Send it to Bluefin Ledger.");
        assert_eq!(
            heard_text(&store, corrected.id).as_deref(),
            Some("Send it to blue fin ledger.")
        );
        assert_eq!(heard_text(&store, untouched.id), None);
    }

    #[test]
    fn corrects_past_transcripts_and_undoes_a_removed_mishearing() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        let segment = heard(&store, meeting.id, 0, "Send it to blue fin ledger.");
        heard(&store, meeting.id, 1000, "Nothing to fix.");

        let changed = store.set_vocabulary(&ledger(&["blue fin ledger"])).unwrap();

        assert_eq!(changed, 1);
        assert_eq!(
            texts(&store, meeting.id),
            ["Send it to Bluefin Ledger.", "Nothing to fix."]
        );
        assert_eq!(found(&store, "Bluefin"), vec![meeting.id]);

        let changed = store.set_vocabulary(&ledger(&[])).unwrap();

        assert_eq!(changed, 1);
        assert_eq!(
            texts(&store, meeting.id),
            ["Send it to blue fin ledger.", "Nothing to fix."]
        );
        assert_eq!(heard_text(&store, segment.id), None);
        assert!(found(&store, "Bluefin").is_empty());
    }

    #[test]
    fn leaves_unchanged_segments_alone_when_reapplying() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        store.set_vocabulary(&ledger(&["blue fin ledger"])).unwrap();
        heard(&store, meeting.id, 0, "Send it to blue fin ledger.");

        assert_eq!(
            store.set_vocabulary(&ledger(&["blue fin ledger"])).unwrap(),
            0
        );
    }

    #[test]
    fn refuses_a_vocabulary_saved_in_a_shape_it_cant_read() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(VOCABULARY, Some("not json")).unwrap();

        assert!(matches!(
            store.vocabulary(),
            Err(StoreError::UnreadableSetting { .. })
        ));
    }

    /// About 1,000 hours of meetings: 360 ten-second segments an hour.
    #[test]
    fn reapplies_to_a_large_archive_quickly() {
        const SEGMENTS: i64 = 360_000;
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Archive").unwrap();
        // Generated in SQL: inserting row by row from a debug build takes longer than the test.
        store
            .conn()
            .execute(
                "WITH RECURSIVE n(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM n WHERE i + 1 < ?2)
                 INSERT INTO transcript_segments (meeting_id, source, start_ms, end_ms, text)
                 SELECT ?1, 'them', i * 10000, i * 10000 + 10000,
                        CASE WHEN i % 100 = 0
                             THEN 'We moved the quarterly invoices over to blue fin ledger before the review.'
                             ELSE 'We went through the open items and agreed on owners for each of them today.'
                        END
                 FROM n",
                params![meeting.id, SEGMENTS],
            )
            .unwrap();

        let started = Instant::now();
        let changed = store.set_vocabulary(&ledger(&["blue fin ledger"])).unwrap();
        let elapsed = started.elapsed();

        assert_eq!(changed, (SEGMENTS / 100) as usize);
        // A debug build; a release build is several times faster.
        assert!(
            elapsed < Duration::from_secs(10),
            "reapplying took {elapsed:?}"
        );
    }
}
