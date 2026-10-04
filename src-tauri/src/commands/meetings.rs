use tauri::State;

use super::CommandResult;
use crate::store::{Meeting, MeetingDetail, Store};

#[tauri::command]
pub fn create_meeting(store: State<'_, Store>, title: String) -> CommandResult<Meeting> {
    Ok(store.create_meeting(&title)?)
}

#[tauri::command]
pub fn list_meetings(store: State<'_, Store>) -> CommandResult<Vec<Meeting>> {
    Ok(store.list_meetings()?)
}

#[tauri::command]
pub fn get_meeting(store: State<'_, Store>, id: i64) -> CommandResult<MeetingDetail> {
    Ok(store.get_meeting(id)?)
}

#[tauri::command]
pub fn rename_meeting(store: State<'_, Store>, id: i64, title: String) -> CommandResult<String> {
    Ok(store.rename_meeting(id, &title)?)
}

#[tauri::command]
pub fn update_notes(store: State<'_, Store>, id: i64, body_markdown: String) -> CommandResult<i64> {
    Ok(store.update_notes(id, &body_markdown)?)
}

#[tauri::command]
pub fn delete_meeting(store: State<'_, Store>, id: i64) -> CommandResult<()> {
    Ok(store.delete_meeting(id)?)
}
