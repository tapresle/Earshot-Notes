use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use super::models::HuggingFaceSource;
use super::models::models_dir;
use super::{CommandError, CommandResult, ErrorKind, blocking};
use crate::audio::mic::{self, InputDevice};
use crate::chunker::Sensitivity;
use crate::llm::resident::KeepLoaded;
use crate::llm::worker::ModelWorker;
use crate::models::{self, RemoveError};
use crate::store::settings::{
    INPUT_DEVICE, KEEP_MODEL_LOADED, RECORD_NEW_MEETINGS, SUMMARIZE_ON_STOP, sensitivity_key,
};
use crate::store::{ModelRecord, Source, Store};
use macos_audio::LoginItem;

/// The microphones connected now. Listing can take a moment, so it runs off the main thread.
#[tauri::command]
pub async fn input_devices() -> CommandResult<Vec<InputDevice>> {
    tauri::async_runtime::spawn_blocking(mic::input_devices)
        .await
        .map_err(|e| CommandError {
            kind: ErrorKind::Internal,
            message: e.to_string(),
        })?
        .map_err(|e| CommandError {
            kind: ErrorKind::Unavailable,
            message: e.to_string(),
        })
}

/// The chosen microphone's id, or null for the system default.
#[tauri::command]
pub async fn input_device(store: State<'_, Arc<Store>>) -> CommandResult<Option<String>> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.setting(INPUT_DEVICE)?)).await
}

/// Chooses the microphone for future recordings; null means the system default.
#[tauri::command]
pub async fn set_input_device(
    store: State<'_, Arc<Store>>,
    device_id: Option<String>,
) -> CommandResult<()> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.set_setting(INPUT_DEVICE, device_id.as_deref())?)).await
}

/// Whether Earshot Notes opens at login. macOS keeps this, not the settings
/// table, so it stays right when you change it in System Settings.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchAtLogin {
    enabled: bool,
    /// Turned on, but waiting for approval in System Settings > General > Login Items.
    needs_approval: bool,
    /// False on a platform without login items.
    available: bool,
}

impl From<LoginItem> for LaunchAtLogin {
    fn from(item: LoginItem) -> Self {
        LaunchAtLogin {
            enabled: matches!(item, LoginItem::On | LoginItem::NeedsApproval),
            needs_approval: item == LoginItem::NeedsApproval,
            available: item != LoginItem::Unavailable,
        }
    }
}

#[tauri::command]
pub async fn launch_at_login() -> CommandResult<LaunchAtLogin> {
    blocking(|| Ok(macos_audio::login_item().into())).await
}

/// Turns opening at login on or off, and returns the state macOS reports after.
#[tauri::command]
pub async fn set_launch_at_login(enabled: bool) -> CommandResult<LaunchAtLogin> {
    blocking(move || {
        macos_audio::set_login_item(enabled).map_err(|error| CommandError {
            kind: ErrorKind::Unavailable,
            message: format!("couldn't change opening at login: {error}"),
        })?;
        let item = macos_audio::login_item();
        log::info!(
            "opening at login turned {}; macOS reports {item:?}",
            if enabled { "on" } else { "off" }
        );
        Ok(item.into())
    })
    .await
}

/// How readily quiet audio counts as speech, for each side of the call.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSensitivity {
    me: Sensitivity,
    them: Sensitivity,
}

fn speech_sensitivity_in(store: &Store) -> CommandResult<SpeechSensitivity> {
    let level = |source| -> CommandResult<Sensitivity> {
        Ok(Sensitivity::from_setting(
            store.setting(sensitivity_key(source))?.as_deref(),
        ))
    };
    Ok(SpeechSensitivity {
        me: level(Source::Me)?,
        them: level(Source::Them)?,
    })
}

fn set_speech_sensitivity_in(
    store: &Store,
    source: Source,
    level: Sensitivity,
) -> CommandResult<()> {
    Ok(store.set_setting(sensitivity_key(source), Some(level.as_setting()))?)
}

/// The speech sensitivity the next recording uses for each source.
#[tauri::command]
pub async fn speech_sensitivity(store: State<'_, Arc<Store>>) -> CommandResult<SpeechSensitivity> {
    let store = Arc::clone(&store);
    blocking(move || speech_sensitivity_in(&store)).await
}

/// Sets a source's speech sensitivity for future recordings.
#[tauri::command]
pub async fn set_speech_sensitivity(
    store: State<'_, Arc<Store>>,
    source: Source,
    level: Sensitivity,
) -> CommandResult<()> {
    let store = Arc::clone(&store);
    blocking(move || set_speech_sensitivity_in(&store, source, level)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModel {
    #[serde(flatten)]
    model: ModelRecord,
    /// Where the model comes from, when it's one the app knows.
    hugging_face: Option<HuggingFaceSource>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModels {
    models: Vec<InstalledModel>,
    /// The model summaries use, if any.
    selected_id: Option<String>,
}

#[tauri::command]
pub async fn installed_models(store: State<'_, Arc<Store>>) -> CommandResult<InstalledModels> {
    let store = Arc::clone(&store);
    blocking(move || installed(&store)).await
}

fn installed(store: &Store) -> CommandResult<InstalledModels> {
    let mut models = store.list_models()?;
    models::sort_for_display(&mut models);
    Ok(InstalledModels {
        models: models
            .into_iter()
            .map(|model| InstalledModel {
                hugging_face: models::known_by_id(&model.id).map(Into::into),
                model,
            })
            .collect(),
        selected_id: store.selected_model()?.map(|m| m.id),
    })
}

/// Selects the model for summaries, and frees the one loaded before.
#[tauri::command]
pub async fn select_model(
    store: State<'_, Arc<Store>>,
    worker: State<'_, Arc<ModelWorker>>,
    model_id: String,
) -> CommandResult<()> {
    let (store, worker) = (Arc::clone(&store), Arc::clone(&worker));
    blocking(move || {
        store.select_model(&model_id)?;
        worker.unload();
        Ok(())
    })
    .await
}

/// Whether **New meeting** in the window also starts recording.
#[tauri::command]
pub async fn record_new_meetings(store: State<'_, Arc<Store>>) -> CommandResult<bool> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.switch(RECORD_NEW_MEETINGS)?)).await
}

/// Turns starting a recording with each new meeting on or off.
#[tauri::command]
pub async fn set_record_new_meetings(
    store: State<'_, Arc<Store>>,
    enabled: bool,
) -> CommandResult<()> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.set_switch(RECORD_NEW_MEETINGS, enabled)?)).await
}

/// Whether stopping a recording summarizes the meeting.
#[tauri::command]
pub async fn summarize_on_stop(store: State<'_, Arc<Store>>) -> CommandResult<bool> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.switch(SUMMARIZE_ON_STOP)?)).await
}

/// Turns summarizing each meeting when its recording stops on or off.
#[tauri::command]
pub async fn set_summarize_on_stop(
    store: State<'_, Arc<Store>>,
    enabled: bool,
) -> CommandResult<()> {
    let store = Arc::clone(&store);
    blocking(move || Ok(store.set_switch(SUMMARIZE_ON_STOP, enabled)?)).await
}

fn keep_model_loaded_in(store: &Store) -> CommandResult<KeepLoaded> {
    Ok(KeepLoaded::from_setting(
        store.setting(KEEP_MODEL_LOADED)?.as_deref(),
    ))
}

/// How long the summary model stays loaded after a request.
#[tauri::command]
pub async fn keep_model_loaded(store: State<'_, Arc<Store>>) -> CommandResult<KeepLoaded> {
    let store = Arc::clone(&store);
    blocking(move || keep_model_loaded_in(&store)).await
}

/// Changes how long the summary model stays loaded, starting now.
#[tauri::command]
pub async fn set_keep_model_loaded(
    store: State<'_, Arc<Store>>,
    worker: State<'_, Arc<ModelWorker>>,
    keep: KeepLoaded,
) -> CommandResult<()> {
    let (store, worker) = (Arc::clone(&store), Arc::clone(&worker));
    blocking(move || {
        store.set_setting(KEEP_MODEL_LOADED, Some(keep.as_setting()))?;
        worker.keep_loaded(keep);
        Ok(())
    })
    .await
}

impl From<RemoveError> for CommandError {
    fn from(error: RemoveError) -> Self {
        match error {
            RemoveError::Store(store) => store.into(),
            RemoveError::Io(_) => Self {
                kind: ErrorKind::Unavailable,
                message: error.to_string(),
            },
        }
    }
}

/// Deletes an installed summary model to free disk space. Refused while a
/// summary is running. If it was the selected model, summaries switch to the
/// newest remaining one.
#[tauri::command]
pub async fn remove_model(
    app: AppHandle,
    store: State<'_, Arc<Store>>,
    worker: State<'_, Arc<ModelWorker>>,
    model_id: String,
) -> CommandResult<()> {
    let dir = models_dir(&app)?;
    let (store, worker) = (Arc::clone(&store), Arc::clone(&worker));
    blocking(move || {
        worker.when_idle(|| {
            worker.unload();
            models::remove(&model_id, &dir, &store)
        })??;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ModelSource;

    fn record(id: &str) -> ModelRecord {
        ModelRecord {
            id: id.into(),
            display_name: "Model".into(),
            file_path: "/models/m.gguf".into(),
            size_bytes: 1,
            sha256: "abc".into(),
            source: ModelSource::Import,
            source_url: None,
            version: None,
            min_runtime: None,
            added_at: 0,
        }
    }

    #[test]
    fn installed_models_carry_their_hugging_face_source_when_known() {
        let known = InstalledModel {
            hugging_face: models::known_by_id("qwen3.5-9b-q4_k_m").map(Into::into),
            model: record("qwen3.5-9b-q4_k_m"),
        };
        let imported = InstalledModel {
            hugging_face: models::known_by_id("local-abc").map(Into::into),
            model: record("local-abc"),
        };

        let known = serde_json::to_value(known).unwrap();
        assert_eq!(known["displayName"], "Model");
        assert_eq!(
            known["huggingFace"],
            serde_json::json!({
                "repo": "unsloth/Qwen3.5-9B-GGUF",
                "quantization": "Q4_K_M",
                "revision": "99a1b2185534379e6e8b5ec869da25d3e7b3f73c",
            })
        );
        assert_eq!(
            serde_json::to_value(imported).unwrap()["huggingFace"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn speech_sensitivity_defaults_to_normal_and_keeps_what_is_set() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_setting(sensitivity_key(Source::Them), Some("unknown"))
            .unwrap();
        assert_eq!(
            serde_json::to_value(speech_sensitivity_in(&store).unwrap()).unwrap(),
            serde_json::json!({ "me": "normal", "them": "normal" })
        );

        set_speech_sensitivity_in(&store, Source::Me, Sensitivity::High).unwrap();
        set_speech_sensitivity_in(&store, Source::Them, Sensitivity::Low).unwrap();

        assert_eq!(
            speech_sensitivity_in(&store).unwrap(),
            SpeechSensitivity {
                me: Sensitivity::High,
                them: Sensitivity::Low,
            }
        );
    }

    #[test]
    fn refuses_an_unknown_sensitivity_level_from_the_ui() {
        assert_eq!(
            serde_json::from_value::<Sensitivity>(serde_json::json!("high")).unwrap(),
            Sensitivity::High
        );
        assert!(serde_json::from_value::<Sensitivity>(serde_json::json!("max")).is_err());
        assert_eq!(
            serde_json::from_value::<Source>(serde_json::json!("them")).unwrap(),
            Source::Them
        );
    }

    #[test]
    fn reports_opening_at_login_from_the_macos_status() {
        let off = LaunchAtLogin::from(LoginItem::Off);
        assert_eq!(
            (off.enabled, off.needs_approval, off.available),
            (false, false, true)
        );
        let on = LaunchAtLogin::from(LoginItem::On);
        assert_eq!(
            (on.enabled, on.needs_approval, on.available),
            (true, false, true)
        );
        let pending = LaunchAtLogin::from(LoginItem::NeedsApproval);
        assert_eq!(
            (pending.enabled, pending.needs_approval, pending.available),
            (true, true, true)
        );
        let unavailable = LaunchAtLogin::from(LoginItem::Unavailable);
        assert_eq!((unavailable.enabled, unavailable.available), (false, false));
        assert_eq!(
            serde_json::to_value(pending).unwrap(),
            serde_json::json!({ "enabled": true, "needsApproval": true, "available": true })
        );
    }

    #[test]
    fn keeps_the_model_loaded_five_minutes_until_you_choose_otherwise() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            keep_model_loaded_in(&store).unwrap(),
            KeepLoaded::FiveMinutes
        );

        store
            .set_setting(KEEP_MODEL_LOADED, Some(KeepLoaded::Always.as_setting()))
            .unwrap();

        assert_eq!(keep_model_loaded_in(&store).unwrap(), KeepLoaded::Always);
        assert_eq!(
            serde_json::to_value(KeepLoaded::FifteenMinutes).unwrap(),
            serde_json::json!("fifteenMinutes")
        );
    }
}
