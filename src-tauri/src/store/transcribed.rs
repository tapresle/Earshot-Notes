//! Saving what Whistle transcribes: corrected with the vocabulary, and
//! without echo (see `transcribe::echo`).

use rusqlite::{Connection, params};

use super::transcript::{NewSegment, Source, TranscriptSegment, insert_segment};
use super::vocabulary::vocabulary;
use super::{Store, StoreError};
use crate::transcribe::echo::{WINDOW_MS, is_echo, without_echo};
use crate::vocabulary::Vocabulary;

/// What saving a transcribed segment changed.
#[derive(Debug, Default, PartialEq)]
pub struct Saved {
    /// The saved segment, or `None` when it was an echo and was dropped.
    pub segment: Option<TranscriptSegment>,
    /// Earlier "Me" segments removed as echoes of this one.
    pub removed: Vec<i64>,
    /// Earlier "Me" segments that kept your words but lost echo of this one.
    pub updated: Vec<TranscriptSegment>,
}

impl Store {
    /// Saves a segment as Whistle transcribed it, corrected with the saved
    /// vocabulary, and drops echo either way round: a "Me" segment that
    /// repeats nearby participants isn't saved, and a participant segment
    /// removes the "Me" segments that repeated it before it arrived. A "Me"
    /// segment that mixes your words with echo keeps only your words. Each
    /// source closes segments on its own pauses, so either can come first.
    /// Everything happens under one lock and in one transaction.
    pub fn insert_transcribed_segment(&self, mut segment: NewSegment) -> Result<Saved, StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let vocabulary = vocabulary(&tx)?;

        let saved = match segment.source {
            Source::Me => {
                let theirs =
                    participant_lines(&tx, segment.meeting_id, segment.start_ms, segment.end_ms)?;
                let theirs = as_refs(&theirs);
                let whole = vocabulary.apply(&segment.text).into_owned();
                let heard = without_echo(&segment.text, &theirs).unwrap_or(segment.text);
                let (text, heard) = corrected(&vocabulary, heard);
                if is_echo(&whole, &theirs) || text.trim().is_empty() {
                    log::debug!(
                        "dropped an echo for meeting {} at {} ms",
                        segment.meeting_id,
                        segment.start_ms
                    );
                    Saved::default()
                } else {
                    segment.text = text;
                    Saved {
                        segment: Some(insert_segment(&tx, segment, heard.as_deref())?),
                        ..Saved::default()
                    }
                }
            }
            Source::Them => {
                let (text, heard) = corrected(&vocabulary, std::mem::take(&mut segment.text));
                segment.text = text;
                let saved = insert_segment(&tx, segment, heard.as_deref())?;
                let (removed, updated) = remove_echoes_of(&tx, &vocabulary, &saved)?;
                Saved {
                    segment: Some(saved),
                    removed,
                    updated,
                }
            }
        };
        tx.commit()?;
        Ok(saved)
    }
}

/// The text to save for what Whistle heard, and what it heard when the
/// vocabulary changed it.
fn corrected(vocabulary: &Vocabulary, heard: String) -> (String, Option<String>) {
    let text = vocabulary.apply(&heard).into_owned();
    let heard = (text != heard).then_some(heard);
    (text, heard)
}

/// Removes the "Me" segments near `theirs` that repeat what participants
/// said, and the echo inside those that mix it with your words. Returns the
/// removed IDs and the updated segments.
fn remove_echoes_of(
    conn: &Connection,
    vocabulary: &Vocabulary,
    theirs: &TranscriptSegment,
) -> Result<(Vec<i64>, Vec<TranscriptSegment>), StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, start_ms, end_ms, text, heard_text, language FROM transcript_segments
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
                let segment = TranscriptSegment {
                    id: row.get(0)?,
                    meeting_id: theirs.meeting_id,
                    source: Source::Me,
                    start_ms: row.get(1)?,
                    end_ms: row.get(2)?,
                    text: row.get(3)?,
                    language: row.get(5)?,
                };
                Ok((segment, row.get::<_, Option<String>>(4)?))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let (mut removed, mut updated) = (Vec::new(), Vec::new());
    for (mut segment, heard) in mine {
        let participants =
            participant_lines(conn, theirs.meeting_id, segment.start_ms, segment.end_ms)?;
        let participants = as_refs(&participants);
        let stripped = without_echo(heard.as_deref().unwrap_or(&segment.text), &participants);
        let (text, heard) = match stripped {
            Some(stripped) => corrected(vocabulary, stripped),
            None => (segment.text.clone(), heard),
        };
        if is_echo(&segment.text, &participants) || text.trim().is_empty() {
            conn.execute(
                "DELETE FROM transcript_segments WHERE id = ?1",
                [segment.id],
            )?;
            log::debug!(
                "removed an echo for meeting {} at {} ms",
                theirs.meeting_id,
                segment.start_ms
            );
            removed.push(segment.id);
        } else if text != segment.text {
            conn.execute(
                "UPDATE transcript_segments SET text = ?2, heard_text = ?3 WHERE id = ?1",
                params![segment.id, text, heard],
            )?;
            log::debug!(
                "removed echo inside a segment for meeting {} at {} ms",
                theirs.meeting_id,
                segment.start_ms
            );
            segment.text = text;
            updated.push(segment);
        }
    }
    Ok((removed, updated))
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

    const THEIRS: &str = "We moved the invoices over to the new ledger yesterday.";
    const MIXED: &str =
        "we moved the invoices over to the ledger yesterday, okay, can you send me the totals";

    #[test]
    fn keeps_your_words_from_a_segment_that_arrives_mixed_with_echo() {
        let store = store();
        heard(&store, Source::Them, 10_000, 14_000, THEIRS);

        let mine = heard(&store, Source::Me, 10_200, 17_000, MIXED);

        assert_eq!(
            mine.segment.map(|s| s.text).as_deref(),
            Some("okay, can you send me the totals")
        );
    }

    #[test]
    fn removes_echo_from_an_earlier_segment_when_its_source_arrives() {
        let store = store();
        let mine = heard(&store, Source::Me, 10_200, 17_000, MIXED)
            .segment
            .unwrap();

        let theirs = heard(&store, Source::Them, 10_000, 14_000, THEIRS);

        assert!(theirs.removed.is_empty());
        assert_eq!(theirs.updated.len(), 1);
        assert_eq!(theirs.updated[0].id, mine.id);
        assert_eq!(theirs.updated[0].text, "okay, can you send me the totals");
        assert_eq!(
            texts(&store),
            [
                (Source::Them, THEIRS.into()),
                (Source::Me, "okay, can you send me the totals".into())
            ]
        );
    }

    #[test]
    fn leaves_a_segment_without_echo_alone_when_a_participant_line_arrives() {
        let store = store();
        heard(
            &store,
            Source::Me,
            10_200,
            13_000,
            "Can you have that done by Thursday?",
        );

        let theirs = heard(&store, Source::Them, 10_000, 14_000, THEIRS);

        assert!(theirs.removed.is_empty() && theirs.updated.is_empty());
    }

    #[test]
    fn keeps_vocabulary_corrections_in_a_segment_that_lost_echo() {
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
        heard(
            &store,
            Source::Me,
            10_200,
            17_000,
            "we moved the invoices over to the ledger yesterday, okay, send the blue fin ledger totals",
        );

        heard(&store, Source::Them, 10_000, 14_000, THEIRS);

        let text = |store: &Store| {
            texts(store)
                .into_iter()
                .find(|(s, _)| *s == Source::Me)
                .unwrap()
                .1
        };
        assert_eq!(text(&store), "okay, send the Bluefin Ledger totals");
        // Reapplying the vocabulary starts from what Whistle heard, without the echo.
        store.set_vocabulary(&store.vocabulary().unwrap()).unwrap();
        assert_eq!(text(&store), "okay, send the Bluefin Ledger totals");
    }
}
