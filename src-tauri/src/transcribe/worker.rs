//! A dedicated thread that owns the Whistle model and transcribes segments in order.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;

use super::whistle::{Transcription, Whistle, WhistleError};
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
}

/// Where the worker loaded its model from.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedModel {
    pub path: PathBuf,
}

#[derive(Clone)]
pub struct TranscriptionWorker {
    jobs: Sender<Job>,
}

#[derive(Debug, thiserror::Error)]
#[error("the transcription worker has stopped")]
pub struct WorkerStopped;

impl TranscriptionWorker {
    /// Loads the first candidate model that works, then starts the worker.
    /// `on_result` runs on the worker thread for every segment that contains speech.
    pub fn spawn<F>(
        candidates: Vec<PathBuf>,
        mut on_result: F,
    ) -> Result<(Self, LoadedModel), WhistleError>
    where
        F: FnMut(Job, Transcription) + Send + 'static,
    {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel();

        thread::Builder::new()
            .name("transcription".into())
            .spawn(move || {
                let mut whistle = match load_first(&candidates) {
                    Ok((whistle, path)) => {
                        let _ = ready_tx.send(Ok(LoadedModel { path }));
                        whistle
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                for mut job in inbox {
                    let samples = std::mem::take(&mut job.samples);
                    match whistle.transcribe(&samples) {
                        Ok(t) if t.text.is_empty() => {}
                        Ok(t) => on_result(job, t),
                        Err(error) => eprintln!(
                            "transcription failed for meeting {} at {} ms: {error}",
                            job.meeting_id, job.start_ms
                        ),
                    }
                }
            })
            .map_err(WhistleError::Io)?;

        let loaded = ready_rx
            .recv()
            .map_err(|_| WhistleError::Load("the transcription thread exited".into()))??;
        Ok((Self { jobs }, loaded))
    }

    /// A worker whose queue the test reads directly, with no model loaded.
    #[cfg(test)]
    pub(crate) fn from_queue() -> (Self, mpsc::Receiver<Job>) {
        let (jobs, inbox) = mpsc::channel();
        (Self { jobs }, inbox)
    }

    pub fn submit(&self, job: Job) -> Result<(), WorkerStopped> {
        self.jobs.send(job).map_err(|_| WorkerStopped)
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
                eprintln!("couldn't load Whistle from {}: {error}", path.display());
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
