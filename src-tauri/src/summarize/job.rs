//! Runs one summary on the model worker's thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::run::{Progress, Request, SummaryError, summarize};
use super::title::suggest_title;
use crate::llm::generator::{GenerateError, Generator};
use crate::llm::resident::Resident;
use crate::llm::worker::Stage;
use crate::store::{
    DEFAULT_TITLE, ModelRecord, NewSummary, Store, StoreError, Summary, TranscriptSegment,
};

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

/// Everything one summary needs, read when it was requested.
pub struct SummaryJob {
    pub meeting_id: i64,
    pub model: ModelRecord,
    pub title: String,
    pub notes_markdown: String,
    pub segments: Vec<TranscriptSegment>,
    pub terms: Vec<String>,
    pub upto_ms: i64,
    pub is_final: bool,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Debug, thiserror::Error)]
enum JobError {
    #[error(transparent)]
    Summary(#[from] SummaryError),
    #[error("couldn't save the summary: {0}")]
    Store(#[from] StoreError),
}

impl From<GenerateError> for JobError {
    fn from(error: GenerateError) -> Self {
        JobError::Summary(error.into())
    }
}

/// Writes and saves the summary, then names the meeting if it's still
/// untitled. Returns the event that reports the result.
pub fn run(
    store: &Store,
    resident: &mut Resident,
    job: SummaryJob,
    on_event: &dyn Fn(SummaryEvent),
) -> SummaryEvent {
    let meeting_id = job.meeting_id;
    match summarize_and_save(store, resident, job, on_event) {
        Ok((summary, title)) => SummaryEvent::Done {
            meeting_id,
            summary,
            title,
        },
        Err(error) => SummaryEvent::Failed {
            meeting_id,
            cancelled: matches!(
                error,
                JobError::Summary(SummaryError::Generate(GenerateError::Cancelled))
            ),
            message: error.to_string(),
        },
    }
}

/// Returns the summary and the new title, if any.
fn summarize_and_save(
    store: &Store,
    resident: &mut Resident,
    job: SummaryJob,
    on_event: &dyn Fn(SummaryEvent),
) -> Result<(Summary, Option<String>), JobError> {
    let meeting_id = job.meeting_id;
    if job.cancel.load(Ordering::Relaxed) {
        return Err(GenerateError::Cancelled.into());
    }
    let generator = resident.get(&job.model, || {
        on_event(SummaryEvent::Progress {
            meeting_id,
            stage: Stage::Loading,
        })
    })?;
    if job.cancel.load(Ordering::Relaxed) {
        return Err(GenerateError::Cancelled.into());
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
