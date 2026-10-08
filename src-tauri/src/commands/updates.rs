//! Checking for a newer release. Both commands refuse while the check is
//! switched off (`updates::ENABLED`), so hiding the button isn't the only guard.

use super::{CommandError, CommandResult, ErrorKind, blocking, open_with_system};
use crate::updates::{self, HttpFetch, UpdateCheck, UpdateError};

/// Whether **Settings** offers **Check for updates**.
#[tauri::command]
pub fn update_check_enabled() -> bool {
    updates::ENABLED
}

/// Asks GitHub for the latest release. Makes a network request, so the UI
/// calls it only when you press the button.
#[tauri::command]
pub async fn check_for_update() -> CommandResult<UpdateCheck> {
    allowed(updates::ENABLED)?;
    blocking(|| {
        let found = updates::check(&HttpFetch::default(), env!("CARGO_PKG_VERSION"))
            .map_err(unavailable)?;
        log::info!(
            "checked for updates: running {}, latest {}",
            found.current,
            found.latest
        );
        Ok(found)
    })
    .await
}

/// Opens a release's page in the browser. The UI passes only the version.
#[tauri::command]
pub fn open_release_page(version: String) -> CommandResult<()> {
    allowed(updates::ENABLED)?;
    let url = updates::release_page(&version).map_err(|error| CommandError {
        kind: ErrorKind::Invalid,
        message: error.to_string(),
    })?;
    open_with_system(&url, "the release page")
}

fn allowed(enabled: bool) -> CommandResult<()> {
    if enabled {
        Ok(())
    } else {
        Err(CommandError {
            kind: ErrorKind::Unavailable,
            message: "checking for updates isn't available in this version".into(),
        })
    }
}

fn unavailable(error: UpdateError) -> CommandError {
    log::warn!("couldn't check for updates: {error}");
    CommandError {
        kind: ErrorKind::Unavailable,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_while_switched_off() {
        assert_eq!(allowed(false).unwrap_err().kind, ErrorKind::Unavailable);
        assert!(allowed(true).is_ok());
    }
}
