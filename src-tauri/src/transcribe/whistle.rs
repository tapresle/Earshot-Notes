//! Safe wrapper over the needle engine's Whistle speech model.
//!
//! The engine holds one process-global, non-thread-safe speech model. [`Whistle`]
//! enforces that: only one instance can exist at a time, it is `Send` but not
//! `Sync`, and transcription takes `&mut self`, so a single owner (the
//! transcription worker) serializes every call.

use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int};
use std::marker::PhantomData;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;

/// Whistle expects 16 kHz mono float PCM in [-1, 1].
pub const SAMPLE_RATE: u32 = 16_000;
/// The engine rejects clips longer than 30 seconds.
pub const MAX_SAMPLES: usize = 30 * SAMPLE_RATE as usize;

const INITIAL_OUTPUT_CAPACITY: usize = 64 * 1024;
const MAX_OUTPUT_CAPACITY: usize = 1024 * 1024;
/// Detecting the language per clip misreads short or quiet call audio as other
/// languages, so every clip is transcribed as English.
const LANGUAGE: &CStr = c"en";

static LOADED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, thiserror::Error)]
pub enum WhistleError {
    #[error("a Whistle model is already loaded in this process")]
    AlreadyLoaded,
    #[error("couldn't read the model file: {0}")]
    Io(#[from] std::io::Error),
    #[error("the engine couldn't load the model: {0}")]
    Load(String),
    #[error("clip has {samples} samples; the limit is {MAX_SAMPLES} (30 s at 16 kHz)")]
    ClipTooLong { samples: usize },
    #[error("transcription failed: {0}")]
    Engine(String),
    #[error("transcription output exceeded {MAX_OUTPUT_CAPACITY} bytes")]
    OutputTooLarge,
    #[error("the engine returned unreadable output: {0}")]
    InvalidOutput(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transcription {
    pub text: String,
    /// `None` when the clip held no speech.
    pub language: Option<String>,
    pub ttft_ms: f32,
    pub decode_tps: f32,
}

pub struct Whistle {
    // Send but not Sync: the owning thread may move it, but never share it.
    _not_sync: PhantomData<Cell<()>>,
}

impl Whistle {
    /// Loads a `.cact` Whistle model. Fails if another instance is alive.
    pub fn load(path: &Path) -> Result<Self, WhistleError> {
        if LOADED.swap(true, Ordering::AcqRel) {
            return Err(WhistleError::AlreadyLoaded);
        }
        // Release the slot on every failure path below.
        let guard = Self {
            _not_sync: PhantomData,
        };

        let cact = std::fs::read(path)?;
        // The engine copies the model during the call; `cact` can drop afterward.
        let code = unsafe { needle_sys::needle_load(cact.as_ptr(), cact.len() as u64) };
        if code < 0 {
            return Err(WhistleError::Load(last_error()));
        }
        if unsafe { needle_sys::needle_models() } & needle_sys::NEEDLE_SPEECH == 0 {
            return Err(WhistleError::Load(
                "the file doesn't contain a speech model".into(),
            ));
        }
        Ok(guard)
    }

    /// Transcribes up to 30 seconds of 16 kHz mono PCM as English.
    pub fn transcribe(&mut self, pcm: &[f32]) -> Result<Transcription, WhistleError> {
        self.transcribe_with_capacity(pcm, INITIAL_OUTPUT_CAPACITY)
    }

    fn transcribe_with_capacity(
        &mut self,
        pcm: &[f32],
        initial_capacity: usize,
    ) -> Result<Transcription, WhistleError> {
        if pcm.len() > MAX_SAMPLES {
            return Err(WhistleError::ClipTooLong { samples: pcm.len() });
        }

        let mut capacity = initial_capacity;
        loop {
            let mut out = vec![0u8; capacity];
            let code = unsafe {
                needle_sys::needle_transcribe(
                    pcm.as_ptr(),
                    pcm.len() as c_int,
                    LANGUAGE.as_ptr(),
                    std::ptr::null(),
                    0,
                    out.as_mut_ptr().cast::<c_char>(),
                    capacity as c_int,
                )
            };
            if code < 0 {
                return Err(WhistleError::Engine(last_error()));
            }

            // The engine truncates silently when the buffer is too small, so a
            // full buffer means the output may be cut off.
            let len = out.iter().position(|&b| b == 0).unwrap_or(capacity);
            if len + 1 >= capacity {
                if capacity >= MAX_OUTPUT_CAPACITY {
                    return Err(WhistleError::OutputTooLarge);
                }
                capacity *= 4;
                continue;
            }
            return parse_output(&out[..len]);
        }
    }
}

impl Drop for Whistle {
    fn drop(&mut self) {
        LOADED.store(false, Ordering::Release);
    }
}

fn last_error() -> String {
    let ptr = unsafe { needle_sys::needle_last_error() };
    if ptr.is_null() {
        return "unknown engine error".into();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

#[derive(Deserialize)]
struct RawTranscription {
    text: String,
    language: String,
    ttft_ms: f32,
    decode_tps: f32,
}

fn parse_output(json: &[u8]) -> Result<Transcription, WhistleError> {
    let raw: RawTranscription = serde_json::from_slice(json)?;
    Ok(Transcription {
        text: raw.text.trim().to_owned(),
        language: (!raw.language.is_empty()).then_some(raw.language),
        ttft_ms: raw.ttft_ms,
        decode_tps: raw.decode_tps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_speech_output() {
        let json =
            br#"{"text":" Hello there. ","language":"en","ttft_ms":5.9,"decode_tps":1159.7}"#;
        let t = parse_output(json).unwrap();
        assert_eq!(t.text, "Hello there.");
        assert_eq!(t.language.as_deref(), Some("en"));
    }

    #[test]
    fn maps_empty_language_to_none() {
        let json = br#"{"text":"","language":"","ttft_ms":0.0,"decode_tps":0.0}"#;
        let t = parse_output(json).unwrap();
        assert_eq!(t.text, "");
        assert_eq!(t.language, None);
    }

    #[test]
    fn grows_the_output_buffer_when_the_engine_fills_it() {
        let mut reader = hound::WavReader::open("tests/fixtures/speech-en.wav").unwrap();
        let pcm: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32768.0)
            .collect();
        let mut whistle =
            Whistle::load(std::path::Path::new("resources/models/whistle.cact")).unwrap();

        let result = whistle.transcribe_with_capacity(&pcm, 16).unwrap();

        assert!(result.text.ends_with("budget report."));
    }

    #[test]
    fn rejects_truncated_output() {
        let json = br#"{"text":"The qu"#;
        assert!(matches!(
            parse_output(json),
            Err(WhistleError::InvalidOutput(_))
        ));
    }
}
