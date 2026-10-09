//! The transcription model choice: Whistle, bundled, or Whisper large-v3-turbo,
//! downloaded into the models folder (#25).
//!
//! Whisper isn't recorded in the models table like summary models: it's
//! installed when its verified file is in the models folder. The download
//! writes to a `.part` file and renames it only after checking the size and
//! checksum, so a file at the final path is always a verified one.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::WHISPER_TURBO;
use super::download::partial_path;
use crate::store::settings::TRANSCRIPTION_MODEL;
use crate::store::{Store, StoreError};
use crate::transcribe::worker::Engine;

/// The id of the bundled model, which is the default.
pub const WHISTLE_ID: &str = "whistle";

#[derive(Debug, thiserror::Error)]
pub enum SelectError {
    #[error("there's no transcription model called {0}")]
    Unknown(String),
    #[error("download {0} before choosing it")]
    NotInstalled(&'static str),
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    #[error("couldn't delete the model file: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Where Whisper's verified file lives.
pub fn whisper_path(models_dir: &Path) -> PathBuf {
    models_dir.join(WHISPER_TURBO.file_name)
}

pub fn whisper_installed(models_dir: &Path) -> bool {
    whisper_path(models_dir).is_file()
}

/// Bytes saved by an earlier, unfinished Whisper download.
pub fn whisper_partial_bytes(models_dir: &Path) -> u64 {
    std::fs::metadata(partial_path(&whisper_path(models_dir)))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// The id of the chosen model. Whisper counts only while its file is there,
/// so a deleted file falls back to Whistle.
pub fn selected_id(store: &Store, models_dir: &Path) -> Result<&'static str, StoreError> {
    let chosen = store.setting(TRANSCRIPTION_MODEL)?;
    Ok(
        if chosen.as_deref() == Some(WHISPER_TURBO.id) && whisper_installed(models_dir) {
            WHISPER_TURBO.id
        } else {
            WHISTLE_ID
        },
    )
}

/// The engine a recording starting now transcribes with.
pub fn selected_engine(store: &Store, models_dir: &Path) -> Result<Engine, StoreError> {
    Ok(match selected_id(store, models_dir)? {
        id if id == WHISPER_TURBO.id => Engine::Whisper(Arc::new(whisper_path(models_dir))),
        _ => Engine::Whistle,
    })
}

/// Chooses the model for recordings started from now on. Whisper must be
/// downloaded first.
pub fn select(store: &Store, models_dir: &Path, id: &str) -> Result<(), SelectError> {
    let value = match id {
        WHISTLE_ID => None,
        id if id == WHISPER_TURBO.id => {
            if !whisper_installed(models_dir) {
                return Err(SelectError::NotInstalled(WHISPER_TURBO.display_name));
            }
            Some(WHISPER_TURBO.id)
        }
        other => return Err(SelectError::Unknown(other.to_owned())),
    };
    Ok(store.set_setting(TRANSCRIPTION_MODEL, value)?)
}

/// Deletes Whisper's file, and any partial download, and goes back to Whistle.
/// A recording in progress keeps the model it loaded until it's idle.
pub fn remove_whisper(store: &Store, models_dir: &Path) -> Result<(), RemoveError> {
    store.set_setting(TRANSCRIPTION_MODEL, None)?;
    let path = whisper_path(models_dir);
    for file in [partial_path(&path), path] {
        match std::fs::remove_file(&file) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_whisper(dir: &Path) {
        std::fs::write(whisper_path(dir), b"verified weights").unwrap();
    }

    #[test]
    fn defaults_to_whistle() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();

        assert_eq!(selected_id(&store, dir.path()).unwrap(), WHISTLE_ID);
        assert_eq!(
            selected_engine(&store, dir.path()).unwrap(),
            Engine::Whistle
        );
    }

    #[test]
    fn selects_whisper_once_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        install_whisper(dir.path());

        select(&store, dir.path(), WHISPER_TURBO.id).unwrap();

        assert_eq!(
            selected_engine(&store, dir.path()).unwrap(),
            Engine::Whisper(Arc::new(whisper_path(dir.path())))
        );
    }

    #[test]
    fn refuses_whisper_before_it_is_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();

        let result = select(&store, dir.path(), WHISPER_TURBO.id);

        assert!(matches!(result, Err(SelectError::NotInstalled(_))));
        assert_eq!(selected_id(&store, dir.path()).unwrap(), WHISTLE_ID);
    }

    #[test]
    fn refuses_an_unknown_model() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();

        assert!(matches!(
            select(&store, dir.path(), "../model.bin"),
            Err(SelectError::Unknown(_))
        ));
    }

    #[test]
    fn falls_back_to_whistle_when_the_whisper_file_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        install_whisper(dir.path());
        select(&store, dir.path(), WHISPER_TURBO.id).unwrap();

        std::fs::remove_file(whisper_path(dir.path())).unwrap();

        assert_eq!(
            selected_engine(&store, dir.path()).unwrap(),
            Engine::Whistle
        );
    }

    #[test]
    fn removing_whisper_deletes_its_files_and_selects_whistle() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        install_whisper(dir.path());
        std::fs::write(partial_path(&whisper_path(dir.path())), b"part").unwrap();
        select(&store, dir.path(), WHISPER_TURBO.id).unwrap();

        remove_whisper(&store, dir.path()).unwrap();

        assert!(!whisper_installed(dir.path()));
        assert_eq!(whisper_partial_bytes(dir.path()), 0);
        assert_eq!(store.setting(TRANSCRIPTION_MODEL).unwrap(), None);
        // Removing again, with nothing there, is fine.
        remove_whisper(&store, dir.path()).unwrap();
    }
}
