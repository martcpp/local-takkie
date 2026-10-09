//! The receiving side: one jitter buffer and Opus decoder per sender, mixed
//! into 20 ms frames on a thread paced by the speaker.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::Receiver;

use std::collections::HashMap;
use std::time::Instant;

use takkie_core::dsp::mix_into;
use takkie_core::jitter::{JitterBuffer, Playout};
use takkie_core::{PeerId, Seq};

use super::codec::{CodecError, VoiceDecoder};
use super::io::AudioSink;
use super::resample::{FRAME, ResampleError, Resampler};

/// An audio packet from the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RxPacket {
    /// Who sent it.
    pub sender: PeerId,
    /// Its sequence number.
    pub seq: Seq,
    /// The Opus packet.
    pub payload: Vec<u8>,
    /// Last packet of a push-to-talk press.
    pub end: bool,
}

struct Talker {
    jitter: JitterBuffer<Vec<u8>>,
    decoder: VoiceDecoder,
}

/// Every sender's buffer and decoder, mixed one frame at a time.
pub struct Mixer {
    talkers: HashMap<PeerId, Talker>,
    frame: Vec<f32>,
}

impl Mixer {
    /// A mixer with no senders yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            talkers: HashMap::new(),
            frame: vec![0.0; FRAME],
        }
    }

    /// Queues a packet that arrived at `now`, adding its sender on their
    /// first packet.
    ///
    /// # Errors
    /// [`CodecError`] if a new sender's decoder can't be created.
    pub fn receive(&mut self, packet: RxPacket, now: Instant) -> Result<(), CodecError> {
        let talker = match self.talkers.entry(packet.sender) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => entry.insert(Talker {
                jitter: JitterBuffer::new(),
                decoder: VoiceDecoder::new()?,
            }),
        };
        if packet.end {
            talker.jitter.insert_end(packet.seq, packet.payload, now);
        } else {
            talker.jitter.insert(packet.seq, packet.payload, now);
        }
        Ok(())
    }

    /// Forgets a sender who left.
    pub fn remove(&mut self, sender: PeerId) {
        self.talkers.remove(&sender);
    }

    /// How many senders have a decoder.
    #[must_use]
    pub fn senders(&self) -> usize {
        self.talkers.len()
    }

    /// Most packets waiting for any one sender.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.talkers
            .values()
            .map(|t| t.jitter.len())
            .max()
            .unwrap_or(0)
    }

    /// Mixes the next 20 ms from every sender into `out`.
    pub fn tick(&mut self, now: Instant, out: &mut [f32]) {
        out.fill(0.0);
        for talker in self.talkers.values_mut() {
            if let Playout::Packet { payload, .. } = talker.jitter.pop_next(now)
                && let Ok(FRAME) = talker.decoder.decode(&payload, &mut self.frame)
            {
                mix_into(out, &self.frame);
            }
        }
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

/// How full the speaker queue is kept. Enough to ride out scheduling
/// hiccups without adding much delay.
const TARGET_MS: usize = 40;

/// Tops the speaker queue up from the mixer, so the speaker's own clock
/// decides how fast frames are made and delay can't build up.
pub struct Pacer {
    resampler: Resampler,
    frame: Vec<f32>,
    device: Vec<f32>,
    target: usize,
}

impl Pacer {
    /// A pacer for a speaker at `device_rate`.
    ///
    /// # Errors
    /// [`ResampleError`] if the rate can't be converted.
    pub fn new(device_rate: u32) -> Result<Self, ResampleError> {
        let resampler = Resampler::playback(device_rate)?;
        let device = vec![0.0; resampler.output_max()];
        Ok(Self {
            resampler,
            frame: vec![0.0; FRAME],
            device,
            target: device_rate as usize * TARGET_MS / 1_000,
        })
    }

    /// Mixes frames into `sink` until it holds the target, and returns how
    /// many it made.
    ///
    /// # Errors
    /// [`ResampleError`] if resampling fails.
    pub fn fill(
        &mut self,
        mixer: &mut Mixer,
        sink: &mut dyn AudioSink,
        now: Instant,
    ) -> Result<usize, ResampleError> {
        let mut made = 0;
        while sink.queued() < self.target {
            mixer.tick(now, &mut self.frame);
            let written = self.resampler.process(&self.frame, &mut self.device)?;
            made += 1;
            if sink.write(&self.device[..written]) < written {
                break;
            }
        }
        Ok(made)
    }
}

/// The running mix thread. Dropping it stops the thread.
pub struct MixThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MixThread {
    /// Starts mixing `packets` into `sink`.
    ///
    /// # Errors
    /// [`MixError`] for an unusable speaker rate, or if the thread can't start.
    pub fn spawn(
        packets: Receiver<RxPacket>,
        mut sink: Box<dyn AudioSink>,
    ) -> Result<Self, MixError> {
        let mut pacer = Pacer::new(sink.sample_rate())?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("takkie-mix".into())
            .spawn(move || {
                let mut mixer = Mixer::new();
                while !stopping.load(Relaxed) {
                    for packet in packets.try_iter() {
                        if let Err(error) = mixer.receive(packet, Instant::now()) {
                            log::warn!("dropped a packet: {error}");
                        }
                    }
                    if let Err(error) = pacer.fill(&mut mixer, sink.as_mut(), Instant::now()) {
                        log::error!("mix stopped: {error}");
                        return;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for MixThread {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Why the mix thread couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum MixError {
    /// The speaker rate can't be converted.
    #[error(transparent)]
    Resample(#[from] ResampleError),
    /// The OS wouldn't start the thread.
    #[error("couldn't start the mix thread: {0}")]
    Thread(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;
    use std::time::Duration;

    use super::*;
    use crate::audio::codec::VoiceEncoder;

    fn packets(frequency: f32, count: usize) -> Vec<Vec<u8>> {
        let mut encoder = VoiceEncoder::new().unwrap();
        (0..count)
            .map(|f| {
                let frame: Vec<f32> = (0..FRAME)
                    .map(|i| 0.3 * (TAU * frequency * (f * FRAME + i) as f32 / 48_000.0).sin())
                    .collect();
                encoder.encode(&frame).unwrap().to_vec()
            })
            .collect()
    }

    fn decoded(packets: &[Vec<u8>]) -> Vec<f32> {
        let mut decoder = VoiceDecoder::new().unwrap();
        let mut out = vec![0.0; FRAME];
        packets
            .iter()
            .flat_map(|packet| {
                decoder.decode(packet, &mut out).unwrap();
                out.clone()
            })
            .collect()
    }

    fn rx(sender: u64, seq: usize, payload: &[u8]) -> RxPacket {
        RxPacket {
            sender: PeerId::new(sender),
            seq: Seq::new(seq as u32),
            payload: payload.to_vec(),
            end: false,
        }
    }

    #[test]
    fn two_senders_mix_to_the_sum_of_each_decoded_alone() {
        let a = packets(440.0, 10);
        let b = packets(1_200.0, 10);
        let t0 = Instant::now();
        let mut mixer = Mixer::new();
        for (seq, (pa, pb)) in a.iter().zip(&b).enumerate() {
            mixer.receive(rx(1, seq, pa), t0).unwrap();
            mixer.receive(rx(2, seq, pb), t0).unwrap();
        }
        assert_eq!(mixer.senders(), 2);

        let mut mixed = Vec::new();
        let mut out = vec![0.0; FRAME];
        for _ in 0..10 {
            mixer.tick(t0 + Duration::from_millis(60), &mut out);
            mixed.extend_from_slice(&out);
        }
        let expected: Vec<f32> = decoded(&a)
            .iter()
            .zip(decoded(&b))
            .map(|(x, y)| x + y)
            .collect();
        assert_eq!(mixed.len(), expected.len());
        assert!(
            mixed
                .iter()
                .zip(&expected)
                .all(|(m, e)| (m - e).abs() < 1e-6)
        );
    }

    #[test]
    fn nothing_plays_before_the_jitter_delay_or_without_senders() {
        let t0 = Instant::now();
        let mut mixer = Mixer::new();
        let mut out = vec![9.0; FRAME];
        mixer.tick(t0, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        mixer.receive(rx(1, 0, &packets(440.0, 1)[0]), t0).unwrap();
        out.fill(9.0);
        mixer.tick(t0 + Duration::from_millis(10), &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_sender_who_leaves_is_dropped() {
        let t0 = Instant::now();
        let mut mixer = Mixer::new();
        mixer.receive(rx(7, 0, &packets(440.0, 1)[0]), t0).unwrap();
        mixer.remove(PeerId::new(7));
        assert_eq!(mixer.senders(), 0);
    }

    fn simulate(minutes: u64, sender_drift: f64, speaker_drift: f64) -> (usize, usize, usize) {
        use crate::audio::fake::FakeSink;
        use crate::audio::io::AudioSink as _;

        let rate = 48_000_u32;
        let payload = packets(440.0, 1).remove(0);
        let mut mixer = Mixer::new();
        let mut pacer = Pacer::new(rate).unwrap();
        let mut sink = FakeSink::new(rate, rate as usize / 5);
        let t0 = Instant::now();
        let step = Duration::from_millis(5);
        let sender_period = 0.020 * (1.0 + sender_drift);
        let mut next_packet = 0.0;
        let mut seq = 0_usize;
        let mut owed = 0.0;
        let (mut most_queued, mut most_buffered) = (0, 0);
        for i in 0..minutes * 60 * 200 {
            let now = t0 + step * i as u32;
            let elapsed = (step * i as u32).as_secs_f64();
            while next_packet <= elapsed {
                mixer.receive(rx(1, seq, &payload), now).unwrap();
                seq += 1;
                next_packet += sender_period;
            }
            owed += f64::from(rate) * 0.005 * (1.0 + speaker_drift);
            sink.play(owed as usize);
            owed -= owed.floor();
            pacer.fill(&mut mixer, &mut sink, now).unwrap();
            if i > 200 {
                most_queued = most_queued.max(sink.queued());
                most_buffered = most_buffered.max(mixer.buffered());
            }
        }
        (most_queued, most_buffered, sink.underruns())
    }

    #[test]
    fn delay_stays_bounded_when_the_sender_runs_fast() {
        let (queued, buffered, _) = simulate(5, -0.001, 0.001);
        assert!(
            queued <= 48_000 * 60 / 1_000,
            "speaker queue reached {queued}"
        );
        assert!(buffered <= 10, "jitter buffer reached {buffered} packets");
    }

    #[test]
    fn delay_stays_bounded_when_the_speaker_runs_fast() {
        let (queued, buffered, _) = simulate(5, 0.001, -0.001);
        assert!(
            queued <= 48_000 * 60 / 1_000,
            "speaker queue reached {queued}"
        );
        assert!(buffered <= 10, "jitter buffer reached {buffered} packets");
    }

    #[test]
    fn the_pacer_keeps_the_queue_at_the_target() {
        use crate::audio::fake::FakeSink;
        use crate::audio::io::AudioSink as _;

        let mut mixer = Mixer::new();
        let mut pacer = Pacer::new(44_100).unwrap();
        let mut sink = FakeSink::new(44_100, 44_100 / 5);
        let made = pacer.fill(&mut mixer, &mut sink, Instant::now()).unwrap();
        assert!(made >= 2);
        assert!(sink.queued() >= 44_100 * 40 / 1_000);
        assert_eq!(
            pacer.fill(&mut mixer, &mut sink, Instant::now()).unwrap(),
            0
        );
    }
}
