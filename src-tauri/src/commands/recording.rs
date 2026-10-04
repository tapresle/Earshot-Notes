use std::sync::Arc;

use tauri::State;

use super::CommandResult;
use crate::audio::{Capture, mic};
use crate::recording::{self, Recorder};
use crate::store::{Store, TranscriptSegment};
use crate::transcribe::worker::TranscriptionWorker;

/// The transcription worker, or why it couldn't start.
pub struct Transcription(pub Result<TranscriptionWorker, String>);

#[tauri::command]
pub fn start_recording(
    store: State<'_, Arc<Store>>,
    recorder: State<'_, Recorder>,
    transcription: State<'_, Transcription>,
    meeting_id: i64,
) -> CommandResult<()> {
    recording::start_recording(
        &store,
        &recorder,
        transcription.0.as_ref().map_err(String::as_str),
        meeting_id,
        |frames| {
            mic::start(frames)
                .map(|(capture, format)| (Box::new(capture) as Box<dyn Capture>, format))
        },
    )?;
    Ok(())
}

#[tauri::command]
pub fn stop_recording(
    store: State<'_, Arc<Store>>,
    recorder: State<'_, Recorder>,
) -> CommandResult<Option<i64>> {
    Ok(recording::stop_recording(&store, &recorder)?)
}

#[tauri::command]
pub fn active_recording(recorder: State<'_, Recorder>) -> Option<i64> {
    recorder.active_meeting()
}

#[tauri::command]
pub fn list_transcript(
    store: State<'_, Arc<Store>>,
    meeting_id: i64,
) -> CommandResult<Vec<TranscriptSegment>> {
    Ok(store.list_segments(meeting_id)?)
}
