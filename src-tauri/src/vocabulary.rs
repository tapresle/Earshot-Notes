//! Names your meetings use, and what Whistle tends to hear instead. The terms
//! bias transcription and guide summaries; the mishearings are corrected in
//! transcripts.

use std::borrow::Cow;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Limits that keep the keyword list and the summary prompt small.
pub const MAX_TERMS: usize = 200;
pub const MAX_HEARD_AS: usize = 20;
pub const MAX_LENGTH: usize = 100;

/// One name, spelled correctly, and what Whistle tends to hear instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    pub term: String,
    #[serde(default)]
    pub heard_as: Vec<String>,
}

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum VocabularyError {
    #[error("the vocabulary can have at most {MAX_TERMS} terms")]
    TooManyTerms,
    #[error("a term can't be empty")]
    EmptyTerm,
    #[error("\"{0}\" is longer than {MAX_LENGTH} characters")]
    TooLong(String),
    #[error("\"{0}\" can't contain a line break")]
    LineBreak(String),
    #[error("\"{0}\" is listed more than once")]
    DuplicateTerm(String),
    #[error("\"{0}\" can have at most {MAX_HEARD_AS} mishearings")]
    TooManyHeardAs(String),
    #[error("the mishearing \"{0}\" can only contain letters, numbers, and spaces")]
    UnmatchableHeardAs(String),
    #[error("\"{heard}\" is listed under both \"{first}\" and \"{second}\"")]
    HeardTwice {
        heard: String,
        first: String,
        second: String,
    },
}

/// A checked vocabulary, ready to apply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Vocabulary {
    terms: Vec<Term>,
    /// Mishearings as lowercase words, longest first so the longest match wins.
    rules: Vec<(Vec<String>, String)>,
}

impl Vocabulary {
    /// Trims every entry, drops empty mishearings, and checks the limits.
    pub fn new(terms: Vec<Term>) -> Result<Self, VocabularyError> {
        if terms.len() > MAX_TERMS {
            return Err(VocabularyError::TooManyTerms);
        }
        let mut cleaned = Vec::with_capacity(terms.len());
        let mut seen_terms = HashMap::new();
        let mut owners: HashMap<Vec<String>, String> = HashMap::new();
        let mut rules = Vec::new();
        for term in terms {
            let name = checked(&term.term)?;
            if name.is_empty() {
                return Err(VocabularyError::EmptyTerm);
            }
            if seen_terms.insert(name.to_lowercase(), ()).is_some() {
                return Err(VocabularyError::DuplicateTerm(name));
            }
            let mut heard_as = Vec::new();
            for heard in &term.heard_as {
                let heard = checked(heard)?;
                if heard.is_empty() {
                    continue;
                }
                let words = words_of(&heard)
                    .ok_or_else(|| VocabularyError::UnmatchableHeardAs(heard.clone()))?;
                match owners.get(&words) {
                    Some(owner) if *owner == name => continue,
                    Some(owner) => {
                        return Err(VocabularyError::HeardTwice {
                            heard,
                            first: owner.clone(),
                            second: name,
                        });
                    }
                    None => {}
                }
                owners.insert(words.clone(), name.clone());
                rules.push((words, name.clone()));
                heard_as.push(heard);
            }
            if heard_as.len() > MAX_HEARD_AS {
                return Err(VocabularyError::TooManyHeardAs(name));
            }
            cleaned.push(Term {
                term: name,
                heard_as,
            });
        }
        rules.sort_by_key(|(words, _)| std::cmp::Reverse(words.len()));
        Ok(Self {
            terms: cleaned,
            rules,
        })
    }

    pub fn terms(&self) -> &[Term] {
        &self.terms
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The correct spellings, one per line, as needle's keyword biasing expects.
    pub fn keywords(&self) -> Option<String> {
        (!self.terms.is_empty()).then(|| {
            self.terms
                .iter()
                .map(|t| t.term.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    /// Replaces each listed mishearing in `text` with its term. Matches whole
    /// words in any case, with any whitespace between words.
    pub fn apply<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.rules.is_empty() {
            return Cow::Borrowed(text);
        }
        let words = word_spans(text);
        let mut out = String::new();
        let mut copied = 0;
        let mut i = 0;
        while i < words.len() {
            match self.rule_at(text, &words, i) {
                Some((len, term)) => {
                    out.push_str(&text[copied..words[i].0]);
                    out.push_str(term);
                    copied = words[i + len - 1].1;
                    i += len;
                }
                None => i += 1,
            }
        }
        if copied == 0 && out.is_empty() {
            return Cow::Borrowed(text);
        }
        out.push_str(&text[copied..]);
        Cow::Owned(out)
    }

    /// The longest rule matching the words starting at `start`, as its word count and term.
    fn rule_at(&self, text: &str, words: &[(usize, usize)], start: usize) -> Option<(usize, &str)> {
        self.rules.iter().find_map(|(rule, term)| {
            let candidate = words.get(start..start + rule.len())?;
            let same_words = candidate
                .iter()
                .zip(rule)
                .all(|(&(from, to), word)| text[from..to].to_lowercase() == *word);
            let spaced = candidate
                .windows(2)
                .all(|pair| text[pair[0].1..pair[1].0].chars().all(char::is_whitespace));
            (same_words && spaced).then_some((rule.len(), term.as_str()))
        })
    }
}

/// Trims an entry and checks its length and line breaks.
fn checked(entry: &str) -> Result<String, VocabularyError> {
    let entry = entry.trim();
    if entry.contains(['\n', '\r']) {
        return Err(VocabularyError::LineBreak(entry.to_owned()));
    }
    if entry.chars().count() > MAX_LENGTH {
        return Err(VocabularyError::TooLong(entry.to_owned()));
    }
    Ok(entry.to_owned())
}

/// A mishearing's lowercase words, or `None` when it has anything besides
/// letters, numbers, and spaces, which whole-word matching couldn't find.
fn words_of(heard: &str) -> Option<Vec<String>> {
    if !heard
        .chars()
        .all(|c| c.is_alphanumeric() || c.is_whitespace())
    {
        return None;
    }
    Some(heard.split_whitespace().map(str::to_lowercase).collect())
}

/// The byte ranges of each run of letters and numbers in `text`.
fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (index, c) in text.char_indices() {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                spans.push((from, index));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        spans.push((from, text.len()));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    // Invented names only.
    fn term(name: &str, heard_as: &[&str]) -> Term {
        Term {
            term: name.into(),
            heard_as: heard_as.iter().map(|h| (*h).into()).collect(),
        }
    }

    fn ledger() -> Vocabulary {
        Vocabulary::new(vec![
            term("Bluefin Ledger", &["blue fin ledger", "bluefinn ledger"]),
            term("Corvina", &["corvena"]),
        ])
        .unwrap()
    }

    #[test]
    fn corrects_mishearings_in_any_case() {
        assert_eq!(
            ledger().apply("We moved the invoices to Blue Fin Ledger yesterday."),
            "We moved the invoices to Bluefin Ledger yesterday."
        );
        assert_eq!(
            ledger().apply("BLUEFINN LEDGER and corvena."),
            "Bluefin Ledger and Corvina."
        );
    }

    #[test]
    fn matches_whole_words_with_any_spacing() {
        let vocabulary = ledger();

        assert_eq!(vocabulary.apply("blue  fin\tledger's"), "Bluefin Ledger's");
        assert_eq!(vocabulary.apply("corvenas"), "corvenas");
        assert_eq!(vocabulary.apply("blue-fin ledger"), "blue-fin ledger");
        assert_eq!(vocabulary.apply("the blue fin"), "the blue fin");
    }

    #[test]
    fn leaves_text_without_mishearings_untouched() {
        let text = "Nothing to correct here.";

        assert!(matches!(ledger().apply(text), Cow::Borrowed(_)));
        assert!(matches!(
            Vocabulary::default().apply(text),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn prefers_the_longest_mishearing() {
        let vocabulary = Vocabulary::new(vec![
            term("Fin", &["fin"]),
            term("Bluefin Ledger", &["blue fin ledger"]),
        ])
        .unwrap();

        assert_eq!(vocabulary.apply("blue fin ledger"), "Bluefin Ledger");
        assert_eq!(vocabulary.apply("a fin"), "a Fin");
    }

    #[test]
    fn lists_terms_as_keywords() {
        assert_eq!(
            ledger().keywords().as_deref(),
            Some("Bluefin Ledger\nCorvina")
        );
        assert_eq!(Vocabulary::default().keywords(), None);
    }

    #[test]
    fn trims_entries_and_drops_empty_mishearings() {
        let vocabulary = Vocabulary::new(vec![term("  Corvina ", &[" corvena ", "  "])]).unwrap();

        assert_eq!(vocabulary.terms(), &[term("Corvina", &["corvena"])]);
    }

    #[test]
    fn refuses_entries_it_cant_use() {
        let long = "x".repeat(MAX_LENGTH + 1);
        let cases = [
            (vec![term(" ", &[])], VocabularyError::EmptyTerm),
            (
                vec![term(&long, &[])],
                VocabularyError::TooLong(long.clone()),
            ),
            (
                vec![term("A\nB", &[])],
                VocabularyError::LineBreak("A\nB".into()),
            ),
            (
                vec![term("Corvina", &[]), term("corvina", &[])],
                VocabularyError::DuplicateTerm("corvina".into()),
            ),
            (
                vec![term("Corvina", &["cor-vena"])],
                VocabularyError::UnmatchableHeardAs("cor-vena".into()),
            ),
            (
                vec![term("Corvina", &["corvena"]), term("Corvino", &["Corvena"])],
                VocabularyError::HeardTwice {
                    heard: "Corvena".into(),
                    first: "Corvina".into(),
                    second: "Corvino".into(),
                },
            ),
        ];
        for (terms, expected) in cases {
            assert_eq!(Vocabulary::new(terms), Err(expected));
        }
        let too_many = (0..=MAX_TERMS)
            .map(|i| term(&format!("T{i}"), &[]))
            .collect();
        assert_eq!(
            Vocabulary::new(too_many),
            Err(VocabularyError::TooManyTerms)
        );
        let heard: Vec<String> = (0..=MAX_HEARD_AS).map(|i| format!("h{i}")).collect();
        let heard: Vec<&str> = heard.iter().map(String::as_str).collect();
        assert_eq!(
            Vocabulary::new(vec![term("Corvina", &heard)]),
            Err(VocabularyError::TooManyHeardAs("Corvina".into()))
        );
    }
}
