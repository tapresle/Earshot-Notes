//! Capture of everything the system plays (the other side of a call), through
//! a Core Audio process tap on macOS.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};

use macos_audio::SystemAudioTap;

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

/// Starts capturing system audio. The first start triggers macOS's audio
/// recording permission prompt. If permission is denied, macOS delivers
/// silence rather than an error, so the UI explains where to allow it.
pub fn start(
    frames: SyncSender<Vec<f32>>,
    _on_error: ErrorSink,
) -> Result<(SystemCapture, CaptureFormat), CaptureError> {
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
