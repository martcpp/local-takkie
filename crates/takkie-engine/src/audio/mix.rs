//! The receiving side: one jitter buffer and Opus decoder per sender, mixed
//! into 20 ms frames on a thread paced by the speaker.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;

use takkie_core::dsp::{apply_gain, mix_into, soft_limit};
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

/// What the mix thread is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MixInput {
    /// Audio to play.
    Packet(RxPacket),
    /// This sender left, so their decoder and buffer can go.
    Left(PeerId),
}

struct Talker {
    jitter: JitterBuffer<Vec<u8>>,
    decoder: VoiceDecoder,
    concealed: u32,
    talking: bool,
}

/// After 100 ms of guessing, silence sounds better than a drone.
const MAX_CONCEALED: u32 = 5;

/// How lost frames were covered, for stats.
#[derive(Debug, Default)]
pub struct MixCounters {
    /// Rebuilt from the next packet's FEC data.
    pub recovered: AtomicU64,
    /// Guessed with packet loss concealment.
    pub concealed: AtomicU64,
    /// Played as silence.
    pub silenced: AtomicU64,
}

/// Playback settings the UI changes while audio runs.
#[derive(Debug)]
pub struct MixControls {
    volume: AtomicU32,
    muted: AtomicBool,
    half_duplex: AtomicBool,
}

impl Default for MixControls {
    fn default() -> Self {
        Self {
            volume: AtomicU32::new(1.0_f32.to_bits()),
            muted: AtomicBool::new(false),
            half_duplex: AtomicBool::new(true),
        }
    }
}

impl MixControls {
    /// Sets the playback gain, kept within 0 to 2.
    pub fn set_volume(&self, volume: f32) {
        let volume = if volume.is_nan() {
            1.0
        } else {
            volume.clamp(0.0, 2.0)
        };
        self.volume.store(volume.to_bits(), Relaxed);
    }

    /// The playback gain.
    #[must_use]
    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Relaxed))
    }

    /// Silences playback; decoding carries on.
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Relaxed);
    }

    /// Whether playback is silenced.
    #[must_use]
    pub fn is_muted(&self) -> bool {
        self.muted.load(Relaxed)
    }

    /// Whether playback goes quiet while we transmit. On by default, like a
    /// walkie-talkie, and it stops the speaker echoing into the mic.
    pub fn set_half_duplex(&self, on: bool) {
        self.half_duplex.store(on, Relaxed);
    }

    /// Whether half-duplex is on.
    #[must_use]
    pub fn is_half_duplex(&self) -> bool {
        self.half_duplex.load(Relaxed)
    }
}

/// State that outlives one mixer, so a speaker restart keeps it.
#[derive(Clone, Debug, Default)]
pub struct MixShared {
    /// The tx thread's push-to-talk flag.
    pub transmitting: Arc<AtomicBool>,
    /// Volume, mute and half-duplex.
    pub controls: Arc<MixControls>,
    /// Lost-frame counts.
    pub counters: Arc<MixCounters>,
}

/// Every sender's buffer and decoder, mixed one frame at a time.
pub struct Mixer {
    talkers: HashMap<PeerId, Talker>,
    frame: Vec<f32>,
    counters: Arc<MixCounters>,
    controls: Arc<MixControls>,
    transmitting: Arc<AtomicBool>,
}

impl Mixer {
    /// A mixer with no senders yet, never transmitting.
    #[must_use]
    pub fn new() -> Self {
        Self::with_shared(MixShared::default())
    }

    /// A mixer that follows the same push-to-talk flag as the tx thread.
    #[must_use]
    pub fn with_transmitting(transmitting: Arc<AtomicBool>) -> Self {
        Self::with_shared(MixShared {
            transmitting,
            ..MixShared::default()
        })
    }

    /// A mixer using state that outlives it.
    #[must_use]
    pub fn with_shared(shared: MixShared) -> Self {
        Self {
            talkers: HashMap::new(),
            frame: vec![0.0; FRAME],
            counters: shared.counters,
            controls: shared.controls,
            transmitting: shared.transmitting,
        }
    }

    /// Volume and mute, shared with the UI.
    #[must_use]
    pub fn controls(&self) -> Arc<MixControls> {
        Arc::clone(&self.controls)
    }

    /// Senders whose voice is playing, in id order.
    #[must_use]
    pub fn talking(&self) -> Vec<PeerId> {
        let mut talking: Vec<PeerId> = self
            .talkers
            .iter()
            .filter(|(_, talker)| talker.talking)
            .map(|(id, _)| *id)
            .collect();
        talking.sort();
        talking
    }

    /// Lost-frame counts, shared with other threads.
    #[must_use]
    pub fn counters(&self) -> Arc<MixCounters> {
        Arc::clone(&self.counters)
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
                concealed: 0,
                talking: false,
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

    /// Mixes the next 20 ms from every sender into `out`, covering lost
    /// frames with FEC when the next packet is here and concealment if not,
    /// then applies volume, the limiter, mute and half-duplex.
    pub fn tick(&mut self, now: Instant, out: &mut [f32]) {
        out.fill(0.0);
        let frame = &mut self.frame;
        let counters = &self.counters;
        for Talker {
            jitter,
            decoder,
            concealed,
            talking,
        } in self.talkers.values_mut()
        {
            let played = match jitter.pop_next(now) {
                Playout::NotReady => {
                    *talking = false;
                    continue;
                }
                Playout::Packet { payload, .. } => {
                    let decoded = matches!(decoder.decode(&payload, frame), Ok(FRAME));
                    if decoded {
                        *concealed = 0;
                        *talking = true;
                    }
                    decoded || conceal(decoder, concealed, frame, counters)
                }
                Playout::Fec { next, .. } => {
                    let rebuilt = matches!(decoder.decode_fec(next, frame), Ok(FRAME));
                    if rebuilt {
                        counters.recovered.fetch_add(1, Relaxed);
                        *concealed = 0;
                        *talking = true;
                    }
                    rebuilt || conceal(decoder, concealed, frame, counters)
                }
                Playout::Plc { .. } => conceal(decoder, concealed, frame, counters),
            };
            if played {
                mix_into(out, frame);
            } else {
                *talking = false;
            }
        }
        apply_gain(out, self.controls.volume());
        soft_limit(out);
        let talking_over = self.controls.is_half_duplex() && self.transmitting.load(Relaxed);
        if self.controls.is_muted() || talking_over {
            out.fill(0.0);
        }
    }
}

fn conceal(
    decoder: &mut VoiceDecoder,
    concealed: &mut u32,
    frame: &mut [f32],
    counters: &MixCounters,
) -> bool {
    if *concealed < MAX_CONCEALED && matches!(decoder.conceal(frame), Ok(FRAME)) {
        *concealed += 1;
        counters.concealed.fetch_add(1, Relaxed);
        true
    } else {
        counters.silenced.fetch_add(1, Relaxed);
        false
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
    counters: Arc<MixCounters>,
    controls: Arc<MixControls>,
}

impl MixThread {
    /// Starts mixing `packets` into `sink`, with `shared` state that
    /// outlives the thread.
    ///
    /// # Errors
    /// [`MixError`] for an unusable speaker rate, or if the thread can't start.
    pub fn spawn(
        packets: Receiver<MixInput>,
        mut sink: Box<dyn AudioSink>,
        shared: MixShared,
    ) -> Result<Self, MixError> {
        let mut pacer = Pacer::new(sink.sample_rate())?;
        let mut mixer = Mixer::with_shared(shared);
        let counters = mixer.counters();
        let controls = mixer.controls();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("takkie-mix".into())
            .spawn(move || {
                while !stopping.load(Relaxed) {
                    for input in packets.try_iter() {
                        match input {
                            MixInput::Packet(packet) => {
                                if let Err(error) = mixer.receive(packet, Instant::now()) {
                                    log::warn!("dropped a packet: {error}");
                                }
                            }
                            MixInput::Left(sender) => mixer.remove(sender),
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
            counters,
            controls,
        })
    }

    /// Lost-frame counts.
    #[must_use]
    pub fn counters(&self) -> Arc<MixCounters> {
        Arc::clone(&self.counters)
    }

    /// Volume and mute.
    #[must_use]
    pub fn controls(&self) -> Arc<MixControls> {
        Arc::clone(&self.controls)
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

    fn rms(frame: &[f32]) -> f32 {
        (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
    }

    fn live(mixer: &mut Mixer, sent: &[Vec<u8>], keep: impl Fn(usize) -> bool) -> Vec<Vec<f32>> {
        let t0 = Instant::now();
        let mut out = vec![0.0; FRAME];
        let mut played = Vec::new();
        for tick in 0..sent.len() + 3 {
            let now = t0 + Duration::from_millis(20 * tick as u64);
            if let Some(payload) = sent.get(tick).filter(|_| keep(tick)) {
                mixer.receive(rx(1, tick, payload), now).unwrap();
            }
            mixer.tick(now, &mut out);
            if tick >= 3 {
                played.push(out.clone());
            }
        }
        played
    }

    #[test]
    fn five_percent_random_loss_leaves_no_frame_missing() {
        let sent = packets(440.0, 200);
        let mut seed = 7_u32;
        let lost: Vec<bool> = (0..sent.len())
            .map(|seq| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                seq > 2 && seq < 197 && (seed >> 16).is_multiple_of(20)
            })
            .collect();
        let dropped = lost.iter().filter(|l| **l).count() as u64;
        assert!(dropped >= 5, "only {dropped} packets dropped");

        let mut mixer = Mixer::new();
        let played = live(&mut mixer, &sent, |seq| !lost[seq]);
        let silent = played
            .iter()
            .skip(1)
            .filter(|frame| rms(frame) < 0.01)
            .count();
        assert_eq!(silent, 0, "{silent} frames came out silent");
        let counters = mixer.counters();
        let covered = counters.recovered.load(Relaxed) + counters.concealed.load(Relaxed);
        assert_eq!(covered, dropped);
        assert_eq!(counters.silenced.load(Relaxed), 0);
    }

    #[test]
    fn a_single_gap_is_rebuilt_from_fec() {
        let sent = packets(440.0, 10);
        let mut mixer = Mixer::new();
        live(&mut mixer, &sent, |seq| seq != 5);
        let counters = mixer.counters();
        assert_eq!(counters.recovered.load(Relaxed), 1);
        assert_eq!(counters.concealed.load(Relaxed), 0);
    }

    #[test]
    fn a_long_gap_is_concealed_then_silenced() {
        let sent = packets(440.0, 20);
        let mut mixer = Mixer::new();
        live(&mut mixer, &sent, |seq| !(3..12).contains(&seq));
        let counters = mixer.counters();
        assert_eq!(counters.concealed.load(Relaxed), 5);
        assert_eq!(counters.silenced.load(Relaxed), 3);
        assert_eq!(counters.recovered.load(Relaxed), 1);
    }

    #[test]
    fn a_corrupt_packet_is_concealed() {
        let mut sent = packets(440.0, 3);
        sent[1] = vec![0xFF, 0xFF, 0xFF];
        let mut mixer = Mixer::new();
        let played = live(&mut mixer, &sent, |_| true);
        assert!(rms(&played[1]) > 0.0);
        assert_eq!(mixer.counters().concealed.load(Relaxed), 1);
    }

    fn chorus(mixer: &mut Mixer, senders: u64, frames: usize, end_last: bool) -> Vec<Vec<f32>> {
        let streams: Vec<Vec<Vec<u8>>> = (0..senders)
            .map(|i| packets(300.0 + 170.0 * i as f32, frames))
            .collect();
        let t0 = Instant::now();
        let mut out = vec![0.0; FRAME];
        let mut played = Vec::new();
        for tick in 0..frames + 3 {
            let now = t0 + Duration::from_millis(20 * tick as u64);
            for (i, stream) in streams.iter().enumerate() {
                if let Some(payload) = stream.get(tick) {
                    let mut packet = rx(i as u64 + 1, tick, payload);
                    packet.end = end_last && tick + 1 == frames;
                    mixer.receive(packet, now).unwrap();
                }
            }
            mixer.tick(now, &mut out);
            if tick >= 3 {
                played.push(out.clone());
            }
        }
        played
    }

    fn peak(frames: &[Vec<f32>]) -> f32 {
        frames.iter().flatten().fold(0.0, |p, s| p.max(s.abs()))
    }

    #[test]
    fn one_two_and_five_senders_mix_and_stay_within_one() {
        for senders in [1, 2, 5] {
            let mut mixer = Mixer::new();
            let played = chorus(&mut mixer, senders, 20, false);
            assert_eq!(mixer.talking().len(), senders as usize);
            assert!(peak(&played) <= 1.0, "{senders} senders peaked over 1");
            assert!(played.iter().skip(1).all(|frame| rms(frame) > 0.01));
        }
    }

    #[test]
    fn five_loud_senders_are_limited() {
        let mut mixer = Mixer::new();
        let played = chorus(&mut mixer, 5, 20, false);
        let unlimited: f32 = 5.0 * 0.3;
        assert!(unlimited > 1.0);
        assert!(peak(&played) <= 1.0);
    }

    #[test]
    fn volume_scales_the_output() {
        let mut full = Mixer::new();
        let loud = rms(&chorus(&mut full, 1, 10, false)[5]);
        let mut half = Mixer::new();
        half.controls().set_volume(0.5);
        let quiet = rms(&chorus(&mut half, 1, 10, false)[5]);
        assert!((quiet / loud - 0.5).abs() < 0.01, "{quiet} vs {loud}");
    }

    #[test]
    fn volume_is_kept_in_range() {
        let controls = MixControls::default();
        assert_eq!(controls.volume(), 1.0);
        controls.set_volume(5.0);
        assert_eq!(controls.volume(), 2.0);
        controls.set_volume(-1.0);
        assert_eq!(controls.volume(), 0.0);
        controls.set_volume(f32::NAN);
        assert_eq!(controls.volume(), 1.0);
    }

    #[test]
    fn mute_silences_but_keeps_decoding() {
        let mut mixer = Mixer::new();
        mixer.controls().set_muted(true);
        let played = chorus(&mut mixer, 2, 10, false);
        assert!(played.iter().all(|frame| rms(frame) == 0.0));
        assert_eq!(mixer.talking().len(), 2);
        assert_eq!(mixer.counters().silenced.load(Relaxed), 0);
        assert!(mixer.controls().is_muted());
    }

    #[test]
    fn talking_stops_when_the_press_ends() {
        let mut mixer = Mixer::new();
        chorus(&mut mixer, 2, 10, true);
        let mut out = vec![0.0; FRAME];
        mixer.tick(Instant::now() + Duration::from_secs(1), &mut out);
        assert!(mixer.talking().is_empty());
    }

    fn speaker(mixer: &mut Mixer, transmitting: &AtomicBool, talk_from: usize) -> Vec<f32> {
        use crate::audio::fake::FakeSink;

        let sent = packets(440.0, 30);
        let mut pacer = Pacer::new(48_000).unwrap();
        let mut sink = FakeSink::new(48_000, 48_000 / 5);
        let t0 = Instant::now();
        for (tick, payload) in sent.iter().enumerate() {
            let now = t0 + Duration::from_millis(20 * tick as u64);
            transmitting.store(tick >= talk_from && tick < talk_from + 10, Relaxed);
            mixer.receive(rx(1, tick, payload), now).unwrap();
            pacer.fill(mixer, &mut sink, now).unwrap();
            sink.play(FRAME);
        }
        sink.played().to_vec()
    }

    fn frame_rms(samples: &[f32], frame: usize) -> f32 {
        rms(&samples[frame * FRAME..(frame + 1) * FRAME])
    }

    #[test]
    fn half_duplex_silences_the_speaker_while_transmitting() {
        let transmitting = Arc::new(AtomicBool::new(false));
        let mut mixer = Mixer::with_transmitting(Arc::clone(&transmitting));
        let played = speaker(&mut mixer, &transmitting, 10);
        assert!(frame_rms(&played, 8) > 0.01);
        assert!((12..18).all(|f| frame_rms(&played, f) == 0.0));
        assert!(frame_rms(&played, 24) > 0.01);
        assert!(mixer.controls().is_half_duplex());
    }

    #[test]
    fn decoding_continues_while_transmitting() {
        let transmitting = Arc::new(AtomicBool::new(false));
        let mut mixer = Mixer::with_transmitting(Arc::clone(&transmitting));
        speaker(&mut mixer, &transmitting, 10);
        assert_eq!(mixer.counters().silenced.load(Relaxed), 0);
        assert_eq!(mixer.counters().concealed.load(Relaxed), 0);
    }

    #[test]
    fn full_duplex_keeps_playing_while_transmitting() {
        let transmitting = Arc::new(AtomicBool::new(false));
        let mut mixer = Mixer::with_transmitting(Arc::clone(&transmitting));
        mixer.controls().set_half_duplex(false);
        let played = speaker(&mut mixer, &transmitting, 10);
        assert!((12..18).all(|f| frame_rms(&played, f) > 0.01));
    }
}
