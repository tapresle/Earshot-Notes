use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use super::recording::{Transcription, TranscriptionModel};
use super::summaries::models_dir;
use super::{CommandError, CommandResult, ErrorKind};
use crate::audio::mic::{self, InputDevice};
use crate::models::{self, RemoveError};
use crate::store::settings::INPUT_DEVICE;
use crate::store::{ModelRecord, Store};
use crate::summarize::service::Summarizer;

/// The transcription model in use, or why transcription is unavailable.
#[tauri::command]
pub fn transcription_model(
    transcription: State<'_, Arc<Transcription>>,
) -> CommandResult<TranscriptionModel> {
    match &transcription.0 {
        Ok((_, model)) => Ok(model.clone()),
        Err(reason) => Err(CommandError {
            kind: ErrorKind::Unavailable,
            message: format!("transcription is unavailable: {reason}"),
        }),
    }
}

/// The microphones connected now. Listing can take a moment, so it runs off the main thread.
#[tauri::command]
pub async fn input_devices() -> CommandResult<Vec<InputDevice>> {
    tauri::async_runtime::spawn_blocking(mic::input_devices)
        .await
        .map_err(|e| CommandError {
            kind: ErrorKind::Internal,
            message: e.to_string(),
        })?
        .map_err(|e| CommandError {
            kind: ErrorKind::Unavailable,
            message: e.to_string(),
        })
}

/// The chosen microphone's id, or null for the system default.
#[tauri::command]
pub fn input_device(store: State<'_, Arc<Store>>) -> CommandResult<Option<String>> {
    Ok(store.setting(INPUT_DEVICE)?)
}

/// Chooses the microphone for future recordings; null means the system default.
#[tauri::command]
pub fn set_input_device(
    store: State<'_, Arc<Store>>,
    device_id: Option<String>,
) -> CommandResult<()> {
    Ok(store.set_setting(INPUT_DEVICE, device_id.as_deref())?)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModels {
    models: Vec<ModelRecord>,
    /// The model summaries use, if any.
    selected_id: Option<String>,
}

#[tauri::command]
pub fn installed_models(store: State<'_, Arc<Store>>) -> CommandResult<InstalledModels> {
    let mut models = store.list_models()?;
    models::sort_for_display(&mut models);
    Ok(InstalledModels {
        models,
        selected_id: store.summary_model()?.map(|m| m.id),
    })
}

#[tauri::command]
pub fn select_summary_model(store: State<'_, Arc<Store>>, model_id: String) -> CommandResult<()> {
    Ok(store.select_summary_model(&model_id)?)
}

impl From<RemoveError> for CommandError {
    fn from(error: RemoveError) -> Self {
        match error {
            RemoveError::Store(store) => store.into(),
            RemoveError::Io(_) => Self {
                kind: ErrorKind::Unavailable,
                message: error.to_string(),
            },
        }
    }
}

/// Deletes an installed summary model to free disk space. Refused while a
/// summary is running. If it was the selected model, summaries switch to the
/// newest remaining one.
#[tauri::command]
pub fn remove_summary_model(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    summarizer: State<'_, Summarizer>,
    model_id: String,
) -> CommandResult<()> {
    let dir = models_dir(&app)?;
    summarizer.when_idle(|| models::remove(&model_id, &dir, &store))??;
    Ok(())
}
