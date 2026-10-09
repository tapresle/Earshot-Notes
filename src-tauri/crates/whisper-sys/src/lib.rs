//! Loads `libearshot_whisper.dylib` and binds the functions in
//! `native/earshot_whisper.h`.
//!
//! The library is opened with `dlopen` when Whisper is first used, rather than
//! linked, so the app starts and transcribes with Whistle even when the library
//! is missing. It stays loaded for the life of the process: ggml keeps global
//! state that unloading would leave dangling.

use std::ffi::{CStr, CString, c_char, c_float, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// The library's file name, in the app's `Contents/Frameworks` folder.
pub const LIBRARY_FILE: &str = "libearshot_whisper.dylib";

/// Where `scripts/build-whisper.sh` puts the library. Only debug builds look
/// there, so a shipped app never loads code from outside its bundle.
#[cfg(debug_assertions)]
pub const VENDOR_LIBRARY: &str = env!("EARSHOT_WHISPER_VENDOR_LIBRARY");

/// An opaque `earshot_whisper`: one loaded model.
#[repr(C)]
pub struct EarshotWhisper {
    _private: [u8; 0],
}

pub type LoadFn =
    unsafe extern "C" fn(model_path: *const c_char, use_gpu: c_int) -> *mut EarshotWhisper;
pub type TranscribeFn = unsafe extern "C" fn(
    whisper: *mut EarshotWhisper,
    pcm: *const c_float,
    samples: c_int,
    out: *mut c_char,
    out_capacity: c_int,
) -> c_int;
pub type FreeFn = unsafe extern "C" fn(whisper: *mut EarshotWhisper);
pub type LastErrorFn = unsafe extern "C" fn() -> *const c_char;

/// The library's functions. See `native/earshot_whisper.h` for their contracts.
#[derive(Clone, Copy)]
pub struct Library {
    pub load: LoadFn,
    pub transcribe: TranscribeFn,
    pub free: FreeFn,
    pub last_error: LastErrorFn,
}

const RTLD_NOW: c_int = 0x2;
const RTLD_LOCAL: c_int = 0x4;

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

impl Library {
    /// Opens the library at `path` and resolves its functions.
    pub fn open(path: &Path) -> Result<Self, String> {
        let path_c = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| format!("{} contains a NUL byte", path.display()))?;
        let handle = unsafe { dlopen(path_c.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if handle.is_null() {
            return Err(dl_error());
        }
        unsafe {
            Ok(Self {
                load: std::mem::transmute::<*mut c_void, LoadFn>(symbol(
                    handle,
                    c"earshot_whisper_load",
                )?),
                transcribe: std::mem::transmute::<*mut c_void, TranscribeFn>(symbol(
                    handle,
                    c"earshot_whisper_transcribe",
                )?),
                free: std::mem::transmute::<*mut c_void, FreeFn>(symbol(
                    handle,
                    c"earshot_whisper_free",
                )?),
                last_error: std::mem::transmute::<*mut c_void, LastErrorFn>(symbol(
                    handle,
                    c"earshot_whisper_last_error",
                )?),
            })
        }
    }
}

unsafe fn symbol(handle: *mut c_void, name: &CStr) -> Result<*mut c_void, String> {
    let ptr = unsafe { dlsym(handle, name.as_ptr()) };
    if ptr.is_null() {
        return Err(dl_error());
    }
    Ok(ptr)
}

fn dl_error() -> String {
    let ptr = unsafe { dlerror() };
    if ptr.is_null() {
        return "unknown dynamic loader error".into();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}
