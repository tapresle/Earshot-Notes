//! macOS system audio capture (Core Audio process taps), microphone and
//! system audio recording permissions, and opening at login.
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

#[derive(Debug, Clone, PartialEq)]
pub struct Format {
    /// The capture device's rate when capture started. Each buffer carries the
    /// current rate, which can differ from this one later.
    pub sample_rate: u32,
    /// The rate the tap's format reports. For diagnostics only.
    pub tap_sample_rate: u32,
    pub channels: u16,
    /// The output device being captured.
    pub device_name: String,
}

type Callback = Box<dyn FnMut(&[f32], u32) + Send>;

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
    /// `on_samples` receives interleaved samples and the rate they were captured
    /// at on a real-time thread, and must not block.
    pub fn start(
        on_samples: impl FnMut(&[f32], u32) + Send + 'static,
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

type ChangeCallback = Box<dyn Fn() + Send + Sync>;

/// Watches the default output device, the one calls play through. Dropping it
/// stops watching; after drop returns, the callback has finished its last call.
pub struct OutputDeviceWatcher {
    handle: *mut c_void,
    // Owned here and freed only after the native side has stopped calling it.
    callback: *mut ChangeCallback,
}

// The handle is only touched in `drop`, and the native side serializes its callbacks.
unsafe impl Send for OutputDeviceWatcher {}

impl OutputDeviceWatcher {
    /// Calls `on_change` on a background thread each time the default output
    /// device changes. `on_change` must not block.
    pub fn start(on_change: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        imp::watch(Box::new(on_change))
    }
}

impl Drop for OutputDeviceWatcher {
    fn drop(&mut self) {
        imp::unwatch(self.handle);
        // SAFETY: the native side has stopped and drained its callback queue.
        drop(unsafe { Box::from_raw(self.callback) });
    }
}

/// Sets the Dock icon from image data such as a PNG. Only development builds
/// need this: a bundled app's icon comes from its bundle.
pub fn set_app_icon(image: &[u8]) -> Result<(), String> {
    imp::set_app_icon(image)
}

/// Whether the app opens at login, from macOS's own record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginItem {
    Off,
    On,
    /// Turned on, but waiting for approval in System Settings > General > Login Items.
    NeedsApproval,
    /// macOS can't register this app, for example a development build that isn't bundled.
    Unavailable,
}

impl LoginItem {
    /// Decodes `SMAppServiceStatus`.
    fn from_code(code: i32) -> Self {
        match code {
            0 => LoginItem::Off,
            1 => LoginItem::On,
            2 => LoginItem::NeedsApproval,
            _ => LoginItem::Unavailable,
        }
    }
}

pub fn login_item() -> LoginItem {
    imp::login_item()
}

/// Turns opening at login on or off.
pub fn set_login_item(enabled: bool) -> Result<(), String> {
    imp::set_login_item(enabled)
}

/// Whether macOS opened the app as a login item. Only meaningful while the app
/// is finishing launching, such as during Tauri's setup.
pub fn launched_as_login_item() -> bool {
    imp::launched_as_login_item()
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
        tap_sample_rate: f64,
        channels: u32,
        device_name: [c_char; 128],
    }

    type RawCallback = unsafe extern "C" fn(*mut c_void, *const f32, u32, u32, f64);

    unsafe extern "C" {
        fn gc_system_audio_start(
            callback: RawCallback,
            context: *mut c_void,
            format: *mut RawFormat,
            error: *mut c_char,
            error_len: usize,
        ) -> *mut c_void;
        fn gc_system_audio_stop(handle: *mut c_void);
        fn gc_output_device_watch(
            callback: unsafe extern "C" fn(*mut c_void),
            context: *mut c_void,
            error: *mut c_char,
            error_len: usize,
        ) -> *mut c_void;
        fn gc_output_device_unwatch(handle: *mut c_void);
        fn gc_set_app_icon(image: *const u8, len: usize) -> i32;
        fn gc_login_item_status() -> i32;
        fn gc_login_item_set(enabled: i32, error: *mut c_char, error_len: usize) -> i32;
        fn gc_launched_as_login_item() -> i32;
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
        sample_rate: f64,
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
        callback(samples, sample_rate.round() as u32);
    }

    pub fn start(callback: Callback) -> Result<(SystemAudioTap, Format), String> {
        let callback = Box::into_raw(Box::new(callback));
        let mut format = RawFormat {
            sample_rate: 0.0,
            tap_sample_rate: 0.0,
            channels: 0,
            device_name: [0; 128],
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
                tap_sample_rate: format.tap_sample_rate.round() as u32,
                channels: format.channels as u16,
                // SAFETY: the native side writes a NUL-terminated string, and the
                // buffer starts zeroed in case it writes nothing.
                device_name: unsafe { std::ffi::CStr::from_ptr(format.device_name.as_ptr()) }
                    .to_string_lossy()
                    .into_owned(),
            },
        ))
    }

    pub fn stop(handle: *mut c_void) {
        unsafe { gc_system_audio_stop(handle) };
    }

    unsafe extern "C" fn change_trampoline(context: *mut c_void) {
        // SAFETY: `context` is the `*mut ChangeCallback` passed to watch, alive until
        // unwatch returns, and the native side never calls this concurrently.
        let callback = unsafe { &*(context as *const ChangeCallback) };
        callback();
    }

    pub fn watch(callback: ChangeCallback) -> Result<OutputDeviceWatcher, String> {
        let callback = Box::into_raw(Box::new(callback));
        let mut error = [0 as c_char; 512];
        let handle = unsafe {
            gc_output_device_watch(
                change_trampoline,
                callback.cast(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if handle.is_null() {
            drop(unsafe { Box::from_raw(callback) });
            let message = unsafe { std::ffi::CStr::from_ptr(error.as_ptr()) };
            return Err(message.to_string_lossy().into_owned());
        }
        Ok(OutputDeviceWatcher { handle, callback })
    }

    pub fn unwatch(handle: *mut c_void) {
        unsafe { gc_output_device_unwatch(handle) };
    }

    pub fn set_app_icon(image: &[u8]) -> Result<(), String> {
        match unsafe { gc_set_app_icon(image.as_ptr(), image.len()) } {
            1 => Ok(()),
            _ => Err("the icon isn't a readable image".into()),
        }
    }

    pub fn login_item() -> LoginItem {
        LoginItem::from_code(unsafe { gc_login_item_status() })
    }

    pub fn set_login_item(enabled: bool) -> Result<(), String> {
        let mut error = [0 as c_char; 512];
        if unsafe { gc_login_item_set(enabled.into(), error.as_mut_ptr(), error.len()) } == 1 {
            return Ok(());
        }
        // SAFETY: the native side writes a NUL-terminated message into the zeroed buffer.
        let message = unsafe { std::ffi::CStr::from_ptr(error.as_ptr()) };
        Err(message.to_string_lossy().into_owned())
    }

    pub fn launched_as_login_item() -> bool {
        unsafe { gc_launched_as_login_item() == 1 }
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

    pub fn watch(_callback: ChangeCallback) -> Result<OutputDeviceWatcher, String> {
        Err("watching the output device isn't supported on this platform yet".into())
    }

    pub fn unwatch(_handle: *mut c_void) {}

    pub fn set_app_icon(_image: &[u8]) -> Result<(), String> {
        Ok(())
    }

    pub fn login_item() -> LoginItem {
        LoginItem::Unavailable
    }

    pub fn set_login_item(_enabled: bool) -> Result<(), String> {
        Err("opening at login isn't supported on this platform yet".into())
    }

    pub fn launched_as_login_item() -> bool {
        false
    }

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
    fn watches_and_stops_watching_the_output_device() {
        // No device changes during the test; this proves the listener installs and
        // that dropping it returns, so the callback can be freed safely.
        let watcher = OutputDeviceWatcher::start(|| {}).unwrap();
        drop(watcher);
    }

    #[test]
    fn reads_the_login_item_status_without_changing_it() {
        // A test binary isn't an app bundle, so macOS can't register it; this
        // proves the call links and runs.
        let _ = login_item();
        assert!(!launched_as_login_item());
    }

    #[test]
    fn decodes_login_item_status_codes() {
        assert_eq!(LoginItem::from_code(0), LoginItem::Off);
        assert_eq!(LoginItem::from_code(1), LoginItem::On);
        assert_eq!(LoginItem::from_code(2), LoginItem::NeedsApproval);
        assert_eq!(LoginItem::from_code(3), LoginItem::Unavailable);
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
