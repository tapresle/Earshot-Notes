//! Runs the real needle engine against synthetic audio. Requires scripts/fetch-native.sh.

use std::path::Path;
use std::sync::Mutex;

use earshot_lib::transcribe::whistle::{MAX_SAMPLES, SAMPLE_RATE, Whistle, WhistleError};

const MODEL: &str = "resources/models/whistle.cact";

// The engine allows one speech model per process, so tests take turns.
static ENGINE: Mutex<()> = Mutex::new(());

fn speech_fixture() -> Vec<f32> {
    read_fixture("tests/fixtures/speech-en.wav")
}

fn read_fixture(path: &str) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).unwrap();
    assert_eq!(reader.spec().sample_rate, SAMPLE_RATE);
    assert_eq!(reader.spec().channels, 1);
    reader
        .samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect()
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect()
}

#[test]
fn transcribes_english_speech() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();

    let result = whistle.transcribe(&speech_fixture()).unwrap();

    assert_eq!(
        normalize(&result.text),
        "the quarterly planning meeting starts at 10 please bring the budget report"
    );
    assert_eq!(result.language.as_deref(), Some("en"));
}

#[test]
fn transcribes_as_english_without_detecting_the_language() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();

    let result = whistle
        .transcribe(&read_fixture("tests/fixtures/speech-es.wav"))
        .unwrap();

    assert_eq!(result.language.as_deref(), Some("en"));
}

#[test]
fn returns_empty_text_for_silence() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();

    let result = whistle
        .transcribe(&vec![0.0; SAMPLE_RATE as usize * 5])
        .unwrap();

    assert_eq!(result.text, "");
    assert_eq!(result.language, None);
}

#[test]
fn accepts_a_full_thirty_second_clip() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();
    let clip: Vec<f32> = speech_fixture()
        .into_iter()
        .cycle()
        .take(MAX_SAMPLES)
        .collect();

    let result = whistle.transcribe(&clip).unwrap();

    assert!(normalize(&result.text).contains("budget report"));
}

#[test]
fn rejects_clips_over_thirty_seconds() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();

    let result = whistle.transcribe(&vec![0.0; MAX_SAMPLES + 1]);

    assert!(matches!(
        result,
        Err(WhistleError::ClipTooLong { samples }) if samples == MAX_SAMPLES + 1
    ));
}

#[test]
fn allows_only_one_instance_at_a_time() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
    let first = Whistle::load(Path::new(MODEL)).unwrap();

    assert!(matches!(
        Whistle::load(Path::new(MODEL)),
        Err(WhistleError::AlreadyLoaded)
    ));

    drop(first);
    assert!(Whistle::load(Path::new(MODEL)).is_ok());
}

#[test]
fn reports_a_missing_model_file() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());

    assert!(matches!(
        Whistle::load(Path::new("resources/models/missing.cact")),
        Err(WhistleError::Io(_))
    ));
    // A failed load releases the slot.
    assert!(Whistle::load(Path::new(MODEL)).is_ok());
}

#[test]
fn rejects_a_file_that_is_not_a_model() {
    let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());

    assert!(matches!(
        Whistle::load(Path::new("tests/fixtures/speech-en.wav")),
        Err(WhistleError::Load(_))
    ));
}

mod worker {
    use super::*;
    use earshot_lib::chunker::Speech;
    use earshot_lib::store::Source;
    use earshot_lib::transcribe::worker::{Job, TranscriptionWorker, load_first};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn prefers_the_first_candidate_and_falls_back_past_bad_ones() {
        let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
        let candidates = vec![
            PathBuf::from("resources/models/missing.cact"),
            PathBuf::from("tests/fixtures/speech-en.wav"), // exists but isn't a model
            PathBuf::from(MODEL),
        ];

        let (_whistle, path) = load_first(&candidates).unwrap();

        assert_eq!(path, PathBuf::from(MODEL));
    }

    #[test]
    fn explains_when_no_candidate_exists() {
        let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());

        let error = load_first(&[PathBuf::from("nowhere/whistle.cact")])
            .err()
            .unwrap();

        assert!(error.to_string().contains("nowhere/whistle.cact"));
    }

    #[test]
    fn transcribes_jobs_in_order_and_skips_silence() {
        let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
        let (results_tx, results) = mpsc::channel();
        let (worker, loaded) =
            TranscriptionWorker::spawn(vec![PathBuf::from(MODEL)], move |job, t| {
                results_tx.send((job.start_ms, t.text)).unwrap();
            })
            .unwrap();
        assert_eq!(loaded.path, PathBuf::from(MODEL));
        let job = |start_ms, samples| Job {
            meeting_id: 1,
            source: Source::Me,
            start_ms,
            end_ms: start_ms + 5000,
            samples,
            speech: Speech {
                ms: 5000,
                margin: 10.0,
            },
        };

        worker.submit(job(0, vec![0.0; 16_000])).unwrap();
        worker.submit(job(5000, speech_fixture())).unwrap();
        drop(worker); // closes the queue; the thread drains it, then releases the model

        let (start_ms, text) = results.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(start_ms, 5000);
        assert!(normalize(&text).contains("quarterly planning meeting"));
        assert!(results.recv_timeout(Duration::from_secs(2)).is_err());
    }
}

mod pipeline {
    use super::*;
    use earshot_lib::audio::resample::To16k;
    use earshot_lib::audio::{Buffer, CaptureFormat};
    use earshot_lib::recording::run_pipeline;
    use earshot_lib::store::Source;
    use std::sync::mpsc::sync_channel;

    /// Linear interpolation to stereo at a device rate above 16 kHz.
    fn as_stereo_at(rate: u32, samples_16k: &[f32]) -> Vec<f32> {
        let frames = (samples_16k.len() - 1) * rate as usize / SAMPLE_RATE as usize;
        let mut out = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let t = i as f64 * SAMPLE_RATE as f64 / rate as f64;
            let (index, fraction) = (t as usize, t.fract() as f32);
            let s = samples_16k[index] + (samples_16k[index + 1] - samples_16k[index]) * fraction;
            out.extend([s, s]);
        }
        out
    }

    fn as_48k_stereo(samples_16k: &[f32]) -> Vec<f32> {
        as_stereo_at(48_000, samples_16k)
    }

    /// Splits stereo audio into 10 ms device buffers at `rate`.
    fn buffers_at(rate: u32, stereo: &[f32]) -> Vec<Buffer> {
        stereo
            .chunks(rate as usize / 100 * 2)
            .map(|chunk| Buffer {
                samples: chunk.to_vec(),
                sample_rate: rate,
            })
            .collect()
    }

    #[test]
    fn transcribes_speech_captured_at_48k_stereo() {
        let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
        let mut audio = vec![0.0; SAMPLE_RATE as usize];
        audio.extend(speech_fixture());
        audio.extend(vec![0.0; SAMPLE_RATE as usize]);
        let device_audio = as_48k_stereo(&audio);
        let (tx, rx) = sync_channel(4096);
        for buffer in buffers_at(48_000, &device_audio) {
            tx.send(buffer).unwrap();
        }
        drop(tx);
        let mut jobs = Vec::new();

        run_pipeline(
            rx,
            CaptureFormat {
                sample_rate: 48_000,
                channels: 2,
            },
            To16k::new(48_000).unwrap(),
            Source::Me,
            1,
            0,
            |job| jobs.push(job),
        );

        let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();
        let text: Vec<String> = jobs
            .iter()
            .map(|job| whistle.transcribe(&job.samples).unwrap().text)
            .collect();
        assert_eq!(
            normalize(&text.join(" ")),
            "the quarterly planning meeting starts at 10 please bring the budget report"
        );
        assert!(
            (700..=900).contains(&jobs[0].start_ms),
            "{}",
            jobs[0].start_ms
        );
    }

    #[test]
    fn transcribes_speech_after_the_device_switches_to_a_lower_rate() {
        // A Bluetooth headset drops from 48 kHz to 24 kHz when its microphone opens.
        let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
        let mut audio = vec![0.0; SAMPLE_RATE as usize];
        audio.extend(speech_fixture());
        audio.extend(vec![0.0; SAMPLE_RATE as usize]);
        let (tx, rx) = sync_channel(8192);
        for rate in [48_000, 24_000] {
            for buffer in buffers_at(rate, &as_stereo_at(rate, &audio)) {
                tx.send(buffer).unwrap();
            }
        }
        drop(tx);
        let mut jobs = Vec::new();

        run_pipeline(
            rx,
            CaptureFormat {
                sample_rate: 48_000,
                channels: 2,
            },
            To16k::new(48_000).unwrap(),
            Source::Them,
            1,
            0,
            |job| jobs.push(job),
        );

        let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();
        let expected = "the quarterly planning meeting starts at 10 please bring the budget report";
        let segments: Vec<(i64, String)> = jobs
            .iter()
            .map(|job| {
                let text = whistle.transcribe(&job.samples).unwrap().text;
                (job.start_ms, normalize(&text))
            })
            .collect();
        assert_eq!(segments.len(), 2, "{segments:#?}");
        for (_, text) in &segments {
            assert_eq!(text, expected, "{segments:#?}");
        }
        // The second copy starts exactly one copy's length after the first.
        let copy_ms = (audio.len() - 1) as i64 * 1000 / SAMPLE_RATE as i64;
        let gap = segments[1].0 - segments[0].0;
        assert!((gap - copy_ms).abs() <= 20, "{gap} vs {copy_ms}");
    }

    #[test]
    fn keeps_transcribing_across_multiple_chunks_past_thirty_seconds() {
        let _lock = ENGINE.lock().unwrap_or_else(|e| e.into_inner());
        let sentence = speech_fixture();
        let pause = |ms: usize| vec![0.0; SAMPLE_RATE as usize * ms / 1000];
        let mut audio = pause(1000);
        // Three sentences with clear pauses: one segment each.
        for _ in 0..3 {
            audio.extend(&sentence);
            audio.extend(pause(1000));
        }
        // Then about 35 s of near-continuous speech, longer than one Whistle pass.
        for _ in 0..8 {
            audio.extend(&sentence);
            audio.extend(pause(100));
        }
        audio.extend(pause(1000));
        let total_ms = audio.len() as i64 * 1000 / SAMPLE_RATE as i64;
        assert!(total_ms >= 45_000, "{total_ms}");

        let (tx, rx) = sync_channel(16_384);
        for buffer in buffers_at(48_000, &as_48k_stereo(&audio)) {
            tx.send(buffer).unwrap();
        }
        drop(tx);
        let mut jobs = Vec::new();
        run_pipeline(
            rx,
            CaptureFormat {
                sample_rate: 48_000,
                channels: 2,
            },
            To16k::new(48_000).unwrap(),
            Source::Me,
            1,
            0,
            |job| jobs.push(job),
        );

        let mut whistle = Whistle::load(Path::new(MODEL)).unwrap();
        let segments: Vec<(i64, i64, String)> = jobs
            .iter()
            .map(|job| {
                let text = whistle.transcribe(&job.samples).unwrap().text;
                (job.start_ms, job.end_ms, normalize(&text))
            })
            .collect();
        let summary = format!("{segments:#?}");

        // Several chunks, each within the engine's limit, in timeline order.
        assert!(segments.len() >= 5, "{summary}");
        for (start, end, _) in &segments {
            assert!(end - start <= 28_000, "{summary}");
        }
        assert!(segments.windows(2).all(|w| w[0].1 <= w[1].0), "{summary}");

        // Every sentence was transcribed, including those after the 30 s mark.
        let all_text = segments
            .iter()
            .map(|(_, _, text)| text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            all_text.matches("quarterly planning meeting").count(),
            11,
            "{summary}"
        );
        assert_eq!(all_text.matches("budget report").count(), 11, "{summary}");
        let late: Vec<_> = segments
            .iter()
            .filter(|(start, _, _)| *start >= 30_000)
            .collect();
        assert!(!late.is_empty(), "{summary}");
        assert!(
            late.iter().all(|(_, _, text)| !text.is_empty()),
            "{summary}"
        );

        // The last segment ends where the audio's speech ends, so timestamps didn't drift.
        let last_end = segments.last().unwrap().1;
        assert!(
            (total_ms - 1200..=total_ms).contains(&last_end),
            "{last_end} vs {total_ms}"
        );
    }
}
