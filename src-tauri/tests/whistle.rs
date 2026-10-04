//! Runs the real needle engine against synthetic audio. Requires scripts/fetch-native.sh.

use std::path::Path;
use std::sync::Mutex;

use earshot_lib::transcribe::whistle::{MAX_SAMPLES, SAMPLE_RATE, Whistle, WhistleError};

const MODEL: &str = "resources/models/whistle.cact";

// The engine allows one speech model per process, so tests take turns.
static ENGINE: Mutex<()> = Mutex::new(());

fn speech_fixture() -> Vec<f32> {
    let mut reader = hound::WavReader::open("tests/fixtures/speech-en.wav").unwrap();
    assert_eq!(reader.spec().sample_rate, SAMPLE_RATE);
    assert_eq!(reader.spec().channels, 1);
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
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();

    let result = whistle.transcribe(&speech_fixture()).unwrap();

    assert_eq!(
        normalize(&result.text),
        "the quarterly planning meeting starts at 10 please bring the budget report"
    );
    assert_eq!(result.language.as_deref(), Some("en"));
}

#[test]
fn returns_empty_text_for_silence() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();

    let result = whistle
        .transcribe(&vec![0.0; SAMPLE_RATE as usize * 5])
        .unwrap();

    assert_eq!(result.text, "");
    assert_eq!(result.language, None);
}

#[test]
fn accepts_a_full_thirty_second_clip() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();
    let clip: Vec<f32> = speech_fixture()
        .into_iter()
        .cycle()
        .take(MAX_SAMPLES)
        .collect();

    let result = whistle.transcribe(&clip).unwrap();

    assert!(normalize(&result.text).contains("budget report"));
}

#[test]
fn rejects_clips_over_thirty_seconds() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();

    let result = whistle.transcribe(&vec![0.0; MAX_SAMPLES + 1]);

    assert!(matches!(
        result,
        Err(WhistleError::ClipTooLong { samples }) if samples == MAX_SAMPLES + 1
    ));
}

#[test]
fn allows_only_one_instance_at_a_time() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let first = Whistle::load(Path::new(MODEL)).unwrap();

    assert!(matches!(
        Whistle::load(Path::new(MODEL)),
        Err(WhistleError::AlreadyLoaded)
    ));

    drop(first);
    assert!(Whistle::load(Path::new(MODEL)).is_ok());
}

#[test]
fn reports_a_missing_model_file() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());

    assert!(matches!(
        Whistle::load(Path::new("resources/models/missing.cact")),
        Err(WhistleError::Io(_))
    ));
    // A failed load releases the slot.
    assert!(Whistle::load(Path::new(MODEL)).is_ok());
}

#[test]
fn rejects_a_file_that_is_not_a_model() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());

    assert!(matches!(
        Whistle::load(Path::new("tests/fixtures/speech-en.wav")),
        Err(WhistleError::Load(_))
    ));
}
