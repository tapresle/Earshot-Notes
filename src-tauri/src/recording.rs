//! Runs a recording: capture, then downmix, resample, chunk, and hand speech
//! segments to the transcription worker.

use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use serde::Serialize;

use crate::audio::resample::{ResampleError, To16k};
use crate::audio::{Capture, CaptureError, CaptureFormat, ErrorSink, downmix};
use crate::chunker::{Chunker, MIC_MIN_SPEECH_RMS, SYSTEM_MIN_SPEECH_RMS};
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
    #[error("couldn't record any audio: {}", describe(.0))]
    NoAudio(Vec<SourceProblem>),
    #[error("couldn't start the audio pipeline: {0}")]
    Thread(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// A stream that couldn't start, or that failed during the recording.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProblem {
    pub source: Source,
    pub message: String,
}

fn describe(problems: &[SourceProblem]) -> String {
    problems
        .iter()
        .map(|p| p.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

/// What started. Recording continues when at least one stream is running.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartReport {
    pub unavailable: Vec<SourceProblem>,
}

/// The result of starting a capture: the running capture and its buffer layout.
pub type CaptureStart = Result<(Box<dyn Capture>, CaptureFormat), CaptureError>;

/// Starts one capture stream, sending interleaved buffers to the sender and
/// runtime problems to the sink.
pub type CaptureStarter = Box<dyn FnOnce(SyncSender<Vec<f32>>, ErrorSink) -> CaptureStart + Send>;

/// Receives problems that happen while recording, such as a disconnected device.
pub type IssueSink = Arc<dyn Fn(i64, SourceProblem) + Send + Sync>;

#[derive(Default)]
pub struct Recorder {
    active: Mutex<Option<Active>>,
}

struct Active {
    meeting_id: i64,
    captures: Vec<Box<dyn Capture>>,
    pipelines: Vec<JoinHandle<()>>,
}

impl Recorder {
    pub fn active_meeting(&self) -> Option<i64> {
        self.lock().as_ref().map(|a| a.meeting_id)
    }

    fn lock(&self) -> MutexGuard<'_, Option<Active>> {
        self.active.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Starts every source it can and records the meeting if at least one works.
/// The meeting is marked as recording only after a capture is running, so a
/// failed start leaves it unchanged.
pub fn start_recording(
    store: &Store,
    recorder: &Recorder,
    worker: Result<&TranscriptionWorker, &str>,
    meeting_id: i64,
    sources: Vec<(Source, CaptureStarter)>,
    on_issue: IssueSink,
) -> Result<StartReport, RecordingError> {
    let worker = worker
        .map_err(|reason| RecordingError::TranscriptionUnavailable(reason.to_owned()))?
        .clone();
    let mut active = recorder.lock();
    if let Some(current) = active.as_ref() {
        return Err(RecordingError::AlreadyRecording(current.meeting_id));
    }
    let offset_ms = store.recording_offset(meeting_id)?;

    let mut started = Vec::new();
    let mut report = StartReport::default();
    for (source, start) in sources {
        let (frames_tx, frames) = sync_channel(CAPTURE_BACKLOG);
        let issues = on_issue.clone();
        let sink: ErrorSink =
            Arc::new(move |message| issues(meeting_id, SourceProblem { source, message }));
        let attempt = start(frames_tx, sink).and_then(|(capture, format)| {
            let resampler = To16k::new(format.sample_rate)
                .map_err(|e: ResampleError| CaptureError::Device(e.to_string()))?;
            Ok((capture, format, resampler))
        });
        match attempt {
            Ok((capture, format, resampler)) => {
                started.push((source, capture, format, resampler, frames))
            }
            Err(error) => report.unavailable.push(SourceProblem {
                source,
                message: error.to_string(),
            }),
        }
    }
    if started.is_empty() {
        return Err(RecordingError::NoAudio(report.unavailable));
    }
    store.begin_recording(meeting_id)?;

    let mut captures = Vec::new();
    let mut pipelines = Vec::new();
    for (source, capture, format, resampler, frames) in started {
        let worker = worker.clone();
        let pipeline = thread::Builder::new()
            .name(format!("audio-pipeline-{source:?}"))
            .spawn(move || {
                run_pipeline(frames, format, resampler, source, meeting_id, offset_ms, |job| {
                    if worker.submit(job).is_err() {
                        log::warn!(
                            "dropped a segment for meeting {meeting_id}: the transcription worker stopped"
                        );
                    }
                })
            });
        captures.push(capture);
        match pipeline {
            Ok(handle) => pipelines.push(handle),
            Err(error) => {
                drop(captures);
                for handle in pipelines {
                    let _ = handle.join();
                }
                let _ = store.end_recording(meeting_id);
                return Err(error.into());
            }
        }
    }

    *active = Some(Active {
        meeting_id,
        captures,
        pipelines,
    });
    Ok(report)
}

/// Stops the active recording, waits for its last segments to be queued for
/// transcription, and returns the meeting it belonged to.
pub fn stop_recording(store: &Store, recorder: &Recorder) -> Result<Option<i64>, RecordingError> {
    let Some(active) = recorder.lock().take() else {
        return Ok(None);
    };
    drop(active.captures);
    for pipeline in active.pipelines {
        if pipeline.join().is_err() {
            log::error!(
                "an audio pipeline for meeting {} panicked",
                active.meeting_id
            );
        }
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
    let mut chunker = Chunker::new(match source {
        Source::Me => MIC_MIN_SPEECH_RMS,
        Source::Them => SYSTEM_MIN_SPEECH_RMS,
    });
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
    fn fake_capture(buffers: Vec<Vec<f32>>) -> CaptureStarter {
        Box::new(move |tx, _on_error| {
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
        })
    }

    fn failing_capture(error: fn() -> CaptureError) -> CaptureStarter {
        Box::new(move |_tx, _on_error| Err(error()))
    }

    fn mic_only(starter: CaptureStarter) -> Vec<(Source, CaptureStarter)> {
        vec![(Source::Me, starter)]
    }

    fn ignore_issues() -> IssueSink {
        Arc::new(|_, _| {})
    }

    /// Runs mono audio through the pipeline as 48 kHz stereo and collects its jobs.
    fn pipeline_jobs(source: Source, audio: &[f32]) -> Vec<Job> {
        let (tx, rx) = sync_channel(1024);
        for buffer in stereo_buffers(audio) {
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
            source,
            7,
            60_000,
            |job| jobs.push(job),
        );
        jobs
    }

    #[test]
    fn turns_device_audio_into_timed_jobs() {
        let audio = [silence(1000), tone(1500), silence(1000)].concat();

        let jobs = pipeline_jobs(Source::Me, &audio);

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

    #[test]
    fn hears_quieter_speech_from_system_audio_than_from_the_microphone() {
        // About 0.005 RMS while speaking: a quiet call participant.
        let audio: Vec<f32> = [silence(1000), tone(1500), silence(1000)]
            .concat()
            .iter()
            .map(|s| s * 0.025)
            .collect();

        assert_eq!(pipeline_jobs(Source::Them, &audio).len(), 1);
        assert!(pipeline_jobs(Source::Me, &audio).is_empty());
    }

    fn worker_unavailable() -> Result<&'static TranscriptionWorker, &'static str> {
        Err("no model")
    }

    /// Submitted jobs land in the returned receiver instead of being transcribed.
    fn worker() -> (TranscriptionWorker, mpsc::Receiver<Job>) {
        TranscriptionWorker::from_queue()
    }

    fn started_at(store: &Store, id: i64) -> Option<i64> {
        store.get_meeting(id).unwrap().meeting.started_at
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
            mic_only(fake_capture(vec![])),
            ignore_issues(),
        );

        assert!(matches!(
            result,
            Err(RecordingError::TranscriptionUnavailable(_))
        ));
        assert_eq!(started_at(&store, meeting.id), None);
    }

    #[test]
    fn records_flushes_the_last_segment_on_stop_and_marks_the_meeting() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        let recorder = Recorder::default();
        let (worker, jobs) = worker();
        // Ends mid-speech, so only stopping flushes the segment.
        let buffers = stereo_buffers(&[silence(500), tone(1000)].concat());

        let report = start_recording(
            &store,
            &recorder,
            Ok(&worker),
            meeting.id,
            mic_only(fake_capture(buffers)),
            ignore_issues(),
        )
        .unwrap();
        assert!(report.unavailable.is_empty());
        assert_eq!(recorder.active_meeting(), Some(meeting.id));
        assert!(started_at(&store, meeting.id).is_some());

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
    fn transcribes_both_speakers_from_separate_streams() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Call").unwrap();
        let recorder = Recorder::default();
        let (worker, jobs) = worker();
        let me = stereo_buffers(&[silence(500), tone(1000), silence(1000)].concat());
        let them = stereo_buffers(&[silence(2000), tone(1000), silence(1000)].concat());

        start_recording(
            &store,
            &recorder,
            Ok(&worker),
            meeting.id,
            vec![
                (Source::Me, fake_capture(me)),
                (Source::Them, fake_capture(them)),
            ],
            ignore_issues(),
        )
        .unwrap();
        stop_recording(&store, &recorder).unwrap();

        let mut received: Vec<(Source, i64)> =
            jobs.try_iter().map(|j| (j.source, j.start_ms)).collect();
        received.sort_by_key(|(_, start)| *start);
        assert_eq!(received.len(), 2, "{received:?}");
        assert_eq!(received[0].0, Source::Me);
        assert_eq!(received[1].0, Source::Them);
        assert!((1780..=1800).contains(&received[1].1), "{received:?}");
    }

    #[test]
    fn keeps_recording_when_one_source_fails() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Call").unwrap();
        let recorder = Recorder::default();
        let (worker, _jobs) = worker();

        let report = start_recording(
            &store,
            &recorder,
            Ok(&worker),
            meeting.id,
            vec![
                (Source::Me, fake_capture(vec![])),
                (
                    Source::Them,
                    failing_capture(|| CaptureError::SystemAudio("tap failed".into())),
                ),
            ],
            ignore_issues(),
        )
        .unwrap();

        assert_eq!(
            report.unavailable,
            vec![SourceProblem {
                source: Source::Them,
                message: "system audio couldn't start: tap failed".into(),
            }]
        );
        assert_eq!(recorder.active_meeting(), Some(meeting.id));
        stop_recording(&store, &recorder).unwrap();
    }

    #[test]
    fn fails_and_leaves_the_meeting_unchanged_when_no_source_starts() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        let recorder = Recorder::default();
        let (worker, _jobs) = worker();

        let result = start_recording(
            &store,
            &recorder,
            Ok(&worker),
            meeting.id,
            vec![
                (
                    Source::Me,
                    failing_capture(|| CaptureError::MicPermissionDenied),
                ),
                (
                    Source::Them,
                    failing_capture(|| CaptureError::SystemAudio("tap failed".into())),
                ),
            ],
            ignore_issues(),
        );

        let Err(error @ RecordingError::NoAudio(_)) = result else {
            panic!("expected NoAudio, got {result:?}");
        };
        let message = error.to_string();
        assert!(
            message.contains("Privacy & Security > Microphone"),
            "{message}"
        );
        assert!(message.contains("tap failed"), "{message}");
        assert_eq!(recorder.active_meeting(), None);
        assert_eq!(started_at(&store, meeting.id), None);
    }

    #[test]
    fn reports_problems_during_recording_with_their_source() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Call").unwrap();
        let recorder = Recorder::default();
        let (worker, _jobs) = worker();
        let (issues_tx, issues) = mpsc::channel();
        let issues_tx = Mutex::new(issues_tx);
        let on_issue: IssueSink = Arc::new(move |meeting_id, problem| {
            issues_tx
                .lock()
                .unwrap()
                .send((meeting_id, problem))
                .unwrap();
        });
        let unplugged: CaptureStarter = Box::new(|_tx, on_error| {
            on_error("the microphone stopped: device unplugged".into());
            Ok((
                Box::new(FakeCapture) as Box<dyn Capture>,
                CaptureFormat {
                    sample_rate: RATE,
                    channels: 1,
                },
            ))
        });

        start_recording(
            &store,
            &recorder,
            Ok(&worker),
            meeting.id,
            mic_only(unplugged),
            on_issue,
        )
        .unwrap();

        let (meeting_id, problem) = issues.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(meeting_id, meeting.id);
        assert_eq!(problem.source, Source::Me);
        assert_eq!(problem.message, "the microphone stopped: device unplugged");
        stop_recording(&store, &recorder).unwrap();
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
            mic_only(fake_capture(vec![])),
            ignore_issues(),
        )
        .unwrap();

        let result = start_recording(
            &store,
            &recorder,
            Ok(&worker),
            second.id,
            mic_only(fake_capture(vec![])),
            ignore_issues(),
        );

        assert!(matches!(result, Err(RecordingError::AlreadyRecording(id)) if id == first.id));
        assert_eq!(started_at(&store, second.id), None);
        stop_recording(&store, &recorder).unwrap();
    }

    #[test]
    fn stopping_with_nothing_active_is_a_no_op() {
        let store = Store::open_in_memory().unwrap();

        assert_eq!(stop_recording(&store, &Recorder::default()).unwrap(), None);
    }
}
