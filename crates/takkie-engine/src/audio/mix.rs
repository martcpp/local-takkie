//! The receiving side: one jitter buffer and Opus decoder per sender, mixed
//! into 20 ms frames on a thread paced by the speaker.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use crossbeam_channel::Receiver;

use takkie_core::dsp::{Level, Tone, apply_gain, mix_into, soft_limit};
use takkie_core::jitter::{Insert, JitterBuffer, Playout};
use takkie_core::{PeerId, Seq};

use super::codec::{CodecError, VoiceDecoder};
use super::config::PREFERRED_RATE;
use super::io::AudioSink;
use super::resample::{FRAME, ResampleError, Resampler};
use super::tx::LevelMeter;
use crate::threads::{STOP_WITHIN, join_within};

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
    end: Option<Seq>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cue {
    Start,
    End,
}

impl Cue {
    fn tone(self) -> Tone {
        match self {
            Self::Start => Tone::new(1_500.0, 60, PREFERRED_RATE),
            Self::End => Tone::new(1_000.0, 120, PREFERRED_RATE),
        }
    }
}

const BEEP_LEVEL: f32 = 0.2;

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
    /// Audio waiting to be heard: jitter buffer plus speaker queue, in ms.
    pub buffer_ms: AtomicU32,
}

/// Playback settings the UI changes while audio runs.
#[derive(Debug)]
pub struct MixControls {
    volume: AtomicU32,
    muted: AtomicBool,
    half_duplex: AtomicBool,
    beeps: AtomicBool,
    muted_peers: ArcSwap<Vec<PeerId>>,
}

impl Default for MixControls {
    fn default() -> Self {
        Self {
            volume: AtomicU32::new(1.0_f32.to_bits()),
            muted: AtomicBool::new(false),
            half_duplex: AtomicBool::new(true),
            beeps: AtomicBool::new(false),
            muted_peers: ArcSwap::default(),
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

    /// Turns the beeps on or off: one when someone finishes talking, one
    /// when we start. They are made here, not sent.
    pub fn set_beeps(&self, on: bool) {
        self.beeps.store(on, Relaxed);
    }

    /// Whether the beeps are on.
    #[must_use]
    pub fn beeps(&self) -> bool {
        self.beeps.load(Relaxed)
    }

    /// Leaves one sender out of the mix, or puts them back.
    pub fn set_peer_muted(&self, peer: PeerId, muted: bool) {
        self.muted_peers.rcu(|peers| {
            let mut peers: Vec<PeerId> = peers.iter().copied().filter(|id| *id != peer).collect();
            if muted {
                peers.push(peer);
            }
            peers
        });
    }

    /// Whether this sender is left out of the mix.
    #[must_use]
    pub fn is_peer_muted(&self, peer: PeerId) -> bool {
        self.muted_peers.load().contains(&peer)
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
    /// Senders whose voice is playing, published by the mix thread.
    pub talking: Arc<ArcSwap<Vec<PeerId>>>,
    /// What goes to the speaker, after volume and mute.
    pub level: Arc<LevelMeter>,
}

/// Every sender's buffer and decoder, mixed one frame at a time.
pub struct Mixer {
    talkers: HashMap<PeerId, Talker>,
    frame: Vec<f32>,
    counters: Arc<MixCounters>,
    controls: Arc<MixControls>,
    transmitting: Arc<AtomicBool>,
    level: Arc<LevelMeter>,
    beep: Option<(Tone, Cue)>,
    was_transmitting: bool,
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
            level: shared.level,
            beep: None,
            was_transmitting: false,
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
                end: None,
            }),
        };
        if packet.end {
            let stored = talker.jitter.insert_end(packet.seq, packet.payload, now);
            if matches!(stored, Insert::Stored | Insert::Restarted) {
                talker.end = Some(packet.seq);
            }
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
    /// then applies volume, mute, half-duplex, the beeps and the limiter.
    pub fn tick(&mut self, now: Instant, out: &mut [f32]) {
        out.fill(0.0);
        let frame = &mut self.frame;
        let counters = &self.counters;
        let muted_peers = self.controls.muted_peers.load();
        let mut ended = false;
        for (
            id,
            Talker {
                jitter,
                decoder,
                concealed,
                talking,
                end,
            },
        ) in &mut self.talkers
        {
            let (seq, played) = match jitter.pop_next(now) {
                Playout::NotReady => {
                    *talking = false;
                    continue;
                }
                Playout::Packet { seq, payload } => {
                    let decoded = matches!(decoder.decode(&payload, frame), Ok(FRAME));
                    if decoded {
                        *concealed = 0;
                        *talking = true;
                    }
                    (seq, decoded || conceal(decoder, concealed, frame, counters))
                }
                Playout::Fec { seq, next } => {
                    let rebuilt = matches!(decoder.decode_fec(next, frame), Ok(FRAME));
                    if rebuilt {
                        counters.recovered.fetch_add(1, Relaxed);
                        *concealed = 0;
                        *talking = true;
                    }
                    (seq, rebuilt || conceal(decoder, concealed, frame, counters))
                }
                Playout::Plc { seq } => (seq, conceal(decoder, concealed, frame, counters)),
            };
            let audible = !muted_peers.contains(id);
            if !played {
                *talking = false;
            } else if audible {
                mix_into(out, frame);
            }
            if *end == Some(seq) {
                *end = None;
                ended |= audible;
            }
        }
        let volume = self.controls.volume();
        let muted = self.controls.is_muted();
        let transmitting = self.transmitting.load(Relaxed);
        let talking_over = self.controls.is_half_duplex() && transmitting;
        apply_gain(out, volume);
        if muted || talking_over {
            out.fill(0.0);
        }
        let over = self.beep.as_mut().is_some_and(|(tone, cue)| {
            let heard = !muted && (*cue == Cue::Start || !talking_over);
            tone.mix_into(out, if heard { BEEP_LEVEL * volume } else { 0.0 });
            tone.is_done()
        });
        if over {
            self.beep = None;
        }
        soft_limit(out);

        // Armed after mixing, so a beep starts on the frame after its cause.
        if self.controls.beeps() {
            if ended {
                self.beep = Some((Cue::End.tone(), Cue::End));
            }
            if transmitting && !self.was_transmitting {
                self.beep = Some((Cue::Start.tone(), Cue::Start));
            }
        }
        self.was_transmitting = transmitting;
        self.level.set(Level::of(out));
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
        let rate = sink.sample_rate().max(1) as usize;
        let talking = Arc::clone(&shared.talking);
        let mut mixer = Mixer::with_shared(shared);
        let counters = mixer.counters();
        let counting = Arc::clone(&counters);
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
                                    tracing::warn!("dropped a packet: {error}");
                                }
                            }
                            MixInput::Left(sender) => mixer.remove(sender),
                        }
                    }
                    if let Err(error) = pacer.fill(&mut mixer, sink.as_mut(), Instant::now()) {
                        tracing::error!("mix stopped: {error}");
                        return;
                    }
                    let waiting = mixer.buffered() * 20 + sink.queued() * 1_000 / rate;
                    counting
                        .buffer_ms
                        .store(u32::try_from(waiting).unwrap_or(u32::MAX), Relaxed);
                    let now_talking = mixer.talking();
                    if **talking.load() != now_talking {
                        talking.store(Arc::new(now_talking));
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
            join_within(handle, STOP_WITHIN);
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
    fn a_muted_peer_is_left_out_of_the_mix_but_still_decoded() {
        let mut both = Mixer::new();
        let two = rms(&chorus(&mut both, 2, 10, false)[5]);
        let mut alone = Mixer::new();
        let one = rms(&chorus(&mut alone, 1, 10, false)[5]);

        let mut mixer = Mixer::new();
        mixer.controls().set_peer_muted(PeerId::new(2), true);
        let played = chorus(&mut mixer, 2, 10, false);
        assert!((rms(&played[5]) - one).abs() < 1e-6);
        assert!((two - one).abs() > 0.01);
        assert_eq!(mixer.talking().len(), 2);
        assert_eq!(mixer.counters().silenced.load(Relaxed), 0);
    }

    #[test]
    fn muting_a_peer_twice_and_unmuting_once_leaves_them_unmuted() {
        let controls = MixControls::default();
        let (kitchen, attic) = (PeerId::new(1), PeerId::new(2));
        controls.set_peer_muted(kitchen, true);
        controls.set_peer_muted(kitchen, true);
        controls.set_peer_muted(attic, true);
        assert!(controls.is_peer_muted(kitchen) && controls.is_peer_muted(attic));
        controls.set_peer_muted(kitchen, false);
        assert!(!controls.is_peer_muted(kitchen));
        assert!(controls.is_peer_muted(attic));
    }

    fn crossings(frame: &[f32]) -> usize {
        frame
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count()
    }

    fn after_a_press(mixer: &mut Mixer) -> Vec<Vec<f32>> {
        let spoken = chorus(mixer, 1, 10, true).len();
        let t0 = Instant::now() + Duration::from_millis(20 * spoken as u64 + 60);
        let mut out = vec![0.0; FRAME];
        (0..10)
            .map(|tick| {
                mixer.tick(t0 + Duration::from_millis(20 * tick), &mut out);
                out.clone()
            })
            .collect()
    }

    #[test]
    fn a_beep_follows_the_end_of_a_transmission() {
        let mut mixer = Mixer::new();
        mixer.controls().set_beeps(true);
        let tail = after_a_press(&mut mixer);
        assert!(
            tail[..6].iter().all(|frame| rms(frame) > 0.05),
            "the beep lasts six frames"
        );
        assert!(
            (38..=42).contains(&crossings(&tail[2])),
            "the beep is 1 kHz"
        );
        assert!(tail[6..].iter().all(|frame| rms(frame) == 0.0));
        assert!(mixer.controls().beeps());
    }

    #[test]
    fn no_beep_unless_asked_for_or_when_muted() {
        let mut off = Mixer::new();
        assert!(after_a_press(&mut off).iter().all(|f| rms(f) == 0.0));

        let mut muted = Mixer::new();
        muted.controls().set_beeps(true);
        muted.controls().set_muted(true);
        assert!(after_a_press(&mut muted).iter().all(|f| rms(f) == 0.0));

        let mut peer_muted = Mixer::new();
        peer_muted.controls().set_beeps(true);
        peer_muted.controls().set_peer_muted(PeerId::new(1), true);
        assert!(after_a_press(&mut peer_muted).iter().all(|f| rms(f) == 0.0));
    }

    #[test]
    fn the_beep_follows_the_volume() {
        let mut full = Mixer::new();
        full.controls().set_beeps(true);
        let loud = rms(&after_a_press(&mut full)[2]);
        let mut half = Mixer::new();
        half.controls().set_beeps(true);
        half.controls().set_volume(0.5);
        let quiet = rms(&after_a_press(&mut half)[2]);
        assert!((quiet / loud - 0.5).abs() < 0.01, "{quiet} vs {loud}");
    }

    #[test]
    fn pressing_to_talk_beeps_even_in_half_duplex() {
        let transmitting = Arc::new(AtomicBool::new(false));
        let mut mixer = Mixer::with_transmitting(Arc::clone(&transmitting));
        mixer.controls().set_beeps(true);
        let t0 = Instant::now();
        let mut out = vec![0.0; FRAME];
        let mut frames = Vec::new();
        for tick in 0..8 {
            transmitting.store(tick >= 1, Relaxed);
            mixer.tick(t0 + Duration::from_millis(20 * tick), &mut out);
            frames.push(out.clone());
        }
        assert!(frames[..2].iter().all(|frame| rms(frame) == 0.0));
        assert!(frames[2..5].iter().all(|frame| rms(frame) > 0.05));
        assert!((58..=62).contains(&crossings(&frames[3])));
        assert!(frames[5..].iter().all(|frame| rms(frame) == 0.0));
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
