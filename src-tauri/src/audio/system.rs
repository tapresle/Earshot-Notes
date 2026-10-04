//! Capture of everything the system plays (the other side of a call), through
//! a Core Audio process tap on macOS.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};

use macos_audio::{Permission, SystemAudioTap};

use super::{Capture, CaptureError, CaptureFormat, ErrorSink};

pub struct SystemCapture {
    tap: Option<SystemAudioTap>,
    dropped: Arc<AtomicU64>,
}

impl Capture for SystemCapture {}

impl Drop for SystemCapture {
    fn drop(&mut self) {
        // Stops the tap and drops the callback, which closes the sender.
        drop(self.tap.take());
        let dropped = self.dropped.load(Ordering::Relaxed);
        if dropped > 0 {
            eprintln!(
                "system audio capture dropped {dropped} buffers because transcription fell behind"
            );
        }
    }
}

/// Starts capturing system audio, asking for permission first if needed (this
/// blocks until the prompt is answered). When macOS won't report the
/// permission, capture starts anyway; a denied permission then records silence.
pub fn start(
    frames: SyncSender<Vec<f32>>,
    _on_error: ErrorSink,
) -> Result<(SystemCapture, CaptureFormat), CaptureError> {
    check_access(
        macos_audio::system_audio_permission(),
        macos_audio::request_system_audio_access,
    )?;
    let dropped = Arc::new(AtomicU64::new(0));
    let counter = dropped.clone();
    let (tap, format) = SystemAudioTap::start(move |samples| {
        if let Err(TrySendError::Full(_)) = frames.try_send(samples.to_vec()) {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    })
    .map_err(CaptureError::SystemAudio)?;
    Ok((
        SystemCapture {
            tap: Some(tap),
            dropped,
        },
        CaptureFormat {
            sample_rate: format.sample_rate,
            channels: format.channels,
        },
    ))
}

/// Decides whether to start capture from the permission status, asking first
/// when it's undecided. An unknown status lets capture proceed.
fn check_access(
    status: Option<Permission>,
    request: impl FnOnce() -> Option<bool>,
) -> Result<(), CaptureError> {
    match status {
        Some(Permission::Authorized) | None => Ok(()),
        Some(Permission::NotDetermined) => match request() {
            Some(false) => Err(CaptureError::SystemAudioPermissionDenied),
            Some(true) | None => Ok(()),
        },
        Some(Permission::Denied | Permission::Restricted) => {
            Err(CaptureError::SystemAudioPermissionDenied)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never_asked() -> Option<bool> {
        panic!("shouldn't ask");
    }

    #[test]
    fn starts_when_allowed_or_unknown() {
        assert!(check_access(Some(Permission::Authorized), never_asked).is_ok());
        assert!(check_access(None, never_asked).is_ok());
    }

    #[test]
    fn asks_when_undecided_and_follows_the_answer() {
        assert!(check_access(Some(Permission::NotDetermined), || Some(true)).is_ok());
        assert!(matches!(
            check_access(Some(Permission::NotDetermined), || Some(false)),
            Err(CaptureError::SystemAudioPermissionDenied)
        ));
        // If macOS won't let the app ask, starting the tap shows the prompt instead.
        assert!(check_access(Some(Permission::NotDetermined), || None).is_ok());
    }

    #[test]
    fn refuses_when_denied_or_restricted() {
        for status in [Permission::Denied, Permission::Restricted] {
            assert!(matches!(
                check_access(Some(status), never_asked),
                Err(CaptureError::SystemAudioPermissionDenied)
            ));
        }
    }
}
