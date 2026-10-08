//! Decides between one pass and condensing in parts, then drives the model.

use std::sync::atomic::AtomicBool;

use super::prompt::{self, combine_messages, part_messages, summary_messages, transcript_line};
use super::template::{Message, TemplateError};
use crate::store::TranscriptSegment;

/// Room for the finished notes.
pub const MAX_SUMMARY_TOKENS: usize = 1536;
/// Room for each condensed part of a long transcript.
pub const MAX_PART_TOKENS: usize = 512;
/// Slack for tokenizer differences between counting and generating.
const MARGIN_TOKENS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("there's nothing to summarize yet: no transcript and no notes")]
    NothingToSummarize,
    #[error("the summary was cancelled")]
    Cancelled,
    #[error("the model's context is too small to summarize this meeting")]
    ContextTooSmall,
    #[error(transparent)]
    Template(#[from] TemplateError),
    #[error("the model failed: {0}")]
    Model(String),
}

/// A loaded model that can format prompts and generate text.
pub trait Generator {
    /// Total tokens the model can attend to, prompt plus output.
    fn context_size(&self) -> usize;
    /// Tokens the rendered prompt for these messages takes.
    fn prompt_tokens(&mut self, messages: &[Message]) -> Result<usize, SummaryError>;
    fn count_tokens(&mut self, text: &str) -> Result<usize, SummaryError>;
    /// Generates a reply, passing each piece of text to `on_text` as it arrives.
    /// Returns `SummaryError::Cancelled` soon after `cancel` is set.
    fn generate(
        &mut self,
        messages: &[Message],
        max_tokens: usize,
        cancel: &AtomicBool,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<String, SummaryError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Condensing part `part` of `parts` of a long transcript.
    Reading { part: usize, parts: usize },
    /// Writing the notes; text now streams.
    Writing,
}

pub struct Request<'a> {
    pub title: &'a str,
    pub notes_markdown: &'a str,
    pub segments: &'a [TranscriptSegment],
    /// The vocabulary's terms, spelled as the summary should spell them.
    pub terms: &'a [String],
}

/// Summarizes a meeting. Only the final notes stream through `on_text`.
pub fn summarize(
    generator: &mut dyn Generator,
    request: &Request,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
    on_text: &mut dyn FnMut(&str),
) -> Result<String, SummaryError> {
    if request.segments.is_empty() && request.notes_markdown.trim().is_empty() {
        return Err(SummaryError::NothingToSummarize);
    }
    let context = generator.context_size();
    let lines: Vec<String> = request.segments.iter().map(transcript_line).collect();

    let single = summary_messages(
        request.title,
        request.notes_markdown,
        request.terms,
        &lines.join("\n"),
    );
    if fits(generator, &single, MAX_SUMMARY_TOKENS, context)? {
        on_progress(Progress::Writing);
        return generator.generate(&single, MAX_SUMMARY_TOKENS, cancel, on_text);
    }

    // Too long for one pass: condense in parts until the parts fit together.
    let overhead = generator.prompt_tokens(&part_messages(1, 1, request.terms, ""))?;
    let budget = context
        .checked_sub(overhead + MAX_PART_TOKENS + MARGIN_TOKENS)
        .filter(|b| *b > 0)
        .ok_or(SummaryError::ContextTooSmall)?;
    let mut pieces = lines;
    loop {
        let costs = pieces
            .iter()
            .map(|p| generator.count_tokens(p))
            .collect::<Result<Vec<_>, _>>()?;
        let windows = prompt::windows(&pieces, &costs, budget);
        if windows.len() >= pieces.len() && pieces.len() > 1 {
            // Condensing can't shrink this any further.
            return Err(SummaryError::ContextTooSmall);
        }
        let mut parts = Vec::with_capacity(windows.len());
        for (i, window) in windows.iter().enumerate() {
            on_progress(Progress::Reading {
                part: i + 1,
                parts: windows.len(),
            });
            let messages = part_messages(i + 1, windows.len(), request.terms, window);
            parts.push(generator.generate(&messages, MAX_PART_TOKENS, cancel, &mut |_| {})?);
        }

        let combined =
            combine_messages(request.title, request.notes_markdown, request.terms, &parts);
        if fits(generator, &combined, MAX_SUMMARY_TOKENS, context)? {
            on_progress(Progress::Writing);
            return generator.generate(&combined, MAX_SUMMARY_TOKENS, cancel, on_text);
        }
        pieces = parts;
    }
}

fn fits(
    generator: &mut dyn Generator,
    messages: &[Message],
    output: usize,
    context: usize,
) -> Result<bool, SummaryError> {
    Ok(generator.prompt_tokens(messages)? + output + MARGIN_TOKENS <= context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Source;
    use std::sync::atomic::Ordering;

    /// Counts one token per word and replies with canned text.
    struct FakeModel {
        context: usize,
        calls: Vec<String>,
    }

    impl FakeModel {
        fn new(context: usize) -> Self {
            Self {
                context,
                calls: Vec::new(),
            }
        }
    }

    impl Generator for FakeModel {
        fn context_size(&self) -> usize {
            self.context
        }

        fn prompt_tokens(&mut self, messages: &[Message]) -> Result<usize, SummaryError> {
            Ok(messages
                .iter()
                .map(|m| m.content.split_whitespace().count() + 4)
                .sum())
        }

        fn count_tokens(&mut self, text: &str) -> Result<usize, SummaryError> {
            Ok(text.split_whitespace().count())
        }

        fn generate(
            &mut self,
            messages: &[Message],
            _max_tokens: usize,
            cancel: &AtomicBool,
            on_text: &mut dyn FnMut(&str),
        ) -> Result<String, SummaryError> {
            if cancel.load(Ordering::Relaxed) {
                return Err(SummaryError::Cancelled);
            }
            let user = &messages[1].content;
            let reply = if user.starts_with("Part ") {
                format!("- condensed part {}", self.calls.len() + 1)
            } else {
                "## Notes\n- done".to_string()
            };
            self.calls.push(user.clone());
            for piece in reply.split_inclusive(' ') {
                on_text(piece);
            }
            Ok(reply)
        }
    }

    fn segments(count: usize, words_each: usize) -> Vec<TranscriptSegment> {
        (0..count)
            .map(|i| TranscriptSegment {
                id: i as i64,
                meeting_id: 1,
                source: if i % 2 == 0 { Source::Me } else { Source::Them },
                start_ms: i as i64 * 5000,
                end_ms: i as i64 * 5000 + 4000,
                text: vec!["word"; words_each].join(" "),
                language: None,
            })
            .collect()
    }

    fn run(
        model: &mut FakeModel,
        segments: &[TranscriptSegment],
        notes: &str,
    ) -> (Result<String, SummaryError>, Vec<Progress>, String) {
        let mut progress = Vec::new();
        let mut streamed = String::new();
        let request = Request {
            title: "Sync",
            notes_markdown: notes,
            segments,
            terms: &[],
        };
        let result = summarize(
            model,
            &request,
            &AtomicBool::new(false),
            &mut |p| progress.push(p),
            &mut |t| streamed.push_str(t),
        );
        (result, progress, streamed)
    }

    #[test]
    fn summarizes_a_short_meeting_in_one_pass() {
        let mut model = FakeModel::new(8192);

        let (result, progress, streamed) = run(&mut model, &segments(10, 10), "- agenda");

        assert_eq!(result.unwrap(), "## Notes\n- done");
        assert_eq!(progress, vec![Progress::Writing]);
        assert_eq!(streamed, "## Notes\n- done");
        assert_eq!(model.calls.len(), 1);
    }

    #[test]
    fn condenses_a_long_meeting_in_parts_first() {
        // 200 segments of 30 words is about 6,600 tokens; a 4,096 context leaves
        // about 3,500 per part, so it takes two parts.
        let mut model = FakeModel::new(4096);

        let (result, progress, streamed) = run(&mut model, &segments(200, 30), "- agenda");

        assert_eq!(result.unwrap(), "## Notes\n- done");
        let parts = progress.len() - 1;
        assert_eq!(parts, 2, "{progress:?}");
        assert_eq!(progress[0], Progress::Reading { part: 1, parts });
        assert_eq!(progress[parts], Progress::Writing);
        // Only the final notes stream to the UI, not the condensed parts.
        assert_eq!(streamed, "## Notes\n- done");
        let last = model.calls.last().unwrap();
        assert!(last.contains("Part 1:\n- condensed part 1"), "{last}");
        assert!(last.contains("<notes>\n- agenda\n</notes>"), "{last}");
    }

    #[test]
    fn summarizes_notes_alone_when_there_is_no_transcript() {
        let mut model = FakeModel::new(8192);

        let (result, _, _) = run(&mut model, &[], "- remember the budget");

        assert!(result.is_ok());
    }

    #[test]
    fn refuses_when_there_is_nothing_to_summarize() {
        let mut model = FakeModel::new(8192);

        let (result, _, _) = run(&mut model, &[], "  ");

        assert!(matches!(result, Err(SummaryError::NothingToSummarize)));
    }

    #[test]
    fn reports_a_context_too_small_to_make_progress() {
        let mut model = FakeModel::new(600);

        let (result, _, _) = run(&mut model, &segments(5, 400), "");

        assert!(
            matches!(result, Err(SummaryError::ContextTooSmall)),
            "{result:?}"
        );
    }

    #[test]
    fn stops_when_cancelled() {
        let mut model = FakeModel::new(8192);
        let request = Request {
            title: "Sync",
            notes_markdown: "- x",
            segments: &segments(3, 5),
            terms: &[],
        };

        let result = summarize(
            &mut model,
            &request,
            &AtomicBool::new(true),
            &mut |_| {},
            &mut |_| {},
        );

        assert!(matches!(result, Err(SummaryError::Cancelled)));
    }
}
