//! Tauri command handlers. Keep them thin: validate, call into a module, map errors.

pub mod meetings;
pub mod recording;

use serde::Serialize;

use crate::recording::RecordingError;
use crate::store::StoreError;

/// The error shape the UI receives from every command.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    NotFound,
    /// The action conflicts with the current state, such as an active recording.
    Conflict,
    /// A device or model the action needs isn't available.
    Unavailable,
    Internal,
}

impl From<StoreError> for CommandError {
    fn from(error: StoreError) -> Self {
        let kind = match error {
            StoreError::MeetingNotFound(_) => ErrorKind::NotFound,
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

        assert_eq!(error.kind, ErrorKind::NotFound);
        assert_eq!(error.message, "meeting 7 doesn't exist");
    }

    #[test]
    fn maps_recording_errors_to_kinds() {
        let conflict = CommandError::from(RecordingError::AlreadyRecording(3));
        let unavailable =
            CommandError::from(RecordingError::TranscriptionUnavailable("no model".into()));
        let missing = CommandError::from(RecordingError::Store(StoreError::MeetingNotFound(4)));

        assert_eq!(conflict.kind, ErrorKind::Conflict);
        assert_eq!(unavailable.kind, ErrorKind::Unavailable);
        assert_eq!(missing.kind, ErrorKind::NotFound);
    }

    #[test]
    fn serializes_in_the_shape_the_ui_expects() {
        let error = CommandError::from(StoreError::MeetingNotFound(7));

        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({ "kind": "notFound", "message": "meeting 7 doesn't exist" })
        );
    }
}
