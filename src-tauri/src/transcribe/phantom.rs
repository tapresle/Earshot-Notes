//! Catches phrases the engines invent from clips with little real speech.
//!
//! Given a clip that is mostly silence, a cough, or call noise, Whistle and
//! Whisper tend to reply with a common phrase such as "Thank you." Neither
//! reports confidence, so a phrase on the list is dropped only when its clip
//! was weak. The same phrase from a clear clip is kept.

use crate::chunker::Speech;

/// Less speech than this is too little to trust a listed phrase.
const WEAK_SPEECH_MS: u64 = 300;
/// Speech that only just cleared its threshold. A quiet call participant
/// measures about 1.7, so this stays below that.
const WEAK_MARGIN: f32 = 1.5;
/// Phrases the engines invent, after `normalize`. A phrase repeated, such as
/// "you you you" or "bye bye", counts too.
const PHRASES: &[&str] = &[
    "thank you",
    "thank you very much",
    "thank you so much",
    "thanks",
    "thank you for watching",
    "thanks for watching",
    "thank you for listening",
    "thanks for listening",
    "please subscribe",
    "no",
    "you",
    "bye",
];
/// Credits Whisper learned from subtitled videos, followed by any name, such
/// as "Subtitles by the community".
const PREFIXES: &[&str] = &["subtitles by"];

/// Whether `text` is likely invented rather than heard.
pub fn is_phantom(text: &str, speech: Speech) -> bool {
    let weak = speech.ms < WEAK_SPEECH_MS || speech.margin < WEAK_MARGIN;
    weak && is_listed(&normalize(text))
}

fn is_listed(text: &str) -> bool {
    PREFIXES
        .iter()
        .any(|prefix| text == *prefix || text.starts_with(&format!("{prefix} ")))
        || PHRASES.iter().any(|phrase| is_repeated(text, phrase))
}

/// Whether `text` is `phrase` once or more, such as "you you you".
fn is_repeated(text: &str, phrase: &str) -> bool {
    let words: Vec<&str> = text.split(' ').collect();
    let phrase: Vec<&str> = phrase.split(' ').collect();
    words.len().is_multiple_of(phrase.len())
        && words.chunks(phrase.len()).all(|chunk| chunk == phrase)
}

/// Lowercases, turns punctuation into spaces, and collapses whitespace.
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRONG: Speech = Speech {
        ms: 600,
        margin: 8.0,
    };
    const SHORT: Speech = Speech {
        ms: 120,
        margin: 8.0,
    };
    const FAINT: Speech = Speech {
        ms: 600,
        margin: 1.2,
    };

    #[test]
    fn drops_a_listed_phrase_from_a_short_clip() {
        assert!(is_phantom("Thank you.", SHORT));
        assert!(is_phantom("No.", SHORT));
    }

    #[test]
    fn drops_a_listed_phrase_from_a_faint_clip() {
        assert!(is_phantom("Thank you.", FAINT));
        assert!(is_phantom("No.", FAINT));
    }

    #[test]
    fn keeps_a_listed_phrase_from_a_strong_clip() {
        assert!(!is_phantom("No.", STRONG));
        assert!(!is_phantom("Thank you.", STRONG));
    }

    #[test]
    fn keeps_a_quiet_participant() {
        let quiet = Speech {
            ms: 600,
            margin: 1.7,
        };

        assert!(!is_phantom("Thank you.", quiet));
    }

    #[test]
    fn keeps_text_that_isnt_listed() {
        assert!(!is_phantom("Ship it Friday.", SHORT));
        assert!(!is_phantom("No, let's wait.", FAINT));
        assert!(!is_phantom("Thank you, Dana.", SHORT));
    }

    #[test]
    fn drops_new_phrases_only_from_weak_clips() {
        for text in [
            "Thanks for listening.",
            "Thank you for listening!",
            "Please subscribe.",
            "Subtitles by the community.",
            "You you you.",
            "Thank you. Thank you.",
        ] {
            assert!(is_phantom(text, SHORT), "{text}");
            assert!(is_phantom(text, FAINT), "{text}");
            assert!(!is_phantom(text, STRONG), "{text}");
        }
    }

    #[test]
    fn keeps_text_that_only_starts_like_a_phrase() {
        assert!(!is_phantom("You you you and Dana.", SHORT));
        assert!(!is_phantom("Thank you thank.", SHORT));
        assert!(!is_phantom("Subtitles byline.", SHORT));
        assert!(!is_phantom("The subtitles by Friday.", SHORT));
    }

    #[test]
    fn ignores_case_punctuation_and_spacing() {
        assert!(is_phantom("THANK YOU!", SHORT));
        assert!(is_phantom(" no. ", SHORT));
        assert!(is_phantom("Thank  you...", SHORT));
        assert!(is_phantom("Bye-bye.", SHORT));
        assert!(!is_phantom("", SHORT));
    }
}
