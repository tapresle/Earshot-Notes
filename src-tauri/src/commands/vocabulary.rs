//! The vocabulary: names to spell correctly, and what Whistle hears instead.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use super::{CommandError, CommandResult, ErrorKind, blocking};
use crate::store::Store;
use crate::vocabulary::{Term, Vocabulary};

/// The saved vocabulary, and how many transcript lines saving it corrected.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedVocabulary {
    terms: Vec<Term>,
    updated: usize,
}

#[tauri::command]
pub async fn vocabulary(store: State<'_, Arc<Store>>) -> CommandResult<Vec<Term>> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.vocabulary()?.terms().to_vec())).await
}

/// Saves the vocabulary and corrects every stored transcript with it. Returns
/// the terms as saved, trimmed, so the UI shows exactly what's stored.
#[tauri::command]
pub async fn set_vocabulary(
    store: State<'_, Arc<Store>>,
    terms: Vec<Term>,
) -> CommandResult<SavedVocabulary> {
    let store = Arc::clone(&store);
    blocking(move || save(&store, terms)).await
}

fn save(store: &Store, terms: Vec<Term>) -> CommandResult<SavedVocabulary> {
    let vocabulary = Vocabulary::new(terms).map_err(|error| CommandError {
        kind: ErrorKind::Invalid,
        message: error.to_string(),
    })?;
    let updated = store.set_vocabulary(&vocabulary)?;
    log::info!(
        "saved {} vocabulary terms; corrected {updated} transcript segments",
        vocabulary.terms().len()
    );
    Ok(SavedVocabulary {
        terms: vocabulary.terms().to_vec(),
        updated,
    })
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

    #[test]
    fn saves_trimmed_terms_and_reports_corrections() {
        let store = Store::open_in_memory().unwrap();

        let saved = save(&store, vec![term(" Corvina ", &["corvena", " "])]).unwrap();

        assert_eq!(
            saved,
            SavedVocabulary {
                terms: vec![term("Corvina", &["corvena"])],
                updated: 0,
            }
        );
        assert_eq!(store.vocabulary().unwrap().terms(), saved.terms.as_slice());
    }

    #[test]
    fn refuses_an_invalid_vocabulary_and_keeps_the_saved_one() {
        let store = Store::open_in_memory().unwrap();
        save(&store, vec![term("Corvina", &[])]).unwrap();

        let error = save(&store, vec![term("Corvina", &["cor-vena"])]).unwrap_err();

        assert_eq!(error.kind, ErrorKind::Invalid);
        assert!(error.message.contains("letters, numbers, and spaces"));
        assert_eq!(store.vocabulary().unwrap().terms(), &[term("Corvina", &[])]);
    }
}
