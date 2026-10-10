//! Runs one summary or chat question at a time on a background thread, which
//! owns the model they share, and reports progress. The model stays loaded
//! between requests for as long as `KeepLoaded` says.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use serde::Serialize;

use super::resident::{KeepLoaded, ModelLoader, Resident};
use crate::chat::job::{self as chat_job, ChatEvent, ChatJob, MAX_QUESTION_CHARS};
use crate::store::settings::KEEP_MODEL_LOADED;
use crate::store::{Store, StoreError};
use crate::summarize::job::{self as summary_job, SummaryEvent, SummaryJob};

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

/// The kind of request the model is working on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Summary,
    Chat,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("{}", match .0 {
        Work::Summary => "a summary is being written; wait for it to finish or cancel it",
        Work::Chat => "an answer is being written; wait for it to finish or cancel it",
    })]
    Busy(Work),
    #[error("type a question first")]
    EmptyQuestion,
    #[error("questions can be at most {MAX_QUESTION_CHARS} characters")]
    QuestionTooLong,
    #[error("there's nothing to ask about yet. Record the meeting or write some notes first")]
    NothingToAsk,
    #[error("add a local AI model first")]
    NoModel,
    #[error("there's nothing to summarize yet. Record the meeting or write some notes first")]
    NothingToSummarize,
    #[error("the summarizer stopped unexpectedly; restart the app to summarize again")]
    Stopped,
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The request in progress, if any.
struct Running {
    meeting_id: i64,
    work: Work,
    cancel: Arc<AtomicBool>,
}

type Active = Arc<Mutex<Option<Running>>>;

enum Message {
    Summary(Box<SummaryJob>),
    Chat(Box<ChatJob>),
    /// How long an idle model stays loaded; `None` keeps it indefinitely.
    IdleLimit(Option<Duration>),
    Unload,
    /// Frees the model and stops the thread, then confirms.
    Shutdown(Sender<()>),
}

pub struct ModelWorker {
    inbox: Sender<Message>,
    active: Active,
}

impl ModelWorker {
    /// Starts the worker thread, which keeps the model loaded as the stored
    /// `KEEP_MODEL_LOADED` setting says. Summaries report through `on_event`,
    /// and chat questions through `on_chat`.
    pub fn spawn(
        store: Arc<Store>,
        load: ModelLoader,
        on_event: impl Fn(SummaryEvent) + Send + 'static,
        on_chat: impl Fn(ChatEvent) + Send + 'static,
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
            .name("model-worker".into())
            .spawn(move || {
                run_worker(
                    &store,
                    Resident::new(load),
                    idle_limit,
                    &messages,
                    &finished,
                    &on_event,
                    &on_chat,
                )
            })
            .expect("couldn't start the worker thread");
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

    /// Cancels any running summary or answer, frees the model, and waits up
    /// to `timeout` for the thread to confirm. ggml's Metal backend aborts the
    /// process at exit if a model is still loaded.
    pub fn shutdown(&self, timeout: Duration) {
        if let Some(running) = lock(&self.active).as_ref() {
            running.cancel.store(true, Ordering::Relaxed);
        }
        let (done, confirmed) = mpsc::channel();
        if self.inbox.send(Message::Shutdown(done)).is_err() {
            return;
        }
        if confirmed.recv_timeout(timeout).is_err() {
            log::warn!("the model worker didn't stop within {timeout:?}");
        }
    }

    /// Queues a summary of the meeting as it stands now. `is_final` marks a
    /// meeting that has stopped recording.
    pub fn summarize(
        &self,
        store: &Store,
        meeting_id: i64,
        is_final: bool,
    ) -> Result<(), WorkerError> {
        let mut active = lock(&self.active);
        refuse_if_busy(&active)?;
        let model = store.selected_model()?.ok_or(WorkerError::NoModel)?;
        let detail = store.get_meeting(meeting_id)?;
        let segments = store.list_segments(meeting_id)?;
        if segments.is_empty() && detail.notes_markdown.trim().is_empty() {
            return Err(WorkerError::NothingToSummarize);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let job = SummaryJob {
            meeting_id,
            model,
            title: detail.meeting.title,
            notes_markdown: detail.notes_markdown,
            upto_ms: segments.iter().map(|s| s.end_ms).max().unwrap_or(0),
            segments,
            terms: terms(store)?,
            is_final,
            cancel: cancel.clone(),
        };
        self.inbox
            .send(Message::Summary(Box::new(job)))
            .map_err(|_| WorkerError::Stopped)?;
        *active = Some(Running {
            meeting_id,
            work: Work::Summary,
            cancel,
        });
        Ok(())
    }

    /// Queues a question about the meeting as it stands now. The answer sees
    /// the earlier questions and answers.
    pub fn ask(&self, store: &Store, meeting_id: i64, question: &str) -> Result<(), WorkerError> {
        let question = question.trim();
        if question.is_empty() {
            return Err(WorkerError::EmptyQuestion);
        }
        if question.chars().count() > MAX_QUESTION_CHARS {
            return Err(WorkerError::QuestionTooLong);
        }
        let mut active = lock(&self.active);
        refuse_if_busy(&active)?;
        let model = store.selected_model()?.ok_or(WorkerError::NoModel)?;
        let detail = store.get_meeting(meeting_id)?;
        let segments = store.list_segments(meeting_id)?;
        if segments.is_empty() && detail.notes_markdown.trim().is_empty() {
            return Err(WorkerError::NothingToAsk);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let job = ChatJob {
            meeting_id,
            model,
            title: detail.meeting.title,
            notes_markdown: detail.notes_markdown,
            segments,
            terms: terms(store)?,
            history: store.chat_messages(meeting_id)?,
            question: question.to_owned(),
            cancel: cancel.clone(),
        };
        self.inbox
            .send(Message::Chat(Box::new(job)))
            .map_err(|_| WorkerError::Stopped)?;
        *active = Some(Running {
            meeting_id,
            work: Work::Chat,
            cancel,
        });
        Ok(())
    }

    /// Asks the running summary to stop. Returns whether one was running.
    pub fn cancel_summary(&self) -> bool {
        self.cancel_work(Work::Summary)
    }

    /// Asks the answer being written to stop. Returns whether one was.
    pub fn cancel_chat(&self) -> bool {
        self.cancel_work(Work::Chat)
    }

    fn cancel_work(&self, work: Work) -> bool {
        match lock(&self.active).as_ref() {
            Some(running) if running.work == work => {
                running.cancel.store(true, Ordering::Relaxed);
                true
            }
            _ => false,
        }
    }

    /// Runs `f` only while no summary or answer is being written, and keeps
    /// one from starting until it returns. Used to change models safely.
    pub fn when_idle<T>(&self, f: impl FnOnce() -> T) -> Result<T, WorkerError> {
        let active = lock(&self.active);
        refuse_if_busy(&active)?;
        Ok(f())
    }

    /// The meeting being summarized, if any.
    pub fn active_summary(&self) -> Option<i64> {
        self.active_for(Work::Summary)
    }

    /// The meeting whose question is being answered, if any.
    pub fn active_chat(&self) -> Option<i64> {
        self.active_for(Work::Chat)
    }

    fn active_for(&self, work: Work) -> Option<i64> {
        lock(&self.active)
            .as_ref()
            .filter(|running| running.work == work)
            .map(|running| running.meeting_id)
    }
}

fn lock(active: &Active) -> MutexGuard<'_, Option<Running>> {
    active.lock().unwrap_or_else(|e| e.into_inner())
}

fn refuse_if_busy(active: &Option<Running>) -> Result<(), WorkerError> {
    match active {
        Some(running) => Err(WorkerError::Busy(running.work)),
        None => Ok(()),
    }
}

/// The vocabulary's terms, for the model to spell.
fn terms(store: &Store) -> Result<Vec<String>, WorkerError> {
    Ok(store
        .vocabulary()?
        .terms()
        .iter()
        .map(|t| t.term.clone())
        .collect())
}

/// Runs jobs as they arrive, and frees the model once it has been idle for
/// `idle_limit`.
fn run_worker(
    store: &Store,
    mut resident: Resident,
    mut idle_limit: Option<Duration>,
    messages: &Receiver<Message>,
    finished: &Active,
    on_summary: &dyn Fn(SummaryEvent),
    on_chat: &dyn Fn(ChatEvent),
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
            Message::Summary(job) => {
                let event = summary_job::run(store, &mut resident, *job, on_summary);
                // Clear the slot before announcing, so the UI can start the next one.
                *lock(finished) = None;
                on_summary(event);
            }
            Message::Chat(job) => {
                let event = chat_job::run(store, &mut resident, *job, on_chat);
                *lock(finished) = None;
                on_chat(event);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::generator::{GenerateError, Generator};
    use crate::llm::template::Message;
    use crate::store::{ChatMessage, ModelSource, NewSegment, Source};
    use crate::store::{DEFAULT_TITLE, ModelRecord, Summary};
    use crate::summarize::job::SummaryEvent;
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
        fn prompt_tokens(&mut self, _: &[Message]) -> Result<usize, GenerateError> {
            Ok(100)
        }
        fn count_tokens(&mut self, text: &str) -> Result<usize, GenerateError> {
            Ok(text.len())
        }
        fn generate(
            &mut self,
            _: &[Message],
            _: usize,
            cancel: &AtomicBool,
            on_text: &mut dyn FnMut(&str),
        ) -> Result<String, GenerateError> {
            if self.generations.fetch_add(1, Ordering::Relaxed) > 0 {
                return self
                    .title_reply
                    .map(String::from)
                    .map_err(|()| GenerateError::Model("title failed".into()));
            }
            on_text("## Notes");
            if let Some(gate) = self.gate.take() {
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(GenerateError::Cancelled);
            }
            on_text("\n- done");
            Ok("## Notes\n- done".into())
        }
    }

    struct Harness {
        store: Arc<Store>,
        worker: ModelWorker,
        events: mpsc::Receiver<SummaryEvent>,
        chat: mpsc::Receiver<ChatEvent>,
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
                return Err(GenerateError::Model("bad file".into()));
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
        let (chat_tx, chat) = mpsc::channel();
        let chat_tx = Mutex::new(chat_tx);
        let worker = ModelWorker::spawn(
            store.clone(),
            loader,
            move |event| tx.lock().unwrap().send(event).unwrap(),
            move |event| chat_tx.lock().unwrap().send(event).unwrap(),
        );
        Harness {
            store,
            worker,
            events,
            chat,
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

        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

        let (summary, title) = done(&h.events);
        assert_eq!(title.as_deref(), Some("Budget review"));
        assert_eq!(title_of(&h), "Budget review");
        assert_eq!(h.store.latest_summary(h.meeting_id).unwrap(), Some(summary));
    }

    #[test]
    fn names_an_untitled_meeting_from_a_partial_summary() {
        let h = harness_with(untitled());

        h.worker.summarize(&h.store, h.meeting_id, false).unwrap();

        let (summary, title) = done(&h.events);
        assert!(!summary.is_final);
        assert_eq!(title.as_deref(), Some("Budget review"));
        assert_eq!(title_of(&h), "Budget review");
    }

    #[test]
    fn names_a_meeting_renamed_to_blank() {
        let h = harness_with(Setup::default());
        h.store.rename_meeting(h.meeting_id, " \t\n ").unwrap();

        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

        let (_, title) = done(&h.events);
        assert_eq!(title.as_deref(), Some("Budget review"));
        assert_eq!(title_of(&h), "Budget review");
    }

    #[test]
    fn keeps_a_title_of_its_own_without_generating_one() {
        let h = harness_with(Setup::default());

        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

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
        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

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

        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

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

        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

        let (_, title) = done(&h.events);
        assert_eq!(title, None);
        assert_eq!(title_of(&h), DEFAULT_TITLE);
    }

    #[test]
    fn streams_progress_and_text_then_saves_the_summary() {
        let h = harness(None, true);

        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

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
        assert_eq!(h.worker.active_summary(), None);
    }

    #[test]
    fn runs_one_summary_at_a_time() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        h.worker.summarize(&h.store, h.meeting_id, false).unwrap();

        assert!(matches!(
            h.worker.summarize(&h.store, h.meeting_id, false),
            Err(WorkerError::Busy(Work::Summary))
        ));
        assert_eq!(h.worker.active_summary(), Some(h.meeting_id));

        release.send(()).unwrap();
        while !matches!(next(&h.events), SummaryEvent::Done { .. }) {}
        assert!(h.worker.summarize(&h.store, h.meeting_id, true).is_ok());
    }

    #[test]
    fn runs_work_only_while_idle() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        assert_eq!(h.worker.when_idle(|| 7).unwrap(), 7);
        h.worker.summarize(&h.store, h.meeting_id, false).unwrap();

        assert!(matches!(
            h.worker.when_idle(|| unreachable!()),
            Err(WorkerError::Busy(Work::Summary))
        ));

        release.send(()).unwrap();
        while !matches!(next(&h.events), SummaryEvent::Done { .. }) {}
        assert!(h.worker.when_idle(|| ()).is_ok());
    }

    #[test]
    fn cancels_without_saving() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        h.worker.summarize(&h.store, h.meeting_id, false).unwrap();

        assert!(h.worker.cancel_summary());
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
            h.worker.summarize(&h.store, h.meeting_id, true),
            Err(WorkerError::NoModel)
        ));
    }

    #[test]
    fn refuses_an_empty_meeting_up_front() {
        let h = harness(None, true);
        let empty = h.store.create_meeting("Empty").unwrap();

        assert!(matches!(
            h.worker.summarize(&h.store, empty.id, true),
            Err(WorkerError::NothingToSummarize)
        ));
        assert_eq!(h.worker.active_summary(), None);
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

        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

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
        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();
        done(&h.events);
    }

    #[test]
    fn keeps_the_model_loaded_for_the_next_summary() {
        let h = harness(None, true);
        summarize_once(&h);

        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

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
        h.worker.keep_loaded(KeepLoaded::Never);

        summarize_once(&h);
        assert!(eventually(|| count(&h.unloads) == 1));
        summarize_once(&h);

        assert_eq!(count(&h.loads), 2);
        assert!(eventually(|| count(&h.unloads) == 2));
    }

    #[test]
    fn frees_the_model_once_idle() {
        let h = harness(None, true);
        h.worker.set_idle_limit(Some(Duration::from_millis(50)));

        summarize_once(&h);

        assert!(eventually(|| count(&h.unloads) == 1));
        assert_eq!(count(&h.loads), 1);
    }

    #[test]
    fn keeps_the_model_while_idle_when_set_to_always() {
        let h = harness(None, true);
        h.worker.set_idle_limit(Some(Duration::from_millis(10)));
        h.worker.keep_loaded(KeepLoaded::Always);

        summarize_once(&h);
        thread::sleep(Duration::from_millis(100));

        assert_eq!(count(&h.unloads), 0);
    }

    #[test]
    fn frees_the_model_when_asked() {
        let h = harness(None, true);
        summarize_once(&h);

        h.worker.unload();

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
        h.store.select_model("other-model").unwrap();

        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

        let (summary, _) = done(&h.events);
        assert_eq!(summary.model_id, "other-model");
        assert_eq!((count(&h.loads), count(&h.unloads)), (2, 1));
    }

    #[test]
    fn frees_the_model_on_shutdown() {
        let h = harness(None, true);
        summarize_once(&h);

        h.worker.shutdown(Duration::from_secs(5));

        // Shutdown waits for the thread to confirm, so the model is already freed.
        assert_eq!(count(&h.unloads), 1);
        assert!(matches!(
            h.worker.summarize(&h.store, h.meeting_id, true),
            Err(WorkerError::Stopped)
        ));
    }

    #[test]
    fn cancels_a_running_summary_on_shutdown() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();
        // The model is generating once the first text arrives.
        while !matches!(next(&h.events), SummaryEvent::Text { .. }) {}
        let cancelled = || {
            lock(&h.worker.active)
                .as_ref()
                .is_some_and(|running| running.cancel.load(Ordering::Relaxed))
        };

        thread::scope(|scope| {
            scope.spawn(|| h.worker.shutdown(Duration::from_secs(5)));
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

    /// Waits for the answer, and returns the saved question and answer.
    fn answered(h: &Harness) -> (ChatMessage, ChatMessage) {
        loop {
            match h.chat.recv_timeout(Duration::from_secs(5)).unwrap() {
                ChatEvent::Done {
                    question, answer, ..
                } => return (question, answer),
                ChatEvent::Failed { message, .. } => panic!("the answer failed: {message}"),
                _ => {}
            }
        }
    }

    #[test]
    fn answers_a_question_and_saves_it_with_the_answer() {
        let h = harness(None, true);

        h.worker
            .ask(&h.store, h.meeting_id, "  When do we ship?  ")
            .unwrap();

        let id = h.meeting_id;
        let first = h.chat.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            first,
            ChatEvent::Progress {
                meeting_id: id,
                stage: Stage::Loading
            }
        );
        let (question, answer) = answered(&h);
        assert_eq!(question.body_markdown, "When do we ship?");
        assert_eq!(answer.body_markdown, "## Notes\n- done");
        assert_eq!(answer.model_id.as_deref(), Some("test-model"));
        assert_eq!(h.store.chat_messages(id).unwrap(), [question, answer]);
        assert_eq!(h.worker.active_chat(), None);
    }

    #[test]
    fn shares_the_loaded_model_between_summaries_and_chat() {
        let h = harness(None, true);
        summarize_once(&h);

        h.worker.ask(&h.store, h.meeting_id, "Ship?").unwrap();
        answered(&h);

        assert_eq!(count(&h.loads), 1);
    }

    #[test]
    fn refuses_a_question_while_a_summary_runs() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        h.worker.summarize(&h.store, h.meeting_id, true).unwrap();

        let refused = h.worker.ask(&h.store, h.meeting_id, "Ship?");

        assert!(matches!(refused, Err(WorkerError::Busy(Work::Summary))));
        assert_eq!(
            refused.unwrap_err().to_string(),
            "a summary is being written; wait for it to finish or cancel it"
        );
        assert!(!h.worker.cancel_chat(), "nothing to cancel in chat");
        release.send(()).unwrap();
        done(&h.events);
    }

    #[test]
    fn refuses_a_summary_while_answering() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        h.worker.ask(&h.store, h.meeting_id, "Ship?").unwrap();

        let refused = h.worker.summarize(&h.store, h.meeting_id, true);

        assert!(matches!(refused, Err(WorkerError::Busy(Work::Chat))));
        assert_eq!(h.worker.active_chat(), Some(h.meeting_id));
        assert_eq!(h.worker.active_summary(), None);
        assert!(
            !h.worker.cancel_summary(),
            "a summary's cancel leaves chat alone"
        );
        release.send(()).unwrap();
        answered(&h);
    }

    #[test]
    fn cancels_an_answer_without_saving() {
        let (release, gate) = mpsc::channel();
        let h = harness(Some(gate), true);
        h.worker.ask(&h.store, h.meeting_id, "Ship?").unwrap();

        assert!(h.worker.cancel_chat());
        release.send(()).unwrap();

        let failed = loop {
            if let event @ ChatEvent::Failed { .. } =
                h.chat.recv_timeout(Duration::from_secs(5)).unwrap()
            {
                break event;
            }
        };
        assert!(matches!(
            failed,
            ChatEvent::Failed {
                cancelled: true,
                ..
            }
        ));
        assert!(h.store.chat_messages(h.meeting_id).unwrap().is_empty());
    }

    #[test]
    fn checks_the_question_and_the_meeting_before_asking() {
        let h = harness(None, true);
        let long = "x".repeat(MAX_QUESTION_CHARS + 1);
        let empty = h.store.create_meeting("Empty").unwrap();

        assert!(matches!(
            h.worker.ask(&h.store, h.meeting_id, " \n "),
            Err(WorkerError::EmptyQuestion)
        ));
        assert!(matches!(
            h.worker.ask(&h.store, h.meeting_id, &long),
            Err(WorkerError::QuestionTooLong)
        ));
        assert!(matches!(
            h.worker.ask(&h.store, empty.id, "Ship?"),
            Err(WorkerError::NothingToAsk)
        ));
        assert_eq!(h.worker.active_chat(), None);
        let no_model = harness(None, false);
        assert!(matches!(
            no_model
                .worker
                .ask(&no_model.store, no_model.meeting_id, "Ship?"),
            Err(WorkerError::NoModel)
        ));
    }
}
