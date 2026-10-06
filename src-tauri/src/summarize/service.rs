//! Runs one summary at a time on a background thread and reports progress.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use serde::Serialize;

use super::run::{Generator, Progress, Request, SummaryError, summarize};
use super::title::suggest_title;
use crate::store::{
    DEFAULT_TITLE, ModelRecord, NewSummary, Store, StoreError, Summary, TranscriptSegment,
};

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
        /// The title given to a meeting that was still untitled, if any.
        title: Option<String>,
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
                        Ok((summary, title)) => SummaryEvent::Done {
                            meeting_id,
                            summary,
                            title,
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

    /// Runs `f` only while no summary is running, and keeps one from starting
    /// until it returns. Used to change models safely.
    pub fn when_idle<T>(&self, f: impl FnOnce() -> T) -> Result<T, SummarizeError> {
        let active = lock(&self.active);
        if active.is_some() {
            return Err(SummarizeError::Busy);
        }
        Ok(f())
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

/// Writes and saves the summary, then names the meeting if it's still untitled.
/// Returns the summary and the new title, if any.
fn run_job(
    store: &Store,
    load: &mut ModelLoader,
    job: Job,
    on_event: &dyn Fn(SummaryEvent),
) -> Result<(Summary, Option<String>), JobError> {
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
    let summary = store.insert_summary(NewSummary {
        meeting_id,
        upto_ms: job.upto_ms,
        is_final: job.is_final,
        body_markdown: body,
        model_id: job.model.id,
    })?;
    let title = name_untitled(
        store,
        generator.as_mut(),
        meeting_id,
        &summary.body_markdown,
        &job.cancel,
    );
    Ok((summary, title))
}

/// Names a meeting that still has the default title from its saved summary, and
/// returns the new title. Never fails the summary: a problem is logged and the
/// meeting keeps the default title.
fn name_untitled(
    store: &Store,
    generator: &mut dyn Generator,
    meeting_id: i64,
    summary_markdown: &str,
    cancel: &AtomicBool,
) -> Option<String> {
    let named = (|| -> Result<Option<String>, Box<dyn std::error::Error>> {
        // Re-read the title, since you may have renamed the meeting while the summary ran.
        if store.get_meeting(meeting_id)?.meeting.title != DEFAULT_TITLE {
            return Ok(None);
        }
        match suggest_title(generator, summary_markdown, cancel)? {
            // Saved only if the title is still the default, so a rename made
            // during generation wins.
            Some(title) => Ok(store.name_untitled_meeting(meeting_id, &title)?),
            None => Ok(None),
        }
    })();
    named.unwrap_or_else(|error| {
        log::warn!("couldn't name meeting {meeting_id} from its summary: {error}");
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ModelSource, NewSegment, Source};
    use crate::summarize::template::Message;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    /// Streams two pieces for the summary, optionally waiting for permission
    /// between them, then replies to the title request with `title_reply`.
    struct FakeModel {
        gate: Option<mpsc::Receiver<()>>,
        generations: Arc<AtomicUsize>,
        title_reply: Result<&'static str, ()>,
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
            if self.generations.fetch_add(1, Ordering::Relaxed) > 0 {
                return self
                    .title_reply
                    .map(String::from)
                    .map_err(|()| SummaryError::Model("title failed".into()));
            }
            on_text("## Notes");
            if let Some(gate) = self.gate.take() {
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
        /// Calls to `generate` across all jobs: the summary, then any title.
        generations: Arc<AtomicUsize>,
    }

    struct Setup {
        gate: Option<mpsc::Receiver<()>>,
        with_model: bool,
        meeting_title: &'static str,
        title_reply: Result<&'static str, ()>,
    }

    impl Default for Setup {
        fn default() -> Self {
            Self {
                gate: None,
                with_model: true,
                meeting_title: "Sync",
                title_reply: Ok("Budget review"),
            }
        }
    }

    fn harness(gate: Option<mpsc::Receiver<()>>, with_model: bool) -> Harness {
        harness_with(Setup {
            gate,
            with_model,
            ..Setup::default()
        })
    }

    fn harness_with(setup: Setup) -> Harness {
        let Setup {
            gate,
            with_model,
            meeting_title,
            title_reply,
        } = setup;
        let store = Arc::new(Store::open_in_memory().unwrap());
        let meeting = store.create_meeting(meeting_title).unwrap();
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
        let generations = Arc::new(AtomicUsize::new(0));
        let counter = generations.clone();
        let loader: ModelLoader = Box::new(move |model| {
            if model.id == "broken" {
                return Err(SummaryError::Model("bad file".into()));
            }
            Ok(Box::new(FakeModel {
                gate: gate.lock().unwrap().take(),
                generations: counter.clone(),
                title_reply,
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
            generations,
        }
    }

    fn next(events: &mpsc::Receiver<SummaryEvent>) -> SummaryEvent {
        events.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    /// Waits for the job to finish, and returns its summary and new title.
    fn done(events: &mpsc::Receiver<SummaryEvent>) -> (Summary, Option<String>) {
        loop {
            match next(events) {
                SummaryEvent::Done { summary, title, .. } => return (summary, title),
                SummaryEvent::Failed { message, .. } => panic!("the summary failed: {message}"),
                _ => {}
            }
        }
    }

    fn title_of(h: &Harness) -> String {
        h.store.get_meeting(h.meeting_id).unwrap().meeting.title
    }

    fn untitled() -> Setup {
        Setup {
            meeting_title: "",
            ..Setup::default()
        }
    }

    #[test]
    fn names_an_untitled_meeting_from_its_summary() {
        let h = harness_with(untitled());

        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();

        let (summary, title) = done(&h.events);
        assert_eq!(title.as_deref(), Some("Budget review"));
        assert_eq!(title_of(&h), "Budget review");
        assert_eq!(h.store.latest_summary(h.meeting_id).unwrap(), Some(summary));
    }

    #[test]
    fn names_an_untitled_meeting_from_a_partial_summary() {
        let h = harness_with(untitled());

        h.summarizer.start(&h.store, h.meeting_id, false).unwrap();

        let (summary, title) = done(&h.events);
        assert!(!summary.is_final);
        assert_eq!(title.as_deref(), Some("Budget review"));
        assert_eq!(title_of(&h), "Budget review");
    }

    #[test]
    fn keeps_a_title_of_its_own_without_generating_one() {
        let h = harness_with(Setup::default());

        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();

        let (_, title) = done(&h.events);
        assert_eq!(title, None);
        assert_eq!(title_of(&h), "Sync");
        assert_eq!(h.generations.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn keeps_a_rename_made_while_the_summary_ran() {
        let (release, gate) = mpsc::channel();
        let h = harness_with(Setup {
            gate: Some(gate),
            ..untitled()
        });
        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();

        h.store.rename_meeting(h.meeting_id, "Mine").unwrap();
        release.send(()).unwrap();

        let (_, title) = done(&h.events);
        assert_eq!(title, None);
        assert_eq!(title_of(&h), "Mine");
    }

    #[test]
    fn keeps_the_summary_when_naming_fails() {
        let h = harness_with(Setup {
            title_reply: Err(()),
            ..untitled()
        });

        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();

        let (summary, title) = done(&h.events);
        assert_eq!(title, None);
        assert_eq!(title_of(&h), DEFAULT_TITLE);
        assert_eq!(h.store.latest_summary(h.meeting_id).unwrap(), Some(summary));
        assert_eq!(h.generations.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn keeps_the_default_when_the_reply_is_empty() {
        let h = harness_with(Setup {
            title_reply: Ok("  \n"),
            ..untitled()
        });

        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();

        let (_, title) = done(&h.events);
        assert_eq!(title, None);
        assert_eq!(title_of(&h), DEFAULT_TITLE);
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
    fn runs_work_only_while_idle() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        assert_eq!(h.summarizer.when_idle(|| 7).unwrap(), 7);
        h.summarizer.start(&h.store, h.meeting_id, false).unwrap();

        assert!(matches!(
            h.summarizer.when_idle(|| unreachable!()),
            Err(SummarizeError::Busy)
        ));

        release.send(()).unwrap();
        while !matches!(next(&h.events), SummaryEvent::Done { .. }) {}
        assert!(h.summarizer.when_idle(|| ()).is_ok());
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
