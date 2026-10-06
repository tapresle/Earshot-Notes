use std::sync::Arc;

use tauri::State;

use super::{CommandError, CommandResult, ErrorKind, blocking};
use crate::recording::Recorder;
use crate::recordings::Recordings;
use crate::store::{Meeting, MeetingCursor, MeetingDetail, MeetingPage, Store};

#[tauri::command]
pub async fn create_meeting(store: State<'_, Arc<Store>>, title: String) -> CommandResult<Meeting> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.create_meeting(&title)?)).await
}

/// One page of meetings, newest first, limited to those that match `query`
/// when it has any words. Pass the previous page's `next` as `after` to get
/// the page that follows it.
#[tauri::command]
pub async fn list_meetings(
    store: State<'_, Arc<Store>>,
    query: Option<String>,
    after: Option<MeetingCursor>,
    limit: u32,
) -> CommandResult<MeetingPage> {
    let store = Arc::clone(&store);
    blocking(move || {
        let query = query.as_deref().unwrap_or_default();
        Ok(store.search_meetings(query, after, limit)?)
    })
    .await
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

fn delete_meeting_in(
    store: &Store,
    recorder: &Recorder,
    recordings: &Recordings,
    id: i64,
) -> CommandResult<()> {
    if recorder.active_meeting() == Some(id) {
        return Err(CommandError {
            kind: ErrorKind::Conflict,
            message: "stop recording before deleting this meeting".into(),
        });
    }
    store.delete_meeting(id)?;
    recordings.delete_meeting(id).map_err(|error| {
        log::error!("deleted meeting {id}, but not its kept recordings: {error}");
        CommandError {
            kind: ErrorKind::Internal,
            message: format!(
                "the meeting was deleted, but its audio recordings couldn't be: {error}"
            ),
        }
    })
}

/// Deletes a meeting and any audio kept from its recordings.
#[tauri::command]
pub async fn delete_meeting(
    store: State<'_, Arc<Store>>,
    recorder: State<'_, Arc<Recorder>>,
    recordings: State<'_, Arc<Recordings>>,
    id: i64,
) -> CommandResult<()> {
    let (store, recorder, recordings) = (
        Arc::clone(&store),
        Arc::clone(&recorder),
        Arc::clone(&recordings),
    );
    blocking(move || delete_meeting_in(&store, &recorder, &recordings, id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Source;

    #[test]
    fn deleting_a_meeting_deletes_its_kept_recordings() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = Recordings::new(dir.path().to_path_buf());
        let store = Store::open_in_memory().unwrap();
        let (gone, kept) = (
            store.create_meeting("Gone").unwrap(),
            store.create_meeting("Kept").unwrap(),
        );
        for id in [gone.id, kept.id] {
            recordings.keep(id, 0, Source::Me).unwrap().finish();
        }

        delete_meeting_in(&store, &Recorder::default(), &recordings, gone.id).unwrap();

        assert!(!recordings.file_path(gone.id, 0, Source::Me).exists());
        assert!(recordings.file_path(kept.id, 0, Source::Me).exists());
    }
}
