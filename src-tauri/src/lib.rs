// Log through the `log` macros so messages reach the log file.
#![deny(clippy::print_stdout, clippy::print_stderr)]

pub mod audio;
pub mod chunker;
pub mod commands;
pub mod export;
pub mod models;
pub mod recording;
pub mod recordings;
pub mod store;
pub mod summarize;
pub mod transcribe;
pub mod tray;
pub mod updates;
pub mod vocabulary;
pub mod window;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{App, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

use commands::recording::{Transcription, TranscriptionModel};
use recording::Recorder;
use recordings::Recordings;
use serde_json::json;
use store::{NewSegment, Store};
use summarize::llama::LlamaGenerator;
use summarize::run::Generator;
use summarize::service::{Summarizer, SummaryEvent};
use transcribe::worker::TranscriptionWorker;

const DATABASE_FILE: &str = "earshot.db";
const WHISTLE_FILE: &str = "models/whistle.cact";
/// Kept audio, when the setting is on.
const RECORDINGS_DIR: &str = "recordings";
/// How long quitting waits for the transcription worker to free its models.
const EXIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
/// The log file rolls over at this size, keeping only the newest file.
const MAX_LOG_BYTES: u128 = 1_000_000;

/// Logs to stderr and to a file in the system log folder
/// (`~/Library/Logs/<bundle id>/Earshot Notes.log` on macOS, named after the product). Nothing is sent anywhere else.
fn logger() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_log::Builder::new()
        .clear_targets()
        .targets([
            Target::new(TargetKind::Stderr),
            Target::new(TargetKind::LogDir { file_name: None }),
        ])
        // Dependencies only report problems; the app itself also reports what it did.
        .level(log::LevelFilter::Warn)
        .level_for(
            "earshot_lib",
            if cfg!(debug_assertions) {
                log::LevelFilter::Debug
            } else {
                log::LevelFilter::Info
            },
        )
        .max_file_size(MAX_LOG_BYTES)
        .rotation_strategy(RotationStrategy::KeepOne)
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(logger())
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            // Closing the window keeps the app, and any recording, running in the menu bar.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                window::hide(window);
            }
        })
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let store = Arc::new(Store::open(&data_dir.join(DATABASE_FILE))?);
            match store.close_interrupted_recordings() {
                Ok(0) => {}
                Ok(n) => log::info!("closed {n} recordings interrupted by the last exit"),
                Err(error) => log::error!("couldn't close interrupted recordings: {error}"),
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
            app.manage(Arc::new(Recordings::new(data_dir.join(RECORDINGS_DIR))));
            app.manage(Arc::new(Transcription(worker)));
            app.manage(Arc::new(summarizer));
            app.manage(models::manager::Downloads::default());

            tray::create(app)?;
            // The window starts hidden (see tauri.conf.json), so a launch at
            // login can stay in the menu bar without the window flashing open.
            if macos_audio::launched_as_login_item() {
                log::info!("opened at login; staying in the menu bar");
                window::hide_dock_icon(app.handle());
            } else {
                window::show(app.handle());
            }
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
            commands::recordings::keep_audio,
            commands::recordings::set_keep_audio,
            commands::recordings::recordings_size,
            commands::recordings::delete_all_recordings,
            commands::recordings::show_recordings,
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
            commands::settings::launch_at_login,
            commands::settings::set_launch_at_login,
            commands::settings::speech_sensitivity,
            commands::settings::set_speech_sensitivity,
            commands::settings::installed_models,
            commands::settings::select_summary_model,
            commands::settings::remove_summary_model,
            commands::models::open_model_page,
            commands::setup::mic_permission,
            commands::setup::request_mic_permission,
            commands::setup::system_audio_permission,
            commands::setup::request_system_audio_permission,
            commands::setup::open_privacy_settings,
            commands::setup::setup_complete,
            commands::setup::set_setup_complete,
            commands::updates::update_check_enabled,
            commands::updates::check_for_update,
            commands::updates::open_release_page,
            commands::vocabulary::vocabulary,
            commands::vocabulary::set_vocabulary,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Clicking the Dock icon, while it shows, opens the window.
            if let RunEvent::Reopen { .. } = event {
                window::show(app);
            }
            if let RunEvent::Exit = event {
                let store = app.state::<Arc<Store>>();
                if let Err(error) = recording::stop_recording(&store, &app.state::<Arc<Recorder>>())
                {
                    log::error!("couldn't stop the recording on exit: {error}");
                }
                if let Ok((worker, _)) = &app.state::<Arc<Transcription>>().0 {
                    worker.shutdown(EXIT_TIMEOUT);
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
        // Corrected with the vocabulary, and without echo, as it's saved.
        let meeting_id = job.meeting_id;
        let saved = store.insert_transcribed_segment(NewSegment {
            meeting_id,
            source: job.source,
            start_ms: job.start_ms,
            end_ms: job.end_ms,
            text: transcription.text,
            language: transcription.language,
        });
        match saved {
            Ok(saved) => {
                // An updated segment goes out again under its ID, replacing the shown one.
                for segment in saved.segment.iter().chain(&saved.updated) {
                    if let Err(error) = handle.emit("transcript:segment", segment) {
                        log::warn!("couldn't send a transcript segment to the UI: {error}");
                    }
                }
                if !saved.removed.is_empty() {
                    let removed =
                        serde_json::json!({ "meetingId": meeting_id, "ids": saved.removed });
                    if let Err(error) = handle.emit("transcript:removed", removed) {
                        log::warn!("couldn't tell the UI about removed echoes: {error}");
                    }
                }
            }
            Err(error) => {
                log::error!("couldn't save a transcript segment for meeting {meeting_id}: {error}")
            }
        }
    })
    .map(|(worker, loaded)| {
        log::info!("loaded Whistle from {}", loaded.path.display());
        (worker, loaded.path)
    })
    .map_err(|error| {
        log::error!("transcription is unavailable: {error}");
        error.to_string()
    })
}

/// Starts the summarizer. Progress, streamed text, and results reach the UI as
/// `summary:progress`, `summary:text`, `summary:done`, and `summary:failed` events.
fn spawn_summarizer(app: &App, store: Arc<Store>) -> Summarizer {
    let handle = app.handle().clone();
    let load = Box::new(|model: &store::ModelRecord| {
        LlamaGenerator::load_with_context_limit(
            std::path::Path::new(&model.file_path),
            models::context_limit(&model.sha256),
        )
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
                title,
            } => (
                "summary:done",
                json!({ "meetingId": meeting_id, "summary": summary, "title": title }),
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
            log::warn!("couldn't send {name} to the UI: {error}");
        }
    })
}
