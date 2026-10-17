//! Removes filler words such as "um" and "uh" from transcribed text.
//!
//! People really say them, but they clutter the transcript, the chat context,
//! and the summary input. Both engines write them out, and both spell a spoken
//! "uh" as "ah", so "ah" counts too. Only whole words go: "umbrella" and
//! "uh-huh" stay.

/// Fillers, lowercase, without punctuation. Elongated forms such as "ummm"
/// and "hmmm" match too (see `is_filler`).
const FILLERS: &[&str] = &[
    "um", "uh", "uhm", "ah", "er", "erm", "hm", "hmm", "mm", "mhm",
];
/// Punctuation that only joins a phrase to the next one.
const JOINERS: [char; 3] = [',', ';', ':'];

/// `text` without fillers, with spacing, punctuation, and capitalization fixed.
/// Returns an empty string when no words are left.
pub fn clean(text: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    // Whether the next word starts a sentence.
    let mut sentence_start = true;
    // Whether a filler that started a sentence was removed, so the next kept
    // word takes its capital.
    let mut capitalize = false;
    let mut ended_on_filler = false;
    for token in text.split_whitespace() {
        let (core, after) = split_trailing_punctuation(token);
        if is_filler(core) {
            ended_on_filler = true;
            if sentence_start {
                capitalize = true;
            } else if ends_sentence(after) {
                // "We ship, uh." keeps its full stop: "We ship."
                if let Some(last) = kept.last_mut() {
                    trim_joiners(last);
                    last.push_str(after);
                }
                sentence_start = true;
            }
            continue;
        }
        ended_on_filler = false;
        sentence_start = ends_sentence(after);
        kept.push(if std::mem::take(&mut capitalize) {
            capitalize_first(token)
        } else {
            token.to_string()
        });
    }
    // "We ship, um" doesn't end on a comma.
    if ended_on_filler && let Some(last) = kept.last_mut() {
        trim_joiners(last);
    }
    let cleaned = kept.join(" ");
    if cleaned.chars().any(char::is_alphanumeric) {
        cleaned
    } else {
        String::new()
    }
}

fn is_filler(word: &str) -> bool {
    let word = word.to_lowercase();
    if FILLERS.contains(&word.as_str()) {
        return true;
    }
    // "ummm", "uhhh", "hmmm", "mmm". Not "err", which is a word.
    let mut collapsed: Vec<char> = word.chars().collect();
    collapsed.dedup();
    let collapsed: String = collapsed.into_iter().collect();
    word.len() > 2 && ["um", "uh", "hm", "m"].contains(&collapsed.as_str())
}

/// Splits "so," into "so" and ",". A hyphen or apostrophe inside a word stays
/// with it, so "uh-huh" is never a filler.
fn split_trailing_punctuation(token: &str) -> (&str, &str) {
    let end = token.trim_end_matches(|c: char| !c.is_alphanumeric()).len();
    let (word, after) = token.split_at(end);
    (
        word.trim_start_matches(|c: char| !c.is_alphanumeric()),
        after,
    )
}

fn ends_sentence(punctuation: &str) -> bool {
    punctuation.contains(['.', '?', '!'])
}

fn trim_joiners(word: &mut String) {
    word.truncate(word.trim_end_matches(JOINERS).len());
}

fn capitalize_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_fillers_and_fixes_capitals() {
        assert_eq!(clean("Um, so, uh, we ship Friday."), "So, we ship Friday.");
        assert_eq!(clean("We um ship Friday."), "We ship Friday.");
        assert_eq!(
            clean("The webhook, uh, retries three times. Um, then it gives up."),
            "The webhook, retries three times. Then it gives up."
        );
    }

    #[test]
    fn keeps_the_sentence_end_of_a_removed_filler() {
        assert_eq!(clean("We ship Friday, uh."), "We ship Friday.");
        assert_eq!(clean("Does that work, um?"), "Does that work?");
        assert_eq!(clean("We ship Friday, uh. Okay?"), "We ship Friday. Okay?");
        assert_eq!(clean("We ship Friday. Um. Okay?"), "We ship Friday. Okay?");
    }

    #[test]
    fn doesnt_end_on_a_comma() {
        assert_eq!(clean("So we ship Friday, um"), "So we ship Friday");
        assert_eq!(clean("So we ship Friday; uh,"), "So we ship Friday");
    }

    #[test]
    fn matches_every_listed_filler_in_any_case() {
        for filler in [
            "um", "Uh", "UHM", "ah", "er", "erm", "hm", "hmm", "mm", "mhm", "Ummm", "uhhh",
            "hmmmm", "mmm",
        ] {
            assert_eq!(
                clean(&format!("{filler}, ship it.")),
                "Ship it.",
                "{filler}"
            );
        }
    }

    #[test]
    fn keeps_words_that_contain_a_filler() {
        for text in [
            "The umbrella is in the humming server room.",
            "Uhura reviewed the erratum.",
            "Ahead of the err message, mmap it.",
            "Uh-huh, that works.",
            "Sign it, M. Ortiz.",
        ] {
            assert_eq!(clean(text), text);
        }
    }

    #[test]
    fn keeps_words_with_meaning() {
        let text = "So, like, you know, we ship Friday.";

        assert_eq!(clean(text), text);
    }

    #[test]
    fn returns_nothing_when_only_fillers_are_left() {
        assert_eq!(clean("Um. Uh."), "");
        assert_eq!(clean("Um, ah, hmm."), "");
        assert_eq!(clean("..."), "");
        assert_eq!(clean(""), "");
    }

    #[test]
    fn tidies_spacing() {
        assert_eq!(clean("  We  ship,   um,  Friday. "), "We ship, Friday.");
    }
}
