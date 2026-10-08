//! Recognizes echo: without headphones, the microphone also picks up the call
//! from the speakers, so participants' words come back a moment later as "Me".
//!
//! This compares text, not audio. A "Me" segment is an echo when most of its
//! words appear, in order, in what participants said around the same time.
//! A segment that mixes your words with echo keeps your words: runs that
//! repeat the participants are removed ([`without_echo`]). Echo that speech
//! recognition heard too differently to match stays; cancelling echo in the
//! audio would handle that (#88).

/// How far apart in time an echo and its source can be, beyond overlapping.
/// Each source closes its segments on its own pauses, so boundaries differ.
pub const WINDOW_MS: i64 = 2000;
/// Segments shorter than this are kept: "yeah" or "okay" from you often
/// matches a participant's by chance.
const MIN_WORDS: usize = 3;
/// The share of a "Me" segment's words that must appear, in order, in the
/// participants' words. Speech recognition hears an echo slightly
/// differently from the original, so this allows for some changed words.
const MIN_COVERAGE: f32 = 0.7;

/// Whether `me`, said by the note-taker, repeats `participants`: what the
/// participants said within [`WINDOW_MS`] of it, in timeline order.
pub fn is_echo(me: &str, participants: &[&str]) -> bool {
    let mine = words(me);
    if mine.len() < MIN_WORDS {
        return false;
    }
    let theirs: Vec<String> = participants.iter().flat_map(|text| words(text)).collect();
    if theirs.is_empty() {
        return false;
    }
    let shared = common_subsequence(&mine, &theirs);
    shared as f32 >= MIN_COVERAGE * mine.len() as f32
}

/// The shortest run of repeated words removed from inside a segment. Shorter
/// runs, such as "send the report", match by chance too often.
const MIN_RUN_WORDS: usize = 4;
/// How far apart, in words, two matched words can be and still belong to
/// one run. Allows for a word speech recognition heard differently.
const RUN_GAP: usize = 2;

/// `me` without the runs of words that repeat `participants`, for a segment
/// that mixes the note-taker's words with echo. Longer segments often do,
/// and [`is_echo`] keeps them whole. Returns `None` when nothing repeats.
pub fn without_echo(me: &str, participants: &[&str]) -> Option<String> {
    let tokens: Vec<&str> = me.split_whitespace().collect();
    // Each word with the token it came from.
    let mine: Vec<(usize, String)> = tokens
        .iter()
        .enumerate()
        .flat_map(|(t, token)| words(token).into_iter().map(move |w| (t, w)))
        .collect();
    let theirs: Vec<String> = participants.iter().flat_map(|text| words(text)).collect();
    let words_only: Vec<String> = mine.iter().map(|(_, w)| w.clone()).collect();

    let mut echoed = vec![false; mine.len()];
    let pairs = matched_pairs(&words_only, &theirs);
    let mut start = 0;
    for end in 1..=pairs.len() {
        let breaks = end == pairs.len()
            || pairs[end].0 - pairs[end - 1].0 > RUN_GAP
            || pairs[end].1 - pairs[end - 1].1 > RUN_GAP;
        if !breaks {
            continue;
        }
        let run = &pairs[start..end];
        let span = run[run.len() - 1].0 - run[0].0 + 1;
        if span >= MIN_RUN_WORDS && run.len() as f32 >= MIN_COVERAGE * span as f32 {
            echoed[run[0].0..=run[run.len() - 1].0].fill(true);
        }
        start = end;
    }
    if !echoed.contains(&true) {
        return None;
    }

    // Drop a token when every word in it was echoed.
    let mut keep = vec![true; tokens.len()];
    for (t, token) in tokens.iter().enumerate() {
        let mut its_words = mine.iter().zip(&echoed).filter(|((i, _), _)| *i == t);
        keep[t] = words(token).is_empty() || !its_words.all(|(_, echoed)| *echoed);
    }
    let kept: Vec<&str> = tokens
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(t, _)| *t)
        .collect();
    Some(
        kept.join(" ")
            .trim_matches(|c: char| c == ',' || c.is_whitespace())
            .to_owned(),
    )
}

/// The pairs of positions, `(mine, theirs)`, of a longest common subsequence.
fn matched_pairs(a: &[String], b: &[String]) -> Vec<(usize, usize)> {
    let mut table = vec![vec![0u16; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i][j] = if a[i] == b[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut pairs) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// Lowercase words, without punctuation.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The length of the longest common subsequence of two word lists. Segments
/// are at most 28 seconds, so both lists stay short.
fn common_subsequence(a: &[String], b: &[String]) -> usize {
    let mut previous = vec![0; b.len() + 1];
    let mut current = vec![0; b.len() + 1];
    for word in a {
        for (j, other) in b.iter().enumerate() {
            current[j + 1] = if word == other {
                previous[j] + 1
            } else {
                current[j].max(previous[j + 1])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_a_participant_line_heard_again() {
        assert!(is_echo(
            "I'll send the final strings tonight.",
            &["Agreed, I'll send the final strings tonight."]
        ));
    }

    #[test]
    fn allows_for_words_heard_differently() {
        // Speech recognition hears the quieter echo a little differently.
        assert!(is_echo(
            "we moved the invoices over to the ledger yesterday",
            &["We moved the quarterly invoices over to the new ledger yesterday."]
        ));
    }

    #[test]
    fn spans_participant_segments_split_at_different_pauses() {
        assert!(is_echo(
            "the grouped one tested better people found the model sizes faster",
            &[
                "The grouped one tested better.",
                "People found the model sizes faster.",
            ]
        ));
    }

    #[test]
    fn keeps_what_you_said_yourself() {
        assert!(!is_echo(
            "Can you have that done by Thursday?",
            &["I have one more pass on the second screen, then it's final."]
        ));
    }

    #[test]
    fn keeps_your_words_said_over_the_echo() {
        // Mostly your own words, with a little of theirs mixed in.
        assert!(!is_echo(
            "wait before that can we talk about the budget for the move",
            &["the budget is fine"]
        ));
    }

    #[test]
    fn keeps_short_replies_and_needs_participants() {
        assert!(!is_echo("Yeah, okay.", &["Yeah, okay."]));
        assert!(!is_echo("I'll send the final strings tonight.", &[]));
    }

    #[test]
    fn removes_echo_from_a_segment_that_mixes_it_with_your_words() {
        assert_eq!(
            without_echo(
                "we moved the invoices over to the ledger yesterday, okay, can you send me the totals",
                &["We moved the invoices over to the new ledger yesterday."]
            )
            .as_deref(),
            Some("okay, can you send me the totals")
        );
    }

    #[test]
    fn removes_echo_in_the_middle_and_allows_words_heard_differently() {
        assert_eq!(
            without_echo(
                "right, the grouped one tested batter with people, so let's ship it",
                &["The grouped one tested better with people."]
            )
            .as_deref(),
            Some("right, so let's ship it")
        );
    }

    #[test]
    fn leaves_short_shared_phrases_alone() {
        // Three words in common is chance, not echo.
        assert_eq!(
            without_echo(
                "I'll send the report after lunch",
                &["Did you send the report yet?"]
            ),
            None
        );
    }

    #[test]
    fn finds_nothing_without_a_repeat() {
        assert_eq!(
            without_echo(
                "Can you have that done by Thursday?",
                &["The budget is fine."]
            ),
            None
        );
        assert_eq!(
            without_echo("Can you have that done by Thursday?", &[]),
            None
        );
    }

    #[test]
    fn ignores_case_and_punctuation() {
        assert!(is_echo(
            "SOUNDS GOOD -- talk Friday!",
            &["Sounds good. Talk Friday."]
        ));
    }
}
