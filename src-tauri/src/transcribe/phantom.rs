//! Catches phrases Whistle invents from clips with little real speech.
//!
//! Given a clip that is mostly silence, a cough, or call noise, Whistle tends to
//! reply with a common phrase such as "Thank you." The engine reports no
//! confidence, so a phrase on the list is dropped only when its clip was weak.
//! The same phrase from a clear clip is kept.

use crate::chunker::Speech;

/// Less speech than this is too little to trust a listed phrase.
const WEAK_SPEECH_MS: u64 = 300;
/// Speech that only just cleared its threshold. A quiet call participant
/// measures about 1.7, so this stays below that.
const WEAK_MARGIN: f32 = 1.5;
/// Phrases Whistle invents, after `normalize`.
const PHRASES: &[&str] = &[
    "thank you",
    "thank you very much",
    "thank you so much",
    "thanks",
    "thank you for watching",
    "thanks for watching",
    "no",
    "you",
    "bye",
    "bye bye",
];

/// Whether `text` is likely invented rather than heard.
pub fn is_phantom(text: &str, speech: Speech) -> bool {
    let weak = speech.ms < WEAK_SPEECH_MS || speech.margin < WEAK_MARGIN;
    weak && PHRASES.contains(&normalize(text).as_str())
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
    fn ignores_case_punctuation_and_spacing() {
        assert!(is_phantom("THANK YOU!", SHORT));
        assert!(is_phantom(" no. ", SHORT));
        assert!(is_phantom("Thank  you...", SHORT));
        assert!(is_phantom("Bye-bye.", SHORT));
        assert!(!is_phantom("", SHORT));
    }
}
