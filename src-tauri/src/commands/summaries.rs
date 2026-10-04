use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use super::{CommandError, CommandResult, ErrorKind};
use crate::models::{self, GEMMA_4_E2B, ImportError, KNOWN_MODELS};
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
pub fn summarize_meeting(
    store: State<'_, Arc<Store>>,
    recorder: State<'_, Arc<Recorder>>,
    summarizer: State<'_, Summarizer>,
    meeting_id: i64,
) -> CommandResult<()> {
    let is_final = recorder.active_meeting() != Some(meeting_id);
    Ok(summarizer.start(&store, meeting_id, is_final)?)
}

#[tauri::command]
pub fn cancel_summary(summarizer: State<'_, Summarizer>) -> bool {
    summarizer.cancel()
}

#[tauri::command]
pub fn active_summary(summarizer: State<'_, Summarizer>) -> Option<i64> {
    summarizer.active_meeting()
}

#[tauri::command]
pub fn latest_summary(
    store: State<'_, Arc<Store>>,
    meeting_id: i64,
) -> CommandResult<Option<Summary>> {
    Ok(store.latest_summary(meeting_id)?)
}

#[tauri::command]
pub fn summary_model(store: State<'_, Arc<Store>>) -> CommandResult<Option<ModelRecord>> {
    Ok(store.summary_model()?)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendedModel {
    display_name: &'static str,
    file_name: &'static str,
    size_bytes: u64,
    url: String,
}

#[tauri::command]
pub fn recommended_model() -> RecommendedModel {
    RecommendedModel {
        display_name: GEMMA_4_E2B.display_name,
        file_name: GEMMA_4_E2B.file_name,
        size_bytes: GEMMA_4_E2B.size_bytes,
        url: GEMMA_4_E2B.url(),
    }
}

/// Copies a GGUF file the user picked into the models folder and selects it.
/// Hashing a multi-gigabyte file takes a few seconds, so this runs off the main thread.
#[tauri::command]
pub async fn import_model(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    path: String,
) -> CommandResult<ModelRecord> {
    let store = Arc::clone(&store);
    let models_dir = models_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        models::import(
            PathBuf::from(path).as_path(),
            &models_dir,
            &store,
            KNOWN_MODELS,
        )
    })
    .await
    .map_err(|error| CommandError {
        kind: ErrorKind::Internal,
        message: format!("the import task failed: {error}"),
    })?
    .map_err(Into::into)
}
