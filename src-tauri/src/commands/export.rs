use std::path::{Path, PathBuf};
use std::sync::Arc;

use jiff::tz::TimeZone;
use tauri::State;

use super::{CommandError, CommandResult, ErrorKind, blocking};
use crate::export::{self, ExportReport};
use crate::store::{ExportMeeting, Store, StoreError};

fn one(store: &Store, meeting_id: i64) -> Result<ExportMeeting, CommandError> {
    store
        .export_meetings(Some(&[meeting_id]))?
        .pop()
        .ok_or_else(|| StoreError::MeetingNotFound(meeting_id).into())
}

/// The suggested file name for exporting one meeting.
#[tauri::command]
pub async fn export_file_name(
    store: State<'_, Arc<Store>>,
    meeting_id: i64,
) -> CommandResult<String> {
    let store = Arc::clone(&store);
    blocking(move || {
        Ok(export::file_name(
            &one(&store, meeting_id)?,
            &TimeZone::system(),
        ))
    })
    .await
}

/// Writes one meeting to the path the user chose in the save dialog.
/// Returns the path written, with `.md` added if it was missing.
#[tauri::command]
pub async fn export_meeting(
    store: State<'_, Arc<Store>>,
    meeting_id: i64,
    path: String,
) -> CommandResult<String> {
    let store = Arc::clone(&store);
    blocking(move || {
        let mut path = PathBuf::from(path);
        if path
            .extension()
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("md"))
        {
            path.as_mut_os_string().push(".md");
        }
        let meeting = one(&store, meeting_id)?;
        export::write_meeting(&meeting, &path, &TimeZone::system()).map_err(|e| CommandError {
            kind: ErrorKind::Unavailable,
            message: format!("couldn't write {}: {e}", path.display()),
        })?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
}

/// Writes one file per meeting into a folder: the given meetings, or all of
/// them when `meeting_ids` is null.
#[tauri::command]
pub async fn export_meetings(
    store: State<'_, Arc<Store>>,
    meeting_ids: Option<Vec<i64>>,
    folder: String,
) -> CommandResult<ExportReport> {
    let store = Arc::clone(&store);
    blocking(move || {
        let folder = Path::new(&folder);
        if !folder.is_dir() {
            return Err(CommandError {
                kind: ErrorKind::Invalid,
                message: format!("{} isn't a folder", folder.display()),
            });
        }
        let meetings = store.export_meetings(meeting_ids.as_deref())?;
        Ok(export::write_meetings(
            &meetings,
            folder,
            &TimeZone::system(),
        ))
    })
    .await
}
