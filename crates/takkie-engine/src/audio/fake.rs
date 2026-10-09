//! Fake devices for tests. Time only moves when a test says so: a source
//! hands out samples after [`FakeSource::advance`], and a sink plays them
//! on [`FakeSink::play`].

use std::collections::VecDeque;
use std::f32::consts::TAU;

use super::io::{AudioSink, AudioSource};

/// What a [`FakeSource`] produces.
#[derive(Clone, Debug, PartialEq)]
pub enum Signal {
    /// Zeros.
    Silence,
    /// A sine wave.
    Sine {
        /// In hertz.
        frequency: f32,
        /// Peak level.
        amplitude: f32,
    },
    /// These samples, then nothing.
    Samples(Vec<f32>),
}

/// A microphone that produces a known signal.
#[derive(Clone, Debug)]
pub struct FakeSource {
    rate: u32,
    signal: Signal,
    position: usize,
    ready: usize,
}

impl FakeSource {
    /// A source with nothing ready yet.
    #[must_use]
    pub fn new(rate: u32, signal: Signal) -> Self {
        Self {
            rate,
            signal,
            position: 0,
            ready: 0,
        }
    }

    /// Makes `samples` more samples ready to read.
    pub fn advance(&mut self, samples: usize) {
        self.ready = self.ready.saturating_add(samples);
    }

    /// Makes `ms` milliseconds of audio ready to read.
    pub fn advance_ms(&mut self, ms: u32) {
        self.advance(self.rate as usize * ms as usize / 1000);
    }

    fn sample(&self, index: usize) -> Option<f32> {
        match &self.signal {
            Signal::Silence => Some(0.0),
            Signal::Sine {
                frequency,
                amplitude,
            } => {
                let cycles = index as f64 * f64::from(*frequency) / f64::from(self.rate);
                Some(amplitude * (TAU * cycles.fract() as f32).sin())
            }
            Signal::Samples(samples) => samples.get(index).copied(),
        }
    }
}

impl AudioSource for FakeSource {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn read(&mut self, out: &mut [f32]) -> usize {
        let mut count = 0;
        for slot in out.iter_mut().take(self.ready) {
            let Some(sample) = self.sample(self.position) else {
                break;
            };
            *slot = sample;
            self.position += 1;
            count += 1;
        }
        self.ready -= count;
        count
    }
}

/// A speaker that records what it plays.
#[derive(Clone, Debug)]
pub struct FakeSink {
    rate: u32,
    capacity: usize,
    queued: VecDeque<f32>,
    played: Vec<f32>,
    underruns: usize,
}

impl FakeSink {
    /// A sink whose buffer holds `capacity` samples.
    #[must_use]
    pub fn new(rate: u32, capacity: usize) -> Self {
        Self {
            rate,
            capacity,
            queued: VecDeque::with_capacity(capacity),
            played: Vec::new(),
            underruns: 0,
        }
    }

    /// Plays `samples` samples, as the device clock would. Silence fills in
    /// when the queue runs dry, and counts as an underrun.
    pub fn play(&mut self, samples: usize) {
        let mut short = false;
        for _ in 0..samples {
            let sample = self.queued.pop_front().unwrap_or_else(|| {
                short = true;
                0.0
            });
            self.played.push(sample);
        }
        if short {
            self.underruns += 1;
        }
    }

    /// Everything played so far.
    #[must_use]
    pub fn played(&self) -> &[f32] {
        &self.played
    }

    /// How many [`play`](Self::play) calls ran dry.
    #[must_use]
    pub fn underruns(&self) -> usize {
        self.underruns
    }
}

impl AudioSink for FakeSink {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn free(&self) -> usize {
        self.capacity.saturating_sub(self.queued.len())
    }

    fn queued(&self) -> usize {
        self.queued.len()
    }

    fn write(&mut self, samples: &[f32]) -> usize {
        let count = samples.len().min(self.free());
        self.queued.extend(samples.iter().take(count));
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count()
    }

    #[test]
    fn source_only_hands_out_what_is_ready() {
        let mut source = FakeSource::new(48_000, Signal::Silence);
        let mut out = [1.0; 100];
        assert_eq!(source.read(&mut out), 0);
        source.advance(30);
        assert_eq!(source.read(&mut out), 30);
        assert_eq!(source.read(&mut out), 0);
        source.advance_ms(1);
        assert_eq!(source.read(&mut out), 48);
    }

    #[test]
    fn sine_has_the_right_frequency_and_level() {
        let mut source = FakeSource::new(
            48_000,
            Signal::Sine {
                frequency: 1_000.0,
                amplitude: 0.5,
            },
        );
        source.advance_ms(1_000);
        let mut out = vec![0.0; 48_000];
        assert_eq!(source.read(&mut out), 48_000);
        assert!((1_995..=2_005).contains(&zero_crossings(&out)));
        let peak = out.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.01);
    }

    #[test]
    fn sample_list_runs_out() {
        let mut source = FakeSource::new(16_000, Signal::Samples(vec![0.1, 0.2, 0.3]));
        source.advance(10);
        let mut out = [0.0; 10];
        assert_eq!(source.read(&mut out), 3);
        assert_eq!(out[..3], [0.1, 0.2, 0.3]);
        assert_eq!(source.read(&mut out), 0);
        assert_eq!(source.sample_rate(), 16_000);
    }

    #[test]
    fn sink_queues_up_to_its_capacity() {
        let mut sink = FakeSink::new(48_000, 4);
        assert_eq!(sink.free(), 4);
        assert_eq!(sink.write(&[0.1, 0.2, 0.3]), 3);
        assert_eq!(sink.write(&[0.4, 0.5]), 1);
        assert_eq!(sink.free(), 0);
        assert_eq!(sink.queued(), 4);
    }

    #[test]
    fn sink_plays_in_order_and_counts_underruns() {
        let mut sink = FakeSink::new(44_100, 8);
        sink.write(&[0.1, 0.2]);
        sink.play(1);
        assert_eq!(sink.underruns(), 0);
        sink.play(3);
        assert_eq!(sink.played(), [0.1, 0.2, 0.0, 0.0]);
        assert_eq!(sink.underruns(), 1);
        assert_eq!(sink.sample_rate(), 44_100);
    }
}
