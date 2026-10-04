//! Microphone capture through cpal, using the system's default input device.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use super::{Capture, CaptureError, CaptureFormat};

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

/// Starts capturing from the default microphone. Interleaved buffers go to
/// `frames`; when the pipeline falls behind, buffers are dropped rather than
/// blocking the audio thread.
pub fn start(frames: SyncSender<Vec<f32>>) -> Result<(MicCapture, CaptureFormat), CaptureError> {
    let (ready_tx, ready_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    let thread = thread::Builder::new()
        .name("mic-capture".into())
        .spawn(move || run(frames, ready_tx, stop_rx))
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

fn run(
    frames: SyncSender<Vec<f32>>,
    ready: mpsc::Sender<Result<CaptureFormat, CaptureError>>,
    stop: Receiver<()>,
) {
    let dropped = Arc::new(AtomicU64::new(0));
    let stream = match open(frames, dropped.clone()) {
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

fn open(
    frames: SyncSender<Vec<f32>>,
    dropped: Arc<AtomicU64>,
) -> Result<(cpal::Stream, CaptureFormat), CaptureError> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or(CaptureError::NoDevice)?;
    let config = device
        .default_input_config()
        .map_err(|e| CaptureError::Device(e.to_string()))?;
    let format = CaptureFormat {
        sample_rate: config.sample_rate(),
        channels: config.channels(),
    };

    let stream = match config.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config.into(), frames, dropped),
        SampleFormat::I16 => build::<i16>(&device, config.into(), frames, dropped),
        SampleFormat::I32 => build::<i32>(&device, config.into(), frames, dropped),
        SampleFormat::U16 => build::<u16>(&device, config.into(), frames, dropped),
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
            |error| eprintln!("microphone stream error: {error}"),
            None,
        )
        .map_err(|e| CaptureError::Device(e.to_string()))
}
