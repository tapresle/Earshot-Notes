//! Microphone capture through cpal, using the system's default input device.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, FromSample, SampleFormat, SizedSample};
use serde::Serialize;

use macos_audio::Permission as MacPermission;

use super::{Capture, CaptureError, CaptureFormat, ErrorSink};

/// Holds the capture thread. Dropping it stops the stream and closes the sender.
pub struct MicCapture {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Capture for MicCapture {}

impl Drop for MicCapture {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A microphone the user can choose in settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputDevice {
    /// Stable across restarts and reconnects; stored in settings.
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// The microphones connected now.
pub fn input_devices() -> Result<Vec<InputDevice>, CaptureError> {
    let host = cpal::default_host();
    let default_id = host.default_input_device().and_then(|d| d.id().ok());
    let devices = host
        .input_devices()
        .map_err(|e| CaptureError::Device(e.to_string()))?;
    Ok(devices
        .filter_map(|device| {
            let id = device.id().ok()?;
            let name = device
                .description()
                .map(|d| d.name().to_owned())
                .unwrap_or_else(|_| id.to_string());
            Some(InputDevice {
                is_default: Some(&id) == default_id.as_ref(),
                id: id.to_string(),
                name,
            })
        })
        .collect())
}

/// Shown when the microphone chosen in settings isn't connected.
pub const MISSING_DEVICE: &str =
    "your chosen microphone isn't connected, so the default microphone is being used";

/// Starts capturing from `preferred` (a device id from settings) or the default
/// microphone, asking for permission first if needed (this blocks until the
/// prompt is answered). If the preferred device is missing, capture falls back
/// to the default and says so through `on_error`. Interleaved buffers go to
/// `frames`; when the pipeline falls behind, buffers are dropped rather than
/// blocking the audio thread. Stream failures after start go to `on_error`.
pub fn start(
    frames: SyncSender<Vec<f32>>,
    on_error: ErrorSink,
    preferred: Option<String>,
) -> Result<(MicCapture, CaptureFormat), CaptureError> {
    ensure_permission()?;
    let (ready_tx, ready_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let thread = thread::Builder::new()
        .name("mic-capture".into())
        .spawn(move || run(frames, on_error, preferred, ready_tx, stop_rx))
        .map_err(|e| CaptureError::Device(e.to_string()))?;

    match ready_rx.recv() {
        Ok(Ok(format)) => Ok((
            MicCapture {
                stop: Some(stop_tx),
                thread: Some(thread),
            },
            format,
        )),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(_) => Err(CaptureError::Device("the capture thread exited".into())),
    }
}

fn ensure_permission() -> Result<(), CaptureError> {
    match macos_audio::mic_permission() {
        MacPermission::Authorized => Ok(()),
        MacPermission::NotDetermined if macos_audio::request_mic_access() => Ok(()),
        _ => Err(CaptureError::MicPermissionDenied),
    }
}

fn run(
    frames: SyncSender<Vec<f32>>,
    on_error: ErrorSink,
    preferred: Option<String>,
    ready: mpsc::Sender<Result<CaptureFormat, CaptureError>>,
    stop: Receiver<()>,
) {
    let dropped = Arc::new(AtomicU64::new(0));
    let stream = match open(frames, on_error, preferred, dropped.clone()) {
        Ok((stream, format)) => {
            let _ = ready.send(Ok(format));
            stream
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };

    // Blocks until the MicCapture is dropped.
    let _ = stop.recv();
    drop(stream);

    let dropped = dropped.load(Ordering::Relaxed);
    if dropped > 0 {
        eprintln!("microphone capture dropped {dropped} buffers because transcription fell behind");
    }
}

/// Picks the preferred device if it's connected, otherwise the default.
fn resolve<D>(
    preferred: Option<&str>,
    find: impl FnOnce(&str) -> Option<D>,
    default: impl FnOnce() -> Option<D>,
    on_missing: impl FnOnce(),
) -> Result<D, CaptureError> {
    if let Some(id) = preferred {
        match find(id) {
            Some(device) => return Ok(device),
            None => on_missing(),
        }
    }
    default().ok_or(CaptureError::NoDevice)
}

fn open(
    frames: SyncSender<Vec<f32>>,
    on_error: ErrorSink,
    preferred: Option<String>,
    dropped: Arc<AtomicU64>,
) -> Result<(cpal::Stream, CaptureFormat), CaptureError> {
    let host = cpal::default_host();
    let device = resolve(
        preferred.as_deref(),
        |id| {
            id.parse::<DeviceId>()
                .ok()
                .and_then(|id| host.device_by_id(&id))
        },
        || host.default_input_device(),
        || on_error(MISSING_DEVICE.into()),
    )?;
    let config = device
        .default_input_config()
        .map_err(|e| CaptureError::Device(e.to_string()))?;
    let format = CaptureFormat {
        sample_rate: config.sample_rate(),
        channels: config.channels(),
    };

    let stream = match config.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config.into(), frames, on_error, dropped),
        SampleFormat::I16 => build::<i16>(&device, config.into(), frames, on_error, dropped),
        SampleFormat::I32 => build::<i32>(&device, config.into(), frames, on_error, dropped),
        SampleFormat::U16 => build::<u16>(&device, config.into(), frames, on_error, dropped),
        other => {
            return Err(CaptureError::Device(format!(
                "unsupported sample format {other}"
            )));
        }
    }?;
    stream
        .play()
        .map_err(|e| CaptureError::Device(e.to_string()))?;
    Ok((stream, format))
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    frames: SyncSender<Vec<f32>>,
    on_error: ErrorSink,
    dropped: Arc<AtomicU64>,
) -> Result<cpal::Stream, CaptureError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let buffer: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
                if let Err(TrySendError::Full(_)) = frames.try_send(buffer) {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
            },
            move |error: cpal::Error| {
                eprintln!("microphone stream error: {error}");
                // Overruns, automatic reroutes to a new default device, and a
                // refused real-time priority leave the stream running.
                use cpal::ErrorKind::{DeviceChanged, RealtimeDenied, Xrun};
                if !matches!(error.kind(), Xrun | DeviceChanged | RealtimeDenied) {
                    on_error(format!("the microphone stopped: {error}"));
                }
            },
            None,
        )
        .map_err(|e| match e.kind() {
            cpal::ErrorKind::PermissionDenied => CaptureError::MicPermissionDenied,
            _ => CaptureError::Device(e.to_string()),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn uses_the_preferred_device_when_connected() {
        let warned = Cell::new(false);

        let device = resolve(
            Some("usb"),
            |id| Some(id.to_owned()),
            || Some("default".into()),
            || warned.set(true),
        );

        assert_eq!(device.unwrap(), "usb");
        assert!(!warned.get());
    }

    #[test]
    fn falls_back_to_the_default_and_warns_when_the_preferred_device_is_missing() {
        let warned = Cell::new(false);

        let device = resolve(
            Some("usb"),
            |_| None::<String>,
            || Some("default".into()),
            || warned.set(true),
        );

        assert_eq!(device.unwrap(), "default");
        assert!(warned.get());
    }

    #[test]
    fn uses_the_default_without_a_warning_when_nothing_is_chosen() {
        let warned = Cell::new(false);

        let device = resolve(
            None,
            |_| Some("never".to_owned()),
            || Some("default".into()),
            || warned.set(true),
        );

        assert_eq!(device.unwrap(), "default");
        assert!(!warned.get());
    }

    #[test]
    fn reports_no_device_when_nothing_is_connected() {
        let result = resolve(None, |_| None::<String>, || None, || {});

        assert!(matches!(result, Err(CaptureError::NoDevice)));
    }
}
