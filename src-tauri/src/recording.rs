//! Runs a recording: capture, then downmix, resample, chunk, and hand speech
//! segments to the transcription worker.

use std::ffi::CString;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use serde::Serialize;

use crate::audio::resample::{ResampleError, To16k};
use crate::audio::{Buffer, Capture, CaptureError, CaptureFormat, ErrorSink, downmix};
use crate::chunker::{Chunker, Sensitivity};
use crate::recordings::{KeptAudio, Recordings};
use crate::store::settings::{KEEP_AUDIO, ON, sensitivity_key};
use crate::store::{Source, Store, StoreError};
use crate::transcribe::worker::{Engine, Job, TranscriptionWorker};

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
pub type CaptureStarter = Box<dyn FnOnce(SyncSender<Buffer>, ErrorSink) -> CaptureStart + Send>;

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
/// `transcription` is the worker and the engine for this recording's segments,
/// or why transcription is unavailable.
/// The meeting is marked as recording only after a capture is running, so a
/// failed start leaves it unchanged.
pub fn start_recording(
    store: &Store,
    recorder: &Recorder,
    transcription: Result<(&TranscriptionWorker, Engine), &str>,
    meeting_id: i64,
    sources: Vec<(Source, CaptureStarter)>,
    on_issue: IssueSink,
    recordings: &Recordings,
) -> Result<StartReport, RecordingError> {
    let (worker, engine) = transcription
        .map(|(worker, engine)| (worker.clone(), engine))
        .map_err(|reason| RecordingError::TranscriptionUnavailable(reason.to_owned()))?;
    let mut active = recorder.lock();
    if let Some(current) = active.as_ref() {
        return Err(RecordingError::AlreadyRecording(current.meeting_id));
    }
    let offset_ms = store.recording_offset(meeting_id)?;
    let keep_audio = store.setting(KEEP_AUDIO)?.as_deref() == Some(ON);
    // The vocabulary forbids control characters, so the terms never contain a NUL.
    let keywords = store
        .vocabulary()?
        .keywords()
        .and_then(|terms| CString::new(terms).ok())
        .map(Arc::new);

    let mut started = Vec::new();
    let mut report = StartReport::default();
    for (source, start) in sources {
        let sensitivity = sensitivity(store, source)?;
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
                started.push((source, capture, format, resampler, frames, sensitivity))
            }
            Err(error) => {
                log::warn!("{source:?} audio couldn't start: {error}");
                report.unavailable.push(SourceProblem {
                    source,
                    message: error.to_string(),
                });
            }
        }
    }
    if started.is_empty() {
        return Err(RecordingError::NoAudio(report.unavailable));
    }
    store.begin_recording(meeting_id)?;
    worker.preload(&engine);

    let mut captures = Vec::new();
    let mut pipelines = Vec::new();
    for (source, capture, format, resampler, frames, sensitivity) in started {
        let config = Pipeline::new(source, meeting_id, offset_ms, sensitivity)
            .with_keywords(keywords.clone())
            .with_engine(engine.clone());
        log::info!(
            "{source:?} speech sensitivity is {sensitivity:?} (floor {} RMS)",
            config.min_speech_rms
        );
        let kept = keep_audio
            .then(|| {
                recordings
                    .keep(meeting_id, offset_ms, source)
                    .inspect_err(|error| {
                        log::error!("couldn't keep {source:?} audio: {error}");
                        on_issue(
                            meeting_id,
                            SourceProblem {
                                source,
                                message: format!(
                                    "this recording's audio couldn't be saved: {error}"
                                ),
                            },
                        );
                    })
                    .ok()
            })
            .flatten();
        let worker = worker.clone();
        let pipeline = thread::Builder::new()
            .name(format!("audio-pipeline-{source:?}"))
            .spawn(move || {
                run_pipeline(frames, format, resampler, config, kept, |job| {
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

/// A source's stored speech sensitivity.
fn sensitivity(store: &Store, source: Source) -> Result<Sensitivity, StoreError> {
    Ok(Sensitivity::from_setting(
        store.setting(sensitivity_key(source))?.as_deref(),
    ))
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

/// What one source's pipeline records into, and how it detects speech.
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub source: Source,
    pub meeting_id: i64,
    /// Where this recording starts in the meeting's timeline.
    pub offset_ms: i64,
    /// The absolute floor below which audio isn't speech.
    pub min_speech_rms: f32,
    /// Vocabulary terms that bias transcription, one per line.
    pub keywords: Option<Arc<CString>>,
    /// The model that transcribes this pipeline's segments.
    pub engine: Engine,
}

impl Pipeline {
    /// A pipeline that detects speech at `sensitivity`, using the floor for its source.
    pub fn new(source: Source, meeting_id: i64, offset_ms: i64, sensitivity: Sensitivity) -> Self {
        let min_speech_rms = match source {
            Source::Me => sensitivity.mic_floor(),
            Source::Them => sensitivity.system_floor(),
        };
        Pipeline {
            source,
            meeting_id,
            offset_ms,
            min_speech_rms,
            keywords: None,
            engine: Engine::Whistle,
        }
    }

    /// Biases this pipeline's transcription toward `keywords`.
    pub fn with_keywords(mut self, keywords: Option<Arc<CString>>) -> Self {
        self.keywords = keywords;
        self
    }

    /// Transcribes this pipeline's segments with `engine`.
    pub fn with_engine(mut self, engine: Engine) -> Self {
        self.engine = engine;
        self
    }
}

/// Converts captured buffers into transcription jobs until the capture closes,
/// handing the 16 kHz audio to `kept` first when the recording keeps its audio.
/// `resampler` starts at the capture's initial rate. When a buffer arrives at
/// another rate, the pipeline replaces it, first flushing the old one so later
/// segments keep their timing.
pub fn run_pipeline(
    frames: Receiver<Buffer>,
    format: CaptureFormat,
    resampler: To16k,
    pipeline: Pipeline,
    mut kept: Option<KeptAudio>,
    mut submit: impl FnMut(Job),
) {
    let Pipeline {
        source,
        meeting_id,
        offset_ms,
        min_speech_rms,
        keywords,
        engine,
    } = pipeline;
    let mut chunker = Chunker::new(min_speech_rms);
    let mut emit = |segment: crate::chunker::Segment| {
        submit(Job {
            meeting_id,
            source,
            start_ms: offset_ms + segment.start_ms() as i64,
            end_ms: offset_ms + segment.end_ms() as i64,
            samples: segment.samples,
            speech: segment.speech,
            keywords: keywords.clone(),
            engine: engine.clone(),
        });
    };
    let mut push = |samples: &[f32]| {
        if let Some(kept) = kept.as_mut() {
            kept.push(samples);
        }
        for segment in chunker.push(samples) {
            emit(segment);
        }
    };
    let mut rate = resampler.input_rate();
    // None while the capture runs at a rate that can't be resampled.
    let mut resampler = Some(resampler);
    for buffer in frames {
        if buffer.sample_rate != rate {
            log::info!(
                "{source:?} audio changed from {rate} Hz to {} Hz",
                buffer.sample_rate
            );
            if let Some(previous) = resampler.take() {
                push(&previous.finish());
            }
            resampler = To16k::new(buffer.sample_rate)
                .inspect_err(|e| {
                    log::error!("{source:?} audio is skipped until its rate changes: {e}")
                })
                .ok();
            rate = buffer.sample_rate;
        }
        let Some(resampler) = resampler.as_mut() else {
            continue;
        };
        let mono = downmix(&buffer.samples, format.channels as usize);
        push(&resampler.process(&mono));
    }
    if let Some(segment) = chunker.finish() {
        emit(segment);
    }
    if let Some(kept) = kept {
        kept.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::worker::Queue;
    use std::sync::mpsc;
    use std::time::Duration;

    struct FakeCapture;
    impl Capture for FakeCapture {}

    const RATE: u32 = 48_000;
    const PITCH_HZ: f32 = 440.0;

    /// A 440 Hz tone at `rate` with a 40 ms pause every 250 ms, like speech.
    fn tone_at(rate: u32, ms: u32) -> Vec<f32> {
        let period = rate as usize / 4;
        let gap = rate as usize * 40 / 1000;
        (0..(rate * ms / 1000) as usize)
            .map(|i| {
                if i % period >= period - gap {
                    0.0
                } else {
                    0.3 * (i as f32 * PITCH_HZ * std::f32::consts::TAU / rate as f32).sin()
                }
            })
            .collect()
    }

    fn tone(ms: u32) -> Vec<f32> {
        tone_at(RATE, ms)
    }

    fn silence_at(rate: u32, ms: u32) -> Vec<f32> {
        vec![0.0; (rate * ms / 1000) as usize]
    }

    fn silence(ms: u32) -> Vec<f32> {
        silence_at(RATE, ms)
    }

    /// Interleaves mono audio at `rate` into stereo 10 ms buffers, like a device would deliver.
    fn stereo_buffers_at(rate: u32, mono: &[f32]) -> Vec<Buffer> {
        mono.chunks(rate as usize / 100)
            .map(|chunk| Buffer {
                samples: chunk.iter().flat_map(|&s| [s, s]).collect(),
                sample_rate: rate,
            })
            .collect()
    }

    fn stereo_buffers(mono: &[f32]) -> Vec<Buffer> {
        stereo_buffers_at(RATE, mono)
    }

    /// The pitch of a job's audio over the 100 ms starting `from_ms` into the
    /// recording, from its zero crossings.
    fn pitch_hz(job: &Job, offset_ms: i64, from_ms: i64) -> f32 {
        let start = ((offset_ms + from_ms - job.start_ms) * 16) as usize;
        let window = &job.samples[start..start + 1600];
        let crossings = window
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        crossings as f32 * 10.0 / 2.0
    }

    /// A capture that delivers the given buffers from a background thread, then closes.
    fn fake_capture(buffers: Vec<Buffer>) -> CaptureStarter {
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

    /// Recordings for tests that never keep audio, so nothing is written there.
    fn no_recordings() -> Recordings {
        Recordings::new(std::path::PathBuf::from("/nonexistent/earshot-recordings"))
    }

    fn ignore_issues() -> IssueSink {
        Arc::new(|_, _| {})
    }

    /// Runs stereo buffers through a pipeline that starts at `rate` and collects its jobs.
    fn run_buffers(source: Source, rate: u32, buffers: Vec<Buffer>) -> Vec<Job> {
        run_buffers_at(source, Sensitivity::Normal, rate, buffers)
    }

    /// Like `run_buffers`, detecting speech at `sensitivity`.
    fn run_buffers_at(
        source: Source,
        sensitivity: Sensitivity,
        rate: u32,
        buffers: Vec<Buffer>,
    ) -> Vec<Job> {
        let (tx, rx) = sync_channel(buffers.len().max(1));
        for buffer in buffers {
            tx.send(buffer).unwrap();
        }
        drop(tx);
        let mut jobs = Vec::new();

        run_pipeline(
            rx,
            CaptureFormat {
                sample_rate: rate,
                channels: 2,
            },
            To16k::new(rate).unwrap(),
            Pipeline::new(source, 7, 60_000, sensitivity),
            None,
            |job| jobs.push(job),
        );
        jobs
    }

    /// Runs mono audio through the pipeline as 48 kHz stereo and collects its jobs.
    fn pipeline_jobs(source: Source, audio: &[f32]) -> Vec<Job> {
        run_buffers(source, RATE, stereo_buffers(audio))
    }

    #[test]
    fn passes_the_vocabulary_keywords_to_every_job() {
        let audio = [
            silence(1000),
            tone(1500),
            silence(1000),
            tone(1500),
            silence(1000),
        ]
        .concat();
        let buffers = stereo_buffers(&audio);
        let (tx, rx) = sync_channel(buffers.len());
        for buffer in buffers {
            tx.send(buffer).unwrap();
        }
        drop(tx);
        let keywords = Arc::new(CString::new("Bluefin Ledger\nCorvina").unwrap());
        let mut jobs = Vec::new();

        run_pipeline(
            rx,
            CaptureFormat {
                sample_rate: RATE,
                channels: 2,
            },
            To16k::new(RATE).unwrap(),
            Pipeline::new(Source::Them, 7, 0, Sensitivity::Normal)
                .with_keywords(Some(keywords.clone())),
            None,
            |job| jobs.push(job),
        );

        assert!(!jobs.is_empty());
        assert!(
            jobs.iter()
                .all(|job| job.keywords.as_ref() == Some(&keywords))
        );
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
        assert!(job.speech.ms > 1000, "{:?}", job.speech);
        assert!(job.speech.margin > 1.0, "{:?}", job.speech);
    }

    #[test]
    fn resamples_audio_at_the_rate_it_was_captured_at() {
        let rate = 24_000;
        let audio = [
            silence_at(rate, 1000),
            tone_at(rate, 1500),
            silence_at(rate, 1000),
        ]
        .concat();

        let jobs = run_buffers(Source::Them, rate, stereo_buffers_at(rate, &audio));

        assert_eq!(jobs.len(), 1);
        let job = &jobs[0];
        assert!(
            (60_780..=60_800).contains(&job.start_ms),
            "{}",
            job.start_ms
        );
        // Inside the tone's first 210 ms burst, which starts at 1000 ms.
        let pitch = pitch_hz(job, 60_000, 1050);
        assert!((pitch - PITCH_HZ).abs() <= 10.0, "{pitch}");
    }

    #[test]
    fn keeps_timing_and_pitch_when_the_capture_rate_changes() {
        // A Bluetooth headset drops from 48 kHz to 24 kHz partway through.
        let low = 24_000;
        let before = stereo_buffers(&[silence(1000), tone(1500), silence(1500)].concat());
        let after = stereo_buffers_at(
            low,
            &[
                silence_at(low, 1500),
                tone_at(low, 1500),
                silence_at(low, 1000),
            ]
            .concat(),
        );

        let jobs = run_buffers(Source::Them, RATE, [before, after].concat());

        assert_eq!(
            jobs.len(),
            2,
            "{:?}",
            jobs.iter().map(|j| j.start_ms).collect::<Vec<_>>()
        );
        // Speech starts at 1000 ms and 5500 ms; each job has 200 ms of pre-roll.
        assert!(
            (60_780..=60_800).contains(&jobs[0].start_ms),
            "{}",
            jobs[0].start_ms
        );
        // Audio lost or added at the switch would move the second job by a frame.
        assert_eq!(jobs[1].start_ms - jobs[0].start_ms, 4500);
        for (job, speech_ms) in jobs.iter().zip([1000, 5500]) {
            assert_eq!(job.samples.len() as i64, (job.end_ms - job.start_ms) * 16);
            let pitch = pitch_hz(job, 60_000, speech_ms + 50);
            assert!(
                (pitch - PITCH_HZ).abs() <= 10.0,
                "{pitch} at {speech_ms} ms"
            );
        }
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

    /// Speech at about `rms` while speaking, with a second of silence either side.
    fn speech_at_rms(rms: f32) -> Vec<f32> {
        // The tone's RMS while speaking is about 0.212 (0.3 / sqrt 2).
        let scale = rms / (0.3 / std::f32::consts::SQRT_2);
        [silence(1000), tone(1500), silence(1000)]
            .concat()
            .iter()
            .map(|s| s * scale)
            .collect()
    }

    fn jobs_at(source: Source, sensitivity: Sensitivity, audio: &[f32]) -> usize {
        run_buffers_at(source, sensitivity, RATE, stereo_buffers(audio)).len()
    }

    #[test]
    fn high_sensitivity_hears_quieter_microphone_speech() {
        let audio = speech_at_rms(0.008);
        assert_eq!(jobs_at(Source::Me, Sensitivity::High, &audio), 1);
        assert_eq!(jobs_at(Source::Me, Sensitivity::Normal, &audio), 0);
    }

    #[test]
    fn high_sensitivity_hears_quieter_system_audio() {
        let audio = speech_at_rms(0.0025);
        assert_eq!(jobs_at(Source::Them, Sensitivity::High, &audio), 1);
        assert_eq!(jobs_at(Source::Them, Sensitivity::Normal, &audio), 0);
    }

    #[test]
    fn low_sensitivity_ignores_quiet_microphone_audio() {
        let audio = speech_at_rms(0.012);
        assert_eq!(jobs_at(Source::Me, Sensitivity::Low, &audio), 0);
        assert_eq!(jobs_at(Source::Me, Sensitivity::Normal, &audio), 1);
    }

    fn worker_unavailable() -> Result<(&'static TranscriptionWorker, Engine), &'static str> {
        Err("no model")
    }

    /// Submitted jobs land in the returned receiver instead of being transcribed.
    fn worker() -> (TranscriptionWorker, Queue) {
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
            &no_recordings(),
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
            Ok((&worker, Engine::Whistle)),
            meeting.id,
            mic_only(fake_capture(buffers)),
            ignore_issues(),
            &no_recordings(),
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
    fn transcribes_the_recording_with_its_engine() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Sync").unwrap();
        let recorder = Recorder::default();
        let (worker, jobs) = worker();
        let buffers = stereo_buffers(&[silence(500), tone(1000)].concat());
        let whisper = Engine::Whisper(Arc::new(std::path::PathBuf::from("models/ggml-model.bin")));

        start_recording(
            &store,
            &recorder,
            Ok((&worker, whisper.clone())),
            meeting.id,
            mic_only(fake_capture(buffers)),
            ignore_issues(),
            &no_recordings(),
        )
        .unwrap();
        stop_recording(&store, &recorder).unwrap();

        assert_eq!(
            jobs.preload(Duration::from_secs(5)).map(Engine::Whisper),
            Some(whisper.clone())
        );
        let job = jobs.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(job.engine, whisper);
    }

    #[test]
    fn the_next_recording_uses_the_stored_sensitivity() {
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Quiet").unwrap();
        let recorder = Recorder::default();
        let (worker, jobs) = worker();
        let record = |audio: &[f32]| {
            start_recording(
                &store,
                &recorder,
                Ok((&worker, Engine::Whistle)),
                meeting.id,
                mic_only(fake_capture(stereo_buffers(audio))),
                ignore_issues(),
                &no_recordings(),
            )
            .unwrap();
            stop_recording(&store, &recorder).unwrap();
        };
        let quiet = speech_at_rms(0.008);

        record(&quiet);
        assert!(jobs.recv_timeout(Duration::from_millis(500)).is_err());

        store
            .set_setting(
                sensitivity_key(Source::Me),
                Some(Sensitivity::High.as_setting()),
            )
            .unwrap();
        record(&quiet);
        assert_eq!(
            jobs.recv_timeout(Duration::from_secs(5)).unwrap().source,
            Source::Me
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
            Ok((&worker, Engine::Whistle)),
            meeting.id,
            vec![
                (Source::Me, fake_capture(me)),
                (Source::Them, fake_capture(them)),
            ],
            ignore_issues(),
            &no_recordings(),
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
            Ok((&worker, Engine::Whistle)),
            meeting.id,
            vec![
                (Source::Me, fake_capture(vec![])),
                (
                    Source::Them,
                    failing_capture(|| CaptureError::SystemAudio("tap failed".into())),
                ),
            ],
            ignore_issues(),
            &no_recordings(),
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
            Ok((&worker, Engine::Whistle)),
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
            &no_recordings(),
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
            Ok((&worker, Engine::Whistle)),
            meeting.id,
            mic_only(unplugged),
            on_issue,
            &no_recordings(),
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
            Ok((&worker, Engine::Whistle)),
            first.id,
            mic_only(fake_capture(vec![])),
            ignore_issues(),
            &no_recordings(),
        )
        .unwrap();

        let result = start_recording(
            &store,
            &recorder,
            Ok((&worker, Engine::Whistle)),
            second.id,
            mic_only(fake_capture(vec![])),
            ignore_issues(),
            &no_recordings(),
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

    /// The resampler holds back about 20 ms that only a rate change flushes, so a
    /// kept file can be that much shorter than the audio sent.
    const KEPT_TOLERANCE: usize = 40 * 16;

    fn read_wav(path: &std::path::Path) -> (hound::WavSpec, Vec<i16>) {
        let mut reader = hound::WavReader::open(path).unwrap();
        let samples = reader.samples::<i16>().map(Result::unwrap).collect();
        (reader.spec(), samples)
    }

    /// Records the meeting from both sources with the given audio, keeping its audio.
    fn record_both(
        store: &Store,
        recordings: &Recordings,
        meeting_id: i64,
        me: &[f32],
        them: &[f32],
    ) {
        let recorder = Recorder::default();
        let (worker, _jobs) = worker();
        start_recording(
            store,
            &recorder,
            Ok((&worker, Engine::Whistle)),
            meeting_id,
            vec![
                (Source::Me, fake_capture(stereo_buffers(me))),
                (Source::Them, fake_capture(stereo_buffers(them))),
            ],
            ignore_issues(),
            recordings,
        )
        .unwrap();
        // Stopping waits for the pipelines, which end once the fake captures have
        // sent every buffer, and each pipeline completes its file before it ends.
        stop_recording(store, &recorder).unwrap();
    }

    #[test]
    fn keeps_each_source_s_audio_including_silence_when_the_setting_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = Recordings::new(dir.path().join("recordings"));
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Kept").unwrap();
        store.set_setting(KEEP_AUDIO, Some(ON)).unwrap();
        // Silence only: speech detection drops all of it, but the file keeps it.
        let quiet = silence(2000);

        record_both(&store, &recordings, meeting.id, &quiet, &tone(1000));

        for (source, ms) in [(Source::Me, 2000), (Source::Them, 1000)] {
            let (spec, samples) = read_wav(&recordings.file_path(meeting.id, 0, source));
            assert_eq!((spec.channels, spec.sample_rate), (1, 16_000));
            let expected = ms * 16;
            assert!(
                samples.len().abs_diff(expected) <= KEPT_TOLERANCE,
                "{source:?}: {} samples, expected about {expected}",
                samples.len()
            );
            if source == Source::Me {
                assert!(samples.iter().all(|&s| s == 0));
            }
        }
    }

    #[test]
    fn keeps_no_audio_when_the_setting_is_off_or_missing() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = Recordings::new(dir.path().join("recordings"));
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Not kept").unwrap();

        record_both(&store, &recordings, meeting.id, &tone(500), &tone(500));
        store.set_setting(KEEP_AUDIO, Some("off")).unwrap();
        record_both(&store, &recordings, meeting.id, &tone(500), &tone(500));

        assert!(!recordings.root().exists());
    }

    #[test]
    fn a_second_recording_keeps_a_new_pair_named_by_its_offset() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = Recordings::new(dir.path().join("recordings"));
        let store = Store::open_in_memory().unwrap();
        let meeting = store.create_meeting("Twice").unwrap();
        store.set_setting(KEEP_AUDIO, Some(ON)).unwrap();

        record_both(&store, &recordings, meeting.id, &tone(500), &tone(500));
        thread::sleep(Duration::from_millis(20));
        record_both(&store, &recordings, meeting.id, &tone(300), &tone(300));

        let mut files: Vec<String> =
            std::fs::read_dir(recordings.root().join(meeting.id.to_string()))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
        files.sort();
        assert_eq!(files.len(), 4, "{files:?}");
        assert_eq!(&files[..2], ["0-me.wav", "0-them.wav"]);
        let offset = files[2].strip_suffix("-me.wav").unwrap();
        assert!(offset.parse::<i64>().unwrap() > 0, "{files:?}");
        assert_eq!(files[3], format!("{offset}-them.wav"));
        // The first pair is untouched.
        let (_, first) = read_wav(&recordings.file_path(meeting.id, 0, Source::Me));
        assert!(first.len().abs_diff(500 * 16) <= KEPT_TOLERANCE);
    }

    #[test]
    fn a_blocked_writer_never_delays_transcription_jobs() {
        let (writer, release, written) = crate::recordings::testing::blocked_writer();
        let kept = KeptAudio::spawn(writer).unwrap();
        // Seventeen and a half seconds of speech bursts, more than the writer's backlog holds.
        let audio: Vec<f32> = (0..5)
            .flat_map(|_| [tone(1000), silence(2500)].concat())
            .collect();
        let buffers = stereo_buffers(&audio);
        let buffer_count = buffers.len();
        let (frames_tx, frames) = sync_channel(buffer_count);
        for buffer in buffers {
            frames_tx.send(buffer).unwrap();
        }
        drop(frames_tx);
        let (jobs_tx, jobs) = mpsc::channel();

        let pipeline = thread::spawn(move || {
            run_pipeline(
                frames,
                CaptureFormat {
                    sample_rate: RATE,
                    channels: 2,
                },
                To16k::new(RATE).unwrap(),
                Pipeline::new(Source::Me, 1, 0, Sensitivity::Normal),
                Some(kept),
                |job| jobs_tx.send(job).unwrap(),
            )
        });

        // Every burst becomes a job while the writer is still stuck on its first write.
        for _ in 0..5 {
            jobs.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        assert!(written.lock().unwrap().is_empty());
        release.send(()).unwrap();
        pipeline.join().unwrap();
        // The audio that didn't fit while blocked is kept as silence, so the length still matches.
        let written = written.lock().unwrap().len();
        assert!(written.abs_diff(17_500 * 16) <= KEPT_TOLERANCE, "{written}");
    }
}
