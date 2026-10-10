use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};

use super::models::models_dir;
use super::{CommandResult, blocking};
use crate::audio::{Capture, mic, system};
use crate::models::transcription;
use crate::recording::{self, CaptureStarter, IssueSink, Recorder, SourceProblem, StartReport};
use crate::recordings::Recordings;
use crate::store::settings::INPUT_DEVICE;
use crate::store::{Source, Store, TranscriptSegment};
use crate::transcribe::worker::TranscriptionWorker;
use crate::tray;

/// The transcription worker, or why it couldn't start.
pub struct Transcription(pub Result<TranscriptionWorker, String>);

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingIssue {
    meeting_id: i64,
    #[serde(flatten)]
    problem: SourceProblem,
}

/// Starts recording the microphone ("me") and system audio ("them"). Runs off
/// the main thread because the microphone permission prompt blocks until answered.
/// Sent as `recording:changed` whenever a recording starts or stops, from the
/// window or the menu bar, so every view shows the same state.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingChanged {
    meeting_id: Option<i64>,
}

/// Starts recording the microphone ("me") and system audio ("them") for a
/// meeting, then tells the UI and the menu bar. Blocks until any permission
/// prompt is answered, so never call it on the main thread.
pub(crate) fn start(app: &AppHandle, meeting_id: i64) -> CommandResult<StartReport> {
    let store = app.state::<Arc<Store>>();
    let transcription = app.state::<Arc<Transcription>>();
    let issues = app.clone();
    let on_issue: IssueSink = Arc::new(move |meeting_id, problem| {
        if let Err(error) = issues.emit(
            "recording:issue",
            RecordingIssue {
                meeting_id,
                problem,
            },
        ) {
            log::warn!("couldn't send a recording issue to the UI: {error}");
        }
    });
    let preferred_mic = store.setting(INPUT_DEVICE)?;
    let engine = transcription::selected_engine(&store, &models_dir(app)?)?;
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
                system::start(frames, on_error).map(|(c, f)| (Box::new(c) as Box<dyn Capture>, f))
            }),
        ),
    ];
    let report = recording::start_recording(
        &store,
        &app.state::<Arc<Recorder>>(),
        transcription
            .0
            .as_ref()
            .map(|worker| (worker, engine))
            .map_err(String::as_str),
        meeting_id,
        sources,
        on_issue,
        &app.state::<Arc<Recordings>>(),
    )?;
    let missing = report.unavailable.clone();
    announce(app, Some(meeting_id), tray::Status::Recording { missing });
    Ok(report)
}

/// Stops the active recording and tells the UI and the menu bar. Waits for
/// the audio pipelines to finish, so never call it on the main thread.
pub(crate) fn stop(app: &AppHandle) -> CommandResult<Option<i64>> {
    let stopped =
        recording::stop_recording(&app.state::<Arc<Store>>(), &app.state::<Arc<Recorder>>())?;
    if stopped.is_some() {
        announce(app, None, tray::Status::Idle);
    }
    Ok(stopped)
}

fn announce(app: &AppHandle, meeting_id: Option<i64>, status: tray::Status) {
    if let Err(error) = app.emit("recording:changed", RecordingChanged { meeting_id }) {
        log::warn!("couldn't send the recording state to the UI: {error}");
    }
    tray::show_status(app, status);
}

/// Starts recording a meeting. See [`start`].
#[tauri::command]
pub async fn start_recording(app: AppHandle, meeting_id: i64) -> CommandResult<StartReport> {
    blocking(move || start(&app, meeting_id)).await
}

/// Stops the recording. See [`stop`].
#[tauri::command]
pub async fn stop_recording(app: AppHandle) -> CommandResult<Option<i64>> {
    blocking(move || stop(&app)).await
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
