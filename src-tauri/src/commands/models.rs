use std::sync::Arc;

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use super::summaries::models_dir;
use super::{CommandError, CommandResult, ErrorKind};
use crate::models::download::{HttpFetch, Phase, partial_path};
use crate::models::manager::{DownloadEvent, Downloads, StartError};
use crate::models::{GEMMA_4_E2B, KNOWN_MODELS};
use crate::store::Store;

/// A model the app can download, and where it stands on this machine.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableModel {
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    file_name: &'static str,
    size_bytes: u64,
    recommended: bool,
    installed: bool,
    /// Bytes already saved by an earlier, unfinished download.
    partial_bytes: u64,
    /// Progress, while this model is downloading.
    download: Option<Phase>,
}

impl From<StartError> for CommandError {
    fn from(error: StartError) -> Self {
        let kind = match error {
            StartError::Busy(_) => ErrorKind::Conflict,
            StartError::Thread(_) => ErrorKind::Internal,
        };
        Self {
            kind,
            message: error.to_string(),
        }
    }
}

#[tauri::command]
pub fn available_models(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    downloads: State<'_, Downloads>,
) -> CommandResult<Vec<AvailableModel>> {
    let dir = models_dir(&app)?;
    let status = downloads.status();
    KNOWN_MODELS
        .iter()
        .map(|model| {
            Ok(AvailableModel {
                id: model.id,
                display_name: model.display_name,
                description: model.description,
                file_name: model.file_name,
                size_bytes: model.size_bytes,
                recommended: model.id == GEMMA_4_E2B.id,
                installed: store.model_by_sha256(model.sha256)?.is_some(),
                partial_bytes: std::fs::metadata(partial_path(&dir.join(model.file_name)))
                    .map(|m| m.len())
                    .unwrap_or(0),
                download: status
                    .as_ref()
                    .filter(|(id, _)| id == model.id)
                    .map(|(_, phase)| *phase),
            })
        })
        .collect()
}

/// Downloads a known model by id. Only models in the pinned registry can be
/// fetched; the UI never supplies a URL.
#[tauri::command]
pub fn download_model(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    downloads: State<'_, Downloads>,
    model_id: String,
) -> CommandResult<()> {
    let model = KNOWN_MODELS
        .iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| CommandError {
            kind: ErrorKind::Invalid,
            message: format!("there's no downloadable model called {model_id}"),
        })?;
    let dir = models_dir(&app)?;
    let handle = app.clone();
    let sink = Arc::new(move |event: DownloadEvent| {
        let (name, payload) = match event {
            DownloadEvent::Progress { model_id, phase } => {
                let mut payload = serde_json::to_value(phase).unwrap_or_default();
                payload["modelId"] = json!(model_id);
                ("model:download-progress", payload)
            }
            DownloadEvent::Done { model } => ("model:download-done", json!({ "model": model })),
            DownloadEvent::Failed {
                model_id,
                message,
                cancelled,
            } => (
                "model:download-failed",
                json!({ "modelId": model_id, "message": message, "cancelled": cancelled }),
            ),
        };
        if let Err(error) = handle.emit(name, payload) {
            eprintln!("couldn't send {name} to the UI: {error}");
        }
    });
    downloads.start(
        model,
        dir,
        Arc::clone(&store),
        Arc::new(HttpFetch::default()),
        sink,
    )?;
    Ok(())
}

#[tauri::command]
pub fn cancel_download(downloads: State<'_, Downloads>) -> bool {
    downloads.cancel()
}
