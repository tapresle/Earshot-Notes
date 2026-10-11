//! Checking for a newer release, only when you press **Check for updates**.

use super::{CommandError, CommandResult, ErrorKind, blocking, open_with_system};
use crate::updates::{self, HttpFetch, UpdateCheck, UpdateError};

/// Asks GitHub for the latest release. Makes a network request, so the UI
/// calls it only when you press the button.
#[tauri::command]
pub async fn check_for_update() -> CommandResult<UpdateCheck> {
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
    let url = updates::release_page(&version).map_err(|error| CommandError {
        kind: ErrorKind::Invalid,
        message: error.to_string(),
    })?;
    open_with_system(&url, "the release page")
}

fn unavailable(error: UpdateError) -> CommandError {
    log::warn!("couldn't check for updates: {error}");
    CommandError {
        kind: ErrorKind::Unavailable,
        message: error.to_string(),
    }
}
