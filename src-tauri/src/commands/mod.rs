//! Tauri command handlers. Keep them thin: validate, call into a module, map errors.

pub mod export;
pub mod meetings;
pub mod models;
pub mod recording;
pub mod settings;
pub mod setup;
pub mod summaries;

use serde::Serialize;

use crate::recording::RecordingError;
use crate::store::StoreError;

/// The error shape the UI receives from every command: `kind`, `message`,
/// and for `notFound`, `missing`.
#[derive(Debug, Serialize, PartialEq)]
pub struct CommandError {
    #[serde(flatten)]
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ErrorKind {
    /// The thing the action names doesn't exist, such as a deleted meeting.
    NotFound {
        missing: Missing,
    },
    /// The action conflicts with the current state, such as an active recording.
    Conflict,
    /// A device or model the action needs isn't available.
    Unavailable,
    /// The input isn't acceptable, such as a file that isn't a model.
    Invalid,
    Internal,
}

/// What a `NotFound` error couldn't find, so the UI can say so.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Missing {
    Meeting,
    Model,
}

/// Opens a URL with the system's default handler, such as the browser or
/// System Settings. Callers build `target` themselves; never pass one from the UI.
pub(crate) fn open_with_system(target: &str, what: &str) -> CommandResult<()> {
    let unavailable = |detail: String| CommandError {
        kind: ErrorKind::Unavailable,
        message: format!("couldn't open {what}{detail}"),
    };
    let status = std::process::Command::new("open")
        .arg(target)
        .status()
        .map_err(|e| unavailable(format!(": {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(unavailable(String::new()))
    }
}

impl From<StoreError> for CommandError {
    fn from(error: StoreError) -> Self {
        let kind = match error {
            StoreError::MeetingNotFound(_) => ErrorKind::NotFound {
                missing: Missing::Meeting,
            },
            StoreError::ModelNotFound(_) => ErrorKind::NotFound {
                missing: Missing::Model,
            },
            StoreError::SchemaTooNew { .. } | StoreError::Sqlite(_) => ErrorKind::Internal,
        };
        Self {
            kind,
            message: error.to_string(),
        }
    }
}

impl From<RecordingError> for CommandError {
    fn from(error: RecordingError) -> Self {
        let message = error.to_string();
        let kind = match error {
            RecordingError::Store(store) => return store.into(),
            RecordingError::AlreadyRecording(_) => ErrorKind::Conflict,
            RecordingError::TranscriptionUnavailable(_) | RecordingError::NoAudio(_) => {
                ErrorKind::Unavailable
            }
            RecordingError::Thread(_) => ErrorKind::Internal,
        };
        Self { kind, message }
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_missing_meeting_to_not_found() {
        let error = CommandError::from(StoreError::MeetingNotFound(7));

        assert_eq!(
            error.kind,
            ErrorKind::NotFound {
                missing: Missing::Meeting
            }
        );
        assert_eq!(error.message, "meeting 7 doesn't exist");
    }

    #[test]
    fn maps_a_missing_model_to_not_found() {
        let error = CommandError::from(StoreError::ModelNotFound("local-abc".into()));

        assert_eq!(
            error.kind,
            ErrorKind::NotFound {
                missing: Missing::Model
            }
        );
        assert_eq!(error.message, "model local-abc doesn't exist");
    }

    #[test]
    fn maps_recording_errors_to_kinds() {
        let conflict = CommandError::from(RecordingError::AlreadyRecording(3));
        let unavailable =
            CommandError::from(RecordingError::TranscriptionUnavailable("no model".into()));
        let missing = CommandError::from(RecordingError::Store(StoreError::MeetingNotFound(4)));

        assert_eq!(conflict.kind, ErrorKind::Conflict);
        assert_eq!(unavailable.kind, ErrorKind::Unavailable);
        assert_eq!(
            missing.kind,
            ErrorKind::NotFound {
                missing: Missing::Meeting
            }
        );
    }

    #[test]
    fn serializes_in_the_shape_the_ui_expects() {
        let error = CommandError::from(StoreError::MeetingNotFound(7));

        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({
                "kind": "notFound",
                "missing": "meeting",
                "message": "meeting 7 doesn't exist",
            })
        );
        assert_eq!(
            serde_json::to_value(CommandError {
                kind: ErrorKind::Conflict,
                message: "busy".into(),
            })
            .unwrap(),
            serde_json::json!({ "kind": "conflict", "message": "busy" })
        );
    }
}
