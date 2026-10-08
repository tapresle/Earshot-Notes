//! Recognizes echo: without headphones, the microphone also picks up the call
//! from the speakers, so participants' words come back a moment later as "Me".
//!
//! This compares text, not audio. A "Me" segment is an echo when most of its
//! words appear, in order, in what participants said around the same time.
//! When you talk over the echo, the segment mixes your words with theirs and
//! is kept whole; cancelling echo in the audio would handle that case.

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
    fn ignores_case_and_punctuation() {
        assert!(is_echo(
            "SOUNDS GOOD -- talk Friday!",
            &["Sounds good. Talk Friday."]
        ));
    }
}
