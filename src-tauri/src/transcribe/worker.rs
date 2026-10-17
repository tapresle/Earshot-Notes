//! A dedicated thread that owns the transcription models and transcribes
//! segments in order.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use super::cleanup::clean;
use super::phantom::is_phantom;
use super::whisper::Whisper;
use super::whistle::{Transcription, Whistle, WhistleError};
use crate::chunker::Speech;
use crate::store::Source;

/// A speech segment waiting for transcription.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub meeting_id: i64,
    pub source: Source,
    /// Offsets from the start of the meeting.
    pub start_ms: i64,
    pub end_ms: i64,
    pub samples: Vec<f32>,
    pub speech: Speech,
    /// Vocabulary terms that bias transcription, one per line. Only Whistle
    /// uses them.
    pub keywords: Option<Arc<CString>>,
    /// The model to transcribe with. After transcription, the model that did.
    pub engine: Engine,
}

/// Which model transcribes a segment.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Engine {
    /// Whistle, bundled and always loaded.
    #[default]
    Whistle,
    /// Whisper, from this ggml model file (#25).
    Whisper(Arc<PathBuf>),
}

/// How the worker runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkerOptions {
    /// Whether Whisper runs on the GPU. Tests on CI turn it off.
    pub use_gpu: bool,
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self { use_gpu: true }
    }
}

/// Where the worker loaded its model from.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedModel {
    pub path: PathBuf,
}

#[derive(Clone)]
pub struct TranscriptionWorker {
    inbox: Sender<Message>,
    shutting_down: Arc<AtomicBool>,
}

enum Message {
    Job(Job),
    /// Load this Whisper model now, before its first job arrives, retrying
    /// one that failed to load before.
    Preload(Arc<PathBuf>),
    /// Free Whisper's memory: it's no longer the chosen model.
    UnloadWhisper,
    /// Run this once every job queued before it is transcribed and handled.
    AfterQueued(Box<dyn FnOnce() + Send>),
    /// Skip the queued jobs, free every model, confirm, and stop.
    Shutdown(Sender<()>),
}

#[derive(Debug, thiserror::Error)]
#[error("the transcription worker has stopped")]
pub struct WorkerStopped;

impl TranscriptionWorker {
    /// Loads the first candidate model that works, then starts the worker.
    /// `on_result` runs on the worker thread for every segment that contains
    /// speech, with fillers removed (see `cleanup`), except segments with only
    /// fillers and likely phantom phrases (see `phantom`).
    pub fn spawn<F>(
        candidates: Vec<PathBuf>,
        on_result: F,
    ) -> Result<(Self, LoadedModel), WhistleError>
    where
        F: FnMut(Job, Transcription) + Send + 'static,
    {
        Self::spawn_with(candidates, WorkerOptions::default(), on_result)
    }

    /// Like [`TranscriptionWorker::spawn`], with `options`.
    pub fn spawn_with<F>(
        candidates: Vec<PathBuf>,
        options: WorkerOptions,
        mut on_result: F,
    ) -> Result<(Self, LoadedModel), WhistleError>
    where
        F: FnMut(Job, Transcription) + Send + 'static,
    {
        let (sender, inbox) = mpsc::channel::<Message>();
        let (ready_tx, ready_rx) = mpsc::channel();
        let flag = Arc::new(AtomicBool::new(false));
        let shutting_down = flag.clone();

        thread::Builder::new()
            .name("transcription".into())
            .spawn(move || {
                let whistle = match load_first(&candidates) {
                    Ok((whistle, path)) => {
                        let _ = ready_tx.send(Ok(LoadedModel { path }));
                        whistle
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let mut engines = Engines::new(whistle, options);
                loop {
                    let mut job = match inbox.recv() {
                        Ok(Message::Job(job)) => job,
                        Ok(Message::Preload(model)) => {
                            engines.failed = None;
                            engines.whisper(&model);
                            continue;
                        }
                        Ok(Message::UnloadWhisper) => {
                            engines.unload_whisper();
                            continue;
                        }
                        // Skipped once a shutdown is queued, like the jobs before it.
                        Ok(Message::AfterQueued(then)) => {
                            if !shutting_down.load(Ordering::Acquire) {
                                then();
                            }
                            continue;
                        }
                        Ok(Message::Shutdown(done)) => {
                            drop(engines);
                            drop(inbox);
                            let _ = done.send(());
                            return;
                        }
                        Err(_) => return,
                    };
                    // Once a shutdown is queued, the jobs before it are skipped,
                    // as they would be if the app exited with them waiting.
                    if shutting_down.load(Ordering::Acquire) {
                        continue;
                    }
                    // Both engines' results go through the same cleanup.
                    match engines.transcribe(&mut job).map(cleaned) {
                        Ok(t) if t.text.is_empty() => {}
                        // Never log the text itself.
                        Ok(t) if is_phantom(&t.text, job.speech) => log::debug!(
                            "dropped a likely phantom phrase for meeting {} at {} ms ({} ms of speech)",
                            job.meeting_id,
                            job.start_ms,
                            job.speech.ms
                        ),
                        Ok(t) => on_result(job, t),
                        Err(error) => log::warn!(
                            "transcription failed for meeting {} at {} ms: {error}",
                            job.meeting_id,
                            job.start_ms
                        ),
                    }
                }
            })
            .map_err(WhistleError::Io)?;

        let loaded = ready_rx
            .recv()
            .map_err(|_| WhistleError::Load("the transcription thread exited".into()))??;
        Ok((
            Self {
                inbox: sender,
                shutting_down: flag,
            },
            loaded,
        ))
    }

    /// A worker whose queue the test reads directly, with no model loaded.
    #[cfg(test)]
    pub(crate) fn from_queue() -> (Self, Queue) {
        let (inbox, queue) = mpsc::channel();
        (
            Self {
                inbox,
                shutting_down: Arc::default(),
            },
            Queue(queue),
        )
    }

    pub fn submit(&self, job: Job) -> Result<(), WorkerStopped> {
        self.inbox
            .send(Message::Job(job))
            .map_err(|_| WorkerStopped)
    }

    /// Runs `then` on the worker thread once every job submitted before it
    /// has been transcribed and its result handled, such as the segments a
    /// stopped recording flushes. It doesn't run if the app quits first.
    pub fn after_queued(&self, then: impl FnOnce() + Send + 'static) -> Result<(), WorkerStopped> {
        self.inbox
            .send(Message::AfterQueued(Box::new(then)))
            .map_err(|_| WorkerStopped)
    }

    /// Loads `engine` now if it's Whisper, which takes a few seconds, so no
    /// recording waits for it; it then stays loaded, like Whistle. For
    /// Whistle, frees Whisper's memory instead.
    pub fn preload(&self, engine: &Engine) {
        let message = match engine {
            Engine::Whisper(model) => Message::Preload(model.clone()),
            Engine::Whistle => Message::UnloadWhisper,
        };
        let _ = self.inbox.send(message);
    }

    /// Frees the models before the app exits, skipping queued jobs, and waits
    /// up to `timeout` for the worker to confirm. ggml's Metal backend aborts
    /// the process at exit if a model is still loaded.
    pub fn shutdown(&self, timeout: Duration) {
        self.shutting_down.store(true, Ordering::Release);
        let (done, confirmed) = mpsc::channel();
        if self.inbox.send(Message::Shutdown(done)).is_err() {
            return;
        }
        if confirmed.recv_timeout(timeout).is_err() {
            log::warn!("the transcription worker didn't stop within {timeout:?}");
        }
    }
}

/// The worker's queue, read directly by tests.
#[cfg(test)]
pub(crate) struct Queue(mpsc::Receiver<Message>);

#[cfg(test)]
impl Queue {
    fn job(message: Message) -> Option<Job> {
        match message {
            Message::Job(job) => Some(job),
            Message::Preload(_)
            | Message::UnloadWhisper
            | Message::AfterQueued(_)
            | Message::Shutdown(_) => None,
        }
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<Job, RecvTimeoutError> {
        loop {
            if let Some(job) = Self::job(self.0.recv_timeout(timeout)?) {
                return Ok(job);
            }
        }
    }

    /// The next message, if it asks to preload a Whisper model.
    pub fn preload(&self, timeout: Duration) -> Option<Arc<PathBuf>> {
        match self.0.recv_timeout(timeout) {
            Ok(Message::Preload(model)) => Some(model),
            _ => None,
        }
    }

    pub fn try_iter(&self) -> impl Iterator<Item = Job> + '_ {
        self.0.try_iter().filter_map(Self::job)
    }
}

/// The models on the worker thread: Whistle, always loaded, and Whisper,
/// loaded for the first job that asks for it.
struct Engines {
    whistle: Whistle,
    whisper: Option<(Arc<PathBuf>, Whisper)>,
    /// A Whisper model that failed to load, so later jobs don't retry it
    /// until the next preload.
    failed: Option<Arc<PathBuf>>,
    options: WorkerOptions,
}

impl Engines {
    fn new(whistle: Whistle, options: WorkerOptions) -> Self {
        Self {
            whistle,
            whisper: None,
            failed: None,
            options,
        }
    }

    /// Transcribes on the job's engine. A Whisper job runs on Whistle instead
    /// when its model won't load, so the meeting is still transcribed;
    /// `job.engine` then says Whistle.
    fn transcribe(&mut self, job: &mut Job) -> Result<Transcription, String> {
        let samples = std::mem::take(&mut job.samples);
        if let Engine::Whisper(model) = &job.engine {
            if let Some(whisper) = self.whisper(&model.clone()) {
                return whisper
                    .transcribe(&samples)
                    .map(|text| Transcription {
                        text,
                        language: Some("en".into()),
                        ttft_ms: 0.0,
                        decode_tps: 0.0,
                    })
                    .map_err(|error| error.to_string());
            }
            job.engine = Engine::Whistle;
        }
        self.whistle
            .transcribe_with_keywords(&samples, job.keywords.as_deref().map(CString::as_c_str))
            .map_err(|error| error.to_string())
    }

    /// The loaded Whisper model at `model`, loading it first if needed.
    fn whisper(&mut self, model: &Arc<PathBuf>) -> Option<&mut Whisper> {
        if self.failed.as_ref() == Some(model) {
            return None;
        }
        if self
            .whisper
            .as_ref()
            .is_some_and(|(loaded, _)| loaded == model)
        {
            return self.whisper.as_mut().map(|(_, whisper)| whisper);
        }
        // Free a different model before loading this one.
        self.whisper = None;
        match Whisper::load(model, self.options.use_gpu) {
            Ok(whisper) => {
                log::info!("loaded Whisper from {}", model.display());
                self.whisper = Some((model.clone(), whisper));
                self.whisper.as_mut().map(|(_, whisper)| whisper)
            }
            Err(error) => {
                log::error!("transcribing with Whistle instead of Whisper: {error}");
                self.failed = Some(model.clone());
                None
            }
        }
    }

    fn unload_whisper(&mut self) {
        self.failed = None;
        if self.whisper.take().is_some() {
            log::info!("unloaded Whisper");
        }
    }
}

fn cleaned(transcription: Transcription) -> Transcription {
    Transcription {
        text: clean(&transcription.text),
        ..transcription
    }
}

/// Tries each existing candidate in order, so a newer model in the user's
/// models folder takes precedence and a damaged one falls back to the bundled copy.
pub fn load_first(candidates: &[PathBuf]) -> Result<(Whistle, PathBuf), WhistleError> {
    let mut last_error = None;
    for path in candidates.iter().filter(|p| Path::exists(p)) {
        match Whistle::load(path) {
            Ok(whistle) => return Ok((whistle, path.clone())),
            Err(error) => {
                log::warn!("couldn't load Whistle from {}: {error}", path.display());
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        WhistleError::Load(format!(
            "no Whistle model found (looked in {})",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }))
}
