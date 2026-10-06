//! The audio kept from recordings: the setting, its size, and deleting it.

use std::sync::Arc;

use tauri::State;

use super::{CommandError, CommandResult, ErrorKind, blocking, open_with_system};
use crate::recording::Recorder;
use crate::recordings::Recordings;
use crate::store::Store;
use crate::store::settings::{KEEP_AUDIO, ON};

fn io_error(what: &str, error: std::io::Error) -> CommandError {
    CommandError {
        kind: ErrorKind::Internal,
        message: format!("couldn't {what}: {error}"),
    }
}

/// Whether the next recording keeps its audio.
#[tauri::command]
pub async fn keep_audio(store: State<'_, Arc<Store>>) -> CommandResult<bool> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.setting(KEEP_AUDIO)?.as_deref() == Some(ON))).await
}

/// Turns keeping audio on or off for future recordings. Turning it off keeps
/// the recordings that already exist.
#[tauri::command]
pub async fn set_keep_audio(store: State<'_, Arc<Store>>, enabled: bool) -> CommandResult<()> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.set_setting(KEEP_AUDIO, enabled.then_some(ON))?)).await
}

/// The total size of the kept recordings, in bytes.
#[tauri::command]
pub async fn recordings_size(recordings: State<'_, Arc<Recordings>>) -> CommandResult<u64> {
    let recordings = Arc::clone(&recordings);
    blocking(move || {
        recordings
            .total_size()
            .map_err(|e| io_error("measure the recordings", e))
    })
    .await
}

fn delete_all_in(recorder: &Recorder, recordings: &Recordings) -> CommandResult<u64> {
    if recorder.active_meeting().is_some() {
        return Err(CommandError {
            kind: ErrorKind::Conflict,
            message: "stop recording to delete recordings".into(),
        });
    }
    recordings
        .delete_all()
        .map_err(|e| io_error("delete the recordings", e))?;
    log::info!("deleted every kept recording");
    recordings
        .total_size()
        .map_err(|e| io_error("measure the recordings", e))
}

/// Deletes every kept recording and returns the size left, which is 0 unless
/// something was added meanwhile. Refused while a recording is running.
#[tauri::command]
pub async fn delete_all_recordings(
    recorder: State<'_, Arc<Recorder>>,
    recordings: State<'_, Arc<Recordings>>,
) -> CommandResult<u64> {
    let (recorder, recordings) = (Arc::clone(&recorder), Arc::clone(&recordings));
    blocking(move || delete_all_in(&recorder, &recordings)).await
}

/// Opens the recordings folder in Finder, creating it if needed.
#[tauri::command]
pub async fn show_recordings(recordings: State<'_, Arc<Recordings>>) -> CommandResult<()> {
    let recordings = Arc::clone(&recordings);
    blocking(move || {
        let root = recordings.root();
        std::fs::create_dir_all(root).map_err(|e| io_error("create the recordings folder", e))?;
        open_with_system(&root.to_string_lossy(), "the recordings folder")
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::{CaptureStarter, IssueSink, start_recording, stop_recording};
    use crate::store::Source;
    use crate::transcribe::worker::TranscriptionWorker;

    fn keep_one(recordings: &Recordings) {
        let mut kept = recordings.keep(1, 0, Source::Me).unwrap();
        kept.push(&[0.1; 1600]);
        kept.finish();
    }

    #[test]
    fn deletes_every_recording_and_reports_zero() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = Recordings::new(dir.path().join("recordings"));
        keep_one(&recordings);
        assert!(recordings.total_size().unwrap() > 0);

        assert_eq!(delete_all_in(&Recorder::default(), &recordings).unwrap(), 0);
        assert!(!recordings.root().exists());
    }

    #[test]
    fn refuses_to_delete_recordings_while_recording() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = Recordings::new(dir.path().join("recordings"));
        keep_one(&recordings);
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Live").unwrap();
        let recorder = Recorder::default();
        let (worker, _jobs) = TranscriptionWorker::from_queue();
        // A capture that stays open until the recording stops.
        let starter: CaptureStarter = Box::new(|tx, _| {
            Ok((
                Box::new(HeldCapture(tx)),
                crate::audio::CaptureFormat {
                    sample_rate: 48_000,
                    channels: 1,
                },
            ))
        });
        let issues: IssueSink = Arc::new(|_, _| {});
        start_recording(
            &store,
            &recorder,
            Ok(&worker),
            meeting.id,
            vec![(Source::Me, starter)],
            issues,
            &recordings,
        )
        .unwrap();

        let error = delete_all_in(&recorder, &recordings).unwrap_err();

        assert_eq!(error.kind, ErrorKind::Conflict);
        assert!(recordings.file_path(1, 0, Source::Me).exists());
        stop_recording(&store, &recorder).unwrap();
    }

    struct HeldCapture(#[allow(dead_code)] std::sync::mpsc::SyncSender<crate::audio::Buffer>);
    impl crate::audio::Capture for HeldCapture {}
}
