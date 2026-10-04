pub mod audio;
pub mod chunker;
pub mod commands;
pub mod recording;
pub mod store;
pub mod transcribe;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{App, Emitter, Manager, RunEvent};

use commands::recording::Transcription;
use recording::Recorder;
use store::{NewSegment, Store};
use transcribe::worker::TranscriptionWorker;

const DATABASE_FILE: &str = "earshot.db";
const WHISTLE_FILE: &str = "models/whistle.cact";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            let candidates = vec![
                data_dir.join(WHISTLE_FILE),
                app.path().resource_dir()?.join(WHISTLE_FILE),
            ];
            let worker = spawn_transcription(app, store.clone(), candidates);

            app.manage(store);
            app.manage(Recorder::default());
            app.manage(Transcription(worker));
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                let store = app.state::<Arc<Store>>();
                if let Err(error) = recording::stop_recording(&store, &app.state::<Recorder>()) {
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
) -> Result<TranscriptionWorker, String> {
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
        worker
    })
    .map_err(|error| {
        eprintln!("transcription is unavailable: {error}");
        error.to_string()
    })
}
