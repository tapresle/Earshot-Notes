//! Tauri command handlers. Keep them thin: validate, call into a module, map errors.
//!
//! Synchronous commands run on the main thread, so a command that touches the
//! database, the disk, or another thread is `async` and does that work through
//! [`blocking`].

pub mod chat;
pub mod export;
pub mod meetings;
pub mod models;
pub mod recording;
pub mod recordings;
pub mod settings;
pub mod setup;
pub mod summaries;
pub mod transcription;
pub mod updates;
pub mod vocabulary;

use serde::Serialize;

use crate::llm::worker::WorkerError;
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
            StoreError::SchemaTooNew { .. }
            | StoreError::UnreadableSetting { .. }
            | StoreError::Sqlite(_) => ErrorKind::Internal,
        };
        Self {
            kind,
            message: error.to_string(),
        }
    }
}

impl From<WorkerError> for CommandError {
    fn from(error: WorkerError) -> Self {
        let message = error.to_string();
        let kind = match error {
            WorkerError::Store(store) => return store.into(),
            WorkerError::Busy(_) | WorkerError::NothingToSummarize | WorkerError::NothingToAsk => {
                ErrorKind::Conflict
            }
            WorkerError::EmptyQuestion | WorkerError::QuestionTooLong => ErrorKind::Invalid,
            WorkerError::NoModel | WorkerError::Stopped => ErrorKind::Unavailable,
        };
        Self { kind, message }
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

/// Runs blocking work, such as a SQLite query, on the async runtime's blocking
/// thread pool, so it never holds up the main thread or an async worker.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> CommandResult<T> + Send + 'static,
) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| CommandError {
            kind: ErrorKind::Internal,
            message: format!("a background task failed: {error}"),
        })?
}

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
    fn runs_blocking_work_off_the_calling_thread() {
        let caller = std::thread::current().id();

        let (worker, value) =
            tauri::async_runtime::block_on(blocking(|| Ok((std::thread::current().id(), 7))))
                .unwrap();

        assert_ne!(worker, caller);
        assert_eq!(value, 7);
    }

    #[test]
    fn passes_errors_from_blocking_work_through() {
        let error = tauri::async_runtime::block_on(blocking::<()>(|| {
            Err(StoreError::MeetingNotFound(3).into())
        }))
        .unwrap_err();

        assert_eq!(
            error.kind,
            ErrorKind::NotFound {
                missing: Missing::Meeting
            }
        );
    }

    #[test]
    fn reports_a_panic_in_blocking_work_as_internal() {
        let error = tauri::async_runtime::block_on(blocking::<()>(|| panic!("boom"))).unwrap_err();

        assert_eq!(error.kind, ErrorKind::Internal);
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
