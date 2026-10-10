use std::sync::Arc;

use tauri::State;

use super::{CommandResult, blocking};
use crate::store::{ChatMessage, Store};
use crate::summarize::service::Summarizer;

/// Asks a question about the meeting. The answer streams as `chat:text`
/// events and arrives, saved with the question, as `chat:done`.
#[tauri::command]
pub async fn ask_meeting(
    store: State<'_, Arc<Store>>,
    summarizer: State<'_, Arc<Summarizer>>,
    meeting_id: i64,
    question: String,
) -> CommandResult<()> {
    let (store, summarizer) = (Arc::clone(&store), Arc::clone(&summarizer));
    blocking(move || Ok(summarizer.ask(&store, meeting_id, &question)?)).await
}

#[tauri::command]
pub fn cancel_chat(summarizer: State<'_, Arc<Summarizer>>) -> bool {
    summarizer.cancel_chat()
}

/// The meeting whose question is being answered, if any.
#[tauri::command]
pub fn active_chat(summarizer: State<'_, Arc<Summarizer>>) -> Option<i64> {
    summarizer.active_chat()
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
