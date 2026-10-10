//! Runs whisper.cpp, through libearshot_whisper.dylib, against synthetic audio
//! with the tiny English model. It checks plumbing, not accuracy. Requires
//! scripts/fetch-native.sh.
//!
//! Set `EARSHOT_TEST_CPU=1` to run the model on the CPU, as CI does.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use earshot_lib::chunker::Speech;
use earshot_lib::llm::generator::Generator;
use earshot_lib::llm::llama::{LlamaGenerator, MAX_CONTEXT, Placement};
use earshot_lib::llm::template::Message;
use earshot_lib::transcribe::phantom::is_phantom;
use earshot_lib::transcribe::whisper::{Whisper, WhisperError};
use earshot_lib::transcribe::whistle::{MAX_SAMPLES, SAMPLE_RATE};

const MODEL: &str = "tests/models/ggml-tiny.en-q5_1.bin";
const SUMMARY_MODEL: &str = "tests/models/smollm2-135m-instruct-q4_k_m.gguf";

fn use_gpu() -> bool {
    std::env::var("EARSHOT_TEST_CPU").as_deref() != Ok("1")
}

fn model() -> Whisper {
    Whisper::load(Path::new(MODEL), use_gpu()).unwrap()
}

fn speech_fixture() -> Vec<f32> {
    let mut reader = hound::WavReader::open("tests/fixtures/speech-en.wav").unwrap();
    assert_eq!(reader.spec().sample_rate, SAMPLE_RATE);
    reader
        .samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect()
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect()
}

#[test]
fn transcribes_english_speech() {
    let text = model().transcribe(&speech_fixture()).unwrap();

    let text = normalize(&text);
    assert!(text.contains("quarterly planning meeting"), "{text}");
    assert!(text.contains("budget report"), "{text}");
}

#[test]
fn the_phantom_filter_drops_what_it_invents_from_silence() {
    // Whisper answers silence with a phrase such as "you"; the worker drops it.
    let text = model()
        .transcribe(&vec![0.0; SAMPLE_RATE as usize * 2])
        .unwrap();

    let silence = Speech { ms: 0, margin: 0.0 };
    assert!(text.is_empty() || is_phantom(&text, silence), "{text}");
}

#[test]
fn rejects_a_clip_over_30_seconds() {
    let result = model().transcribe(&vec![0.0; MAX_SAMPLES + 1]);

    assert!(matches!(result, Err(WhisperError::ClipTooLong { .. })));
}

#[test]
fn reports_a_model_that_wont_load() {
    let result = Whisper::load(Path::new("tests/fixtures/speech-en.wav"), use_gpu());

    assert!(matches!(result, Err(WhisperError::Load(_))));
}

/// whisper.cpp and llama.cpp each carry a copy of ggml. Whisper's lives in its
/// own library with hidden symbols, so both engines work in one process.
#[test]
fn runs_alongside_the_summary_engine() {
    let placement = if use_gpu() {
        Placement::Gpu
    } else {
        Placement::Cpu
    };
    let mut llama =
        LlamaGenerator::load_on(Path::new(SUMMARY_MODEL), MAX_CONTEXT, placement).unwrap();
    let mut whisper = model();

    let heard = whisper.transcribe(&speech_fixture()).unwrap();
    let mut said = String::new();
    llama
        .generate(
            &[Message::user("List three colors.")],
            16,
            &AtomicBool::new(false),
            &mut |text| said.push_str(text),
        )
        .unwrap();

    assert!(normalize(&heard).contains("budget report"), "{heard}");
    assert!(!said.trim().is_empty());
    assert!(normalize(&whisper.transcribe(&speech_fixture()).unwrap()).contains("budget report"));
}
