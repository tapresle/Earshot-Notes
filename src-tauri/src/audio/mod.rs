//! Audio capture and conversion to Whistle's input format (16 kHz mono f32).

pub mod mic;
pub mod resample;
pub mod system;

use std::sync::Arc;

/// A running capture. Dropping it stops the capture and closes its sender.
pub trait Capture: Send {}

/// The layout of the interleaved buffers a capture sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Interleaved samples and the rate the device captured them at. The rate can
/// change during a recording, for example when a Bluetooth headset switches
/// profile, so it travels with every buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct Buffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// Receives user-facing descriptions of problems that happen after a capture started.
pub type ErrorSink = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("no microphone is available")]
    NoDevice,
    #[error(
        "microphone access is off. Turn it on in System Settings > Privacy & Security > Microphone"
    )]
    MicPermissionDenied,
    #[error("the microphone couldn't start: {0}")]
    Device(String),
    #[error("system audio couldn't start: {0}")]
    SystemAudio(String),
    #[error(
        "system audio recording is off, so only your side is transcribed. Turn on Earshot Notes in System Settings > Privacy & Security > Screen & System Audio Recording"
    )]
    SystemAudioPermissionDenied,
}

/// Averages interleaved frames into one mono channel.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_app_in_the_system_audio_permission_message() {
        assert!(
            CaptureError::SystemAudioPermissionDenied
                .to_string()
                .contains("Turn on Earshot Notes in System Settings")
        );
    }

    #[test]
    fn passes_mono_through() {
        assert_eq!(downmix(&[0.1, 0.2], 1), vec![0.1, 0.2]);
    }

    #[test]
    fn averages_stereo_frames() {
        assert_eq!(
            downmix(&[1.0, 0.0, 0.5, 0.5, -1.0, 1.0], 2),
            vec![0.5, 0.5, 0.0]
        );
    }

    #[test]
    fn drops_an_incomplete_trailing_frame() {
        assert_eq!(downmix(&[1.0, 1.0, 1.0], 2), vec![1.0]);
    }
}
