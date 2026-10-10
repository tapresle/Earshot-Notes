use std::sync::Arc;

use tauri::State;

use super::{CommandResult, blocking};
use crate::llm::worker::ModelWorker;
use crate::recording::Recorder;
use crate::store::{Store, Summary};

/// Summarizes the meeting as it stands. While it's recording, the summary
/// covers the transcript so far and is marked partial.
#[tauri::command]
pub async fn summarize_meeting(
    store: State<'_, Arc<Store>>,
    recorder: State<'_, Arc<Recorder>>,
    worker: State<'_, Arc<ModelWorker>>,
    meeting_id: i64,
) -> CommandResult<()> {
    let is_final = recorder.active_meeting() != Some(meeting_id);
    let (store, worker) = (Arc::clone(&store), Arc::clone(&worker));
    blocking(move || Ok(worker.summarize(&store, meeting_id, is_final)?)).await
}

#[tauri::command]
pub fn cancel_summary(worker: State<'_, Arc<ModelWorker>>) -> bool {
    worker.cancel_summary()
}

#[tauri::command]
pub fn active_summary(worker: State<'_, Arc<ModelWorker>>) -> Option<i64> {
    worker.active_summary()
}

#[tauri::command]
pub async fn latest_summary(
    store: State<'_, Arc<Store>>,
    meeting_id: i64,
) -> CommandResult<Option<Summary>> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.latest_summary(meeting_id)?)).await
}
