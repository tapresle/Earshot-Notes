//! Safe wrapper over Whisper through whisper.cpp (`whisper-sys`), the optional
//! transcription model (#25).
//!
//! Unlike needle, whisper.cpp keeps no process-global model, but one loaded
//! model takes most of a gigabyte, so the transcription worker owns at most one.
//! [`Whisper`] is `Send` but not `Sync`, and transcription takes `&mut self`.

use std::ffi::{CStr, CString, c_char, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::OnceLock;

use whisper_sys::{EarshotWhisper, Library};

use super::whistle::MAX_SAMPLES;

const INITIAL_OUTPUT_CAPACITY: usize = 4 * 1024;
const MAX_OUTPUT_CAPACITY: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WhisperError {
    #[error("the Whisper engine couldn't be loaded: {0}")]
    Library(String),
    #[error("the Whisper model couldn't be loaded: {0}")]
    Load(String),
    #[error("clip has {samples} samples; the limit is {MAX_SAMPLES} (30 s at 16 kHz)")]
    ClipTooLong { samples: usize },
    #[error("transcription failed: {0}")]
    Engine(String),
    #[error("transcription output exceeded {MAX_OUTPUT_CAPACITY} bytes")]
    OutputTooLarge,
}

pub struct Whisper {
    library: Library,
    model: NonNull<EarshotWhisper>,
}

// The model is used from one thread at a time (`&mut self`), and whisper.cpp
// doesn't tie a context to the thread that created it.
unsafe impl Send for Whisper {}

impl Whisper {
    /// Loads a ggml Whisper model, on the GPU unless `use_gpu` is false.
    pub fn load(model: &Path, use_gpu: bool) -> Result<Self, WhisperError> {
        let library = library()?;
        let path = CString::new(model.as_os_str().as_bytes())
            .map_err(|_| WhisperError::Load("the model path contains a NUL byte".into()))?;
        let ptr = unsafe { (library.load)(path.as_ptr(), c_int::from(use_gpu)) };
        let model = NonNull::new(ptr).ok_or_else(|| WhisperError::Load(last_error(&library)))?;
        Ok(Self { library, model })
    }

    /// Transcribes up to 30 seconds of 16 kHz mono PCM as English, without
    /// sound labels such as `[BLANK_AUDIO]`.
    pub fn transcribe(&mut self, pcm: &[f32]) -> Result<String, WhisperError> {
        if pcm.len() > MAX_SAMPLES {
            return Err(WhisperError::ClipTooLong { samples: pcm.len() });
        }
        let mut capacity = INITIAL_OUTPUT_CAPACITY;
        loop {
            let mut out = vec![0u8; capacity];
            let len = unsafe {
                (self.library.transcribe)(
                    self.model.as_ptr(),
                    pcm.as_ptr(),
                    pcm.len() as c_int,
                    out.as_mut_ptr().cast::<c_char>(),
                    capacity as c_int,
                )
            };
            let Ok(len) = usize::try_from(len) else {
                return Err(WhisperError::Engine(last_error(&self.library)));
            };
            if len >= capacity {
                if capacity >= MAX_OUTPUT_CAPACITY {
                    return Err(WhisperError::OutputTooLarge);
                }
                capacity = (len + 1).min(MAX_OUTPUT_CAPACITY);
                continue;
            }
            return Ok(strip_sound_labels(&String::from_utf8_lossy(&out[..len])));
        }
    }
}

impl Drop for Whisper {
    fn drop(&mut self) {
        unsafe { (self.library.free)(self.model.as_ptr()) };
    }
}

/// Opens the library once per process, from the app bundle's `Frameworks`
/// folder, or in debug builds from the vendor folder.
fn library() -> Result<Library, WhisperError> {
    static LIBRARY: OnceLock<Result<Library, String>> = OnceLock::new();
    LIBRARY
        .get_or_init(|| {
            let candidates = library_candidates(std::env::current_exe().ok().as_deref());
            let mut errors = Vec::new();
            for path in candidates.iter().filter(|p| p.exists()) {
                match Library::open(path) {
                    Ok(library) => return Ok(library),
                    Err(error) => errors.push(format!("{}: {error}", path.display())),
                }
            }
            if errors.is_empty() {
                errors.push(format!(
                    "{} not found (looked in {})",
                    whisper_sys::LIBRARY_FILE,
                    candidates
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Err(errors.join("; "))
        })
        .clone()
        .map_err(WhisperError::Library)
}

fn library_candidates(exe: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    // Contents/MacOS/<binary> -> Contents/Frameworks/<library>
    if let Some(contents) = exe.and_then(Path::parent).and_then(Path::parent) {
        candidates.push(contents.join("Frameworks").join(whisper_sys::LIBRARY_FILE));
    }
    #[cfg(debug_assertions)]
    candidates.push(PathBuf::from(whisper_sys::VENDOR_LIBRARY));
    candidates
}

fn last_error(library: &Library) -> String {
    let ptr = unsafe { (library.last_error)() };
    if ptr.is_null() {
        return "unknown engine error".into();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// Whisper describes sounds it can't transcribe in brackets or parentheses,
/// such as `[BLANK_AUDIO]` or `(clicking)`. They aren't speech, so they go.
fn strip_sound_labels(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut closing: Option<char> = None;
    for c in text.chars() {
        match (closing, c) {
            (None, '[') => closing = Some(']'),
            (None, '(') => closing = Some(')'),
            (Some(end), c) if c == end => closing = None,
            (Some(_), _) => {}
            (None, c) => kept.push(c),
        }
    }
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sound_labels() {
        assert_eq!(strip_sound_labels(" [BLANK_AUDIO]"), "");
        assert_eq!(strip_sound_labels(" (clicking)"), "");
        assert_eq!(
            strip_sound_labels(" So the deploy (keyboard clicking) went out [MUSIC] fine."),
            "So the deploy went out fine."
        );
    }

    #[test]
    fn keeps_text_without_labels() {
        assert_eq!(
            strip_sound_labels(" The webhook retries three times."),
            "The webhook retries three times."
        );
    }

    #[test]
    fn drops_an_unclosed_label_to_the_end() {
        assert_eq!(strip_sound_labels(" Okay. [inaudible"), "Okay.");
    }

    #[test]
    fn looks_for_the_library_in_the_app_bundle_first() {
        let exe = Path::new("/Applications/Earshot Notes.app/Contents/MacOS/earshot");
        let candidates = library_candidates(Some(exe));
        assert_eq!(
            candidates[0],
            Path::new(
                "/Applications/Earshot Notes.app/Contents/Frameworks/libearshot_whisper.dylib"
            )
        );
    }
}
