//! The first-run checklist: permissions and whether setup has been finished.

use std::sync::Arc;

use macos_audio::{Permission as MacPermission, SystemAudioTap};
use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CommandError, CommandResult, ErrorKind, open_with_system};
use crate::store::Store;

const SETUP_COMPLETE: &str = "setup_complete";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
}

impl From<MacPermission> for Permission {
    fn from(value: MacPermission) -> Self {
        match value {
            MacPermission::NotDetermined => Permission::NotDetermined,
            MacPermission::Restricted => Permission::Restricted,
            MacPermission::Denied => Permission::Denied,
            MacPermission::Authorized => Permission::Authorized,
        }
    }
}

#[tauri::command]
pub fn mic_permission() -> Permission {
    macos_audio::mic_permission().into()
}

/// Shows the macOS microphone prompt if it hasn't been answered, and returns
/// the result. Blocks until answered, so it runs off the main thread.
#[tauri::command]
pub async fn request_mic_permission() -> CommandResult<Permission> {
    tauri::async_runtime::spawn_blocking(|| {
        macos_audio::request_mic_access();
        Permission::from(macos_audio::mic_permission())
    })
    .await
    .map_err(|e| CommandError {
        kind: ErrorKind::Internal,
        message: e.to_string(),
    })
}

/// System audio recording permission, or null when macOS doesn't let the app check.
#[tauri::command]
pub fn system_audio_permission() -> Option<Permission> {
    macos_audio::system_audio_permission().map(Permission::from)
}

/// Shows the macOS system audio recording prompt if it hasn't been answered,
/// and returns the result. When macOS doesn't let the app ask directly, this
/// briefly starts the system audio tap instead, which makes macOS show the
/// prompt, and returns null because the answer can't be read.
#[tauri::command]
pub async fn request_system_audio_permission() -> CommandResult<Option<Permission>> {
    tauri::async_runtime::spawn_blocking(|| {
        if macos_audio::request_system_audio_access().is_some() {
            return macos_audio::system_audio_permission().map(Permission::from);
        }
        if let Ok((tap, _)) = SystemAudioTap::start(|_| {}) {
            std::thread::sleep(std::time::Duration::from_secs(2));
            drop(tap);
        }
        None
    })
    .await
    .map_err(|e| CommandError {
        kind: ErrorKind::Internal,
        message: e.to_string(),
    })
}

/// The Privacy & Security panes the app links to. A fixed list, so the UI
/// can't open arbitrary URLs.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrivacyPane {
    Microphone,
    SystemAudio,
}

impl PrivacyPane {
    fn url(self) -> &'static str {
        match self {
            PrivacyPane::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            // System audio recording is listed under Screen & System Audio Recording.
            PrivacyPane::SystemAudio => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
        }
    }
}

#[tauri::command]
pub fn open_privacy_settings(pane: PrivacyPane) -> CommandResult<()> {
    open_with_system(pane.url(), "System Settings")
}

#[tauri::command]
pub fn setup_complete(store: State<'_, Arc<Store>>) -> CommandResult<bool> {
    Ok(store.setting(SETUP_COMPLETE)?.is_some())
}

/// Marks the checklist done, or brings it back when `complete` is false.
#[tauri::command]
pub fn set_setup_complete(store: State<'_, Arc<Store>>, complete: bool) -> CommandResult<()> {
    Ok(store.set_setting(SETUP_COMPLETE, complete.then_some("1"))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_only_to_privacy_panes() {
        for pane in [PrivacyPane::Microphone, PrivacyPane::SystemAudio] {
            assert!(
                pane.url().starts_with(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_"
                )
            );
        }
    }

    #[test]
    fn rejects_unknown_panes() {
        assert!(serde_json::from_str::<PrivacyPane>("\"https://example.com\"").is_err());
        assert!(matches!(
            serde_json::from_str::<PrivacyPane>("\"systemAudio\"").unwrap(),
            PrivacyPane::SystemAudio
        ));
    }
}
