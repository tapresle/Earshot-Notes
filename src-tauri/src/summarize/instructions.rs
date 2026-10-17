//! Your own summary instructions, which replace `prompt::DEFAULT_INSTRUCTIONS`.
//! Only your text is stored, so without it summaries follow the default of
//! whichever version of the app runs them. Never log the text.

use super::prompt::DEFAULT_INSTRUCTIONS;
use crate::store::settings::SUMMARY_INSTRUCTIONS;
use crate::store::{Store, StoreError};

/// The most characters your instructions can have.
pub const MAX_CHARS: usize = 6_000;

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("summary instructions can be at most 6,000 characters")]
    TooLong,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Your instructions, or `None` when summaries follow the default.
pub fn custom(store: &Store) -> Result<Option<String>, StoreError> {
    store.setting(SUMMARY_INSTRUCTIONS)
}

/// The instructions the next summary follows.
pub fn in_use(store: &Store) -> Result<String, StoreError> {
    Ok(custom(store)?.unwrap_or_else(|| DEFAULT_INSTRUCTIONS.to_owned()))
}

/// Saves your instructions, trimmed, and returns what's stored. `None`, empty
/// text, or the default removes them, so summaries follow the default again.
pub fn save(store: &Store, text: Option<&str>) -> Result<Option<String>, SaveError> {
    let text = text
        .map(str::trim)
        .filter(|text| !text.is_empty() && *text != DEFAULT_INSTRUCTIONS);
    if text.is_some_and(|text| text.chars().count() > MAX_CHARS) {
        return Err(SaveError::TooLong);
    }
    store.set_setting(SUMMARY_INSTRUCTIONS, text)?;
    log::info!(
        "summaries now follow {} instructions",
        if text.is_some() {
            "custom"
        } else {
            "the default"
        }
    );
    Ok(text.map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_default_until_you_save_your_own() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(custom(&store).unwrap(), None);
        assert_eq!(in_use(&store).unwrap(), DEFAULT_INSTRUCTIONS);

        let saved = save(&store, Some("  List decisions only.\n")).unwrap();

        assert_eq!(saved.as_deref(), Some("List decisions only."));
        assert_eq!(in_use(&store).unwrap(), "List decisions only.");
    }

    #[test]
    fn keeps_your_instructions_whatever_the_default_becomes() {
        // A stored value is never compared with or rewritten to the current
        // default when read, so an update that changes the default keeps it.
        let store = Store::open_in_memory().unwrap();
        store
            .set_setting(
                SUMMARY_INSTRUCTIONS,
                Some("Instructions saved by an older version."),
            )
            .unwrap();

        assert_eq!(
            in_use(&store).unwrap(),
            "Instructions saved by an older version."
        );
    }

    #[test]
    fn resetting_goes_back_to_the_default() {
        let store = Store::open_in_memory().unwrap();
        save(&store, Some("List decisions only.")).unwrap();

        assert_eq!(save(&store, None).unwrap(), None);

        assert_eq!(store.setting(SUMMARY_INSTRUCTIONS).unwrap(), None);
        assert_eq!(in_use(&store).unwrap(), DEFAULT_INSTRUCTIONS);
    }

    #[test]
    fn stores_nothing_for_empty_text_or_the_default() {
        let store = Store::open_in_memory().unwrap();
        let default_with_spaces = format!("\n{DEFAULT_INSTRUCTIONS}  \n");

        for text in ["", "   \n", default_with_spaces.as_str()] {
            save(&store, Some("List decisions only.")).unwrap();

            assert_eq!(save(&store, Some(text)).unwrap(), None, "{text:?}");
            assert_eq!(store.setting(SUMMARY_INSTRUCTIONS).unwrap(), None);
        }
    }

    #[test]
    fn refuses_text_over_the_limit_and_keeps_what_was_saved() {
        let store = Store::open_in_memory().unwrap();
        save(&store, Some("List decisions only.")).unwrap();
        // Characters, not bytes: 6,000 two-byte letters fit.
        let at_limit = "é".repeat(MAX_CHARS);
        let over = "x".repeat(MAX_CHARS + 1);

        let error = save(&store, Some(&over)).unwrap_err();

        assert!(matches!(error, SaveError::TooLong));
        assert!(error.to_string().contains("6,000 characters"), "{error}");
        assert_eq!(MAX_CHARS, 6_000);
        assert_eq!(in_use(&store).unwrap(), "List decisions only.");
        assert_eq!(save(&store, Some(&at_limit)).unwrap(), Some(at_limit));
    }
}
