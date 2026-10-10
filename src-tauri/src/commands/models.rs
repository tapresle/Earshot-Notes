use std::sync::Arc;

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use super::summaries::models_dir;
use super::{CommandError, CommandResult, ErrorKind, blocking, open_with_system};
use crate::models::download::{HttpFetch, Phase, download, partial_path};
use crate::models::manager::{DownloadEvent, Downloads, EventSink, StartError};
use crate::models::{self, GEMMA_4_E2B, KNOWN_MODELS, KnownModel};
use crate::store::Store;

/// Exactly which file a model is, on Hugging Face.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HuggingFaceSource {
    repo: &'static str,
    quantization: &'static str,
    revision: &'static str,
    /// The file, for a repository that holds several models, where the
    /// quantization alone doesn't say which one.
    #[serde(skip_serializing_if = "Option::is_none")]
    file_name: Option<&'static str>,
}

impl From<&KnownModel> for HuggingFaceSource {
    fn from(model: &KnownModel) -> Self {
        Self {
            repo: model.repo,
            quantization: model.quantization,
            revision: model.revision,
            file_name: None,
        }
    }
}

impl HuggingFaceSource {
    /// The source of a model in a repository that holds several models,
    /// named by its file.
    pub(super) fn naming_file(model: &KnownModel) -> Self {
        Self {
            file_name: Some(model.file_name),
            ..model.into()
        }
    }
}

/// A model the app can download, and where it stands on this machine.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableModel {
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    file_name: &'static str,
    size_bytes: u64,
    hugging_face: HuggingFaceSource,
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
pub async fn available_models(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    downloads: State<'_, Downloads>,
) -> CommandResult<Vec<AvailableModel>> {
    let dir = models_dir(&app)?;
    let status = downloads.status();
    let store = Arc::clone(&store);
    blocking(move || {
        KNOWN_MODELS
            .iter()
            .map(|model| {
                Ok(AvailableModel {
                    id: model.id,
                    display_name: model.display_name,
                    description: model.description,
                    file_name: model.file_name,
                    size_bytes: model.size_bytes,
                    hugging_face: model.into(),
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
    })
    .await
}

/// Opens a known model's Hugging Face page in the browser. The UI names the
/// model; the URL comes from the pinned registry.
#[tauri::command]
pub fn open_model_page(model_id: String) -> CommandResult<()> {
    let model = models::known_by_id(&model_id).ok_or_else(|| CommandError {
        kind: ErrorKind::Invalid,
        message: format!("there's no Hugging Face page for {model_id}"),
    })?;
    open_with_system(&model.page_url(), "the model's Hugging Face page")
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
    let store = Arc::clone(&store);
    let fetch = HttpFetch::default();
    downloads.start(
        model,
        Box::new(move |cancel, progress| download(model, &dir, &store, &fetch, cancel, progress)),
        event_sink(&app, "model", |model| json!({ "model": model })),
    )?;
    Ok(())
}

/// Sends a download's events to the UI as `<prefix>:download-progress`,
/// `<prefix>:download-done` (with `done`'s payload), and `<prefix>:download-failed`.
pub(super) fn event_sink<T: 'static>(
    app: &AppHandle,
    prefix: &'static str,
    done: fn(T) -> serde_json::Value,
) -> EventSink<T> {
    let handle = app.clone();
    Arc::new(move |event: DownloadEvent<T>| {
        let (name, payload) = match event {
            DownloadEvent::Progress { model_id, phase } => {
                let mut payload = serde_json::to_value(phase).unwrap_or_default();
                payload["modelId"] = json!(model_id);
                ("download-progress", payload)
            }
            DownloadEvent::Done(result) => ("download-done", done(result)),
            DownloadEvent::Failed {
                model_id,
                message,
                cancelled,
            } => (
                "download-failed",
                json!({ "modelId": model_id, "message": message, "cancelled": cancelled }),
            ),
        };
        let name = format!("{prefix}:{name}");
        if let Err(error) = handle.emit(&name, payload) {
            log::warn!("couldn't send {name} to the UI: {error}");
        }
    })
}

#[tauri::command]
pub fn cancel_download(downloads: State<'_, Downloads>) -> bool {
    downloads.cancel()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::WHISPER_TURBO;

    #[test]
    fn names_the_file_in_a_repository_of_several_models() {
        let source = serde_json::to_value(HuggingFaceSource::naming_file(&WHISPER_TURBO)).unwrap();

        assert_eq!(
            source,
            json!({
                "repo": "ggerganov/whisper.cpp",
                "quantization": "Q5_0",
                "revision": "98aa99a0a9db05ae2342309f5096248665f7cba3",
                "fileName": "ggml-large-v3-turbo-q5_0.bin",
            })
        );
    }
}
