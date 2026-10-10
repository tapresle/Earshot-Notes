//! Picks the parts of a meeting that match a question, for a meeting too long
//! to send whole. One meeting is small enough to score in memory each time.

use std::collections::{HashMap, HashSet};

/// Words too common to tell sources apart.
const STOP_WORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be",
    "been", "before", "but", "by", "can", "could", "did", "do", "does", "for", "from", "get",
    "got", "had", "has", "have", "he", "her", "him", "his", "how", "i", "if", "in", "into", "is",
    "it", "its", "just", "let", "me", "meet", "meeting", "my", "of", "on", "or", "our", "out",
    "said", "say", "says", "she", "should", "so", "some", "than", "that", "the", "their", "them",
    "then", "there", "these", "they", "this", "those", "to", "up", "us", "was", "we", "were",
    "what", "when", "where", "which", "who", "whom", "why", "will", "with", "would", "you", "your",
];

/// The words of `text` that carry meaning: two or more characters, lowercase, with common suffixes
/// removed so "decided" matches "decide" and "tickets" matches "ticket".
pub fn terms(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() > 1)
        .map(str::to_lowercase)
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .map(|w| stem(&w))
        .collect()
}

fn stem(word: &str) -> String {
    let n = word.chars().count();
    for (suffix, min, replacement) in [
        ("ies", 5, "y"),
        ("ing", 6, ""),
        ("ed", 5, ""),
        ("es", 5, ""),
        ("s", 4, ""),
        ("e", 5, ""),
    ] {
        if n >= min && word.ends_with(suffix) && !(suffix == "s" && word.ends_with("ss")) {
            return format!("{}{replacement}", &word[..word.len() - suffix.len()]);
        }
    }
    word.to_owned()
}

/// What to send: indices into the passages and the segments, each in their
/// original order.
#[derive(Debug, Default, PartialEq)]
pub struct Selection {
    pub passages: Vec<usize>,
    pub segments: Vec<usize>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.passages.is_empty() && self.segments.is_empty()
    }
}

/// A passage or a segment, by its index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Doc {
    Passage(usize),
    Segment(usize),
}

/// Chooses the passages and segments that best match `question`, within
/// `budget` tokens as `cost` counts them. A matching segment brings the
/// segments on either side for context. Empty when nothing matches.
pub fn select(
    question: &str,
    passages: &[&str],
    segments: &[&str],
    budget: usize,
    cost: &mut dyn FnMut(Doc) -> usize,
) -> Selection {
    let wanted = terms(question);
    let docs: Vec<(Doc, HashSet<String>)> = passages
        .iter()
        .enumerate()
        .map(|(i, text)| (Doc::Passage(i), terms(text)))
        .chain(
            segments
                .iter()
                .enumerate()
                .map(|(i, text)| (Doc::Segment(i), terms(text))),
        )
        .collect();
    // Rarer words count for more.
    let mut frequency: HashMap<&str, usize> = HashMap::new();
    for (_, words) in &docs {
        for word in words.iter().filter(|w| wanted.contains(*w)) {
            *frequency.entry(word).or_default() += 1;
        }
    }
    let weight = |word: &str| (1.0 + docs.len() as f64 / frequency[word] as f64).ln();
    let mut scored: Vec<(f64, usize)> = docs
        .iter()
        .enumerate()
        .filter_map(|(i, (_, words))| {
            let score: f64 = words
                .iter()
                .filter(|w| wanted.contains(*w))
                .map(|w| weight(w))
                .sum();
            (score > 0.0).then_some((score, i))
        })
        .collect();
    // Highest score first; earlier in the meeting breaks ties.
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut chosen_passages = HashSet::new();
    let mut chosen_segments = HashSet::new();
    let mut spent = 0;
    for (_, i) in scored {
        let wanted_docs: Vec<Doc> = match docs[i].0 {
            Doc::Passage(p) if !chosen_passages.contains(&p) => vec![Doc::Passage(p)],
            Doc::Segment(s) => (s.saturating_sub(1)..=(s + 1).min(segments.len() - 1))
                .filter(|n| !chosen_segments.contains(n))
                .map(Doc::Segment)
                .collect(),
            Doc::Passage(_) => continue,
        };
        let price: usize = wanted_docs.iter().map(|d| cost(*d)).sum();
        if spent + price > budget {
            continue;
        }
        spent += price;
        for doc in wanted_docs {
            match doc {
                Doc::Passage(p) => chosen_passages.insert(p),
                Doc::Segment(s) => chosen_segments.insert(s),
            };
        }
    }
    let mut selection = Selection {
        passages: chosen_passages.into_iter().collect(),
        segments: chosen_segments.into_iter().collect(),
    };
    selection.passages.sort_unstable();
    selection.segments.sort_unstable();
    selection
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(cost: Doc, passages: &[&str], segments: &[&str]) -> usize {
        match cost {
            Doc::Passage(i) => passages[i].split_whitespace().count(),
            Doc::Segment(i) => segments[i].split_whitespace().count(),
        }
    }

    fn pick(question: &str, passages: &[&str], segments: &[&str], budget: usize) -> Selection {
        select(question, passages, segments, budget, &mut |d| {
            words(d, passages, segments)
        })
    }

    #[test]
    fn keeps_meaningful_words_and_matches_word_forms() {
        let found = terms("What did we decide about the tickets? Who's testing ENG-4012?");

        assert_eq!(
            found,
            ["decid", "ticket", "test", "eng", "4012"]
                .into_iter()
                .map(String::from)
                .collect::<HashSet<_>>()
        );
        assert!(terms("decided").is_subset(&terms("We decide Friday")));
        assert!(terms("classes").contains("class"));
        assert!(terms("class").contains("class"));
    }

    #[test]
    fn picks_matching_segments_with_their_neighbors() {
        let segments = [
            "Morning everyone.",
            "Quick update on hiring.",
            "The launch date moves to May 4.",
            "Because QA needs another week.",
            "Anything else?",
            "Lunch is at noon.",
        ];

        let chosen = pick("When is the launch?", &[], &segments, 100);

        assert_eq!(chosen.segments, [1, 2, 3]);
        assert!(chosen.passages.is_empty());
    }

    #[test]
    fn picks_matching_note_passages_alone() {
        let passages = ["- Launch: May 4", "- Hiring: two roles open", "- Budget"];

        let chosen = pick("hiring plans", &passages, &["Hello there."], 100);

        assert_eq!(chosen.passages, [1]);
        assert!(chosen.segments.is_empty());
    }

    #[test]
    fn prefers_rare_words_when_the_budget_is_tight() {
        let segments = [
            "The deploy went fine.",
            "Filler about nothing.",
            "Filler again here.",
            "Another deploy note.",
            "Filler once more.",
            "Filler to separate.",
            "Deploy failed with error 4012.",
            "Filler at the end.",
        ];

        // Room for one segment and its neighbors only.
        let chosen = pick("deploy error 4012", &[], &segments, 12);

        assert_eq!(chosen.segments, [5, 6, 7]);
    }

    #[test]
    fn selects_nothing_when_nothing_matches() {
        let chosen = pick("budget", &["- Launch"], &["Hello.", "Bye."], 100);

        assert!(chosen.is_empty());
    }

    #[test]
    fn selects_nothing_for_a_question_of_stop_words() {
        let chosen = pick(
            "What did they say?",
            &[],
            &["They said what they did."],
            100,
        );

        assert!(chosen.is_empty());
    }
}
