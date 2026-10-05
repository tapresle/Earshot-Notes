use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use super::{CommandResult, blocking};
use crate::audio::{Capture, mic, system};
use crate::recording::{self, CaptureStarter, IssueSink, Recorder, SourceProblem, StartReport};
use crate::store::settings::INPUT_DEVICE;
use crate::store::{Source, Store, TranscriptSegment};
use crate::transcribe::worker::TranscriptionWorker;

/// The transcription worker and the model it loaded, or why it couldn't start.
pub struct Transcription(pub Result<(TranscriptionWorker, TranscriptionModel), String>);

/// Which Whistle model is transcribing, for the settings page.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionModel {
    pub engine: &'static str,
    pub path: String,
    /// False when a model in the user's models folder replaced the bundled one.
    pub bundled: bool,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingIssue {
    meeting_id: i64,
    #[serde(flatten)]
    problem: SourceProblem,
}

/// Starts recording the microphone ("me") and system audio ("them"). Runs off
/// the main thread because the microphone permission prompt blocks until answered.
#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    recorder: State<'_, Arc<Recorder>>,
    transcription: State<'_, Arc<Transcription>>,
    meeting_id: i64,
) -> CommandResult<StartReport> {
    let (store, recorder, transcription) = (
        Arc::clone(&store),
        Arc::clone(&recorder),
        Arc::clone(&transcription),
    );
    let on_issue: IssueSink = Arc::new(move |meeting_id, problem| {
        if let Err(error) = app.emit(
            "recording:issue",
            RecordingIssue {
                meeting_id,
                problem,
            },
        ) {
            log::warn!("couldn't send a recording issue to the UI: {error}");
        }
    });

    blocking(move || {
        let preferred_mic = store.setting(INPUT_DEVICE)?;
        let sources: Vec<(Source, CaptureStarter)> = vec![
            (
                Source::Me,
                Box::new(move |frames, on_error| {
                    mic::start(frames, on_error, preferred_mic)
                        .map(|(c, f)| (Box::new(c) as Box<dyn Capture>, f))
                }),
            ),
            (
                Source::Them,
                Box::new(|frames, on_error| {
                    system::start(frames, on_error)
                        .map(|(c, f)| (Box::new(c) as Box<dyn Capture>, f))
                }),
            ),
        ];
        Ok(recording::start_recording(
            &store,
            &recorder,
            transcription
                .0
                .as_ref()
                .map(|(worker, _)| worker)
                .map_err(String::as_str),
            meeting_id,
            sources,
            on_issue,
        )?)
    })
    .await
}

/// Stops the recording. Waits for the audio pipelines to finish, so it runs
/// off the main thread.
#[tauri::command]
pub async fn stop_recording(
    store: State<'_, Arc<Store>>,
    recorder: State<'_, Arc<Recorder>>,
) -> CommandResult<Option<i64>> {
    let (store, recorder) = (Arc::clone(&store), Arc::clone(&recorder));
    blocking(move || Ok(recording::stop_recording(&store, &recorder)?)).await
}

#[tauri::command]
pub fn active_recording(recorder: State<'_, Arc<Recorder>>) -> Option<i64> {
    recorder.active_meeting()
}

#[tauri::command]
pub async fn list_transcript(
    store: State<'_, Arc<Store>>,
    meeting_id: i64,
) -> CommandResult<Vec<TranscriptSegment>> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.list_segments(meeting_id)?)).await
}
