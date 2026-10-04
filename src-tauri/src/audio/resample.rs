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
    inner: Option<Inner>,
}

struct Inner {
    fft: Fft<f32>,
    pending: Vec<f32>,
    out: Vec<f32>,
    delay_left: usize,
}

impl To16k {
    pub fn new(input_rate: u32) -> Result<Self, ResampleError> {
        if input_rate == SAMPLE_RATE {
            return Ok(Self { inner: None });
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
            inner: Some(Inner {
                fft,
                pending: Vec::with_capacity(CHUNK_FRAMES * 2),
                out,
                delay_left,
            }),
        })
    }

    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        let Some(inner) = self.inner.as_mut() else {
            return samples.to_vec();
        };
        inner.pending.extend_from_slice(samples);

        let mut result = Vec::new();
        let mut consumed = 0;
        while inner.pending.len() - consumed >= inner.fft.input_frames_next() {
            let needed = inner.fft.input_frames_next();
            let input = &inner.pending[consumed..consumed + needed];
            let input = InterleavedSlice::new(input, 1, needed).expect("sized to frames");
            let out_len = inner.out.len();
            let mut output =
                InterleavedSlice::new_mut(&mut inner.out, 1, out_len).expect("sized to frames");
            let (used, written) = inner
                .fft
                .process_into_buffer(&input, &mut output, None)
                .expect("buffers are sized from the resampler's own limits");
            consumed += used;

            let skip = inner.delay_left.min(written);
            inner.delay_left -= skip;
            result.extend_from_slice(&inner.out[skip..written]);
        }
        inner.pending.drain(..consumed);
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
    fn handles_44_1k() {
        let mut resampler = To16k::new(44_100).unwrap();

        let output = resampler.process(&sine(44_100, 1000.0, 1.0));

        let crossings = zero_crossings(&output[2_000..10_000]);
        assert!((998..=1002).contains(&crossings), "{crossings}");
    }
}
