//! Runs the summary engine against a tiny real model (SmolLM2 135M). It checks
//! plumbing, not summary quality. Requires scripts/fetch-native.sh.
//!
//! Set `EARSHOT_TEST_CPU=1` to run the model on the CPU. CI does, because the
//! virtual GPU on hosted Mac runners makes these tests take minutes instead of seconds.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use earshot_lib::llm::generator::{GenerateError, Generator};
use earshot_lib::llm::llama::{LlamaGenerator, MAX_CONTEXT, Placement};
use earshot_lib::llm::template::Message;
use earshot_lib::store::{Source, TranscriptSegment};
use earshot_lib::summarize::run::{Progress, Request, summarize};

const MODEL: &str = "tests/models/smollm2-135m-instruct-q4_k_m.gguf";

fn placement() -> Placement {
    match std::env::var("EARSHOT_TEST_CPU").as_deref() {
        Ok("1") => Placement::Cpu,
        _ => Placement::Gpu,
    }
}

fn model_with_context_limit(limit: usize) -> LlamaGenerator {
    LlamaGenerator::load_on(Path::new(MODEL), limit, placement()).unwrap()
}

fn model() -> LlamaGenerator {
    model_with_context_limit(MAX_CONTEXT)
}

fn question() -> Vec<Message> {
    vec![Message::user("List three colors.")]
}

fn transcript(lines: usize) -> Vec<TranscriptSegment> {
    (0..lines)
        .map(|i| TranscriptSegment {
            id: i as i64,
            meeting_id: 1,
            source: if i % 2 == 0 { Source::Me } else { Source::Them },
            start_ms: i as i64 * 6000,
            end_ms: i as i64 * 6000 + 5000,
            text: format!(
                "We agreed item {i} ships on Friday and Dana owns the budget review for it."
            ),
            language: Some("en".into()),
        })
        .collect()
}

#[test]
fn generates_and_streams_the_same_text() {
    let mut model = model();
    let mut streamed = String::new();

    let output = model
        .generate(&question(), 48, &AtomicBool::new(false), &mut |t| {
            streamed.push_str(t)
        })
        .unwrap();

    assert!(!output.is_empty());
    assert_eq!(streamed.trim(), output);
}

#[test]
fn stops_at_the_token_limit() {
    let mut model = model();

    let output = model
        .generate(
            &[Message::user("Write a very long story about the sea.")],
            8,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();

    assert!(model.count_tokens(&output).unwrap() <= 10, "{output}");
}

#[test]
fn stops_when_cancelled_mid_generation() {
    let mut model = model();
    let cancel = AtomicBool::new(false);
    let mut pieces = 0;

    let result = model.generate(
        &[Message::user("Write a very long story about the sea.")],
        200,
        &cancel,
        &mut |_| {
            pieces += 1;
            cancel.store(true, Ordering::Relaxed);
        },
    );

    assert!(matches!(result, Err(GenerateError::Cancelled)));
    assert_eq!(pieces, 1);
}

#[test]
fn summarizes_a_short_meeting_in_one_pass() {
    let mut model = model();
    let segments = transcript(6);
    let mut progress = Vec::new();

    let summary = summarize(
        &mut model,
        &Request {
            title: "Planning",
            notes_markdown: "- budget",
            segments: &segments,
            terms: &[],
        },
        &AtomicBool::new(false),
        &mut |p| progress.push(p),
        &mut |_| {},
    )
    .unwrap();

    assert!(!summary.is_empty());
    assert_eq!(progress, vec![Progress::Writing]);
}

#[test]
fn condenses_in_parts_when_the_meeting_exceeds_the_context() {
    let mut model = model_with_context_limit(2560);
    let segments = transcript(120);
    let mut progress = Vec::new();

    let summary = summarize(
        &mut model,
        &Request {
            title: "Planning",
            notes_markdown: "",
            segments: &segments,
            terms: &[],
        },
        &AtomicBool::new(false),
        &mut |p| progress.push(p),
        &mut |_| {},
    )
    .unwrap();

    assert!(!summary.is_empty());
    assert!(
        matches!(progress[0], Progress::Reading { part: 1, .. }),
        "{progress:?}"
    );
    assert_eq!(progress.last(), Some(&Progress::Writing));
}

#[test]
fn reports_a_missing_model_file() {
    let result = LlamaGenerator::load(Path::new("tests/models/missing.gguf"));

    assert!(matches!(result, Err(GenerateError::Model(m)) if m.contains("missing")));
}
