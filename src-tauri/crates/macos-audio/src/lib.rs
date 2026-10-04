//! macOS system audio capture (Core Audio process taps), and microphone and
//! system audio recording permissions.
//!
//! On other platforms every function reports that the feature is unsupported.

use std::ffi::{c_char, c_void};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
}

impl Permission {
    /// Decodes the native status codes, which follow `AVAuthorizationStatus`.
    fn from_code(code: i32) -> Self {
        match code {
            1 => Permission::Restricted,
            2 => Permission::Denied,
            3 => Permission::Authorized,
            _ => Permission::NotDetermined,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Format {
    pub sample_rate: u32,
    pub channels: u16,
}

type Callback = Box<dyn FnMut(&[f32]) + Send>;

/// A running system audio capture. Dropping it stops the capture; after drop
/// returns, the callback has finished its last call.
pub struct SystemAudioTap {
    handle: *mut c_void,
    // Owned here and freed only after the native side has stopped calling it.
    callback: *mut Callback,
}

// The handle is only touched in `drop`, and the native side serializes its callbacks.
unsafe impl Send for SystemAudioTap {}

impl SystemAudioTap {
    /// Starts capturing everything the system plays except this app.
    /// `on_samples` receives interleaved samples on a real-time thread and must not block.
    pub fn start(
        on_samples: impl FnMut(&[f32]) + Send + 'static,
    ) -> Result<(Self, Format), String> {
        imp::start(Box::new(on_samples))
    }
}

impl Drop for SystemAudioTap {
    fn drop(&mut self) {
        imp::stop(self.handle);
        // SAFETY: the native side has stopped and drained its callback queue.
        drop(unsafe { Box::from_raw(self.callback) });
    }
}

pub fn mic_permission() -> Permission {
    imp::mic_permission()
}

/// Prompts for microphone access if needed and waits for the answer.
/// Blocks, so never call it on the UI thread.
pub fn request_mic_access() -> bool {
    imp::request_mic_access()
}

/// System audio recording permission, or `None` when macOS doesn't let the app
/// check (the private API it relies on is unavailable).
pub fn system_audio_permission() -> Option<Permission> {
    imp::system_audio_permission()
}

/// Prompts for system audio recording access if needed and waits for the
/// answer. Returns `None` when macOS doesn't let the app ask directly. Blocks,
/// so never call it on the UI thread.
pub fn request_system_audio_access() -> Option<bool> {
    imp::request_system_audio_access()
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

    #[repr(C)]
    struct RawFormat {
        sample_rate: f64,
        channels: u32,
    }

    type RawCallback = unsafe extern "C" fn(*mut c_void, *const f32, u32, u32);

    unsafe extern "C" {
        fn gc_system_audio_start(
            callback: RawCallback,
            context: *mut c_void,
            format: *mut RawFormat,
            error: *mut c_char,
            error_len: usize,
        ) -> *mut c_void;
        fn gc_system_audio_stop(handle: *mut c_void);
        fn gc_mic_authorization_status() -> i32;
        fn gc_mic_request_access() -> i32;
        fn gc_system_audio_authorization_status() -> i32;
        fn gc_system_audio_request_access() -> i32;
    }

    unsafe extern "C" fn trampoline(
        context: *mut c_void,
        samples: *const f32,
        frames: u32,
        channels: u32,
    ) {
        if samples.is_null() || frames == 0 {
            return;
        }
        // SAFETY: `context` is the `*mut Callback` passed to start, alive until stop
        // returns, and the native side never calls this concurrently. `samples` holds
        // `frames * channels` floats for the duration of the call.
        let callback = unsafe { &mut *(context as *mut Callback) };
        let samples =
            unsafe { std::slice::from_raw_parts(samples, frames as usize * channels as usize) };
        callback(samples);
    }

    pub fn start(callback: Callback) -> Result<(SystemAudioTap, Format), String> {
        let callback = Box::into_raw(Box::new(callback));
        let mut format = RawFormat {
            sample_rate: 0.0,
            channels: 0,
        };
        let mut error = [0 as c_char; 512];
        let handle = unsafe {
            gc_system_audio_start(
                trampoline,
                callback.cast(),
                &mut format,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if handle.is_null() {
            drop(unsafe { Box::from_raw(callback) });
            let message = unsafe { std::ffi::CStr::from_ptr(error.as_ptr()) };
            return Err(message.to_string_lossy().into_owned());
        }
        Ok((
            SystemAudioTap { handle, callback },
            Format {
                sample_rate: format.sample_rate.round() as u32,
                channels: format.channels as u16,
            },
        ))
    }

    pub fn stop(handle: *mut c_void) {
        unsafe { gc_system_audio_stop(handle) };
    }

    pub fn mic_permission() -> Permission {
        Permission::from_code(unsafe { gc_mic_authorization_status() })
    }

    pub fn request_mic_access() -> bool {
        unsafe { gc_mic_request_access() == 1 }
    }

    pub fn system_audio_permission() -> Option<Permission> {
        match unsafe { gc_system_audio_authorization_status() } {
            -1 => None,
            code => Some(Permission::from_code(code)),
        }
    }

    pub fn request_system_audio_access() -> Option<bool> {
        match unsafe { gc_system_audio_request_access() } {
            -1 => None,
            granted => Some(granted == 1),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    pub fn start(_callback: Callback) -> Result<(SystemAudioTap, Format), String> {
        Err("system audio capture isn't supported on this platform yet".into())
    }

    pub fn stop(_handle: *mut c_void) {}

    pub fn mic_permission() -> Permission {
        Permission::Authorized
    }

    pub fn request_mic_access() -> bool {
        true
    }

    pub fn system_audio_permission() -> Option<Permission> {
        None
    }

    pub fn request_system_audio_access() -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_microphone_permission_without_prompting() {
        // Any status is valid; this proves the native library links and runs.
        let _ = mic_permission();
    }

    #[test]
    fn reads_system_audio_permission_without_prompting() {
        // On current macOS the private TCC API resolves, so a status comes back.
        // Any status is valid; this proves the lookup links and doesn't crash.
        assert!(
            system_audio_permission().is_some(),
            "TCC preflight is unavailable"
        );
    }

    #[test]
    fn decodes_native_status_codes() {
        assert_eq!(Permission::from_code(0), Permission::NotDetermined);
        assert_eq!(Permission::from_code(1), Permission::Restricted);
        assert_eq!(Permission::from_code(2), Permission::Denied);
        assert_eq!(Permission::from_code(3), Permission::Authorized);
        assert_eq!(Permission::from_code(99), Permission::NotDetermined);
    }
}
