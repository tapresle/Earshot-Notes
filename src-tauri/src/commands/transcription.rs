//! Choosing the transcription model, and downloading Whisper (#25).

use std::sync::Arc;

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, State};

use super::models::models_dir;
use super::models::{HuggingFaceSource, event_sink};
use super::recording::Transcription;
use super::{CommandError, CommandResult, ErrorKind, blocking};
use crate::models::WHISPER_TURBO;
use crate::models::download::{HttpFetch, Phase, fetch_verified};
use crate::models::manager::Downloads;
use crate::models::transcription::{self, RemoveError, SelectError, WHISTLE_ID};
use crate::store::Store;
use crate::transcribe::worker::Engine;

/// One transcription model, and where it stands on this Mac.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionOption {
    id: &'static str,
    display_name: &'static str,
    description: &'static str,
    /// Whether it ships with the app, rather than downloading.
    bundled: bool,
    /// The download's size; 0 for a bundled model.
    size_bytes: u64,
    installed: bool,
    /// Bytes saved by an earlier, unfinished download.
    partial_bytes: u64,
    /// Progress, while this model is downloading.
    download: Option<Phase>,
    hugging_face: Option<HuggingFaceSource>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionModels {
    models: Vec<TranscriptionOption>,
    selected_id: &'static str,
    /// Why transcription can't run at all, when it can't.
    unavailable: Option<String>,
}

impl From<SelectError> for CommandError {
    fn from(error: SelectError) -> Self {
        match error {
            SelectError::Store(store) => store.into(),
            SelectError::Unknown(_) | SelectError::NotInstalled(_) => Self {
                kind: ErrorKind::Invalid,
                message: error.to_string(),
            },
        }
    }
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

const WHISTLE_DESCRIPTION: &str = "Fast and light, and works from the first launch.";

#[tauri::command]
pub async fn transcription_models(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    downloads: State<'_, Downloads>,
    transcription: State<'_, Arc<Transcription>>,
) -> CommandResult<TranscriptionModels> {
    let dir = models_dir(&app)?;
    let status = downloads.status();
    let unavailable = transcription.0.as_ref().err().cloned();
    let store = Arc::clone(&store);
    blocking(move || {
        let whistle = TranscriptionOption {
            id: WHISTLE_ID,
            display_name: "Whistle",
            description: WHISTLE_DESCRIPTION,
            bundled: true,
            size_bytes: 0,
            installed: true,
            partial_bytes: 0,
            download: None,
            hugging_face: None,
        };
        let whisper = TranscriptionOption {
            id: WHISPER_TURBO.id,
            display_name: WHISPER_TURBO.display_name,
            description: WHISPER_TURBO.description,
            bundled: false,
            size_bytes: WHISPER_TURBO.size_bytes,
            installed: transcription::whisper_installed(&dir),
            partial_bytes: transcription::whisper_partial_bytes(&dir),
            download: status
                .as_ref()
                .filter(|(id, _)| id == WHISPER_TURBO.id)
                .map(|(_, phase)| *phase),
            // ggerganov/whisper.cpp holds every Whisper model, so name the variant.
            hugging_face: Some(HuggingFaceSource::naming_variant(&WHISPER_TURBO)),
        };
        Ok(TranscriptionModels {
            models: vec![whistle, whisper],
            selected_id: transcription::selected_id(&store, &dir)?,
            unavailable,
        })
    })
    .await
}

/// Chooses the model for recordings started from now on, and loads it now:
/// choosing Whisper loads it, and choosing Whistle frees Whisper's memory.
#[tauri::command]
pub async fn select_transcription_model(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    worker: State<'_, Arc<Transcription>>,
    model_id: String,
) -> CommandResult<()> {
    let dir = models_dir(&app)?;
    let store = Arc::clone(&store);
    let worker = Arc::clone(&worker);
    blocking(move || {
        transcription::select(&store, &dir, &model_id)?;
        if let Ok(worker) = &worker.0 {
            worker.preload(&transcription::selected_engine(&store, &dir)?);
        }
        Ok(())
    })
    .await
}

/// Downloads Whisper from the pinned registry entry. Progress and the result
/// reach the UI as `transcription:download-progress`, `transcription:download-done`,
/// and `transcription:download-failed`. Choosing it is a separate step.
#[tauri::command]
pub fn download_transcription_model(
    app: AppHandle,
    downloads: State<'_, Downloads>,
    model_id: String,
) -> CommandResult<()> {
    if model_id != WHISPER_TURBO.id {
        return Err(CommandError {
            kind: ErrorKind::Invalid,
            message: format!("there's no downloadable transcription model called {model_id}"),
        });
    }
    let dir = models_dir(&app)?;
    let fetch = HttpFetch::default();
    downloads.start(
        &WHISPER_TURBO,
        Box::new(move |cancel, progress| {
            fetch_verified(&WHISPER_TURBO, &dir, &fetch, cancel, progress)
        }),
        event_sink(
            &app,
            "transcription",
            |_| json!({ "modelId": WHISPER_TURBO.id }),
        ),
    )?;
    Ok(())
}

/// Deletes the downloaded Whisper model and goes back to Whistle.
#[tauri::command]
pub async fn remove_transcription_model(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    worker: State<'_, Arc<Transcription>>,
    model_id: String,
) -> CommandResult<()> {
    if model_id != WHISPER_TURBO.id {
        return Err(CommandError {
            kind: ErrorKind::Invalid,
            message: format!("{model_id} can't be removed"),
        });
    }
    let dir = models_dir(&app)?;
    let store = Arc::clone(&store);
    let worker = Arc::clone(&worker);
    blocking(move || {
        if let Ok(worker) = &worker.0 {
            worker.preload(&Engine::Whistle);
        }
        Ok(transcription::remove_whisper(&store, &dir)?)
    })
    .await
}
