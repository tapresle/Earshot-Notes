//! Runs one summary at a time on a background thread and reports progress.
//! The model stays loaded between summaries for as long as `KeepLoaded` says.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use serde::Serialize;

use super::resident::{KeepLoaded, ModelLoader, Resident};
use super::run::{Generator, Progress, Request, SummaryError, summarize};
use super::title::suggest_title;
use crate::store::settings::KEEP_MODEL_LOADED;
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

struct Job {
    meeting_id: i64,
    model: ModelRecord,
    title: String,
    notes_markdown: String,
    segments: Vec<TranscriptSegment>,
    terms: Vec<String>,
    upto_ms: i64,
    is_final: bool,
    cancel: Arc<AtomicBool>,
}

type Active = Arc<Mutex<Option<(i64, Arc<AtomicBool>)>>>;

enum Message {
    Job(Box<Job>),
    /// How long an idle model stays loaded; `None` keeps it indefinitely.
    IdleLimit(Option<Duration>),
    Unload,
    /// Frees the model and stops the thread, then confirms.
    Shutdown(Sender<()>),
}

pub struct Summarizer {
    inbox: Sender<Message>,
    active: Active,
}

impl Summarizer {
    /// Starts the summary thread, which keeps the model loaded as the stored
    /// `KEEP_MODEL_LOADED` setting says.
    pub fn spawn(
        store: Arc<Store>,
        load: ModelLoader,
        on_event: impl Fn(SummaryEvent) + Send + 'static,
    ) -> Self {
        let keep = store.setting(KEEP_MODEL_LOADED).unwrap_or_else(|error| {
            log::warn!("couldn't read how long to keep the model loaded: {error}");
            None
        });
        let idle_limit = KeepLoaded::from_setting(keep.as_deref()).idle_limit();
        let (inbox, messages) = mpsc::channel();
        let active: Active = Arc::default();
        let finished = active.clone();
        thread::Builder::new()
            .name("summarizer".into())
            .spawn(move || {
                run_worker(
                    &store,
                    Resident::new(load),
                    idle_limit,
                    &messages,
                    &finished,
                    &on_event,
                )
            })
            .expect("couldn't start the summarizer thread");
        Self { inbox, active }
    }

    /// Changes how long the model stays loaded after each request. The idle
    /// time starts over now, and `KeepLoaded::Never` frees it at once.
    pub fn keep_loaded(&self, keep: KeepLoaded) {
        self.set_idle_limit(keep.idle_limit());
    }

    fn set_idle_limit(&self, limit: Option<Duration>) {
        let _ = self.inbox.send(Message::IdleLimit(limit));
    }

    /// Frees the model once any running summary ends. Used when the selected
    /// model changes or is removed.
    pub fn unload(&self) {
        let _ = self.inbox.send(Message::Unload);
    }

    /// Cancels any running summary, frees the model, and waits up to
    /// `timeout` for the thread to confirm. ggml's Metal backend aborts the
    /// process at exit if a model is still loaded.
    pub fn shutdown(&self, timeout: Duration) {
        self.cancel();
        let (done, confirmed) = mpsc::channel();
        if self.inbox.send(Message::Shutdown(done)).is_err() {
            return;
        }
        if confirmed.recv_timeout(timeout).is_err() {
            log::warn!("the summarizer didn't stop within {timeout:?}");
        }
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
            terms: store
                .vocabulary()?
                .terms()
                .iter()
                .map(|t| t.term.clone())
                .collect(),
            is_final,
            cancel: cancel.clone(),
        };
        self.inbox
            .send(Message::Job(Box::new(job)))
            .map_err(|_| SummarizeError::Stopped)?;
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

/// Runs jobs as they arrive, and frees the model once it has been idle for
/// `idle_limit`.
fn run_worker(
    store: &Store,
    mut resident: Resident,
    mut idle_limit: Option<Duration>,
    messages: &Receiver<Message>,
    finished: &Active,
    on_event: &dyn Fn(SummaryEvent),
) {
    loop {
        let message = match idle_limit.filter(|_| resident.is_loaded()) {
            Some(limit) => match messages.recv_timeout(limit) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    resident.unload();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match messages.recv() {
                Ok(message) => message,
                Err(_) => return,
            },
        };
        match message {
            Message::Job(job) => {
                let meeting_id = job.meeting_id;
                let event = match run_job(store, &mut resident, *job, on_event) {
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
                *lock(finished) = None;
                on_event(event);
            }
            Message::IdleLimit(limit) => idle_limit = limit,
            Message::Unload => resident.unload(),
            Message::Shutdown(done) => {
                resident.unload();
                let _ = done.send(());
                return;
            }
        }
    }
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
    resident: &mut Resident,
    job: Job,
    on_event: &dyn Fn(SummaryEvent),
) -> Result<(Summary, Option<String>), JobError> {
    let meeting_id = job.meeting_id;
    if job.cancel.load(Ordering::Relaxed) {
        return Err(SummaryError::Cancelled.into());
    }
    let generator = resident.get(&job.model, || {
        on_event(SummaryEvent::Progress {
            meeting_id,
            stage: Stage::Loading,
        })
    })?;
    if job.cancel.load(Ordering::Relaxed) {
        return Err(SummaryError::Cancelled.into());
    }
    let request = Request {
        title: &job.title,
        notes_markdown: &job.notes_markdown,
        segments: &job.segments,
        terms: &job.terms,
    };
    let body = summarize(
        &mut *generator,
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
        &mut *generator,
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
        unloads: Arc<AtomicUsize>,
    }

    impl Drop for FakeModel {
        fn drop(&mut self) {
            self.unloads.fetch_add(1, Ordering::Relaxed);
        }
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
        loads: Arc<AtomicUsize>,
        unloads: Arc<AtomicUsize>,
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
        let loads = Arc::new(AtomicUsize::new(0));
        let unloads = Arc::new(AtomicUsize::new(0));
        let (counter, loaded, unloaded) = (generations.clone(), loads.clone(), unloads.clone());
        let loader: ModelLoader = Box::new(move |model| {
            if model.id == "broken" {
                return Err(SummaryError::Model("bad file".into()));
            }
            loaded.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(FakeModel {
                gate: gate.lock().unwrap().take(),
                generations: counter.clone(),
                title_reply,
                unloads: unloaded.clone(),
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
            loads,
            unloads,
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
    fn names_a_meeting_renamed_to_blank() {
        let h = harness_with(Setup::default());
        h.store.rename_meeting(h.meeting_id, " \t\n ").unwrap();

        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();

        let (_, title) = done(&h.events);
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

    /// Waits up to five seconds for `condition`, which another thread makes true.
    fn eventually(condition: impl Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if condition() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        condition()
    }

    fn count(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::Relaxed)
    }

    /// Summarizes the meeting and waits for the result.
    fn summarize_once(h: &Harness) {
        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();
        done(&h.events);
    }

    #[test]
    fn keeps_the_model_loaded_for_the_next_summary() {
        let h = harness(None, true);
        summarize_once(&h);

        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();

        // No loading stage when the model is already loaded.
        assert_eq!(
            next(&h.events),
            SummaryEvent::Progress {
                meeting_id: h.meeting_id,
                stage: Stage::Writing
            }
        );
        done(&h.events);
        assert_eq!((count(&h.loads), count(&h.unloads)), (1, 0));
    }

    #[test]
    fn frees_the_model_after_each_summary_when_set_to_never() {
        let h = harness(None, true);
        h.summarizer.keep_loaded(KeepLoaded::Never);

        summarize_once(&h);
        assert!(eventually(|| count(&h.unloads) == 1));
        summarize_once(&h);

        assert_eq!(count(&h.loads), 2);
        assert!(eventually(|| count(&h.unloads) == 2));
    }

    #[test]
    fn frees_the_model_once_idle() {
        let h = harness(None, true);
        h.summarizer.set_idle_limit(Some(Duration::from_millis(50)));

        summarize_once(&h);

        assert!(eventually(|| count(&h.unloads) == 1));
        assert_eq!(count(&h.loads), 1);
    }

    #[test]
    fn keeps_the_model_while_idle_when_set_to_always() {
        let h = harness(None, true);
        h.summarizer.set_idle_limit(Some(Duration::from_millis(10)));
        h.summarizer.keep_loaded(KeepLoaded::Always);

        summarize_once(&h);
        thread::sleep(Duration::from_millis(100));

        assert_eq!(count(&h.unloads), 0);
    }

    #[test]
    fn frees_the_model_when_asked() {
        let h = harness(None, true);
        summarize_once(&h);

        h.summarizer.unload();

        assert!(eventually(|| count(&h.unloads) == 1));
        summarize_once(&h);
        assert_eq!(count(&h.loads), 2);
    }

    #[test]
    fn loads_the_newly_selected_model() {
        let h = harness(None, true);
        summarize_once(&h);
        h.store
            .add_model(&ModelRecord {
                id: "other-model".into(),
                display_name: "Other".into(),
                file_path: "/models/other.gguf".into(),
                size_bytes: 1,
                sha256: "def".into(),
                source: ModelSource::Import,
                source_url: None,
                version: None,
                min_runtime: None,
                added_at: 0,
            })
            .unwrap();
        h.store.select_summary_model("other-model").unwrap();

        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();

        let (summary, _) = done(&h.events);
        assert_eq!(summary.model_id, "other-model");
        assert_eq!((count(&h.loads), count(&h.unloads)), (2, 1));
    }

    #[test]
    fn frees_the_model_on_shutdown() {
        let h = harness(None, true);
        summarize_once(&h);

        h.summarizer.shutdown(Duration::from_secs(5));

        // Shutdown waits for the thread to confirm, so the model is already freed.
        assert_eq!(count(&h.unloads), 1);
        assert!(matches!(
            h.summarizer.start(&h.store, h.meeting_id, true),
            Err(SummarizeError::Stopped)
        ));
    }

    #[test]
    fn cancels_a_running_summary_on_shutdown() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        h.summarizer.start(&h.store, h.meeting_id, true).unwrap();
        // The model is generating once the first text arrives.
        while !matches!(next(&h.events), SummaryEvent::Text { .. }) {}
        let cancelled = || {
            lock(&h.summarizer.active)
                .as_ref()
                .is_some_and(|(_, cancel)| cancel.load(Ordering::Relaxed))
        };

        thread::scope(|scope| {
            scope.spawn(|| h.summarizer.shutdown(Duration::from_secs(5)));
            assert!(eventually(cancelled));
            release.send(()).unwrap();
        });

        assert_eq!(count(&h.unloads), 1);
        assert!(matches!(
            next(&h.events),
            SummaryEvent::Failed {
                cancelled: true,
                ..
            }
        ));
        assert_eq!(h.store.latest_summary(h.meeting_id).unwrap(), None);
    }
}
