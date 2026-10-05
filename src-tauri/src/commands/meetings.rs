use std::sync::Arc;

use tauri::State;

use super::{CommandError, CommandResult, ErrorKind, blocking};
use crate::recording::Recorder;
use crate::store::{Meeting, MeetingDetail, Store};

#[tauri::command]
pub async fn create_meeting(store: State<'_, Arc<Store>>, title: String) -> CommandResult<Meeting> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.create_meeting(&title)?)).await
}

#[tauri::command]
pub async fn list_meetings(store: State<'_, Arc<Store>>) -> CommandResult<Vec<Meeting>> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.list_meetings()?)).await
}

#[tauri::command]
pub async fn get_meeting(store: State<'_, Arc<Store>>, id: i64) -> CommandResult<MeetingDetail> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.get_meeting(id)?)).await
}

#[tauri::command]
pub async fn rename_meeting(
    store: State<'_, Arc<Store>>,
    id: i64,
    title: String,
) -> CommandResult<String> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.rename_meeting(id, &title)?)).await
}

#[tauri::command]
pub async fn update_notes(
    store: State<'_, Arc<Store>>,
    id: i64,
    body_markdown: String,
) -> CommandResult<i64> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.update_notes(id, &body_markdown)?)).await
}

#[tauri::command]
pub async fn delete_meeting(
    store: State<'_, Arc<Store>>,
    recorder: State<'_, Arc<Recorder>>,
    id: i64,
) -> CommandResult<()> {
    if recorder.active_meeting() == Some(id) {
        return Err(CommandError {
            kind: ErrorKind::Conflict,
            message: "stop recording before deleting this meeting".into(),
        });
    }
    let store = Arc::clone(&store);
    blocking(move || Ok(store.delete_meeting(id)?)).await
}
