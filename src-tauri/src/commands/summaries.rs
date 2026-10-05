use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use super::{CommandError, CommandResult, ErrorKind, blocking};
use crate::models::{self, ImportError, KNOWN_MODELS};
use crate::recording::Recorder;
use crate::store::{ModelRecord, Store, Summary};
use crate::summarize::service::{SummarizeError, Summarizer};

/// Where models live: `<app data>/models`.
pub fn models_dir(app: &AppHandle) -> Result<PathBuf, CommandError> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("models"))
        .map_err(|e| CommandError {
            kind: ErrorKind::Internal,
            message: e.to_string(),
        })
}

impl From<SummarizeError> for CommandError {
    fn from(error: SummarizeError) -> Self {
        let message = error.to_string();
        let kind = match error {
            SummarizeError::Store(store) => return store.into(),
            SummarizeError::Busy | SummarizeError::NothingToSummarize => ErrorKind::Conflict,
            SummarizeError::NoModel | SummarizeError::Stopped => ErrorKind::Unavailable,
        };
        Self { kind, message }
    }
}

impl From<ImportError> for CommandError {
    fn from(error: ImportError) -> Self {
        let message = error.to_string();
        let kind = match error {
            ImportError::Store(store) => return store.into(),
            ImportError::NotAModel => ErrorKind::Invalid,
            ImportError::Io(_) => ErrorKind::Unavailable,
        };
        Self { kind, message }
    }
}

/// Summarizes the meeting as it stands. While it's recording, the summary
/// covers the transcript so far and is marked partial.
#[tauri::command]
pub async fn summarize_meeting(
    store: State<'_, Arc<Store>>,
    recorder: State<'_, Arc<Recorder>>,
    summarizer: State<'_, Arc<Summarizer>>,
    meeting_id: i64,
) -> CommandResult<()> {
    let is_final = recorder.active_meeting() != Some(meeting_id);
    let (store, summarizer) = (Arc::clone(&store), Arc::clone(&summarizer));
    blocking(move || Ok(summarizer.start(&store, meeting_id, is_final)?)).await
}

#[tauri::command]
pub fn cancel_summary(summarizer: State<'_, Arc<Summarizer>>) -> bool {
    summarizer.cancel()
}

#[tauri::command]
pub fn active_summary(summarizer: State<'_, Arc<Summarizer>>) -> Option<i64> {
    summarizer.active_meeting()
}

#[tauri::command]
pub async fn latest_summary(
    store: State<'_, Arc<Store>>,
    meeting_id: i64,
) -> CommandResult<Option<Summary>> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.latest_summary(meeting_id)?)).await
}

#[tauri::command]
pub async fn summary_model(store: State<'_, Arc<Store>>) -> CommandResult<Option<ModelRecord>> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.summary_model()?)).await
}

/// Copies a GGUF file the user picked into the models folder. It becomes the
/// summary model only when no other model is installed.
/// Hashing a multi-gigabyte file takes a few seconds, so this runs off the main thread.
#[tauri::command]
pub async fn import_model(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    path: String,
) -> CommandResult<ModelRecord> {
    let store = Arc::clone(&store);
    let models_dir = models_dir(&app)?;
    blocking(move || {
        Ok(models::import(
            PathBuf::from(path).as_path(),
            &models_dir,
            &store,
            KNOWN_MODELS,
        )?)
    })
    .await
}
