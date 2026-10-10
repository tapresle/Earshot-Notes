//! Keeps the summary model loaded between requests, and decides when to free it.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::generator::{GenerateError, Generator};
use crate::store::ModelRecord;

/// How long the model stays loaded after a request. Loading takes seconds,
/// so keeping it saves that wait on the next request, at the cost of the
/// model's memory while it's idle.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeepLoaded {
    /// Freed as soon as each request ends.
    Never,
    #[default]
    FiveMinutes,
    FifteenMinutes,
    /// Freed only when you switch or remove the model, or quit.
    Always,
}

impl KeepLoaded {
    /// Reads a stored value. A missing or unknown value is `FiveMinutes`.
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("never") => KeepLoaded::Never,
            Some("15m") => KeepLoaded::FifteenMinutes,
            Some("always") => KeepLoaded::Always,
            _ => KeepLoaded::FiveMinutes,
        }
    }

    /// The value stored for this choice.
    pub fn as_setting(self) -> &'static str {
        match self {
            KeepLoaded::Never => "never",
            KeepLoaded::FiveMinutes => "5m",
            KeepLoaded::FifteenMinutes => "15m",
            KeepLoaded::Always => "always",
        }
    }

    /// How long an idle model stays loaded; `None` keeps it indefinitely.
    pub fn idle_limit(self) -> Option<Duration> {
        match self {
            KeepLoaded::Never => Some(Duration::ZERO),
            KeepLoaded::FiveMinutes => Some(Duration::from_secs(5 * 60)),
            KeepLoaded::FifteenMinutes => Some(Duration::from_secs(15 * 60)),
            KeepLoaded::Always => None,
        }
    }
}

/// Loads a model. Each call loads it again; `Resident` decides when to call it.
pub type ModelLoader =
    Box<dyn FnMut(&ModelRecord) -> Result<Box<dyn Generator>, GenerateError> + Send>;

/// The loaded model, if any, and which file it came from.
pub struct Resident {
    load: ModelLoader,
    loaded: Option<(Key, Box<dyn Generator>)>,
}

/// Identifies a model file, so a different file under the same id still reloads.
#[derive(PartialEq)]
struct Key {
    id: String,
    sha256: String,
}

impl Key {
    fn of(model: &ModelRecord) -> Self {
        Self {
            id: model.id.clone(),
            sha256: model.sha256.clone(),
        }
    }
}

impl Resident {
    pub fn new(load: ModelLoader) -> Self {
        Self { load, loaded: None }
    }

    /// Returns `model`, loading it first if it isn't the loaded one, and calls
    /// `on_loading` before a load. Another loaded model is freed before the
    /// load starts, so two models never take memory at once.
    pub fn get(
        &mut self,
        model: &ModelRecord,
        on_loading: impl FnOnce(),
    ) -> Result<&mut dyn Generator, GenerateError> {
        let key = Key::of(model);
        if self.loaded.as_ref().is_some_and(|(k, _)| *k != key) {
            self.unload();
        }
        if self.loaded.is_none() {
            on_loading();
            let generator = (self.load)(model)?;
            self.loaded = Some((key, generator));
        }
        let (_, generator) = self.loaded.as_mut().expect("loaded above");
        Ok(generator.as_mut())
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded.is_some()
    }

    /// Frees the model's memory.
    pub fn unload(&mut self) {
        if let Some((key, _)) = self.loaded.take() {
            log::info!("unloaded the summary model {}", key.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::template::Message;
    use crate::store::ModelSource;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    /// Records loads and drops in `log`, as "load <id>" and "drop <id>".
    struct Logged {
        id: String,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Drop for Logged {
        fn drop(&mut self) {
            self.log.lock().unwrap().push(format!("drop {}", self.id));
        }
    }

    impl Generator for Logged {
        fn context_size(&self) -> usize {
            1
        }
        fn prompt_tokens(&mut self, _: &[Message]) -> Result<usize, GenerateError> {
            Ok(0)
        }
        fn count_tokens(&mut self, _: &str) -> Result<usize, GenerateError> {
            Ok(0)
        }
        fn generate(
            &mut self,
            _: &[Message],
            _: usize,
            _: &AtomicBool,
            _: &mut dyn FnMut(&str),
        ) -> Result<String, GenerateError> {
            Ok(String::new())
        }
    }

    fn model(id: &str, sha256: &str) -> ModelRecord {
        ModelRecord {
            id: id.into(),
            display_name: id.into(),
            file_path: format!("/models/{id}.gguf"),
            size_bytes: 1,
            sha256: sha256.into(),
            source: ModelSource::Import,
            source_url: None,
            version: None,
            min_runtime: None,
            added_at: 0,
        }
    }

    fn resident() -> (Resident, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let shared = log.clone();
        let load: ModelLoader = Box::new(move |model| {
            if model.id == "broken" {
                return Err(GenerateError::Model("bad file".into()));
            }
            shared.lock().unwrap().push(format!("load {}", model.id));
            Ok(Box::new(Logged {
                id: model.id.clone(),
                log: shared.clone(),
            }))
        });
        (Resident::new(load), log)
    }

    fn entries(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        log.lock().unwrap().clone()
    }

    #[test]
    fn reads_and_writes_each_choice() {
        for keep in [
            KeepLoaded::Never,
            KeepLoaded::FiveMinutes,
            KeepLoaded::FifteenMinutes,
            KeepLoaded::Always,
        ] {
            assert_eq!(KeepLoaded::from_setting(Some(keep.as_setting())), keep);
        }
        assert_eq!(KeepLoaded::from_setting(None), KeepLoaded::FiveMinutes);
        assert_eq!(
            KeepLoaded::from_setting(Some("tuesday")),
            KeepLoaded::FiveMinutes
        );
        assert_eq!(KeepLoaded::default(), KeepLoaded::FiveMinutes);
    }

    #[test]
    fn idles_for_each_choice() {
        assert_eq!(KeepLoaded::Never.idle_limit(), Some(Duration::ZERO));
        assert_eq!(
            KeepLoaded::FiveMinutes.idle_limit(),
            Some(Duration::from_secs(300))
        );
        assert_eq!(
            KeepLoaded::FifteenMinutes.idle_limit(),
            Some(Duration::from_secs(900))
        );
        assert_eq!(KeepLoaded::Always.idle_limit(), None);
    }

    #[test]
    fn loads_once_and_reuses_the_model() {
        let (mut resident, log) = resident();
        let mut loads = 0;

        resident.get(&model("a", "1"), || loads += 1).unwrap();
        resident.get(&model("a", "1"), || loads += 1).unwrap();

        assert_eq!(loads, 1);
        assert_eq!(entries(&log), ["load a"]);
        assert!(resident.is_loaded());
    }

    #[test]
    fn frees_the_old_model_before_loading_another() {
        let (mut resident, log) = resident();

        resident.get(&model("a", "1"), || ()).unwrap();
        resident.get(&model("b", "2"), || ()).unwrap();

        assert_eq!(entries(&log), ["load a", "drop a", "load b"]);
    }

    #[test]
    fn reloads_a_different_file_under_the_same_id() {
        let (mut resident, log) = resident();

        resident.get(&model("a", "1"), || ()).unwrap();
        resident.get(&model("a", "2"), || ()).unwrap();

        assert_eq!(entries(&log), ["load a", "drop a", "load a"]);
    }

    #[test]
    fn unloads_on_request() {
        let (mut resident, log) = resident();
        resident.get(&model("a", "1"), || ()).unwrap();

        resident.unload();

        assert_eq!(entries(&log), ["load a", "drop a"]);
        assert!(!resident.is_loaded());
    }

    #[test]
    fn holds_nothing_after_a_failed_load() {
        let (mut resident, log) = resident();
        resident.get(&model("a", "1"), || ()).unwrap();

        assert!(resident.get(&model("broken", "x"), || ()).is_err());

        assert_eq!(entries(&log), ["load a", "drop a"]);
        assert!(!resident.is_loaded());
    }
}
