//! Converts between a device's rate and 48 kHz, in 20 ms frames, both ways.
//! A device already at 48 kHz skips it.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler as _, ResamplerConstructionError};
use thiserror::Error;

use super::config::PREFERRED_RATE;

/// One 20 ms Opus frame at 48 kHz.
pub const FRAME: usize = 960;

/// Why resampling failed.
#[derive(Debug, Error)]
pub enum ResampleError {
    /// The resampler couldn't be built for this rate.
    #[error("can't resample from {from} Hz to {to} Hz: {source}")]
    Setup {
        /// Input rate.
        from: u32,
        /// Output rate.
        to: u32,
        /// Why.
        source: ResamplerConstructionError,
    },
    /// Input or output buffer had the wrong size.
    #[error("resampler got {got} input samples, needs {needed}")]
    Input {
        /// Samples given.
        got: usize,
        /// Samples needed.
        needed: usize,
    },
    /// The resampler itself failed.
    #[error("resampling failed: {0}")]
    Process(#[from] rubato::ResampleError),
}

/// Mono resampler between a device rate and 48 kHz.
pub struct Resampler {
    inner: Option<Fft<f32>>,
}

impl Resampler {
    /// Device rate in, one 48 kHz [`FRAME`] out per call.
    ///
    /// # Errors
    /// [`ResampleError::Setup`] if rubato can't handle the rate.
    pub fn capture(device_rate: u32) -> Result<Self, ResampleError> {
        Self::new(device_rate, PREFERRED_RATE, FixedSync::Output)
    }

    /// One 48 kHz [`FRAME`] in per call, device rate out.
    ///
    /// # Errors
    /// [`ResampleError::Setup`] if rubato can't handle the rate.
    pub fn playback(device_rate: u32) -> Result<Self, ResampleError> {
        Self::new(PREFERRED_RATE, device_rate, FixedSync::Input)
    }

    fn new(from: u32, to: u32, fixed: FixedSync) -> Result<Self, ResampleError> {
        if from == to {
            return Ok(Self { inner: None });
        }
        let inner = Fft::new(from as usize, to as usize, FRAME, 1, fixed)
            .map_err(|source| ResampleError::Setup { from, to, source })?;
        Ok(Self { inner: Some(inner) })
    }

    /// Input samples the next [`process`](Self::process) call needs.
    #[must_use]
    pub fn input_needed(&self) -> usize {
        self.inner
            .as_ref()
            .map_or(FRAME, |fft| fft.input_frames_next())
    }

    /// Most input one call can need.
    #[must_use]
    pub fn input_max(&self) -> usize {
        self.inner
            .as_ref()
            .map_or(FRAME, |fft| fft.input_frames_max())
    }

    /// Output room one call can need.
    #[must_use]
    pub fn output_max(&self) -> usize {
        self.inner
            .as_ref()
            .map_or(FRAME, |fft| fft.output_frames_max())
    }

    /// Converts exactly [`input_needed`](Self::input_needed) samples and
    /// returns how many it wrote to `output`.
    ///
    /// # Errors
    /// [`ResampleError`] if `input` is the wrong size, `output` is shorter
    /// than [`output_max`](Self::output_max), or rubato fails.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<usize, ResampleError> {
        let needed = self.input_needed();
        if input.len() != needed {
            return Err(ResampleError::Input {
                got: input.len(),
                needed,
            });
        }
        let Some(fft) = self.inner.as_mut() else {
            let copied = input.len().min(output.len());
            output[..copied].copy_from_slice(&input[..copied]);
            return Ok(copied);
        };
        let size = |_| ResampleError::Input {
            got: input.len(),
            needed,
        };
        let input = InterleavedSlice::new(input, 1, needed).map_err(size)?;
        let frames = output.len();
        let mut output = InterleavedSlice::new_mut(output, 1, frames).map_err(size)?;
        let (_, written) = fft.process_into_buffer(&input, &mut output, None)?;
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;

    fn sine(rate: u32, frequency: f32, seconds: f32) -> Vec<f32> {
        let count = (rate as f32 * seconds) as usize;
        (0..count)
            .map(|i| 0.5 * (TAU * frequency * i as f32 / rate as f32).sin())
            .collect()
    }

    fn run(mut resampler: Resampler, input: &[f32]) -> Vec<f32> {
        let mut output = vec![0.0; resampler.output_max()];
        let mut result = Vec::new();
        let mut at = 0;
        while at + resampler.input_needed() <= input.len() {
            let needed = resampler.input_needed();
            let written = resampler
                .process(&input[at..at + needed], &mut output)
                .unwrap();
            result.extend_from_slice(&output[..written]);
            at += needed;
        }
        result
    }

    fn frequency(samples: &[f32], rate: u32) -> f32 {
        let settled = &samples[rate as usize / 10..];
        let crossings = settled
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count();
        crossings as f32 / 2.0 / (settled.len() as f32 / rate as f32)
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() / expected < 0.01,
            "{actual} Hz is more than 1% from {expected} Hz"
        );
    }

    #[test]
    fn capture_44k1_to_48k_keeps_the_frequency() {
        let output = run(
            Resampler::capture(44_100).unwrap(),
            &sine(44_100, 1_000.0, 1.0),
        );
        assert!(output.len() > 40_000);
        assert_close(frequency(&output, 48_000), 1_000.0);
    }

    #[test]
    fn capture_16k_to_48k_keeps_the_frequency() {
        let output = run(
            Resampler::capture(16_000).unwrap(),
            &sine(16_000, 440.0, 1.0),
        );
        assert_close(frequency(&output, 48_000), 440.0);
    }

    #[test]
    fn capture_always_gives_whole_frames() {
        let mut resampler = Resampler::capture(44_100).unwrap();
        let input = sine(44_100, 300.0, 0.5);
        let mut output = vec![0.0; resampler.output_max()];
        let needed = resampler.input_needed();
        let written = resampler.process(&input[..needed], &mut output).unwrap();
        assert_eq!(written, FRAME);
    }

    #[test]
    fn capture_at_48k_is_a_straight_copy() {
        let input = sine(48_000, 500.0, 0.1);
        let resampler = Resampler::capture(48_000).unwrap();
        assert_eq!(resampler.input_needed(), FRAME);
        let output = run(resampler, &input);
        assert_eq!(output, input[..output.len()]);
    }

    #[test]
    fn playback_48k_to_44k1_keeps_the_frequency() {
        let output = run(
            Resampler::playback(44_100).unwrap(),
            &sine(48_000, 1_000.0, 1.0),
        );
        assert!(output.len() > 40_000);
        assert_close(frequency(&output, 44_100), 1_000.0);
    }

    #[test]
    fn playback_48k_to_16k_keeps_the_frequency() {
        let output = run(
            Resampler::playback(16_000).unwrap(),
            &sine(48_000, 440.0, 1.0),
        );
        assert_close(frequency(&output, 16_000), 440.0);
    }

    #[test]
    fn playback_takes_one_frame_per_call() {
        let resampler = Resampler::playback(44_100).unwrap();
        assert_eq!(resampler.input_needed(), FRAME);
        assert!(resampler.output_max() >= FRAME * 44_100 / 48_000);
    }

    #[test]
    fn playback_at_48k_is_a_straight_copy() {
        let input = sine(48_000, 500.0, 0.1);
        let output = run(Resampler::playback(48_000).unwrap(), &input);
        assert_eq!(output, input[..output.len()]);
    }

    #[test]
    fn wrong_input_size_is_an_error() {
        let mut resampler = Resampler::capture(44_100).unwrap();
        let mut output = vec![0.0; resampler.output_max()];
        assert!(matches!(
            resampler.process(&[0.0; 10], &mut output),
            Err(ResampleError::Input { got: 10, .. })
        ));
    }
}
