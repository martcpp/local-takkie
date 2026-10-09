//! cpal streams. The callbacks only convert, downmix and copy into lock-free
//! rings: no locks, allocation, logging or system calls (ROADMAP 5.4).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{
    BuildStreamError, FromSample, PlayStreamError, Sample, SampleFormat, SizedSample, Stream,
    SupportedStreamConfig,
};
use rtrb::{Consumer, Producer, RingBuffer};
use thiserror::Error;

use super::config::StreamSettings;
use super::io::AudioSource;

/// Counters the callbacks update, readable from any thread.
#[derive(Debug, Default)]
pub struct Counters {
    /// Mic samples dropped because the ring was full.
    pub dropped: AtomicU64,
    /// Stream errors reported by the device.
    pub errors: AtomicU64,
}

/// Why a stream couldn't start.
#[derive(Debug, Error)]
pub enum OpenError {
    /// cpal couldn't build it.
    #[error("couldn't open the audio stream: {0}")]
    Build(#[from] BuildStreamError),
    /// cpal couldn't start it.
    #[error("couldn't start the audio stream: {0}")]
    Play(#[from] PlayStreamError),
    /// A sample format we don't handle.
    #[error("the device uses {0} samples, which aren't supported")]
    Format(SampleFormat),
}

/// The input callback body: converts each frame to f32, averages it to mono
/// and pushes it, counting what doesn't fit.
pub(crate) fn capture<T>(data: &[T], channels: usize, ring: &mut Producer<f32>, dropped: &AtomicU64)
where
    T: Sample,
    f32: FromSample<T>,
{
    if channels == 0 {
        return;
    }
    let scale = 1.0 / channels as f32;
    let mut lost = 0;
    for frame in data.chunks_exact(channels) {
        let mono = frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() * scale;
        if ring.push(mono).is_err() {
            lost += 1;
        }
    }
    if lost > 0 {
        dropped.fetch_add(lost, Relaxed);
    }
}

/// A microphone through cpal.
pub struct CpalSource {
    _stream: Stream,
    ring: Consumer<f32>,
    settings: StreamSettings,
    counters: Arc<Counters>,
}

impl CpalSource {
    /// Starts capturing from `device` with `config`.
    ///
    /// # Errors
    /// [`OpenError`] if the stream can't be built or started.
    pub fn open(device: &cpal::Device, config: &SupportedStreamConfig) -> Result<Self, OpenError> {
        match config.sample_format() {
            SampleFormat::F32 => Self::open_as::<f32>(device, config),
            other => Err(OpenError::Format(other)),
        }
    }

    fn open_as<T>(device: &cpal::Device, config: &SupportedStreamConfig) -> Result<Self, OpenError>
    where
        T: SizedSample + Send + 'static,
        f32: FromSample<T>,
    {
        let settings = StreamSettings::from(config);
        let channels = usize::from(settings.channels);
        let (mut producer, consumer) = RingBuffer::new(settings.sample_rate as usize / 2);
        let counters = Arc::new(Counters::default());
        let on_data = Arc::clone(&counters);
        let on_error = Arc::clone(&counters);
        let stream = device.build_input_stream::<T, _, _>(
            &config.config(),
            move |data: &[T], _| capture(data, channels, &mut producer, &on_data.dropped),
            move |_| {
                on_error.errors.fetch_add(1, Relaxed);
            },
            None,
        )?;
        stream.play()?;
        Ok(Self {
            _stream: stream,
            ring: consumer,
            settings,
            counters,
        })
    }

    /// What the stream runs with.
    #[must_use]
    pub fn settings(&self) -> StreamSettings {
        self.settings
    }

    /// Dropped samples and errors so far.
    #[must_use]
    pub fn counters(&self) -> &Counters {
        &self.counters
    }
}

impl AudioSource for CpalSource {
    fn sample_rate(&self) -> u32 {
        self.settings.sample_rate
    }

    fn read(&mut self, out: &mut [f32]) -> usize {
        self.ring.pop_partial_slice(out).0.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(ring: &mut Consumer<f32>) -> Vec<f32> {
        std::iter::from_fn(|| ring.pop().ok()).collect()
    }

    #[test]
    fn stereo_f32_is_averaged_to_mono() {
        let (mut producer, mut consumer) = RingBuffer::new(8);
        let dropped = AtomicU64::new(0);
        capture(
            &[1.0_f32, 0.0, 0.5, -0.5, 0.25, 0.25],
            2,
            &mut producer,
            &dropped,
        );
        assert_eq!(drain(&mut consumer), [0.5, 0.0, 0.25]);
        assert_eq!(dropped.load(Relaxed), 0);
    }

    #[test]
    fn i16_is_converted_to_f32() {
        let (mut producer, mut consumer) = RingBuffer::new(8);
        let dropped = AtomicU64::new(0);
        capture(&[i16::MAX, 0, i16::MIN], 1, &mut producer, &dropped);
        let samples = drain(&mut consumer);
        assert!((samples[0] - 1.0).abs() < 1e-3);
        assert_eq!(samples[1], 0.0);
        assert_eq!(samples[2], -1.0);
    }

    #[test]
    fn u16_silence_is_zero() {
        let (mut producer, mut consumer) = RingBuffer::new(8);
        let dropped = AtomicU64::new(0);
        capture(&[32_768_u16, 32_768], 2, &mut producer, &dropped);
        assert_eq!(drain(&mut consumer), [0.0]);
    }

    #[test]
    fn a_full_ring_counts_dropped_samples() {
        let (mut producer, mut consumer) = RingBuffer::new(2);
        let dropped = AtomicU64::new(0);
        capture(&[0.1_f32; 5], 1, &mut producer, &dropped);
        assert_eq!(dropped.load(Relaxed), 3);
        assert_eq!(drain(&mut consumer).len(), 2);
        capture(&[0.1_f32; 2], 1, &mut producer, &dropped);
        assert_eq!(dropped.load(Relaxed), 3);
    }

    #[test]
    fn a_partial_frame_and_zero_channels_push_nothing() {
        let (mut producer, mut consumer) = RingBuffer::new(8);
        let dropped = AtomicU64::new(0);
        capture(&[0.5_f32, 0.5, 0.5], 2, &mut producer, &dropped);
        assert_eq!(drain(&mut consumer), [0.5]);
        capture(&[0.5_f32], 0, &mut producer, &dropped);
        assert!(drain(&mut consumer).is_empty());
    }
}
