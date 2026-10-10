//! Answers one question from the meeting's transcript and notes.

use std::sync::atomic::AtomicBool;

use super::citations::{cited_labels, rewrite};
use super::passages::passages;
use super::prompt::{self, NOT_COVERED, Turn, passage_source, segment_source};
use super::retrieve::{self, Doc, Selection};
use crate::llm::generator::{GenerateError, Generator};
use crate::llm::template::Message;
use crate::store::{ChatMessage, ChatRole, Citation, TranscriptSegment};
use crate::summarize::note_taker::{NoteTaker, you_for_me};

/// Slack for tokenizer differences between counting and generating.
const MARGIN_TOKENS: usize = 64;

/// Room for the answer: a quarter of the context, at most 1,024 tokens.
pub fn answer_tokens(context: usize) -> usize {
    (context / 4).min(1024)
}

/// Room for earlier questions and answers: an eighth of the context.
fn history_tokens(context: usize) -> usize {
    context / 8
}

pub struct Ask<'a> {
    pub title: &'a str,
    pub notes_markdown: &'a str,
    pub segments: &'a [TranscriptSegment],
    /// The vocabulary's terms, spelled as answers should spell them.
    pub terms: &'a [String],
    /// The conversation so far, oldest first.
    pub history: &'a [ChatMessage],
    pub question: &'a str,
}

#[derive(Debug, PartialEq)]
pub struct Answer {
    /// Keeps the labels it cites, such as `[S3]`, and only those in `sources`.
    pub markdown: String,
    /// The sources the answer cites, in order of first citation.
    pub sources: Vec<Citation>,
}

/// A source sent to the model: its line in the prompt and what it cites.
struct Sent {
    line: String,
    citation: Citation,
}

/// Answers the question. The meeting goes to the model whole when it fits,
/// and otherwise only the parts that match the question. When nothing
/// matches, the answer says so without asking the model. Text streams
/// through `on_text`, and the answer calls the note-taker "you".
pub fn answer(
    generator: &mut dyn Generator,
    ask: &Ask,
    cancel: &AtomicBool,
    on_text: &mut dyn FnMut(&str),
) -> Result<Answer, GenerateError> {
    let context = generator.context_size();
    let answer_room = answer_tokens(context);
    let turns = recent_turns(generator, ask.history, history_tokens(context))?;
    let passages = passages(ask.notes_markdown);

    let all = Selection {
        passages: (0..passages.len()).collect(),
        segments: (0..ask.segments.len()).collect(),
    };
    let everything = label(&passages, ask.segments, &all);
    let messages = build(ask, &turns, &everything);
    let sent = if generator.prompt_tokens(&messages)? + answer_room + MARGIN_TOKENS <= context {
        everything
    } else {
        let overhead = generator.prompt_tokens(&build(ask, &turns, &[]))?;
        let budget = context
            .checked_sub(overhead + answer_room + MARGIN_TOKENS)
            .filter(|b| *b > 0)
            .ok_or(GenerateError::ContextTooSmall)?;
        let texts: Vec<&str> = ask.segments.iter().map(|s| s.text.as_str()).collect();
        let mut failed = None;
        let selection = retrieve::select(ask.question, &passages, &texts, budget, &mut |doc| {
            let line = match doc {
                Doc::Passage(i) => passage_source("N000", passages[i]),
                Doc::Segment(i) => segment_source("S0000", &ask.segments[i]),
            };
            // One more token for the line break.
            generator
                .count_tokens(&line)
                .map(|n| n + 1)
                .unwrap_or_else(|e| {
                    failed = Some(e);
                    usize::MAX / 4
                })
        });
        if let Some(error) = failed {
            return Err(error);
        }
        label(&passages, ask.segments, &selection)
    };

    if sent.is_empty() {
        on_text(NOT_COVERED);
        return Ok(Answer {
            markdown: NOT_COVERED.to_owned(),
            sources: Vec::new(),
        });
    }
    let messages = build(ask, &turns, &sent);
    let mut note_taker = NoteTaker::default();
    let reply = generator.generate(&messages, answer_room, cancel, &mut |text| {
        let shown = note_taker.push(text);
        if !shown.is_empty() {
            on_text(&shown);
        }
    })?;
    let rest = note_taker.finish();
    if !rest.is_empty() {
        on_text(&rest);
    }
    Ok(cite(&you_for_me(&reply), &sent))
}

/// Keeps the citations of sources that were sent, and drops the rest.
fn cite(reply: &str, sent: &[Sent]) -> Answer {
    let find = |label: &str| sent.iter().find(|s| s.citation.label() == label);
    Answer {
        markdown: rewrite(reply.trim(), |label| find(label).map(|_| label.to_owned())),
        sources: cited_labels(reply)
            .iter()
            .filter_map(|label| find(label).map(|s| s.citation.clone()))
            .collect(),
    }
}

fn build(ask: &Ask, turns: &[Turn], sent: &[Sent]) -> Vec<Message> {
    let lines: Vec<String> = sent.iter().map(|s| s.line.clone()).collect();
    prompt::messages(ask.title, ask.terms, turns, &lines, ask.question)
}

/// Labels the chosen notes passages N1, N2, and so on, then the chosen
/// segments S1, S2, and so on.
fn label(passages: &[&str], segments: &[TranscriptSegment], chosen: &Selection) -> Vec<Sent> {
    let notes = chosen.passages.iter().enumerate().map(|(n, &i)| {
        let label = format!("N{}", n + 1);
        Sent {
            line: passage_source(&label, passages[i]),
            citation: Citation::Notes {
                label,
                text: passages[i].to_owned(),
            },
        }
    });
    let transcript = chosen.segments.iter().enumerate().map(|(n, &i)| {
        let label = format!("S{}", n + 1);
        let segment = &segments[i];
        Sent {
            line: segment_source(&label, segment),
            citation: Citation::Transcript {
                label,
                segment_id: segment.id,
                start_ms: segment.start_ms,
            },
        }
    });
    notes.chain(transcript).collect()
}

/// The newest question and answer pairs that fit in `budget` tokens, oldest
/// first, with their citations removed.
fn recent_turns<'a>(
    generator: &mut dyn Generator,
    history: &'a [ChatMessage],
    budget: usize,
) -> Result<Vec<Turn<'a>>, GenerateError> {
    let mut turns = Vec::new();
    let mut spent = 0;
    for pair in history.rchunks_exact(2) {
        let [question, answer] = pair else {
            unreachable!("chunks of two")
        };
        if question.role != ChatRole::User || answer.role != ChatRole::Assistant {
            continue;
        }
        let cost = generator.count_tokens(&question.body_markdown)?
            + generator.count_tokens(&answer.body_markdown)?;
        if spent + cost > budget {
            break;
        }
        spent += cost;
        turns.push(Turn {
            question: &question.body_markdown,
            answer: rewrite(&answer.body_markdown, |_| None),
        });
    }
    turns.reverse();
    Ok(turns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Source;
    use std::sync::atomic::Ordering;

    /// Counts one token per word, replies with `reply`, and records each prompt.
    struct FakeModel {
        context: usize,
        reply: &'static str,
        prompts: Vec<Vec<Message>>,
    }

    impl FakeModel {
        fn new(context: usize, reply: &'static str) -> Self {
            Self {
                context,
                reply,
                prompts: Vec::new(),
            }
        }

        /// The last message of the only prompt: the sources and the question.
        fn sent(&self) -> &str {
            assert_eq!(self.prompts.len(), 1, "one prompt");
            &self.prompts[0].last().unwrap().content
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
            _: usize,
            cancel: &AtomicBool,
            on_text: &mut dyn FnMut(&str),
        ) -> Result<String, GenerateError> {
            if cancel.load(Ordering::Relaxed) {
                return Err(GenerateError::Cancelled);
            }
            self.prompts.push(messages.to_vec());
            for piece in self.reply.split_inclusive(' ') {
                on_text(piece);
            }
            Ok(self.reply.to_owned())
        }
    }

    fn segment(i: i64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: 100 + i,
            meeting_id: 1,
            source: if i % 2 == 0 { Source::Me } else { Source::Them },
            start_ms: i * 5000,
            end_ms: i * 5000 + 4000,
            text: text.into(),
            language: None,
        }
    }

    fn message(role: ChatRole, body: &str) -> ChatMessage {
        ChatMessage {
            id: 0,
            meeting_id: 1,
            role,
            body_markdown: body.into(),
            sources: Vec::new(),
            model_id: None,
            created_at: 0,
        }
    }

    /// A long meeting: filler, with the launch date in the middle.
    fn long_meeting() -> Vec<TranscriptSegment> {
        (0..200)
            .map(|i| {
                if i == 120 {
                    segment(i, "The launch moves to May 4 because QA needs a week.")
                } else {
                    segment(i, "Some routine status about nothing in particular today.")
                }
            })
            .collect()
    }

    fn ask<'a>(
        notes: &'a str,
        segments: &'a [TranscriptSegment],
        history: &'a [ChatMessage],
        question: &'a str,
    ) -> Ask<'a> {
        Ask {
            title: "Sync",
            notes_markdown: notes,
            segments,
            terms: &[],
            history,
            question,
        }
    }

    fn run(model: &mut FakeModel, ask: &Ask) -> (Result<Answer, GenerateError>, String) {
        let mut streamed = String::new();
        let result = answer(model, ask, &AtomicBool::new(false), &mut |t| {
            streamed.push_str(t)
        });
        (result, streamed)
    }

    #[test]
    fn sends_a_meeting_that_fits_whole_and_keeps_only_sent_citations() {
        let segments = [segment(0, "Hello."), segment(1, "Ship it Friday.")];
        let mut model = FakeModel::new(8192, "Friday [S2, N1], says the plan [S9].");

        let (result, streamed) = run(
            &mut model,
            &ask("- Launch: Friday", &segments, &[], "When do we ship?"),
        );

        let answer = result.unwrap();
        assert_eq!(answer.markdown, "Friday [S2, N1], says the plan.");
        assert_eq!(
            answer.sources,
            [
                Citation::Transcript {
                    label: "S2".into(),
                    segment_id: 101,
                    start_ms: 5000,
                },
                Citation::Notes {
                    label: "N1".into(),
                    text: "- Launch: Friday".into(),
                },
            ]
        );
        assert_eq!(streamed, "Friday [S2, N1], says the plan [S9].");
        let sent = model.sent();
        assert!(sent.contains("[N1] (My notes) - Launch: Friday\n[S1] [0:00] Me: Hello.\n[S2] [0:05] Meeting Participants: Ship it Friday."), "{sent}");
        assert!(sent.ends_with("Question: When do we ship?"), "{sent}");
    }

    #[test]
    fn sends_only_the_matching_parts_of_a_long_meeting() {
        let segments = long_meeting();
        let mut model = FakeModel::new(1024, "May 4 [S2].");

        let (result, _) = run(&mut model, &ask("", &segments, &[], "When is the launch?"));

        let answer = result.unwrap();
        let sent = model.sent();
        // The match and one segment on either side.
        assert_eq!(
            sent.lines().filter(|l| l.starts_with("[S")).count(),
            3,
            "{sent}"
        );
        assert!(
            sent.contains("[S2] [10:00] Me: The launch moves to May 4"),
            "{sent}"
        );
        assert_eq!(
            answer.sources,
            [Citation::Transcript {
                label: "S2".into(),
                segment_id: 220,
                start_ms: 600_000,
            }]
        );
    }

    #[test]
    fn says_the_meeting_doesn_t_cover_it_without_asking_the_model() {
        let segments = long_meeting();
        let mut model = FakeModel::new(1024, "unused");

        let (result, streamed) = run(&mut model, &ask("", &segments, &[], "What's the budget?"));

        assert_eq!(
            result.unwrap(),
            Answer {
                markdown: NOT_COVERED.into(),
                sources: Vec::new(),
            }
        );
        assert_eq!(streamed, NOT_COVERED);
        assert!(model.prompts.is_empty());
    }

    #[test]
    fn answers_from_notes_alone() {
        let mut model = FakeModel::new(8192, "Two roles [N2].");

        let (result, _) = run(
            &mut model,
            &ask(
                "- Launch: May 4\n- Hiring: two roles",
                &[],
                &[],
                "How many roles?",
            ),
        );

        let answer = result.unwrap();
        assert_eq!(
            answer.sources,
            [Citation::Notes {
                label: "N2".into(),
                text: "- Hiring: two roles".into(),
            }]
        );
    }

    #[test]
    fn searches_notes_passage_by_passage_in_a_long_meeting() {
        let segments = long_meeting();
        let notes = "- Budget: $48,000 approved\n- Lunch: Thai\n- Parking: level 2";
        let mut model = FakeModel::new(1024, "$48,000 [N1].");

        let (result, _) = run(
            &mut model,
            &ask(notes, &segments, &[], "What budget was approved?"),
        );

        let sent = model.sent();
        assert!(
            sent.contains("[N1] (My notes) - Budget: $48,000 approved"),
            "{sent}"
        );
        assert!(!sent.contains("Lunch") && !sent.contains("[S1]"), "{sent}");
        assert_eq!(result.unwrap().sources.len(), 1);
    }

    #[test]
    fn includes_recent_turns_without_their_citations() {
        let history = [
            message(ChatRole::User, "When is the launch?"),
            message(ChatRole::Assistant, "May 4 [S2]."),
            message(ChatRole::User, "Who owns it?"),
            message(ChatRole::Assistant, "Dana [S5, N1]."),
        ];
        let mut model = FakeModel::new(8192, "Friday.");

        run(
            &mut model,
            &ask("- x", &[segment(0, "hi")], &history, "And QA?"),
        )
        .0
        .unwrap();

        let contents: Vec<(&str, &str)> = model.prompts[0][1..4]
            .iter()
            .map(|m| (m.role, m.content.as_str()))
            .collect();
        assert_eq!(
            contents,
            [
                ("user", "When is the launch?"),
                ("assistant", "May 4."),
                ("user", "Who owns it?"),
            ]
        );
        assert_eq!(model.prompts[0][4].content, "Dana.");
    }

    #[test]
    fn keeps_only_the_newest_turns_that_fit() {
        let long = "word ".repeat(150);
        let history = [
            message(ChatRole::User, "Oldest question?"),
            message(ChatRole::Assistant, &long),
            message(ChatRole::User, "Newest question?"),
            message(ChatRole::Assistant, "Short."),
        ];
        // An eighth of 1,024 is 128 tokens: room for the newest turn only.
        let mut model = FakeModel::new(1024, "Ok.");

        run(&mut model, &ask("- x", &[], &history, "x?")).0.unwrap();

        let roles: Vec<&str> = model.prompts[0].iter().map(|m| m.role).collect();
        assert_eq!(roles, ["system", "user", "assistant", "user"]);
        assert_eq!(model.prompts[0][1].content, "Newest question?");
    }

    #[test]
    fn calls_the_note_taker_you() {
        let mut model = FakeModel::new(8192, "Me will send the strings [S1].");

        let (result, streamed) = run(
            &mut model,
            &ask(
                "",
                &[segment(0, "I'll send the strings.")],
                &[],
                "Who sends the strings?",
            ),
        );

        assert_eq!(result.unwrap().markdown, "You will send the strings [S1].");
        assert_eq!(streamed, "You will send the strings [S1].");
    }

    #[test]
    fn stops_when_cancelled() {
        let mut model = FakeModel::new(8192, "unused");

        let result = answer(
            &mut model,
            &ask("- x", &[], &[], "x?"),
            &AtomicBool::new(true),
            &mut |_| {},
        );

        assert!(matches!(result, Err(GenerateError::Cancelled)));
    }

    #[test]
    fn reports_a_context_too_small_for_any_source() {
        let segments = long_meeting();
        let mut model = FakeModel::new(300, "unused");

        let (result, _) = run(&mut model, &ask("", &segments, &[], "launch?"));

        assert!(
            matches!(result, Err(GenerateError::ContextTooSmall)),
            "{result:?}"
        );
    }
}
