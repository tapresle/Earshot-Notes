pub mod audio;
pub mod chunker;
pub mod commands;
pub mod export;
pub mod models;
pub mod recording;
pub mod store;
pub mod summarize;
pub mod transcribe;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{App, Emitter, Manager, RunEvent};

use commands::recording::{Transcription, TranscriptionModel};
use recording::Recorder;
use serde_json::json;
use store::{NewSegment, Store};
use summarize::llama::LlamaGenerator;
use summarize::run::Generator;
use summarize::service::{Summarizer, SummaryEvent};
use transcribe::worker::TranscriptionWorker;

const DATABASE_FILE: &str = "earshot.db";
const WHISTLE_FILE: &str = "models/whistle.cact";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let store = Arc::new(Store::open(&data_dir.join(DATABASE_FILE))?);
            match store.close_interrupted_recordings() {
                Ok(0) => {}
                Ok(n) => eprintln!("closed {n} recordings interrupted by the last exit"),
                Err(error) => eprintln!("couldn't close interrupted recordings: {error}"),
            }

            // A model in the user's data folder (from a future update) wins over the bundled one.
            let bundled_whistle = app.path().resource_dir()?.join(WHISTLE_FILE);
            let candidates = vec![data_dir.join(WHISTLE_FILE), bundled_whistle.clone()];
            let worker =
                spawn_transcription(app, store.clone(), candidates).map(|(worker, path)| {
                    let model = TranscriptionModel {
                        engine: "Whistle",
                        bundled: path == bundled_whistle,
                        path: path.to_string_lossy().into_owned(),
                    };
                    (worker, model)
                });
            let summarizer = spawn_summarizer(app, store.clone());

            app.manage(store);
            app.manage(Arc::new(Recorder::default()));
            app.manage(Arc::new(Transcription(worker)));
            app.manage(summarizer);
            app.manage(models::manager::Downloads::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::meetings::create_meeting,
            commands::meetings::list_meetings,
            commands::meetings::get_meeting,
            commands::meetings::rename_meeting,
            commands::meetings::update_notes,
            commands::meetings::delete_meeting,
            commands::recording::start_recording,
            commands::recording::stop_recording,
            commands::recording::active_recording,
            commands::recording::list_transcript,
            commands::summaries::summarize_meeting,
            commands::summaries::cancel_summary,
            commands::summaries::active_summary,
            commands::summaries::latest_summary,
            commands::summaries::summary_model,
            commands::summaries::import_model,
            commands::models::available_models,
            commands::models::download_model,
            commands::models::cancel_download,
            commands::export::export_file_name,
            commands::export::export_meeting,
            commands::export::export_meetings,
            commands::settings::transcription_model,
            commands::settings::input_devices,
            commands::settings::input_device,
            commands::settings::set_input_device,
            commands::settings::installed_models,
            commands::settings::select_summary_model,
            commands::setup::mic_permission,
            commands::setup::request_mic_permission,
            commands::setup::system_audio_permission,
            commands::setup::request_system_audio_permission,
            commands::setup::open_privacy_settings,
            commands::setup::setup_complete,
            commands::setup::set_setup_complete,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                let store = app.state::<Arc<Store>>();
                if let Err(error) = recording::stop_recording(&store, &app.state::<Arc<Recorder>>())
                {
                    eprintln!("couldn't stop the recording on exit: {error}");
                }
            }
        });
}

/// Starts the transcription worker. Saved segments are pushed to the UI as
/// `transcript:segment` events. On failure, recording is disabled but the rest
/// of the app keeps working.
fn spawn_transcription(
    app: &App,
    store: Arc<Store>,
    candidates: Vec<PathBuf>,
) -> Result<(TranscriptionWorker, PathBuf), String> {
    let handle = app.handle().clone();
    TranscriptionWorker::spawn(candidates, move |job, transcription| {
        let saved = store.insert_segment(NewSegment {
            meeting_id: job.meeting_id,
            source: job.source,
            start_ms: job.start_ms,
            end_ms: job.end_ms,
            text: transcription.text,
            language: transcription.language,
        });
        match saved {
            Ok(segment) => {
                if let Err(error) = handle.emit("transcript:segment", &segment) {
                    eprintln!("couldn't send a transcript segment to the UI: {error}");
                }
            }
            Err(error) => eprintln!(
                "couldn't save a transcript segment for meeting {}: {error}",
                job.meeting_id
            ),
        }
    })
    .map(|(worker, loaded)| {
        eprintln!("loaded Whistle from {}", loaded.path.display());
        (worker, loaded.path)
    })
    .map_err(|error| {
        eprintln!("transcription is unavailable: {error}");
        error.to_string()
    })
}

/// Starts the summarizer. Progress, streamed text, and results reach the UI as
/// `summary:progress`, `summary:text`, `summary:done`, and `summary:failed` events.
fn spawn_summarizer(app: &App, store: Arc<Store>) -> Summarizer {
    let handle = app.handle().clone();
    let load = Box::new(|model: &store::ModelRecord| {
        LlamaGenerator::load(std::path::Path::new(&model.file_path))
            .map(|generator| Box::new(generator) as Box<dyn Generator>)
    });
    Summarizer::spawn(store, load, move |event| {
        let (name, payload) = match event {
            SummaryEvent::Progress { meeting_id, stage } => {
                let mut payload = serde_json::to_value(stage).unwrap_or_default();
                payload["meetingId"] = json!(meeting_id);
                ("summary:progress", payload)
            }
            SummaryEvent::Text { meeting_id, text } => (
                "summary:text",
                json!({ "meetingId": meeting_id, "text": text }),
            ),
            SummaryEvent::Done {
                meeting_id,
                summary,
            } => (
                "summary:done",
                json!({ "meetingId": meeting_id, "summary": summary }),
            ),
            SummaryEvent::Failed {
                meeting_id,
                message,
                cancelled,
            } => (
                "summary:failed",
                json!({ "meetingId": meeting_id, "message": message, "cancelled": cancelled }),
            ),
        };
        if let Err(error) = handle.emit(name, payload) {
            eprintln!("couldn't send {name} to the UI: {error}");
        }
    })
}
