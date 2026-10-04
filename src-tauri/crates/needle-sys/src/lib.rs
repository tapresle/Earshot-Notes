//! Raw bindings to `vendor/needle/<platform>/needle.h`.
//!
//! The engine keeps one process-global, non-thread-safe model per kind (text and
//! speech). Negative return values signal failure; `needle_last_error` returns the
//! reason. Callers must serialize every call into this library.

use std::ffi::{c_char, c_float, c_int, c_uchar, c_ulonglong};

pub const NEEDLE_TEXT: c_int = 1;
pub const NEEDLE_SPEECH: c_int = 2;

unsafe extern "C" {
    pub fn needle_load(cact: *const c_uchar, n: c_ulonglong) -> c_int;
    pub fn needle_models() -> c_int;
    pub fn needle_last_error() -> *const c_char;
    pub fn needle_reset();
    pub fn needle_transcribe(
        pcm: *const c_float,
        samples: c_int,
        language: *const c_char,
        keywords: *const c_char,
        word_timestamps: c_int,
        out: *mut c_char,
        out_capacity: c_int,
    ) -> c_int;
    pub fn needle_embed(
        input: *const c_char,
        pcm: *const c_float,
        samples: c_int,
        out: *mut c_float,
        out_capacity: c_int,
    ) -> c_int;
}
