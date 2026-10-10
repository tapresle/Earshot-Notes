use std::sync::Arc;

use tauri::State;

use super::{CommandResult, blocking};
use crate::llm::worker::ModelWorker;
use crate::store::{ChatMessage, Store};

/// Asks a question about the meeting. The answer streams as `chat:text`
/// events and arrives, saved with the question, as `chat:done`.
#[tauri::command]
pub async fn ask_meeting(
    store: State<'_, Arc<Store>>,
    worker: State<'_, Arc<ModelWorker>>,
    meeting_id: i64,
    question: String,
) -> CommandResult<()> {
    let (store, worker) = (Arc::clone(&store), Arc::clone(&worker));
    blocking(move || Ok(worker.ask(&store, meeting_id, &question)?)).await
}

#[tauri::command]
pub fn cancel_chat(worker: State<'_, Arc<ModelWorker>>) -> bool {
    worker.cancel_chat()
}

/// The meeting whose question is being answered, if any.
#[tauri::command]
pub fn active_chat(worker: State<'_, Arc<ModelWorker>>) -> Option<i64> {
    worker.active_chat()
}

/// The meeting's saved questions and answers, oldest first.
#[tauri::command]
pub async fn chat_messages(
    store: State<'_, Arc<Store>>,
    meeting_id: i64,
) -> CommandResult<Vec<ChatMessage>> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.chat_messages(meeting_id)?)).await
}
