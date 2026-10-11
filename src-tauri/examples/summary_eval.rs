//! Summarizes synthetic meetings with a real model and checks which planted
//! specifics survive into the notes (#70). Summary quality can't be asserted in
//! tests, so this is a tool for comparing prompts, sampling, and approaches.
//!
//!   cargo run --release --example summary_eval -- \
//!     --model tests/models/Qwen3.5-4B-Q4_K_M.gguf \
//!     [--sampling faithful|gemma4] [--context TOKENS] [--approach single|extract] \
//!     [--noisy] [--pad MINUTES] [--terms "Name One, Name Two"] [--json] \
//!     tests/fixtures/meetings/*.json
//!
//! `--json` prints one JSON object per meeting instead of Markdown, with load
//! time, time to the first text, writing speed, and token counts, for the
//! model benchmark (`npm run benchmark`, #8).
//!
//! Known models run with their registry context limit, as in the app.
//! `--context` sets the limit instead; a small value forces long-meeting
//! condensing, which is how the "chunked" approach is measured. `--noisy`
//! makes each transcript look like real speech recognition output (set
//! `SHOW_TRANSCRIPT=1` to print the transcript each meeting was summarized from) (see
//! [`noisy`]) without touching the planted specifics. `--pad MINUTES` spreads
//! the meeting across that many minutes of unrelated status updates (see
//! [`padded`]), to test whether specifics survive a long meeting.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use earshot_lib::llm::generator::{GenerateError, Generator};
use earshot_lib::llm::llama::{LlamaGenerator, MAX_CONTEXT, Sampling};
use earshot_lib::llm::template::Message;
use earshot_lib::store::{Source, TranscriptSegment};
use earshot_lib::summarize::prompt::{self, transcript_line};
use earshot_lib::summarize::run::{Progress, Request, summarize};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Meeting {
    title: String,
    notes: String,
    segments: Vec<(Source, i64, String)>,
    /// Specifics that must survive; each lists accepted spellings separated by `|`.
    expect: Vec<String>,
    /// Text that must not appear, such as a value a speaker corrected.
    forbid: Vec<String>,
}

struct Options {
    model: PathBuf,
    sampling: Sampling,
    sampling_name: String,
    context: Option<usize>,
    extract: bool,
    noisy: bool,
    pad_minutes: u64,
    /// Vocabulary terms, as the app passes them.
    terms: Vec<String>,
    json: bool,
    meetings: Vec<PathBuf>,
}

/// One meeting's result, as `--json` prints it.
#[derive(Serialize)]
struct Outcome {
    model: String,
    fixture: String,
    title: String,
    pad_minutes: u64,
    noisy: bool,
    load_seconds: f64,
    context_size: usize,
    /// Tokens of the one-pass prompt, whether or not it fit.
    prompt_tokens: usize,
    /// 0 when the meeting fit in one pass.
    parts: usize,
    /// Specifics still present in the condensed parts, before the final pass,
    /// when the meeting was condensed. Shows where specifics are lost (#95).
    found_in_parts: Option<usize>,
    /// From the start of the summary to the first text of the notes, which
    /// includes reading the prompt and any condensing.
    first_text_seconds: Option<f64>,
    total_seconds: f64,
    output_tokens: usize,
    /// Output tokens per second after the first text.
    writing_tokens_per_second: Option<f64>,
    found: usize,
    expected: usize,
    missing: Vec<String>,
    forbidden: Vec<String>,
    /// "Me" written as a name, as in "Me suggested", instead of "you" (#100).
    me_as_name: usize,
    /// "You (you)", which an earlier prompt produced (#70).
    doubled_you: usize,
    notes: String,
}

fn main() {
    let options = options();
    let loading = Instant::now();
    // Like the app, a known model runs with its registry limit. Matching by
    // file name avoids hashing the model on every run.
    let limit = options.context.unwrap_or_else(|| {
        let file_name = options.model.file_name().unwrap().to_string_lossy();
        earshot_lib::models::KNOWN_MODELS
            .iter()
            .find(|k| k.file_name == file_name)
            .map_or(MAX_CONTEXT, |k| k.context_limit)
    });
    let mut generator = LlamaGenerator::load_with_context_limit(&options.model, limit)
        .expect("couldn't load the model");
    let load_seconds = loading.elapsed().as_secs_f64();
    generator.set_sampling(options.sampling);
    let model = options
        .model
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    if !options.json {
        println!(
            "# {model}\n\nsampling: {}, context: {}, approach: {}\n",
            options.sampling_name,
            generator.context_size(),
            if options.extract { "extract" } else { "single" },
        );
        if options.noisy || options.pad_minutes > 0 {
            println!(
                "transcripts: {}padded with {} minutes\n",
                if options.noisy { "noisy, " } else { "" },
                options.pad_minutes
            );
        }
    }
    let mut total_found = 0;
    let mut total_expected = 0;
    for path in &options.meetings {
        let mut outcome = evaluate(&mut generator, path, &options);
        outcome.model = model.clone();
        outcome.load_seconds = load_seconds;
        total_found += outcome.found;
        total_expected += outcome.expected;
        if options.json {
            println!("{}", serde_json::to_string(&outcome).expect("serializable"));
        } else {
            print_markdown(&outcome);
        }
    }
    if !options.json {
        println!("## Total\n\n{total_found} of {total_expected} specifics kept");
    }
}

fn print_markdown(outcome: &Outcome) {
    println!(
        "## {} ({})\n\n{:.1} s, {}, {} of {} specifics kept",
        outcome.title,
        outcome.fixture,
        outcome.total_seconds,
        if outcome.parts > 0 {
            format!("condensed in {} parts", outcome.parts)
        } else {
            "one pass".into()
        },
        outcome.found,
        outcome.expected,
    );
    if !outcome.missing.is_empty() {
        println!("missing: {}", outcome.missing.join(", "));
    }
    if outcome.me_as_name > 0 || outcome.doubled_you > 0 {
        println!(
            "\"Me\" as a name: {}, \"You (you)\": {}",
            outcome.me_as_name, outcome.doubled_you
        );
    }
    if !outcome.forbidden.is_empty() {
        println!("forbidden text present: {}", outcome.forbidden.join(", "));
    }
    println!("\n```markdown\n{}\n```\n", outcome.notes.trim());
}

fn evaluate(generator: &mut LlamaGenerator, path: &Path, options: &Options) -> Outcome {
    let Options {
        extract,
        noisy: noisy_transcript,
        pad_minutes,
        ref terms,
        ..
    } = *options;
    let mut meeting: Meeting =
        serde_json::from_str(&std::fs::read_to_string(path).expect("unreadable fixture"))
            .expect("fixture isn't a meeting");
    if pad_minutes > 0 {
        meeting.segments = padded(&meeting.segments, pad_minutes);
    }
    if noisy_transcript {
        meeting.segments = noisy(&meeting.segments);
    }
    if std::env::var_os("SHOW_TRANSCRIPT").is_some() {
        for (source, start_ms, text) in &meeting.segments {
            eprintln!("[{start_ms}] {}: {text}", source.label());
        }
    }
    let segments: Vec<TranscriptSegment> = meeting
        .segments
        .iter()
        .enumerate()
        .map(|(i, (source, start_ms, text))| TranscriptSegment {
            id: i as i64,
            meeting_id: 1,
            source: *source,
            start_ms: *start_ms,
            end_ms: start_ms + 4000,
            text: text.clone(),
            language: Some("en".into()),
        })
        .collect();
    let transcript = segments
        .iter()
        .map(transcript_line)
        .collect::<Vec<_>>()
        .join("\n");
    let prompt_tokens = generator
        .prompt_tokens(&prompt::summary_messages(
            &meeting.title,
            &meeting.notes,
            terms,
            &transcript,
        ))
        .expect("couldn't count the prompt");
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let mut parts = 0;
    let mut first_text = None;
    let mut condensed = String::new();
    let notes = if extract {
        extract_then_write(generator, &meeting, &transcript, &cancel)
    } else {
        let request = Request {
            title: &meeting.title,
            notes_markdown: &meeting.notes,
            segments: &segments,
            terms,
        };
        let mut recorder = Recorder {
            inner: generator,
            outputs: Vec::new(),
        };
        let notes = summarize(
            &mut recorder,
            &request,
            &cancel,
            &mut |progress| {
                if let Progress::Reading { parts: n, .. } = progress {
                    parts = n;
                }
            },
            &mut |_| {
                first_text.get_or_insert_with(|| started.elapsed());
            },
        )
        .expect("summary failed");
        // Every reply but the last is a condensed part.
        condensed = recorder.outputs[..recorder.outputs.len() - 1].join("\n");
        if std::env::var_os("SHOW_PARTS").is_some() {
            eprintln!("{condensed}");
        }
        notes
    };
    let elapsed = started.elapsed();
    let output_tokens = generator
        .count_tokens(&notes)
        .expect("couldn't count the notes");
    let writing_tokens_per_second = first_text
        .map(|first| output_tokens as f64 / (elapsed - first).as_secs_f64().max(f64::EPSILON));

    let has = |alternatives: &str| contains_any(&notes, alternatives);
    let missing: Vec<String> = meeting.expect.iter().filter(|e| !has(e)).cloned().collect();
    let found_in_parts = (parts > 0).then(|| {
        meeting
            .expect
            .iter()
            .filter(|e| contains_any(&condensed, e))
            .count()
    });
    let forbidden: Vec<String> = meeting.forbid.iter().filter(|f| has(f)).cloned().collect();
    Outcome {
        model: String::new(),
        fixture: path.file_name().unwrap().to_string_lossy().into_owned(),
        title: meeting.title,
        pad_minutes,
        noisy: noisy_transcript,
        load_seconds: 0.0,
        context_size: generator.context_size(),
        prompt_tokens,
        parts,
        found_in_parts,
        first_text_seconds: first_text.map(|d| d.as_secs_f64()),
        total_seconds: elapsed.as_secs_f64(),
        output_tokens,
        writing_tokens_per_second,
        found: meeting.expect.len() - missing.len(),
        expected: meeting.expect.len(),
        missing,
        forbidden,
        me_as_name: me_as_name(&notes),
        doubled_you: notes.to_lowercase().matches("you (you)").count(),
        notes,
    }
}

/// Whether `text` contains any of the `|`-separated spellings, ignoring case.
fn contains_any(text: &str, alternatives: &str) -> bool {
    let lower = text.to_lowercase();
    alternatives
        .split('|')
        .any(|a| lower.contains(&a.to_lowercase()))
}

/// Passes every call to the model through and keeps each reply, so the
/// condensed parts can be checked as well as the final notes.
struct Recorder<'a> {
    inner: &'a mut LlamaGenerator,
    outputs: Vec<String>,
}

impl Generator for Recorder<'_> {
    fn context_size(&self) -> usize {
        self.inner.context_size()
    }

    fn prompt_tokens(&mut self, messages: &[Message]) -> Result<usize, GenerateError> {
        self.inner.prompt_tokens(messages)
    }

    fn count_tokens(&mut self, text: &str) -> Result<usize, GenerateError> {
        self.inner.count_tokens(text)
    }

    fn generate(
        &mut self,
        messages: &[Message],
        max_tokens: usize,
        cancel: &AtomicBool,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<String, GenerateError> {
        let reply = self.inner.generate(messages, max_tokens, cancel, on_text)?;
        self.outputs.push(reply.clone());
        Ok(reply)
    }
}

/// Counts "Me" written as a word with a capital M. Notes have no reason to
/// use it: the note-taker should be "you".
fn me_as_name(notes: &str) -> usize {
    notes
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| *word == "Me")
        .count()
}

const EXTRACT_SYSTEM: &str = "You list the facts in a meeting transcript, so detailed notes can be written from them.

Rules:
- List every name, number, date, ticket or issue number, error code, quoted error message, setting, product, and system mentioned, each with what was said about it.
- List each problem with its cause, each change made during the meeting with its result, each decision with its reason, each open question, and each task with its owner and any condition.
- When a speaker corrects themselves, list only the corrected statement.
- \"Me\" is the note-taker. \"Meeting Participants\" is everyone else on the call.
- Only list what the transcript supports.
- Reply with a bulleted list only.";

/// Extracts facts from the transcript, then writes the notes from the facts
/// and the full transcript in one final pass. An experiment for #70.
fn extract_then_write(
    generator: &mut LlamaGenerator,
    meeting: &Meeting,
    transcript: &str,
    cancel: &AtomicBool,
) -> String {
    let extract = [
        Message::system(EXTRACT_SYSTEM),
        Message::user(format!(
            "Transcript:\n<transcript>\n{transcript}\n</transcript>"
        )),
    ];
    let facts = generator
        .generate(&extract, 2048, cancel, &mut |_| {})
        .expect("extraction failed");
    let mut messages = prompt::summary_messages(&meeting.title, &meeting.notes, &[], transcript);
    messages[1].content.push_str(&format!(
        "\n\nFacts already extracted from the transcript. Make sure the notes keep each of them:\n<facts>\n{}\n</facts>",
        facts.trim()
    ));
    let room = earshot_lib::summarize::run::summary_tokens(generator.context_size());
    generator
        .generate(&messages, room, cancel, &mut |_| {})
        .expect("writing failed")
}

fn options() -> Options {
    let mut args = std::env::args().skip(1);
    let mut model = None;
    let mut sampling = (Sampling::FAITHFUL, "faithful".to_owned());
    let mut context = None;
    let mut extract = false;
    let mut noisy = false;
    let mut json = false;
    let mut pad_minutes = 0;
    let mut terms = Vec::new();
    let mut meetings = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--model" => model = args.next().map(PathBuf::from),
            "--sampling" => {
                let name = args.next().expect("--sampling needs a value");
                let chosen = match name.as_str() {
                    "faithful" => Sampling::FAITHFUL,
                    "gemma4" => Sampling::GEMMA_4,
                    other => panic!("unknown sampling {other}"),
                };
                sampling = (chosen, name);
            }
            "--context" => {
                context = args.next().and_then(|v| v.parse().ok());
            }
            "--noisy" => noisy = true,
            "--json" => json = true,
            "--terms" => {
                terms = args
                    .next()
                    .map(|v| v.split(',').map(|t| t.trim().to_owned()).collect())
                    .unwrap_or_default();
            }
            "--pad" => {
                pad_minutes = args.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            "--approach" => {
                extract = match args.next().as_deref() {
                    Some("single") => false,
                    Some("extract") => true,
                    other => panic!("unknown approach {other:?}"),
                }
            }
            _ => meetings.push(PathBuf::from(arg)),
        }
    }
    Options {
        model: model.expect("--model is required"),
        sampling: sampling.0,
        sampling_name: sampling.1,
        context,
        extract,
        noisy,
        pad_minutes,
        terms,
        json,
        meetings,
    }
}

// Invented projects and people for padding.
const PROJECTS: &[&str] = &[
    "the onboarding redesign",
    "the mobile release",
    "the reporting dashboards",
    "the search rewrite",
    "the data retention work",
    "the accessibility audit",
    "the partner portal",
    "the pricing page",
];
const PEOPLE: &[&str] = &["Ana", "Theo", "Rosa", "Kenji", "Lena", "Omar"];
const DONE: &[&str] = &[
    "finished the first round of reviews",
    "merged the last of the cleanup changes",
    "got through most of the open bugs",
    "wrapped up the design pass",
    "ran the load test again",
];
const NEXT: &[&str] = &[
    "the next step is more testing",
    "we're waiting on feedback from the design team",
    "it should be ready for review next week",
    "there's nothing blocking it right now",
    "we still need to write the docs",
];

/// Spreads the meeting across `minutes` of status updates about unrelated
/// projects, in four blocks between its own lines, and shifts its times.
fn padded(meeting: &[(Source, i64, String)], minutes: u64) -> Vec<(Source, i64, String)> {
    let mut rng = Lcg(8);
    // About ten lines a minute, as in a real call.
    let lines = minutes * 10;
    let block = lines / 4;
    let mut out = Vec::new();
    let mut clock = 0;
    let status = |rng: &mut Lcg, out: &mut Vec<(Source, i64, String)>, clock: &mut i64| {
        for _ in 0..block {
            let pick =
                |rng: &mut Lcg, list: &[&'static str]| list[rng.below(list.len() as u64) as usize];
            let (source, text) = match rng.below(3) {
                0 => (
                    Source::Me,
                    format!(
                        "okay, {}, how's {} going",
                        pick(rng, PEOPLE),
                        pick(rng, PROJECTS)
                    ),
                ),
                1 => (
                    Source::Them,
                    format!(
                        "for {}, we {} and {}",
                        pick(rng, PROJECTS),
                        pick(rng, DONE),
                        pick(rng, NEXT)
                    ),
                ),
                _ => (
                    Source::Them,
                    format!(
                        "{} here, nothing new from me on {}",
                        pick(rng, PEOPLE),
                        pick(rng, PROJECTS)
                    ),
                ),
            };
            out.push((source, *clock, text));
            *clock += 6000;
        }
    };
    let quarter = meeting.len().div_ceil(4);
    for chunk in meeting.chunks(quarter.max(1)) {
        status(&mut rng, &mut out, &mut clock);
        let base = chunk[0].1;
        for (source, start_ms, text) in chunk {
            out.push((*source, clock + start_ms - base, text.clone()));
        }
        clock += chunk.last().map_or(0, |c| c.1 - base) + 6000;
    }
    out
}

/// A small fixed-seed generator, so noisy transcripts are the same every run.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

const FILLERS: &[&str] = &["um", "uh", "so", "like", "you know", "I mean", "yeah"];

/// Small talk and asides that real calls are full of.
const ASIDES: &[(Source, &str)] = &[
    (
        Source::Them,
        "sorry, can you hear me okay? my audio's been weird today",
    ),
    (Source::Me, "yeah, you're a little quiet but it's fine"),
    (
        Source::Them,
        "hang on, let me share my screen. can everyone see that",
    ),
    (Source::Me, "not yet. okay, now it's there"),
    (Source::Them, "sorry, I was on mute"),
    (Source::Me, "no worries. did everyone have a good weekend"),
    (Source::Them, "it was fine. raining the whole time though"),
    (Source::Me, "let me just pull up the ticket, one second"),
    (Source::Them, "I think someone else just joined, hi"),
    (Source::Me, "okay, I think we're good, where were we"),
];

/// Makes a clean transcript look like speech recognition output from a real
/// call: lowercase starts, dropped punctuation, fillers, sentences split
/// across segments, small talk, and echo (the other side's words picked up
/// again by the microphone, as happens without headphones). Planted specifics
/// stay intact, so the checks still apply.
fn noisy(clean: &[(Source, i64, String)]) -> Vec<(Source, i64, String)> {
    let mut rng = Lcg(70);
    let mut out = Vec::new();
    for (index, (source, start_ms, text)) in clean.iter().enumerate() {
        if rng.below(4) == 0 {
            let (aside_source, aside) = ASIDES[index % ASIDES.len()];
            out.push((aside_source, start_ms - 1500, aside.to_owned()));
        }
        let mut words: Vec<String> = text.split_whitespace().map(str::to_owned).collect();
        if rng.below(3) == 0 {
            let filler = FILLERS[rng.below(FILLERS.len() as u64) as usize];
            words.insert(0, format!("{filler},"));
        }
        if let Some(first) = words.first_mut() {
            *first = first.to_lowercase();
        }
        for word in &mut words {
            if rng.below(6) == 0 {
                *word = word.trim_end_matches([',', '.']).to_owned();
            }
        }
        // Split long sentences into two segments, as the chunker does on pauses.
        let pieces: Vec<Vec<String>> = if words.len() > 12 && rng.below(2) == 0 {
            let at = words.len() / 2;
            vec![words[..at].to_vec(), words[at..].to_vec()]
        } else {
            vec![words]
        };
        let mut offset = 0;
        for piece in pieces {
            let line = piece.join(" ");
            out.push((*source, start_ms + offset, line.clone()));
            if *source == Source::Them && rng.below(3) == 0 {
                // Echo arrives a moment later, often clipped.
                let echo: String = piece[..piece.len().min(6)].join(" ");
                out.push((Source::Me, start_ms + offset + 400, echo));
            }
            offset += 2000;
        }
    }
    out
}
