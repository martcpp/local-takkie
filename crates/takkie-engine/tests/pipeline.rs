//! The whole audio chain with fake devices, in simulated time: mic, framing,
//! push-to-talk, Opus, a lossy in-memory network, jitter buffer, decoding,
//! mixing, pacing, speaker.

use std::time::{Duration, Instant};

use takkie_core::{PeerId, Seq};
use takkie_engine::audio::AudioSink;
use takkie_engine::audio::codec::VoiceEncoder;
use takkie_engine::audio::fake::{FakeSink, FakeSource, Signal};
use takkie_engine::audio::mix::{Mixer, Pacer, RxPacket};
use takkie_engine::audio::tx::{Framer, Gate};

const TONE: f32 = 440.0;
const AMPLITUDE: f32 = 0.4;

struct Run {
    played: Vec<f32>,
    rate: u32,
    most_queued: usize,
    most_buffered: usize,
    dropped: usize,
}

fn run(mic_rate: u32, speaker_rate: u32, seconds: u32, loss_percent: u32) -> Run {
    let mut source = FakeSource::new(
        mic_rate,
        Signal::Sine {
            frequency: TONE,
            amplitude: AMPLITUDE,
        },
    );
    let mut framer = Framer::new(mic_rate).unwrap();
    let mut gate = Gate::default();
    let mut encoder = VoiceEncoder::new().unwrap();
    let mut mixer = Mixer::new();
    let mut pacer = Pacer::new(speaker_rate).unwrap();
    let mut sink = FakeSink::new(speaker_rate, speaker_rate as usize / 5);

    let t0 = Instant::now();
    let mut seq = 0_u32;
    let mut seed = 99_u32;
    let mut dropped = 0;
    let (mut most_queued, mut most_buffered) = (0, 0);
    for tick in 0..seconds * 50 {
        let now = t0 + Duration::from_millis(20 * u64::from(tick));
        source.advance_ms(20);
        while let Some(frame) = framer.next(&mut source).unwrap() {
            let Some(mark) = gate.pass(true) else {
                continue;
            };
            let payload = encoder.encode(frame).unwrap().to_vec();
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let lose = tick > 10 && (seed >> 16) % 100 < loss_percent;
            if lose {
                dropped += 1;
            } else {
                let packet = RxPacket {
                    sender: PeerId::new(1),
                    seq: Seq::new(seq),
                    payload,
                    end: mark.end,
                };
                mixer.receive(packet, now).unwrap();
            }
            seq += 1;
        }
        pacer.fill(&mut mixer, &mut sink, now).unwrap();
        sink.play(speaker_rate as usize / 50);
        if tick > 25 {
            most_queued = most_queued.max(sink.queued());
            most_buffered = most_buffered.max(mixer.buffered());
        }
    }
    Run {
        played: sink.played().to_vec(),
        rate: speaker_rate,
        most_queued,
        most_buffered,
        dropped,
    }
}

impl Run {
    fn settled(&self) -> &[f32] {
        &self.played[self.rate as usize / 2..]
    }

    fn frequency(&self) -> f32 {
        let samples = self.settled();
        let crossings = samples
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count();
        crossings as f32 / 2.0 / (samples.len() as f32 / self.rate as f32)
    }

    fn rms(&self) -> f32 {
        let samples = self.settled();
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn silent_frames(&self) -> usize {
        self.settled()
            .chunks(self.rate as usize / 50)
            .filter(|frame| frame.iter().all(|s| s.abs() < 0.01))
            .count()
    }

    fn check(&self) {
        let frequency = self.frequency();
        assert!(
            (frequency - TONE).abs() / TONE < 0.02,
            "tone came out at {frequency} Hz"
        );
        let expected = AMPLITUDE / 2.0_f32.sqrt();
        let rms = self.rms();
        assert!(
            (rms - expected).abs() / expected < 0.3,
            "level came out at {rms}, sent {expected}"
        );
        assert!(
            self.most_queued <= self.rate as usize * 60 / 1_000,
            "speaker queue reached {}",
            self.most_queued
        );
        assert!(
            self.most_buffered <= 10,
            "jitter buffer reached {}",
            self.most_buffered
        );
    }
}

#[test]
fn a_tone_crosses_the_pipeline_at_48k() {
    let run = run(48_000, 48_000, 10, 0);
    run.check();
    assert_eq!(run.silent_frames(), 0);
}

#[test]
fn a_tone_crosses_the_pipeline_between_44k1_devices() {
    let run = run(44_100, 44_100, 10, 0);
    run.check();
    assert_eq!(run.silent_frames(), 0);
}

#[test]
fn five_percent_loss_leaves_no_gap_longer_than_a_frame() {
    let run = run(48_000, 48_000, 20, 5);
    assert!(run.dropped >= 20, "only {} packets dropped", run.dropped);
    run.check();
    assert_eq!(run.silent_frames(), 0);
}
