//! Summarizes synthetic meetings with a real model and checks which planted
//! specifics survive into the notes (#70). Summary quality can't be asserted in
//! tests, so this is a tool for comparing prompts, sampling, and approaches.
//!
//!   cargo run --release --example summary_eval -- \
//!     --model tests/models/gemma-4-E4B-it-Q4_K_M.gguf \
//!     [--sampling faithful|gemma4] [--context TOKENS] [--approach single|extract] \
//!     [--noisy] [--pad MINUTES] [--terms "Name One, Name Two"] \
//!     tests/fixtures/meetings/*.json
//!
//! `--context` caps the model's context; a small value forces long-meeting
//! condensing, which is how the "chunked" approach is measured. `--noisy`
//! makes each transcript look like real speech recognition output (see
//! [`noisy`]) without touching the planted specifics. `--pad MINUTES` spreads
//! the meeting across that many minutes of unrelated status updates (see
//! [`padded`]), to test whether specifics survive a long meeting.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use earshot_lib::store::{Source, TranscriptSegment};
use earshot_lib::summarize::llama::{LlamaGenerator, Sampling};
use earshot_lib::summarize::prompt::{self, transcript_line};
use earshot_lib::summarize::run::{Generator, Progress, Request, summarize};
use earshot_lib::summarize::template::Message;
use serde::Deserialize;

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
    meetings: Vec<PathBuf>,
}

fn main() {
    let options = options();
    let mut generator = match options.context {
        Some(limit) => LlamaGenerator::load_with_context_limit(&options.model, limit),
        None => LlamaGenerator::load(&options.model),
    }
    .expect("couldn't load the model");
    generator.set_sampling(options.sampling);
    println!(
        "# {}\n\nsampling: {}, context: {}, approach: {}\n",
        options.model.file_name().unwrap().to_string_lossy(),
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
    let mut total_found = 0;
    let mut total_expected = 0;
    for path in &options.meetings {
        let (found, expected) = evaluate(
            &mut generator,
            path,
            options.extract,
            options.noisy,
            options.pad_minutes,
            &options.terms,
        );
        total_found += found;
        total_expected += expected;
    }
    println!("## Total\n\n{total_found} of {total_expected} specifics kept");
}

fn evaluate(
    generator: &mut LlamaGenerator,
    path: &Path,
    extract: bool,
    noisy_transcript: bool,
    pad_minutes: u64,
    terms: &[String],
) -> (usize, usize) {
    let mut meeting: Meeting =
        serde_json::from_str(&std::fs::read_to_string(path).expect("unreadable fixture"))
            .expect("fixture isn't a meeting");
    if pad_minutes > 0 {
        meeting.segments = padded(&meeting.segments, pad_minutes);
    }
    if noisy_transcript {
        meeting.segments = noisy(&meeting.segments);
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
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let mut parts = 0;
    let notes = if extract {
        extract_then_write(generator, &meeting, &segments, &cancel)
    } else {
        let request = Request {
            title: &meeting.title,
            notes_markdown: &meeting.notes,
            segments: &segments,
            terms,
        };
        summarize(
            generator,
            &request,
            &cancel,
            &mut |progress| {
                if let Progress::Reading { parts: n, .. } = progress {
                    parts = n;
                }
            },
            &mut |_| {},
        )
        .expect("summary failed")
    };
    let elapsed = started.elapsed();

    let has = |alternatives: &str| {
        let lower = notes.to_lowercase();
        alternatives
            .split('|')
            .any(|a| lower.contains(&a.to_lowercase()))
    };
    let missing: Vec<&str> = meeting
        .expect
        .iter()
        .map(String::as_str)
        .filter(|e| !has(e))
        .collect();
    let forbidden: Vec<&str> = meeting
        .forbid
        .iter()
        .map(String::as_str)
        .filter(|f| has(f))
        .collect();
    let found = meeting.expect.len() - missing.len();
    println!(
        "## {} ({})\n\n{:.1} s, {}, {} of {} specifics kept",
        meeting.title,
        path.file_name().unwrap().to_string_lossy(),
        elapsed.as_secs_f64(),
        if parts > 0 {
            format!("condensed in {parts} parts")
        } else {
            "one pass".into()
        },
        found,
        meeting.expect.len(),
    );
    if !missing.is_empty() {
        println!("missing: {}", missing.join(", "));
    }
    if !forbidden.is_empty() {
        println!("forbidden text present: {}", forbidden.join(", "));
    }
    println!("\n```markdown\n{}\n```\n", notes.trim());
    (found, meeting.expect.len())
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
    segments: &[TranscriptSegment],
    cancel: &AtomicBool,
) -> String {
    let transcript = segments
        .iter()
        .map(transcript_line)
        .collect::<Vec<_>>()
        .join("\n");
    let extract = [
        Message::system(EXTRACT_SYSTEM),
        Message::user(format!(
            "Transcript:\n<transcript>\n{transcript}\n</transcript>"
        )),
    ];
    let facts = generator
        .generate(&extract, 2048, cancel, &mut |_| {})
        .expect("extraction failed");
    let mut messages = prompt::summary_messages(&meeting.title, &meeting.notes, &[], &transcript);
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
