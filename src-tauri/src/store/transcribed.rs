//! Saving what Whistle transcribes: corrected with the vocabulary, and
//! without echo (see `transcribe::echo`).

use rusqlite::{Connection, params};

use super::transcript::{NewSegment, Source, TranscriptSegment, insert_segment};
use super::vocabulary::vocabulary;
use super::{Store, StoreError};
use crate::transcribe::echo::{WINDOW_MS, is_echo};

/// What saving a transcribed segment changed.
#[derive(Debug, Default, PartialEq)]
pub struct Saved {
    /// The saved segment, or `None` when it was an echo and was dropped.
    pub segment: Option<TranscriptSegment>,
    /// Earlier "Me" segments removed as echoes of this one.
    pub removed: Vec<i64>,
}

impl Store {
    /// Saves a segment as Whistle transcribed it, corrected with the saved
    /// vocabulary, and drops echo either way round: a "Me" segment that
    /// repeats nearby participants isn't saved, and a participant segment
    /// removes the "Me" segments that repeated it before it arrived. Each
    /// source closes segments on its own pauses, so either can come first.
    /// Everything happens under one lock and in one transaction.
    pub fn insert_transcribed_segment(&self, mut segment: NewSegment) -> Result<Saved, StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let corrected = vocabulary(&tx)?.apply(&segment.text).into_owned();
        let heard =
            (corrected != segment.text).then(|| std::mem::replace(&mut segment.text, corrected));

        let saved = match segment.source {
            Source::Me => {
                let theirs =
                    participant_lines(&tx, segment.meeting_id, segment.start_ms, segment.end_ms)?;
                if is_echo(&segment.text, &as_refs(&theirs)) {
                    log::debug!(
                        "dropped an echo for meeting {} at {} ms",
                        segment.meeting_id,
                        segment.start_ms
                    );
                    Saved::default()
                } else {
                    Saved {
                        segment: Some(insert_segment(&tx, segment, heard.as_deref())?),
                        removed: Vec::new(),
                    }
                }
            }
            Source::Them => {
                let saved = insert_segment(&tx, segment, heard.as_deref())?;
                let removed = remove_echoes_of(&tx, &saved)?;
                Saved {
                    segment: Some(saved),
                    removed,
                }
            }
        };
        tx.commit()?;
        Ok(saved)
    }
}

/// Removes the "Me" segments near `theirs` that repeat what participants said.
fn remove_echoes_of(conn: &Connection, theirs: &TranscriptSegment) -> Result<Vec<i64>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, start_ms, end_ms, text FROM transcript_segments
         WHERE meeting_id = ?1 AND source = 'me' AND start_ms <= ?3 AND end_ms >= ?2
         ORDER BY start_ms, id",
    )?;
    let mine = stmt
        .query_map(
            params![
                theirs.meeting_id,
                theirs.start_ms - WINDOW_MS,
                theirs.end_ms + WINDOW_MS
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let mut removed = Vec::new();
    for (id, start_ms, end_ms, text) in mine {
        let participants = participant_lines(conn, theirs.meeting_id, start_ms, end_ms)?;
        if is_echo(&text, &as_refs(&participants)) {
            conn.execute("DELETE FROM transcript_segments WHERE id = ?1", [id])?;
            log::debug!(
                "removed an echo for meeting {} at {start_ms} ms",
                theirs.meeting_id
            );
            removed.push(id);
        }
    }
    Ok(removed)
}

/// What participants said within the echo window of `start_ms` to `end_ms`.
fn participant_lines(
    conn: &Connection,
    meeting_id: i64,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT text FROM transcript_segments
         WHERE meeting_id = ?1 AND source = 'them' AND start_ms <= ?3 AND end_ms >= ?2
         ORDER BY start_ms, id",
    )?;
    let lines = stmt
        .query_map(
            params![meeting_id, start_ms - WINDOW_MS, end_ms + WINDOW_MS],
            |row| row.get(0),
        )?
        .collect::<Result<_, _>>()?;
    Ok(lines)
}

fn as_refs(lines: &[String]) -> Vec<&str> {
    lines.iter().map(String::as_str).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(store: &Store, source: Source, start_ms: i64, end_ms: i64, text: &str) -> Saved {
        store
            .insert_transcribed_segment(NewSegment {
                meeting_id: 1,
                source,
                start_ms,
                end_ms,
                text: text.into(),
                language: Some("en".into()),
            })
            .unwrap()
    }

    fn texts(store: &Store) -> Vec<(Source, String)> {
        store
            .list_segments(1)
            .unwrap()
            .into_iter()
            .map(|s| (s.source, s.text))
            .collect()
    }

    fn store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store.create_meeting("Sync").unwrap();
        store
    }

    #[test]
    fn drops_an_echo_that_arrives_after_its_participant_line() {
        let store = store();
        heard(
            &store,
            Source::Them,
            10_000,
            14_000,
            "I'll send the final strings tonight.",
        );

        let echo = heard(
            &store,
            Source::Me,
            10_300,
            14_200,
            "I'll send the final strings tonight",
        );

        assert_eq!(echo, Saved::default());
        assert_eq!(
            texts(&store),
            [(Source::Them, "I'll send the final strings tonight.".into())]
        );
    }

    #[test]
    fn removes_an_echo_that_arrived_before_its_participant_line() {
        let store = store();
        let echo = heard(
            &store,
            Source::Me,
            10_300,
            14_200,
            "I'll send the final strings tonight",
        );

        let theirs = heard(
            &store,
            Source::Them,
            10_000,
            14_000,
            "I'll send the final strings tonight.",
        );

        assert_eq!(theirs.removed, vec![echo.segment.unwrap().id]);
        assert_eq!(
            texts(&store),
            [(Source::Them, "I'll send the final strings tonight.".into())]
        );
    }

    #[test]
    fn keeps_what_you_said_around_the_same_time() {
        let store = store();
        heard(
            &store,
            Source::Them,
            10_000,
            14_000,
            "I'll send the final strings tonight.",
        );

        let mine = heard(
            &store,
            Source::Me,
            14_500,
            16_000,
            "Great, can you copy Sam on that?",
        );

        assert!(mine.segment.is_some());
        assert_eq!(texts(&store).len(), 2);
    }

    #[test]
    fn keeps_a_repeat_from_much_later() {
        let store = store();
        heard(
            &store,
            Source::Them,
            10_000,
            14_000,
            "I'll send the final strings tonight.",
        );

        // You quoting them a minute later isn't echo.
        let quoted = heard(
            &store,
            Source::Me,
            70_000,
            73_000,
            "I'll send the final strings tonight",
        );

        assert!(quoted.segment.is_some());
    }

    #[test]
    fn compares_the_vocabulary_corrected_text() {
        // Invented names only.
        let store = store();
        store
            .set_vocabulary(
                &crate::vocabulary::Vocabulary::new(vec![crate::vocabulary::Term {
                    term: "Bluefin Ledger".into(),
                    heard_as: vec!["blue fin ledger".into()],
                }])
                .unwrap(),
            )
            .unwrap();
        heard(&store, Source::Them, 0, 3000, "Send it to Bluefin Ledger.");

        let echo = heard(&store, Source::Me, 200, 3300, "send it to blue fin ledger");

        assert!(echo.segment.is_none());
    }
}
