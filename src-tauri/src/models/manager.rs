//! Runs one model download at a time in the background and tracks its progress.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use super::KnownModel;
use super::download::{DownloadError, Fetch, Phase, download};
use crate::store::{ModelRecord, Store};

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadEvent {
    Progress {
        model_id: String,
        phase: Phase,
    },
    Done {
        model: ModelRecord,
    },
    Failed {
        model_id: String,
        message: String,
        cancelled: bool,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("{0} is already downloading")]
    Busy(String),
    #[error("couldn't start the download: {0}")]
    Thread(#[from] std::io::Error),
}

pub type EventSink = Arc<dyn Fn(DownloadEvent) + Send + Sync>;

struct Active {
    model_id: String,
    cancel: Arc<AtomicBool>,
    phase: Phase,
}

#[derive(Default)]
pub struct Downloads {
    active: Arc<Mutex<Option<Active>>>,
}

impl Downloads {
    pub fn start(
        &self,
        model: &'static KnownModel,
        models_dir: PathBuf,
        store: Arc<Store>,
        fetch: Arc<dyn Fetch>,
        on_event: EventSink,
    ) -> Result<(), StartError> {
        let mut active = lock(&self.active);
        if let Some(current) = active.as_ref() {
            return Err(StartError::Busy(current.model_id.clone()));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let slot = self.active.clone();
        let stop = cancel.clone();
        thread::Builder::new()
            .name("model-download".into())
            .spawn(move || {
                let model_id = model.id.to_owned();
                let result = download(
                    model,
                    &models_dir,
                    &store,
                    fetch.as_ref(),
                    &stop,
                    &mut |phase| {
                        if let Some(active) = lock(&slot).as_mut() {
                            active.phase = phase;
                        }
                        on_event(DownloadEvent::Progress {
                            model_id: model_id.clone(),
                            phase,
                        });
                    },
                );
                *lock(&slot) = None;
                on_event(match result {
                    Ok(model) => DownloadEvent::Done { model },
                    Err(error) => DownloadEvent::Failed {
                        model_id,
                        cancelled: matches!(error, DownloadError::Cancelled),
                        message: error.to_string(),
                    },
                });
            })?;
        *active = Some(Active {
            model_id: model.id.to_owned(),
            cancel,
            phase: Phase::Downloading {
                received: 0,
                total: model.size_bytes,
            },
        });
        Ok(())
    }

    /// Asks the running download to stop. Returns whether one was running.
    pub fn cancel(&self) -> bool {
        match lock(&self.active).as_ref() {
            Some(active) => {
                active.cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// The model downloading now and how far along it is.
    pub fn status(&self) -> Option<(String, Phase)> {
        lock(&self.active)
            .as_ref()
            .map(|a| (a.model_id.clone(), a.phase))
    }
}

fn lock(active: &Mutex<Option<Active>>) -> MutexGuard<'_, Option<Active>> {
    active.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::download::Fetched;
    use crate::models::sha256_file;
    use std::io::{self, Read};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Serves bytes, blocking before the first read until released.
    struct GatedServer {
        data: Vec<u8>,
        gate: Mutex<Option<mpsc::Receiver<()>>>,
    }

    struct GatedReader {
        inner: io::Cursor<Vec<u8>>,
        gate: Option<mpsc::Receiver<()>>,
    }

    impl Read for GatedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if let Some(gate) = self.gate.take() {
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            self.inner.read(buf)
        }
    }

    impl Fetch for GatedServer {
        fn fetch(&self, _url: &str, _offset: u64) -> io::Result<Fetched> {
            Ok(Fetched {
                body: Box::new(GatedReader {
                    inner: io::Cursor::new(self.data.clone()),
                    gate: self.gate.lock().unwrap().take(),
                }),
                resumed: false,
            })
        }
    }

    fn known(data: &[u8]) -> &'static KnownModel {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, data).unwrap();
        Box::leak(Box::new(KnownModel {
            id: "test-model",
            display_name: "Test",
            file_name: "test.gguf",
            quantization: "Q4_K_M",
            repo: "example/test",
            revision: "abc",
            sha256: Box::leak(sha256_file(&path).unwrap().into_boxed_str()),
            size_bytes: data.len() as u64,
            version: "v1",
            min_runtime: "0.1.158",
            description: "Test model.",
            context_limit: 4096,
        }))
    }

    fn collect() -> (EventSink, mpsc::Receiver<DownloadEvent>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        (Arc::new(move |e| tx.lock().unwrap().send(e).unwrap()), rx)
    }

    fn until_finished(events: &mpsc::Receiver<DownloadEvent>) -> DownloadEvent {
        loop {
            match events.recv_timeout(Duration::from_secs(5)).unwrap() {
                DownloadEvent::Progress { .. } => continue,
                finished => return finished,
            }
        }
    }

    #[test]
    fn downloads_in_the_background_and_reports_progress() {
        let data = vec![7u8; 100_000];
        let (release, gate) = mpsc::channel();
        let server = Arc::new(GatedServer {
            data: data.clone(),
            gate: Mutex::new(Some(gate)),
        });
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let downloads = Downloads::default();
        let (sink, events) = collect();
        let model = known(&data);

        downloads
            .start(
                model,
                dir.path().to_path_buf(),
                store.clone(),
                server.clone(),
                sink.clone(),
            )
            .unwrap();

        assert_eq!(
            downloads.status(),
            Some((
                "test-model".to_owned(),
                Phase::Downloading {
                    received: 0,
                    total: 100_000
                }
            ))
        );
        assert!(matches!(
            downloads.start(model, dir.path().to_path_buf(), store.clone(), server, sink),
            Err(StartError::Busy(id)) if id == "test-model"
        ));

        release.send(()).unwrap();
        let DownloadEvent::Done { model } = until_finished(&events) else {
            panic!("expected Done");
        };
        assert_eq!(model.id, "test-model");
        assert_eq!(downloads.status(), None);
        assert_eq!(store.summary_model().unwrap(), Some(model));
    }

    #[test]
    fn cancels_a_running_download() {
        let data = vec![7u8; 100_000];
        let (release, gate) = mpsc::channel();
        let server = Arc::new(GatedServer {
            data: data.clone(),
            gate: Mutex::new(Some(gate)),
        });
        let dir = tempfile::tempdir().unwrap();
        let downloads = Downloads::default();
        let (sink, events) = collect();
        downloads
            .start(
                known(&data),
                dir.path().to_path_buf(),
                Arc::new(Store::open_in_memory().unwrap()),
                server,
                sink,
            )
            .unwrap();

        assert!(downloads.cancel());
        release.send(()).unwrap();

        assert!(matches!(
            until_finished(&events),
            DownloadEvent::Failed {
                cancelled: true,
                ..
            }
        ));
        assert!(!downloads.cancel());
    }
}
