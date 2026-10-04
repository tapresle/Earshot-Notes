//! Tauri command handlers. Keep them thin: validate, call into a module, map errors.

pub mod meetings;

use serde::Serialize;

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
    fn serializes_in_the_shape_the_ui_expects() {
        let error = CommandError::from(StoreError::MeetingNotFound(7));

        assert_eq!(
            serde_json::to_value(error).unwrap(),
            serde_json::json!({ "kind": "notFound", "message": "meeting 7 doesn't exist" })
        );
    }
}
