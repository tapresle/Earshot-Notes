//! Runs a recording: capture, then downmix, resample, chunk, and hand speech
//! segments to the transcription worker.

use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use crate::audio::resample::{ResampleError, To16k};
use crate::audio::{Capture, CaptureError, CaptureFormat, downmix};
use crate::chunker::Chunker;
use crate::store::{Source, Store, StoreError};
use crate::transcribe::worker::{Job, TranscriptionWorker};

/// Interleaved buffers the pipeline may fall behind by before capture drops audio
/// (about 2.5 s at typical 10 ms device buffers).
const CAPTURE_BACKLOG: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("another meeting is already recording; stop it first")]
    AlreadyRecording(i64),
    #[error("transcription is unavailable: {0}")]
    TranscriptionUnavailable(String),
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Resample(#[from] ResampleError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The result of starting a capture: the running capture and its buffer layout.
pub type CaptureStart = Result<(Box<dyn Capture>, CaptureFormat), CaptureError>;

#[derive(Default)]
pub struct Recorder {
    active: Mutex<Option<Active>>,
}

struct Active {
    meeting_id: i64,
    capture: Box<dyn Capture>,
    pipeline: JoinHandle<()>,
}

impl Recorder {
    pub fn active_meeting(&self) -> Option<i64> {
        self.lock().as_ref().map(|a| a.meeting_id)
    }

    fn lock(&self) -> MutexGuard<'_, Option<Active>> {
        self.active.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Starts recording a meeting. The meeting is marked as recording only after
/// the capture is running, so a failed start leaves it unchanged.
pub fn start_recording<S>(
    store: &Store,
    recorder: &Recorder,
    worker: Result<&TranscriptionWorker, &str>,
    meeting_id: i64,
    start_capture: S,
) -> Result<(), RecordingError>
where
    S: FnOnce(SyncSender<Vec<f32>>) -> CaptureStart,
{
    let worker = worker
        .map_err(|reason| RecordingError::TranscriptionUnavailable(reason.to_owned()))?
        .clone();
    let mut active = recorder.lock();
    if let Some(current) = active.as_ref() {
        return Err(RecordingError::AlreadyRecording(current.meeting_id));
    }

    let offset_ms = store.recording_offset(meeting_id)?;
    let (frames_tx, frames) = sync_channel(CAPTURE_BACKLOG);
    let (capture, format) = start_capture(frames_tx)?;
    let resampler = To16k::new(format.sample_rate)?;
    store.begin_recording(meeting_id)?;

    let pipeline = thread::Builder::new()
        .name("audio-pipeline".into())
        .spawn(move || {
            run_pipeline(
                frames,
                format,
                resampler,
                Source::Me,
                meeting_id,
                offset_ms,
                |job| {
                    if worker.submit(job).is_err() {
                        eprintln!("dropped a segment for meeting {meeting_id}: the transcription worker stopped");
                    }
                },
            )
        })
        .map_err(|e| CaptureError::Device(e.to_string()))?;

    *active = Some(Active {
        meeting_id,
        capture,
        pipeline,
    });
    Ok(())
}

/// Stops the active recording, waits for its last segment to be queued for
/// transcription, and returns the meeting it belonged to.
pub fn stop_recording(store: &Store, recorder: &Recorder) -> Result<Option<i64>, RecordingError> {
    let Some(active) = recorder.lock().take() else {
        return Ok(None);
    };
    drop(active.capture);
    if active.pipeline.join().is_err() {
        eprintln!(
            "the audio pipeline for meeting {} panicked",
            active.meeting_id
        );
    }
    store.end_recording(active.meeting_id)?;
    Ok(Some(active.meeting_id))
}

/// Converts captured buffers into transcription jobs until the capture closes.
pub fn run_pipeline(
    frames: Receiver<Vec<f32>>,
    format: CaptureFormat,
    mut resampler: To16k,
    source: Source,
    meeting_id: i64,
    offset_ms: i64,
    mut submit: impl FnMut(Job),
) {
    let mut chunker = Chunker::new();
    let mut emit = |segment: crate::chunker::Segment| {
        submit(Job {
            meeting_id,
            source,
            start_ms: offset_ms + segment.start_ms() as i64,
            end_ms: offset_ms + segment.end_ms() as i64,
            samples: segment.samples,
        });
    };
    for buffer in frames {
        let mono = downmix(&buffer, format.channels as usize);
        for segment in chunker.push(&resampler.process(&mono)) {
            emit(segment);
        }
    }
    if let Some(segment) = chunker.finish() {
        emit(segment);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    struct FakeCapture;
    impl Capture for FakeCapture {}

    const RATE: u32 = 48_000;

    fn tone(ms: u32) -> Vec<f32> {
        let period = RATE as usize / 4;
        let gap = RATE as usize * 40 / 1000;
        (0..(RATE * ms / 1000) as usize)
            .map(|i| {
                if i % period >= period - gap {
                    0.0
                } else {
                    0.3 * (i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin()
                }
            })
            .collect()
    }

    fn silence(ms: u32) -> Vec<f32> {
        vec![0.0; (RATE * ms / 1000) as usize]
    }

    /// Interleaves mono audio into stereo 10 ms buffers, like a device would deliver.
    fn stereo_buffers(mono: &[f32]) -> Vec<Vec<f32>> {
        mono.chunks(RATE as usize / 100)
            .map(|chunk| chunk.iter().flat_map(|&s| [s, s]).collect())
            .collect()
    }

    /// A capture that delivers the given buffers from a background thread, then closes.
    fn fake_capture(buffers: Vec<Vec<f32>>) -> impl FnOnce(SyncSender<Vec<f32>>) -> CaptureStart {
        move |tx| {
            thread::spawn(move || {
                for buffer in buffers {
                    if tx.send(buffer).is_err() {
                        break;
                    }
                }
            });
            Ok((
                Box::new(FakeCapture),
                CaptureFormat {
                    sample_rate: RATE,
                    channels: 2,
                },
            ))
        }
    }

    #[test]
    fn turns_device_audio_into_timed_jobs() {
        let audio = [silence(1000), tone(1500), silence(1000)].concat();
        let (tx, rx) = sync_channel(1024);
        for buffer in stereo_buffers(&audio) {
            tx.send(buffer).unwrap();
        }
        drop(tx);
        let mut jobs = Vec::new();

        run_pipeline(
            rx,
            CaptureFormat {
                sample_rate: RATE,
                channels: 2,
            },
            To16k::new(RATE).unwrap(),
            Source::Me,
            7,
            60_000,
            |job| jobs.push(job),
        );

        assert_eq!(jobs.len(), 1);
        let job = &jobs[0];
        assert_eq!((job.meeting_id, job.source), (7, Source::Me));
        // Speech starts at 1000 ms; the job includes 200 ms of pre-roll.
        assert!(
            (60_780..=60_800).contains(&job.start_ms),
            "{}",
            job.start_ms
        );
        assert_eq!(job.samples.len() as i64, (job.end_ms - job.start_ms) * 16);
    }

    fn worker_unavailable() -> Result<&'static TranscriptionWorker, &'static str> {
        Err("no model")
    }

    #[test]
    fn refuses_to_start_without_transcription() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        let recorder = Recorder::default();

        let result = start_recording(
            &store,
            &recorder,
            worker_unavailable(),
            meeting.id,
            fake_capture(vec![]),
        );

        assert!(matches!(
            result,
            Err(RecordingError::TranscriptionUnavailable(_))
        ));
        assert_eq!(
            store.get_meeting(meeting.id).unwrap().meeting.started_at,
            None
        );
    }

    /// Submitted jobs land in the returned receiver instead of being transcribed.
    fn worker() -> (TranscriptionWorker, mpsc::Receiver<Job>) {
        TranscriptionWorker::from_queue()
    }

    #[test]
    fn records_flushes_the_last_segment_on_stop_and_marks_the_meeting() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        let recorder = Recorder::default();
        let (worker, jobs) = worker();
        // Ends mid-speech, so only stopping flushes the segment.
        let buffers = stereo_buffers(&[silence(500), tone(1000)].concat());

        start_recording(
            &store,
            &recorder,
            Ok(&worker),
            meeting.id,
            fake_capture(buffers),
        )
        .unwrap();
        assert_eq!(recorder.active_meeting(), Some(meeting.id));
        assert!(
            store
                .get_meeting(meeting.id)
                .unwrap()
                .meeting
                .started_at
                .is_some()
        );

        assert_eq!(stop_recording(&store, &recorder).unwrap(), Some(meeting.id));

        let job = jobs.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(job.meeting_id, meeting.id);
        assert_eq!(recorder.active_meeting(), None);
        assert!(
            store
                .get_meeting(meeting.id)
                .unwrap()
                .meeting
                .ended_at
                .is_some()
        );
    }

    #[test]
    fn allows_only_one_recording_at_a_time() {
        let store = Store::open_in_memory().unwrap();
        let first = store.create_meeting("First").unwrap();
        let second = store.create_meeting("Second").unwrap();
        let recorder = Recorder::default();
        let (worker, _jobs) = worker();
        start_recording(
            &store,
            &recorder,
            Ok(&worker),
            first.id,
            fake_capture(vec![]),
        )
        .unwrap();

        let result = start_recording(
            &store,
            &recorder,
            Ok(&worker),
            second.id,
            fake_capture(vec![]),
        );

        assert!(matches!(result, Err(RecordingError::AlreadyRecording(id)) if id == first.id));
        assert_eq!(
            store.get_meeting(second.id).unwrap().meeting.started_at,
            None
        );
        stop_recording(&store, &recorder).unwrap();
    }

    #[test]
    fn leaves_the_meeting_unchanged_when_capture_fails() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        let recorder = Recorder::default();
        let (worker, _jobs) = worker();

        let result = start_recording(&store, &recorder, Ok(&worker), meeting.id, |_| {
            Err(CaptureError::NoDevice)
        });

        assert!(matches!(
            result,
            Err(RecordingError::Capture(CaptureError::NoDevice))
        ));
        assert_eq!(recorder.active_meeting(), None);
        assert_eq!(
            store.get_meeting(meeting.id).unwrap().meeting.started_at,
            None
        );
    }

    #[test]
    fn stopping_with_nothing_active_is_a_no_op() {
        let store = Store::open_in_memory().unwrap();

        assert_eq!(stop_recording(&store, &Recorder::default()).unwrap(), None);
    }
}
