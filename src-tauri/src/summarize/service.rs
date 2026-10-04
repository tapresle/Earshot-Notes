//! Runs one summary at a time on a background thread and reports progress.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use serde::Serialize;

use super::run::{Generator, Progress, Request, SummaryError, summarize};
use crate::store::{ModelRecord, NewSummary, Store, StoreError, Summary, TranscriptSegment};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "stage")]
pub enum Stage {
    /// Loading the model into memory.
    Loading,
    /// Condensing part `part` of `parts` of a long transcript.
    Reading { part: usize, parts: usize },
    /// Writing the notes; text streams from here on.
    Writing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SummaryEvent {
    Progress {
        meeting_id: i64,
        stage: Stage,
    },
    Text {
        meeting_id: i64,
        text: String,
    },
    Done {
        meeting_id: i64,
        summary: Summary,
    },
    Failed {
        meeting_id: i64,
        message: String,
        cancelled: bool,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SummarizeError {
    #[error("a summary is already being written; wait for it or cancel it")]
    Busy,
    #[error("add a summary model first")]
    NoModel,
    #[error("there's nothing to summarize yet. Record the meeting or write some notes first")]
    NothingToSummarize,
    #[error("the summarizer stopped unexpectedly; restart the app to summarize again")]
    Stopped,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Loads a model for one job. The model is unloaded when the job ends, so it
/// doesn't hold memory between summaries.
pub type ModelLoader =
    Box<dyn FnMut(&ModelRecord) -> Result<Box<dyn Generator>, SummaryError> + Send>;

struct Job {
    meeting_id: i64,
    model: ModelRecord,
    title: String,
    notes_markdown: String,
    segments: Vec<TranscriptSegment>,
    upto_ms: i64,
    is_final: bool,
    cancel: Arc<AtomicBool>,
}

type Active = Arc<Mutex<Option<(i64, Arc<AtomicBool>)>>>;

pub struct Summarizer {
    jobs: Sender<Job>,
    active: Active,
}

impl Summarizer {
    pub fn spawn(
        store: Arc<Store>,
        mut load: ModelLoader,
        on_event: impl Fn(SummaryEvent) + Send + 'static,
    ) -> Self {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let active: Active = Arc::default();
        let finished = active.clone();
        thread::Builder::new()
            .name("summarizer".into())
            .spawn(move || {
                for job in inbox {
                    let meeting_id = job.meeting_id;
                    let event = match run_job(&store, &mut load, job, &on_event) {
                        Ok(summary) => SummaryEvent::Done {
                            meeting_id,
                            summary,
                        },
                        Err(error) => SummaryEvent::Failed {
                            meeting_id,
                            cancelled: matches!(error, JobError::Summary(SummaryError::Cancelled)),
                            message: error.to_string(),
                        },
                    };
                    // Clear the slot before announcing, so the UI can start the next one.
                    *lock(&finished) = None;
                    on_event(event);
                }
            })
            .expect("couldn't start the summarizer thread");
        Self { jobs, active }
    }

    /// Queues a summary of the meeting as it stands now. `is_final` marks a
    /// meeting that has stopped recording.
    pub fn start(
        &self,
        store: &Store,
        meeting_id: i64,
        is_final: bool,
    ) -> Result<(), SummarizeError> {
        let mut active = lock(&self.active);
        if active.is_some() {
            return Err(SummarizeError::Busy);
        }
        let model = store.summary_model()?.ok_or(SummarizeError::NoModel)?;
        let detail = store.get_meeting(meeting_id)?;
        let segments = store.list_segments(meeting_id)?;
        if segments.is_empty() && detail.notes_markdown.trim().is_empty() {
            return Err(SummarizeError::NothingToSummarize);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job {
            meeting_id,
            model,
            title: detail.meeting.title,
            notes_markdown: detail.notes_markdown,
            upto_ms: segments.iter().map(|s| s.end_ms).max().unwrap_or(0),
            segments,
            is_final,
            cancel: cancel.clone(),
        };
        self.jobs.send(job).map_err(|_| SummarizeError::Stopped)?;
        *active = Some((meeting_id, cancel));
        Ok(())
    }

    /// Asks the running summary to stop. Returns whether one was running.
    pub fn cancel(&self) -> bool {
        match lock(&self.active).as_ref() {
            Some((_, cancel)) => {
                cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    pub fn active_meeting(&self) -> Option<i64> {
        lock(&self.active).as_ref().map(|(id, _)| *id)
    }
}

fn lock(active: &Active) -> MutexGuard<'_, Option<(i64, Arc<AtomicBool>)>> {
    active.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, thiserror::Error)]
enum JobError {
    #[error(transparent)]
    Summary(#[from] SummaryError),
    #[error("couldn't save the summary: {0}")]
    Store(#[from] StoreError),
}

fn run_job(
    store: &Store,
    load: &mut ModelLoader,
    job: Job,
    on_event: &dyn Fn(SummaryEvent),
) -> Result<Summary, JobError> {
    let meeting_id = job.meeting_id;
    on_event(SummaryEvent::Progress {
        meeting_id,
        stage: Stage::Loading,
    });
    let mut generator = load(&job.model)?;
    if job.cancel.load(Ordering::Relaxed) {
        return Err(SummaryError::Cancelled.into());
    }
    let request = Request {
        title: &job.title,
        notes_markdown: &job.notes_markdown,
        segments: &job.segments,
    };
    let body = summarize(
        generator.as_mut(),
        &request,
        &job.cancel,
        &mut |progress| {
            let stage = match progress {
                Progress::Reading { part, parts } => Stage::Reading { part, parts },
                Progress::Writing => Stage::Writing,
            };
            on_event(SummaryEvent::Progress { meeting_id, stage });
        },
        &mut |text| {
            on_event(SummaryEvent::Text {
                meeting_id,
                text: text.to_owned(),
            })
        },
    )?;
    drop(generator);
    Ok(store.insert_summary(NewSummary {
        meeting_id,
        upto_ms: job.upto_ms,
        is_final: job.is_final,
        body_markdown: body,
        model_id: job.model.id,
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ModelSource, NewSegment, Source};
    use crate::summarize::template::Message;
    use std::time::Duration;

    /// Streams two pieces, optionally waiting for permission between them.
    struct FakeModel {
        gate: Option<mpsc::Receiver<()>>,
    }

    impl Generator for FakeModel {
        fn context_size(&self) -> usize {
            8192
        }
        fn prompt_tokens(&mut self, _: &[Message]) -> Result<usize, SummaryError> {
            Ok(100)
        }
        fn count_tokens(&mut self, text: &str) -> Result<usize, SummaryError> {
            Ok(text.len())
        }
        fn generate(
            &mut self,
            _: &[Message],
            _: usize,
            cancel: &AtomicBool,
            on_text: &mut dyn FnMut(&str),
        ) -> Result<String, SummaryError> {
            on_text("## Notes");
            if let Some(gate) = &self.gate {
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(SummaryError::Cancelled);
            }
            on_text("\n- done");
            Ok("## Notes\n- done".into())
        }
    }

    struct Harness {
        store: Arc<Store>,
        summarizer: Summarizer,
        events: mpsc::Receiver<SummaryEvent>,
        meeting_id: i64,
    }

    fn harness(gate: Option<mpsc::Receiver<()>>, with_model: bool) -> Harness {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let meeting = store.create_meeting("Sync").unwrap();
        store
            .insert_segment(NewSegment {
                meeting_id: meeting.id,
                source: Source::Me,
                start_ms: 0,
                end_ms: 4200,
                text: "Ship Friday.".into(),
                language: Some("en".into()),
            })
            .unwrap();
        if with_model {
            store
                .add_model(&ModelRecord {
                    id: "test-model".into(),
                    display_name: "Test".into(),
                    file_path: "/models/test.gguf".into(),
                    size_bytes: 1,
                    sha256: "abc".into(),
                    source: ModelSource::Import,
                    source_url: None,
                    version: None,
                    min_runtime: None,
                    added_at: 0,
                })
                .unwrap();
        }
        let gate = Mutex::new(gate);
        let loader: ModelLoader = Box::new(move |model| {
            if model.id == "broken" {
                return Err(SummaryError::Model("bad file".into()));
            }
            Ok(Box::new(FakeModel {
                gate: gate.lock().unwrap().take(),
            }))
        });
        let (tx, events) = mpsc::channel();
        let tx = Mutex::new(tx);
        let summarizer = Summarizer::spawn(store.clone(), loader, move |event| {
            tx.lock().unwrap().send(event).unwrap();
        });
        Harness {
            store,
            summarizer,
            events,
            meeting_id: meeting.id,
        }
    }

    fn next(events: &mpsc::Receiver<SummaryEvent>) -> SummaryEvent {
        events.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    #[test]
    fn streams_progress_and_text_then_saves_the_summary() {
        let h = harness(None, true);

        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();

        let id = h.meeting_id;
        assert_eq!(
            next(&h.events),
            SummaryEvent::Progress {
                meeting_id: id,
                stage: Stage::Loading
            }
        );
        assert_eq!(
            next(&h.events),
            SummaryEvent::Progress {
                meeting_id: id,
                stage: Stage::Writing
            }
        );
        assert_eq!(
            next(&h.events),
            SummaryEvent::Text {
                meeting_id: id,
                text: "## Notes".into()
            }
        );
        assert_eq!(
            next(&h.events),
            SummaryEvent::Text {
                meeting_id: id,
                text: "\n- done".into()
            }
        );
        let SummaryEvent::Done { summary, .. } = next(&h.events) else {
            panic!("expected Done");
        };
        assert_eq!(summary.body_markdown, "## Notes\n- done");
        assert_eq!(summary.upto_ms, 4200);
        assert!(summary.is_final);
        assert_eq!(summary.model_id, "test-model");
        assert_eq!(h.store.latest_summary(id).unwrap(), Some(summary));
        assert_eq!(h.summarizer.active_meeting(), None);
    }

    #[test]
    fn runs_one_summary_at_a_time() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        h.summarizer.start(&h.store, h.meeting_id, false).unwrap();

        assert!(matches!(
            h.summarizer.start(&h.store, h.meeting_id, false),
            Err(SummarizeError::Busy)
        ));
        assert_eq!(h.summarizer.active_meeting(), Some(h.meeting_id));

        release.send(()).unwrap();
        while !matches!(next(&h.events), SummaryEvent::Done { .. }) {}
        assert!(h.summarizer.start(&h.store, h.meeting_id, true).is_ok());
    }

    #[test]
    fn cancels_without_saving() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        h.summarizer.start(&h.store, h.meeting_id, false).unwrap();

        assert!(h.summarizer.cancel());
        release.send(()).unwrap();

        let failed = loop {
            if let event @ SummaryEvent::Failed { .. } = next(&h.events) {
                break event;
            }
        };
        assert!(matches!(
            failed,
            SummaryEvent::Failed {
                cancelled: true,
                ..
            }
        ));
        assert_eq!(h.store.latest_summary(h.meeting_id).unwrap(), None);
    }

    #[test]
    fn requires_a_model() {
        let h = harness(None, false);

        assert!(matches!(
            h.summarizer.start(&h.store, h.meeting_id, true),
            Err(SummarizeError::NoModel)
        ));
    }

    #[test]
    fn refuses_an_empty_meeting_up_front() {
        let h = harness(None, true);
        let empty = h.store.create_meeting("Empty").unwrap();

        assert!(matches!(
            h.summarizer.start(&h.store, empty.id, true),
            Err(SummarizeError::NothingToSummarize)
        ));
        assert_eq!(h.summarizer.active_meeting(), None);
    }

    #[test]
    fn reports_a_model_that_fails_to_load() {
        let h = harness(None, false);
        h.store
            .add_model(&ModelRecord {
                id: "broken".into(),
                display_name: "Broken".into(),
                file_path: "/x.gguf".into(),
                size_bytes: 1,
                sha256: "b".into(),
                source: ModelSource::Import,
                source_url: None,
                version: None,
                min_runtime: None,
                added_at: 0,
            })
            .unwrap();

        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();

        let failed = loop {
            if let event @ SummaryEvent::Failed { .. } = next(&h.events) {
                break event;
            }
        };
        assert!(matches!(
            failed,
            SummaryEvent::Failed { cancelled: false, ref message, .. } if message.contains("bad file")
        ));
    }
}
