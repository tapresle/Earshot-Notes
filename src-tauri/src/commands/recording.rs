use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use super::{CommandError, CommandResult, ErrorKind};
use crate::audio::{Capture, mic, system};
use crate::recording::{self, CaptureStarter, IssueSink, Recorder, SourceProblem, StartReport};
use crate::store::{Source, Store, TranscriptSegment};
use crate::transcribe::worker::TranscriptionWorker;

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
            eprintln!("couldn't send a recording issue to the UI: {error}");
        }
    });
    let sources: Vec<(Source, CaptureStarter)> = vec![
        (
            Source::Me,
            Box::new(|frames, on_error| {
                mic::start(frames, on_error).map(|(c, f)| (Box::new(c) as Box<dyn Capture>, f))
            }),
        ),
        (
            Source::Them,
            Box::new(|frames, on_error| {
                system::start(frames, on_error).map(|(c, f)| (Box::new(c) as Box<dyn Capture>, f))
            }),
        ),
    ];

    tauri::async_runtime::spawn_blocking(move || {
        recording::start_recording(
            &store,
            &recorder,
            transcription.0.as_ref().map_err(String::as_str),
            meeting_id,
            sources,
            on_issue,
        )
    })
    .await
    .map_err(|error| CommandError {
        kind: ErrorKind::Internal,
        message: format!("the recording task failed: {error}"),
    })?
    .map_err(Into::into)
}

#[tauri::command]
pub fn stop_recording(
    store: State<'_, Arc<Store>>,
    recorder: State<'_, Arc<Recorder>>,
) -> CommandResult<Option<i64>> {
    Ok(recording::stop_recording(&store, &recorder)?)
}

#[tauri::command]
pub fn active_recording(recorder: State<'_, Arc<Recorder>>) -> Option<i64> {
    recorder.active_meeting()
}

#[tauri::command]
pub fn list_transcript(
    store: State<'_, Arc<Store>>,
    meeting_id: i64,
) -> CommandResult<Vec<TranscriptSegment>> {
    Ok(store.list_segments(meeting_id)?)
}
