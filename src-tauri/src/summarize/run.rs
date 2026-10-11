//! Decides between one pass and condensing in parts, then drives the model.

use std::sync::atomic::AtomicBool;

use super::note_taker::{NoteTaker, you_for_me};
use super::prompt::{self, combine_messages, part_messages, summary_messages, transcript_line};
use crate::llm::generator::{GenerateError, Generator};
use crate::llm::template::Message;
use crate::store::TranscriptSegment;

/// Room for the finished notes: a quarter of the context, at most 4,096
/// tokens. Output room comes out of the same context as the transcript, so a
/// larger share would push meetings into condensing sooner, which loses detail.
pub fn summary_tokens(context: usize) -> usize {
    (context / 4).min(4096)
}

/// Room for each condensed part of a long transcript: an eighth of the
/// context, at most 1,024 tokens.
pub fn part_tokens(context: usize) -> usize {
    (context / 8).min(1024)
}

/// Slack for tokenizer differences between counting and generating.
const MARGIN_TOKENS: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("there's nothing to summarize yet: no transcript and no notes")]
    NothingToSummarize,
    #[error(transparent)]
    Generate(#[from] GenerateError),
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
/// The notes call the note-taker "you" (see `note_taker`).
pub fn summarize(
    generator: &mut dyn Generator,
    request: &Request,
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(Progress),
    on_text: &mut dyn FnMut(&str),
) -> Result<String, SummaryError> {
    let mut note_taker = NoteTaker::default();
    let notes = write_notes(generator, request, cancel, on_progress, &mut |text| {
        let shown = note_taker.push(text);
        if !shown.is_empty() {
            on_text(&shown);
        }
    })?;
    let rest = note_taker.finish();
    if !rest.is_empty() {
        on_text(&rest);
    }
    Ok(you_for_me(&notes))
}

fn write_notes(
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
    let (summary_room, part_room) = (summary_tokens(context), part_tokens(context));
    let lines: Vec<String> = request.segments.iter().map(transcript_line).collect();

    let single = summary_messages(
        request.title,
        request.notes_markdown,
        request.terms,
        &lines.join("\n"),
    );
    if fits(generator, &single, summary_room, context)? {
        on_progress(Progress::Writing);
        return Ok(generator.generate(&single, summary_room, cancel, on_text)?);
    }

    // Too long for one pass: condense in parts until the parts fit together.
    let overhead = generator.prompt_tokens(&part_messages(
        1,
        1,
        request.notes_markdown,
        request.terms,
        "",
    ))?;
    let budget = context
        .checked_sub(overhead + part_room + MARGIN_TOKENS)
        .filter(|b| *b > 0)
        .ok_or(GenerateError::ContextTooSmall)?;
    let mut pieces = lines;
    loop {
        let costs = pieces
            .iter()
            .map(|p| generator.count_tokens(p))
            .collect::<Result<Vec<_>, _>>()?;
        let windows = prompt::windows(&pieces, &costs, budget);
        if windows.len() >= pieces.len() && pieces.len() > 1 {
            // Condensing can't shrink this any further.
            return Err(GenerateError::ContextTooSmall.into());
        }
        let mut parts = Vec::with_capacity(windows.len());
        for (i, window) in windows.iter().enumerate() {
            on_progress(Progress::Reading {
                part: i + 1,
                parts: windows.len(),
            });
            let messages = part_messages(
                i + 1,
                windows.len(),
                request.notes_markdown,
                request.terms,
                window,
            );
            parts.push(generator.generate(&messages, part_room, cancel, &mut |_| {})?);
        }

        let combined =
            combine_messages(request.title, request.notes_markdown, request.terms, &parts);
        if fits(generator, &combined, summary_room, context)? {
            on_progress(Progress::Writing);
            return Ok(generator.generate(&combined, summary_room, cancel, on_text)?);
        }
        pieces = parts;
    }
}

fn fits(
    generator: &mut dyn Generator,
    messages: &[Message],
    output: usize,
    context: usize,
) -> Result<bool, GenerateError> {
    Ok(generator.prompt_tokens(messages)? + output + MARGIN_TOKENS <= context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Source;
    use std::sync::atomic::Ordering;

    #[test]
    fn scales_output_room_with_the_context() {
        // An 8K context, Ministral 3 8B at 16K, then the 32K models.
        assert_eq!((summary_tokens(8_192), part_tokens(8_192)), (2_048, 1_024));
        assert_eq!(
            (summary_tokens(16_384), part_tokens(16_384)),
            (4_096, 1_024)
        );
        assert_eq!(
            (summary_tokens(32_768), part_tokens(32_768)),
            (4_096, 1_024)
        );
        // Small test models keep the same shares.
        assert_eq!((summary_tokens(2_048), part_tokens(2_048)), (512, 256));
    }

    /// Counts one token per word and replies with canned text.
    struct FakeModel {
        context: usize,
        calls: Vec<String>,
        notes: &'static str,
    }

    impl FakeModel {
        fn new(context: usize) -> Self {
            Self {
                context,
                calls: Vec::new(),
                notes: "## Notes\n- done",
            }
        }
    }

    impl Generator for FakeModel {
        fn context_size(&self) -> usize {
            self.context
        }

        fn prompt_tokens(&mut self, messages: &[Message]) -> Result<usize, GenerateError> {
            Ok(messages
                .iter()
                .map(|m| m.content.split_whitespace().count() + 4)
                .sum())
        }

        fn count_tokens(&mut self, text: &str) -> Result<usize, GenerateError> {
            Ok(text.split_whitespace().count())
        }

        fn generate(
            &mut self,
            messages: &[Message],
            _max_tokens: usize,
            cancel: &AtomicBool,
            on_text: &mut dyn FnMut(&str),
        ) -> Result<String, GenerateError> {
            if cancel.load(Ordering::Relaxed) {
                return Err(GenerateError::Cancelled);
            }
            let user = &messages[1].content;
            let reply = if user.contains(" of the transcript:\n<transcript>") {
                format!("- condensed part {}", self.calls.len() + 1)
            } else {
                self.notes.to_string()
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
    fn calls_the_note_taker_you_in_streamed_and_returned_notes() {
        let mut model = FakeModel::new(8192);
        model.notes = "## Next steps\n- Me will write the alert (Me)";

        let (result, _, streamed) = run(&mut model, &segments(10, 10), "");

        let expected = "## Next steps\n- You will write the alert (you)";
        assert_eq!(result.unwrap(), expected);
        assert_eq!(streamed, expected);
    }

    #[test]
    fn condenses_a_long_meeting_in_parts_first() {
        // 150 segments of 30 words is about 5,000 tokens; a 4,096 context leaves
        // about 3,400 per part, so it takes two parts.
        let mut model = FakeModel::new(4096);

        let (result, progress, streamed) = run(&mut model, &segments(150, 30), "- agenda");

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
        // Each part sees the notes too, so it knows what matters.
        let first = &model.calls[0];
        assert!(first.contains("<notes>\n- agenda\n</notes>"), "{first}");
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
            matches!(
                result,
                Err(SummaryError::Generate(GenerateError::ContextTooSmall))
            ),
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

        assert!(matches!(
            result,
            Err(SummaryError::Generate(GenerateError::Cancelled))
        ));
    }
}
