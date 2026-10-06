//! Splits a 16 kHz mono stream into speech segments that fit Whistle's 30 s limit.
//!
//! An energy-based voice activity detector marks 20 ms frames as speech or
//! silence. A segment starts at the first speech frame (with a short pre-roll so
//! the first syllable isn't clipped) and ends after a pause, earlier on a
//! shorter pause once the segment is long, or at the hard length cap.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::transcribe::whistle::SAMPLE_RATE;

const FRAME: usize = SAMPLE_RATE as usize / 50; // 20 ms
const PREROLL_FRAMES: usize = 10; // 200 ms
const END_SILENCE_FRAMES: usize = 25; // 500 ms
const LONG_SEGMENT_FRAMES: usize = 20 * 50; // 20 s
const LONG_SEGMENT_SILENCE_FRAMES: usize = 8; // 160 ms
const MAX_SEGMENT_FRAMES: usize = 28 * 50; // 28 s, under Whistle's 30 s limit
const MIN_SPEECH_FRAMES: usize = 5; // 100 ms; shorter bursts are clicks or bumps

/// Absolute floor for the microphone, so a near-silent room or the
/// microphone's own hiss isn't treated as speech.
pub const MIC_MIN_SPEECH_RMS: f32 = 0.01;
/// Absolute floor for system audio (about -50 dBFS). Call audio is quieter than
/// a close microphone and arrives noise-suppressed, so the microphone floor drops
/// quiet speakers. Silence there is digital silence, so a lower floor is safe.
pub const SYSTEM_MIN_SPEECH_RMS: f32 = 0.003;
/// Speech must be this many times louder than the tracked background noise.
const NOISE_MULTIPLIER: f32 = 3.0;

/// How readily quiet audio counts as speech. Each level sets the absolute
/// floor; `Normal` uses `MIC_MIN_SPEECH_RMS` and `SYSTEM_MIN_SPEECH_RMS`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Sensitivity {
    /// Ignores quieter audio, for a noisy room.
    Low,
    #[default]
    Normal,
    /// Hears quieter speech, at the risk of transcribing background noise.
    High,
}

impl Sensitivity {
    /// Reads a stored level. A missing or unknown value is `Normal`.
    pub fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("low") => Sensitivity::Low,
            Some("high") => Sensitivity::High,
            _ => Sensitivity::Normal,
        }
    }

    /// The value stored for this level.
    pub fn as_setting(self) -> &'static str {
        match self {
            Sensitivity::Low => "low",
            Sensitivity::Normal => "normal",
            Sensitivity::High => "high",
        }
    }

    /// The microphone's absolute speech floor at this level.
    pub fn mic_floor(self) -> f32 {
        match self {
            Sensitivity::Low => 0.014,
            Sensitivity::Normal => MIC_MIN_SPEECH_RMS,
            Sensitivity::High => 0.007,
        }
    }

    /// System audio's absolute speech floor at this level.
    pub fn system_floor(self) -> f32 {
        match self {
            Sensitivity::Low => 0.0045,
            Sensitivity::Normal => SYSTEM_MIN_SPEECH_RMS,
            Sensitivity::High => 0.002,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Offset of the first sample from the start of the stream.
    pub start_sample: u64,
    pub samples: Vec<f32>,
    pub speech: Speech,
}

/// How much speech a segment holds. A segment can be mostly pre-roll and
/// trailing silence, which Whistle may still turn into words.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speech {
    /// Time spent in speech frames.
    pub ms: u64,
    /// How many times louder than the speech threshold the speech frames were,
    /// on average. Always above 1.
    pub margin: f32,
}

impl Segment {
    pub fn start_ms(&self) -> u64 {
        self.start_sample * 1000 / SAMPLE_RATE as u64
    }

    pub fn end_ms(&self) -> u64 {
        (self.start_sample + self.samples.len() as u64) * 1000 / SAMPLE_RATE as u64
    }
}

pub struct Chunker {
    partial: Vec<f32>,
    preroll: VecDeque<Vec<f32>>,
    current: Option<Current>,
    frames_seen: u64,
    /// Tracked background level; seeded from the first frame.
    noise_floor: Option<f32>,
    /// Frames at or below this level are never speech.
    min_speech_rms: f32,
}

struct Current {
    start_sample: u64,
    samples: Vec<f32>,
    frames: usize,
    speech_frames: usize,
    /// Sum of each speech frame's margin over its threshold.
    margin_sum: f32,
    silence_run: usize,
}

impl Chunker {
    pub fn new(min_speech_rms: f32) -> Self {
        Self {
            partial: Vec::with_capacity(FRAME),
            preroll: VecDeque::with_capacity(PREROLL_FRAMES),
            current: None,
            frames_seen: 0,
            noise_floor: None,
            min_speech_rms,
        }
    }

    /// Feeds samples of any length and returns the segments they complete.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Segment> {
        let mut done = Vec::new();
        let mut rest = samples;
        while !rest.is_empty() {
            let take = (FRAME - self.partial.len()).min(rest.len());
            self.partial.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.partial.len() == FRAME {
                let frame = std::mem::replace(&mut self.partial, Vec::with_capacity(FRAME));
                if let Some(segment) = self.process_frame(frame) {
                    done.push(segment);
                }
            }
        }
        done
    }

    /// Ends the stream and returns the segment in progress, if it holds speech.
    pub fn finish(mut self) -> Option<Segment> {
        if let Some(current) = self.current.as_mut() {
            current.samples.append(&mut self.partial);
        }
        self.current.take().and_then(into_segment)
    }

    fn process_frame(&mut self, frame: Vec<f32>) -> Option<Segment> {
        let frame_start = self.frames_seen * FRAME as u64;
        self.frames_seen += 1;
        let margin = self.speech_margin(&frame);

        let Some(current) = self.current.as_mut() else {
            if let Some(margin) = margin {
                let preroll_frames = self.preroll.len();
                let mut samples = Vec::with_capacity(FRAME * 64);
                for f in self.preroll.drain(..) {
                    samples.extend_from_slice(&f);
                }
                samples.extend_from_slice(&frame);
                self.current = Some(Current {
                    start_sample: frame_start - (preroll_frames * FRAME) as u64,
                    samples,
                    frames: preroll_frames + 1,
                    speech_frames: 1,
                    margin_sum: margin,
                    silence_run: 0,
                });
            } else {
                if self.preroll.len() == PREROLL_FRAMES {
                    self.preroll.pop_front();
                }
                self.preroll.push_back(frame);
            }
            return None;
        };

        current.samples.extend_from_slice(&frame);
        current.frames += 1;
        if let Some(margin) = margin {
            current.speech_frames += 1;
            current.margin_sum += margin;
            current.silence_run = 0;
        } else {
            current.silence_run += 1;
        }

        let pause_ends_it = current.silence_run >= END_SILENCE_FRAMES
            || (current.frames >= LONG_SEGMENT_FRAMES
                && current.silence_run >= LONG_SEGMENT_SILENCE_FRAMES);
        if pause_ends_it || current.frames >= MAX_SEGMENT_FRAMES {
            return self.current.take().and_then(into_segment);
        }
        None
    }

    /// Returns how many times louder than the speech threshold the frame is, or
    /// `None` when it isn't speech.
    fn speech_margin(&mut self, frame: &[f32]) -> Option<f32> {
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt();
        let floor = *self.noise_floor.get_or_insert(rms);
        let threshold = self.min_speech_rms.max(floor * NOISE_MULTIPLIER);
        let speech = rms > threshold;
        // Follow the background down quickly and up slowly. During speech the
        // floor still creeps up, so noise that starts mid-stream (a fan, an air
        // conditioner) is absorbed in about 15 s. Pauses between words pull it
        // back down, so real speech isn't affected.
        let rate = match (speech, rms < floor) {
            (_, true) => 0.5,
            (false, false) => 0.02,
            (true, false) => 0.0005,
        };
        self.noise_floor = Some(floor + (rms - floor) * rate);
        speech.then_some(rms / threshold)
    }
}

fn into_segment(current: Current) -> Option<Segment> {
    (current.speech_frames >= MIN_SPEECH_FRAMES).then(|| Segment {
        start_sample: current.start_sample,
        samples: current.samples,
        speech: Speech {
            ms: (current.speech_frames * FRAME) as u64 * 1000 / SAMPLE_RATE as u64,
            margin: current.margin_sum / current.speech_frames as f32,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: usize = SAMPLE_RATE as usize;

    fn silence(ms: usize) -> Vec<f32> {
        vec![0.0; RATE * ms / 1000]
    }

    /// A speech-like signal: 210 ms "syllables" of a 440 Hz tone with 40 ms gaps.
    fn tone(ms: usize) -> Vec<f32> {
        let period = RATE / 4;
        let gap = RATE * 40 / 1000;
        (0..RATE * ms / 1000)
            .map(|i| {
                if i % period >= period - gap {
                    0.0
                } else {
                    0.3 * (i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin()
                }
            })
            .collect()
    }

    fn concat(parts: &[Vec<f32>]) -> Vec<f32> {
        parts.concat()
    }

    /// Scales audio down to a quiet call participant: about 0.005 RMS while
    /// speaking, between the system and microphone floors.
    fn quiet(audio: &[f32]) -> Vec<f32> {
        audio.iter().map(|s| s * 0.025).collect()
    }

    fn run(samples: &[f32], push_size: usize) -> Vec<Segment> {
        run_with(MIC_MIN_SPEECH_RMS, samples, push_size)
    }

    fn run_with(min_speech_rms: f32, samples: &[f32], push_size: usize) -> Vec<Segment> {
        let mut chunker = Chunker::new(min_speech_rms);
        let mut segments: Vec<Segment> = samples
            .chunks(push_size)
            .flat_map(|chunk| chunker.push(chunk))
            .collect();
        segments.extend(chunker.finish());
        segments
    }

    #[test]
    fn ignores_silence() {
        assert!(run(&silence(5000), 512).is_empty());
    }

    #[test]
    fn emits_one_segment_for_one_utterance_with_preroll() {
        let audio = concat(&[silence(1000), tone(1500), silence(1000)]);

        let segments = run(&audio, 512);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_ms(), 800); // 200 ms of pre-roll
        assert!(segments[0].end_ms() >= 2500);
    }

    #[test]
    fn measures_the_speech_in_a_segment() {
        let audio = concat(&[silence(1000), tone(1000), silence(1000)]);

        let speech = run(&audio, 512)[0].speech;

        // 40 ms of every 250 ms of tone is a gap, so about 840 ms is speech.
        assert!((780..=1000).contains(&speech.ms), "{speech:?}");
        // The tone is about 0.21 RMS against the 0.01 microphone floor.
        assert!(speech.margin > 15.0, "{speech:?}");
    }

    #[test]
    fn measures_a_short_burst_as_little_speech() {
        let audio = concat(&[silence(1000), tone(120), silence(1000)]);

        let speech = run(&audio, 512)[0].speech;

        assert!(speech.ms <= 120, "{speech:?}");
    }

    #[test]
    fn measures_a_quiet_speaker_with_a_low_margin() {
        let audio = concat(&[silence(1000), quiet(&tone(1000)), silence(1000)]);

        let speech = run_with(SYSTEM_MIN_SPEECH_RMS, &audio, 512)[0].speech;

        assert!(speech.margin > 1.0 && speech.margin < 2.5, "{speech:?}");
    }

    #[test]
    fn splits_utterances_separated_by_a_pause() {
        let audio = concat(&[tone(1000), silence(1000), tone(1000), silence(1000)]);

        let segments = run(&audio, 512);

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[1].start_ms(), 1800);
    }

    #[test]
    fn keeps_short_pauses_inside_one_segment() {
        let audio = concat(&[tone(1000), silence(200), tone(1000), silence(1000)]);

        assert_eq!(run(&audio, 512).len(), 1);
    }

    #[test]
    fn caps_continuous_speech_under_the_whistle_limit() {
        let audio = concat(&[silence(500), tone(70_000)]);

        let segments = run(&audio, 4096);

        assert_eq!(segments.len(), 3);
        for segment in &segments {
            assert!(segment.samples.len() <= MAX_SEGMENT_FRAMES * FRAME);
        }
        // Everything from the pre-roll before the first syllable onward is covered, without gaps.
        assert_eq!(segments[0].start_ms(), 300);
        for pair in segments.windows(2) {
            assert_eq!(pair[0].end_ms(), pair[1].start_ms());
        }
        assert_eq!(segments[2].end_ms(), 70_500);
    }

    #[test]
    fn picks_up_speech_that_starts_in_the_first_frame_within_two_syllables() {
        // The noise floor is seeded from the first frame, so speech that starts
        // at the very first frame is detected only once pauses pull the floor
        // down. Here detection begins at the third syllable (500 ms).
        let audio = concat(&[tone(1000), silence(1000)]);

        let segments = run(&audio, 512);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_ms(), 300);
    }

    #[test]
    fn cuts_long_segments_on_a_shorter_pause() {
        let audio = concat(&[tone(21_000), silence(200), tone(2000), silence(1000)]);

        let segments = run(&audio, 512);

        assert_eq!(segments.len(), 2);
        assert!(segments[0].end_ms() < 21_500);
    }

    #[test]
    fn drops_clicks_shorter_than_the_minimum() {
        let audio = concat(&[silence(500), tone(60), silence(1000)]);

        assert!(run(&audio, 512).is_empty());
    }

    #[test]
    fn flushes_the_segment_in_progress_on_finish() {
        let audio = concat(&[silence(500), tone(1000)]);

        let segments = run(&audio, 333);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].end_ms(), 1500);
    }

    #[test]
    fn produces_the_same_segments_regardless_of_push_size() {
        let audio = concat(&[
            silence(700),
            tone(1200),
            silence(900),
            tone(800),
            silence(600),
        ]);

        assert_eq!(run(&audio, 1), run(&audio, 7919));
    }

    #[test]
    fn picks_up_quiet_speech_at_the_system_audio_floor() {
        let audio = quiet(&concat(&[silence(1000), tone(1500), silence(1000)]));

        let segments = run_with(SYSTEM_MIN_SPEECH_RMS, &audio, 512);

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_ms(), 800);
    }

    #[test]
    fn ignores_quiet_speech_at_the_microphone_floor() {
        let audio = quiet(&concat(&[silence(1000), tone(1500), silence(1000)]));

        assert!(run_with(MIC_MIN_SPEECH_RMS, &audio, 512).is_empty());
    }

    #[test]
    fn treats_steady_background_noise_as_silence() {
        // Deterministic pseudo-random noise around RMS 0.02, above the absolute floor.
        let mut state = 1u32;
        let noise: Vec<f32> = (0..RATE * 3)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 8) as f32 / (1u32 << 24) as f32 * 0.07 - 0.035
            })
            .collect();
        let mut audio = noise.clone();
        audio.extend(tone(1000).iter().zip(&noise).map(|(t, n)| t + n));
        audio.extend(&noise);

        let segments = run(&audio, 512);

        assert_eq!(segments.len(), 1);
        assert!(segments[0].start_ms() >= 2500);
    }

    #[test]
    fn reads_stored_sensitivity_and_defaults_to_normal() {
        for level in [Sensitivity::Low, Sensitivity::Normal, Sensitivity::High] {
            assert_eq!(Sensitivity::from_setting(Some(level.as_setting())), level);
        }
        assert_eq!(Sensitivity::from_setting(None), Sensitivity::Normal);
        assert_eq!(Sensitivity::from_setting(Some("loud")), Sensitivity::Normal);
        assert_eq!(Sensitivity::Normal.mic_floor(), MIC_MIN_SPEECH_RMS);
        assert_eq!(Sensitivity::Normal.system_floor(), SYSTEM_MIN_SPEECH_RMS);
    }
}
