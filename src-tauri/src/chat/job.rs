//! Runs one question on the summary thread, which owns the shared model.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::answer::{Ask, answer};
use crate::store::{ChatMessage, ModelRecord, NewExchange, Store, StoreError, TranscriptSegment};
use crate::summarize::resident::Resident;
use crate::summarize::run::SummaryError;
use crate::summarize::service::Stage;

/// Longest question accepted, in characters.
pub const MAX_QUESTION_CHARS: usize = 2000;

#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    /// `Stage::Loading` while the model loads, then `Stage::Writing`.
    Progress {
        meeting_id: i64,
        stage: Stage,
    },
    Text {
        meeting_id: i64,
        text: String,
    },
    /// The question and answer, as saved.
    Done {
        meeting_id: i64,
        question: ChatMessage,
        answer: ChatMessage,
    },
    Failed {
        meeting_id: i64,
        message: String,
        cancelled: bool,
    },
}

/// Everything one question needs, read when it was asked.
pub struct ChatJob {
    pub meeting_id: i64,
    pub model: ModelRecord,
    pub title: String,
    pub notes_markdown: String,
    pub segments: Vec<TranscriptSegment>,
    pub terms: Vec<String>,
    pub history: Vec<ChatMessage>,
    pub question: String,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Debug, thiserror::Error)]
enum JobError {
    #[error(transparent)]
    Answer(#[from] SummaryError),
    #[error("couldn't save the answer: {0}")]
    Store(#[from] StoreError),
}

/// Answers the question and saves it with its answer. Returns the event that
/// reports the result.
pub fn run(
    store: &Store,
    resident: &mut Resident,
    job: ChatJob,
    on_event: &dyn Fn(ChatEvent),
) -> ChatEvent {
    let meeting_id = job.meeting_id;
    match answer_and_save(store, resident, job, on_event) {
        Ok((question, answer)) => ChatEvent::Done {
            meeting_id,
            question,
            answer,
        },
        Err(error) => ChatEvent::Failed {
            meeting_id,
            cancelled: matches!(error, JobError::Answer(SummaryError::Cancelled)),
            message: error.to_string(),
        },
    }
}

fn answer_and_save(
    store: &Store,
    resident: &mut Resident,
    job: ChatJob,
    on_event: &dyn Fn(ChatEvent),
) -> Result<(ChatMessage, ChatMessage), JobError> {
    let meeting_id = job.meeting_id;
    let cancelled = || job.cancel.load(Ordering::Relaxed);
    if cancelled() {
        return Err(SummaryError::Cancelled.into());
    }
    let generator = resident.get(&job.model, || {
        on_event(ChatEvent::Progress {
            meeting_id,
            stage: Stage::Loading,
        })
    })?;
    if cancelled() {
        return Err(SummaryError::Cancelled.into());
    }
    on_event(ChatEvent::Progress {
        meeting_id,
        stage: Stage::Writing,
    });
    let ask = Ask {
        title: &job.title,
        notes_markdown: &job.notes_markdown,
        segments: &job.segments,
        terms: &job.terms,
        history: &job.history,
        question: &job.question,
    };
    let answer = answer(generator, &ask, &job.cancel, &mut |text| {
        on_event(ChatEvent::Text {
            meeting_id,
            text: text.to_owned(),
        })
    })?;
    Ok(store.insert_chat_exchange(NewExchange {
        meeting_id,
        question: job.question,
        answer_markdown: answer.markdown,
        sources: answer.sources,
        model_id: job.model.id,
    })?)
}
