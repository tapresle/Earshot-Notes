//! Streaming conversion of mono audio from a device rate to 16 kHz.

use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::transcribe::whistle::SAMPLE_RATE;

const CHUNK_FRAMES: usize = 1024;

#[derive(Debug, thiserror::Error)]
#[error("couldn't set up resampling from {from} Hz: {message}")]
pub struct ResampleError {
    from: u32,
    message: String,
}

/// Accepts mono samples in pieces of any size and returns 16 kHz samples as
/// they become available. The resampler's startup delay is trimmed, so output
/// sample N lines up with input time N / 16 kHz.
pub struct To16k {
    input_rate: u32,
    inner: Option<Inner>,
}

struct Inner {
    fft: Fft<f32>,
    pending: Vec<f32>,
    out: Vec<f32>,
    delay_left: usize,
    received: u64,
    emitted: u64,
}

impl Inner {
    /// Resamples every whole chunk in `pending` and appends the output to `result`.
    fn drain(&mut self, result: &mut Vec<f32>) {
        let mut consumed = 0;
        while self.pending.len() - consumed >= self.fft.input_frames_next() {
            let needed = self.fft.input_frames_next();
            let input = &self.pending[consumed..consumed + needed];
            let input = InterleavedSlice::new(input, 1, needed).expect("sized to frames");
            let out_len = self.out.len();
            let mut output =
                InterleavedSlice::new_mut(&mut self.out, 1, out_len).expect("sized to frames");
            let (used, written) = self
                .fft
                .process_into_buffer(&input, &mut output, None)
                .expect("buffers are sized from the resampler's own limits");
            consumed += used;

            let skip = self.delay_left.min(written);
            self.delay_left -= skip;
            result.extend_from_slice(&self.out[skip..written]);
            self.emitted += (written - skip) as u64;
        }
        self.pending.drain(..consumed);
    }
}

impl To16k {
    pub fn new(input_rate: u32) -> Result<Self, ResampleError> {
        if input_rate == SAMPLE_RATE {
            return Ok(Self {
                input_rate,
                inner: None,
            });
        }
        let fft = Fft::<f32>::new(
            input_rate as usize,
            SAMPLE_RATE as usize,
            CHUNK_FRAMES,
            1,
            FixedSync::Input,
        )
        .map_err(|e| ResampleError {
            from: input_rate,
            message: e.to_string(),
        })?;
        let out = vec![0.0; fft.output_frames_max()];
        let delay_left = fft.output_delay();
        Ok(Self {
            input_rate,
            inner: Some(Inner {
                fft,
                pending: Vec::with_capacity(CHUNK_FRAMES * 2),
                out,
                delay_left,
                received: 0,
                emitted: 0,
            }),
        })
    }

    pub fn input_rate(&self) -> u32 {
        self.input_rate
    }

    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        let Some(inner) = self.inner.as_mut() else {
            return samples.to_vec();
        };
        inner.received += samples.len() as u64;
        inner.pending.extend_from_slice(samples);

        let mut result = Vec::new();
        inner.drain(&mut result);
        result
    }

    /// Returns the output still held back, so that everything this resampler
    /// returned lasts as long as everything it received, to the nearest 16 kHz
    /// sample. Call it before replacing the resampler, so later audio keeps
    /// its timing.
    pub fn finish(self) -> Vec<f32> {
        let Some(mut inner) = self.inner else {
            return Vec::new();
        };
        let rate = u64::from(self.input_rate);
        let total = (inner.received * u64::from(SAMPLE_RATE) + rate / 2) / rate;
        let mut result = Vec::new();
        // Silence pushes the held-back input, and the resampler's delay, through.
        while inner.emitted < total {
            let needed = inner.fft.input_frames_next();
            inner.pending.resize(needed, 0.0);
            inner.drain(&mut result);
        }
        let excess = (inner.emitted - total) as usize;
        result.truncate(result.len().saturating_sub(excess));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f32, seconds: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize)
            .map(|i| (i as f32 * freq * std::f32::consts::TAU / rate as f32).sin())
            .collect()
    }

    fn zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    }

    #[test]
    fn passes_16k_through_unchanged() {
        let input = sine(16_000, 440.0, 0.1);

        assert_eq!(To16k::new(16_000).unwrap().process(&input), input);
    }

    #[test]
    fn downsamples_48k_and_preserves_pitch() {
        let mut resampler = To16k::new(48_000).unwrap();
        let input = sine(48_000, 440.0, 2.0);

        let output: Vec<f32> = input
            .chunks(480)
            .flat_map(|chunk| resampler.process(chunk))
            .collect();

        // Up to one chunk stays buffered, plus the trimmed delay.
        let expected = 32_000;
        assert!(
            output.len() <= expected && output.len() > expected - 1500,
            "{}",
            output.len()
        );
        // 440 Hz has 880 zero crossings per second; check the middle second.
        let crossings = zero_crossings(&output[8_000..24_000]);
        assert!((878..=882).contains(&crossings), "{crossings}");
    }

    #[test]
    fn finishing_makes_the_output_as_long_as_the_input() {
        for (rate, frames) in [(48_000, 48_123), (44_100, 22_051), (24_000, 777)] {
            let mut resampler = To16k::new(rate).unwrap();
            let input = sine(rate, 440.0, frames as f32 / rate as f32);

            let mut output: Vec<f32> = input
                .chunks(rate as usize / 100)
                .flat_map(|chunk| resampler.process(chunk))
                .collect();
            output.extend(resampler.finish());

            let expected = (input.len() as f64 * 16_000.0 / rate as f64).round() as usize;
            assert_eq!(output.len(), expected, "{rate} Hz");
        }
    }

    #[test]
    fn finishing_a_passthrough_returns_nothing() {
        let mut resampler = To16k::new(16_000).unwrap();
        resampler.process(&[0.5; 100]);

        assert!(resampler.finish().is_empty());
    }

    #[test]
    fn handles_44_1k() {
        let mut resampler = To16k::new(44_100).unwrap();

        let output = resampler.process(&sine(44_100, 1000.0, 1.0));

        let crossings = zero_crossings(&output[2_000..10_000]);
        assert!((998..=1002).contains(&crossings), "{crossings}");
    }
}
