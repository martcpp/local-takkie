//! The API every UI uses. One [`Engine`] owns the audio, network and
//! discovery threads, and dropping it stops them all.

use std::hash::{BuildHasher, RandomState};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use arc_swap::{ArcSwap, ArcSwapOption};
use crossbeam_channel::{Receiver, Sender, never, select, unbounded};
use takkie_core::dsp::Level;
use takkie_core::peers::{Peer, PeerEvent};
use takkie_core::{ChannelId, PeerId};
use thiserror::Error;

use crate::audio::devices::{DeviceList, Direction, list_devices};
use crate::audio::mix::MixShared;
use crate::audio::session::{
    AudioEvent, AudioSession, CpalOpener, DeviceOpener, SessionError, Wiring,
};
use crate::audio::stream::Counters;
use crate::audio::tx::LevelMeter;
use crate::net::discovery::{Announcement, Browser, Discovery, DiscoveryError};
use crate::net::hello::{HELLO_EVERY, HelloThread};
use crate::net::peers::{PEER_TIMEOUT, PeerOutputs, PeerThread, Peers};
use crate::net::rx::{RxCounters, RxOutputs, RxThread};
use crate::net::send::{PacketSender, SendCounters, SendThread};
use crate::net::{Transport, UdpTransport};
use crate::threads::{STOP_WITHIN, join_within};

const POLL_EVERY: Duration = Duration::from_millis(500);
// The control thread owns mDNS, whose goodbye can take up to two seconds.
const MDNS_GOODBYE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tuning {
    pub(crate) hello_every: Duration,
    pub(crate) peer_timeout: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            hello_every: HELLO_EVERY,
            peer_timeout: PEER_TIMEOUT,
        }
    }
}
const TALK_CHECK_EVERY: Duration = Duration::from_millis(50);

/// How to start an [`Engine`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    /// The name peers see.
    pub display_name: String,
    /// UDP port; 0 lets the OS pick one.
    pub port: u16,
    /// The channel to join.
    pub channel: ChannelId,
    /// Microphone name, or `None` for the system default.
    pub input_device: Option<String>,
    /// Speaker name, or `None` for the system default.
    pub output_device: Option<String>,
    /// Peers to greet directly, for networks that block mDNS.
    pub static_peers: Vec<SocketAddr>,
    /// Mute playback while transmitting.
    pub half_duplex: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            display_name: "takkie".into(),
            port: 0,
            channel: ChannelId::MIN,
            input_device: None,
            output_device: None,
            static_peers: Vec::new(),
            half_duplex: true,
        }
    }
}

/// Why the engine couldn't start.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EngineError {
    /// The UDP port is taken or not allowed.
    #[error("couldn't open UDP port {port}: {source}")]
    Bind {
        /// The port asked for.
        port: u16,
        /// What the OS said.
        #[source]
        source: io::Error,
    },
    /// A microphone or speaker couldn't be found or opened.
    #[error("audio: {0}")]
    Audio(#[from] SessionError),
    /// mDNS couldn't start.
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    /// An engine thread couldn't start.
    #[error("couldn't start an engine thread: {0}")]
    Thread(#[source] io::Error),
}

/// Another takkie, as the UI shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerInfo {
    /// Their id.
    pub id: PeerId,
    /// Display name; empty until their first Hello or mDNS record.
    pub name: String,
    /// Their channel.
    pub channel: ChannelId,
    /// Where we send to them.
    pub addr: SocketAddr,
    /// Whether their voice is playing right now.
    pub talking: bool,
    /// When we last heard from them: a Hello, audio or mDNS.
    pub last_seen: Instant,
}

/// Something the UI should react to.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineEvent {
    /// A peer appeared.
    PeerJoined(PeerInfo),
    /// A peer's name, channel or address changed.
    PeerUpdated(PeerInfo),
    /// A peer left or went quiet.
    PeerLeft(PeerId),
    /// A peer's voice started playing.
    TalkStarted(PeerId),
    /// A peer's voice stopped.
    TalkStopped(PeerId),
    /// A microphone or speaker is running.
    DeviceStarted {
        /// Which side.
        direction: Direction,
        /// Name and settings.
        description: String,
    },
    /// A device went away; the default is retried every couple of seconds.
    DeviceLost(Direction),
    /// Worth showing, like a device name that fell back to the default.
    Warning(String),
}

/// Running totals, for a stats line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineStats {
    /// Datagrams sent.
    pub packets_sent: u64,
    /// Sends that failed.
    pub send_errors: u64,
    /// Datagrams received.
    pub packets_received: u64,
    /// Received datagrams that weren't takkie packets.
    pub packets_invalid: u64,
    /// Lost frames rebuilt from FEC.
    pub frames_recovered: u64,
    /// Lost frames guessed by concealment.
    pub frames_concealed: u64,
    /// Lost frames played as silence.
    pub frames_silenced: u64,
    /// Mic samples dropped because the tx thread fell behind.
    pub mic_dropped: u64,
    /// Times the speaker ran dry.
    pub speaker_underruns: u64,
}

/// Everything a UI draws, read without locks.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineSnapshot {
    /// Our id.
    pub id: PeerId,
    /// Our channel.
    pub channel: ChannelId,
    /// Push-to-talk is held.
    pub transmitting: bool,
    /// Playback is muted.
    pub muted: bool,
    /// Playback volume.
    pub volume: f32,
    /// The latest mic level.
    pub mic: Level,
    /// The latest level we played, after volume and mute.
    pub speaker: Level,
    /// Audio waiting to be heard, in ms.
    pub buffer_ms: u32,
    /// Every known peer, in id order.
    pub peers: Vec<PeerInfo>,
    /// Running totals.
    pub stats: EngineStats,
}

enum Command {
    SetChannel(ChannelId),
}

#[derive(Clone)]
struct Shared {
    channel: Arc<AtomicU8>,
    transmitting: Arc<AtomicBool>,
    level: Arc<LevelMeter>,
    mix: MixShared,
    peers: Arc<ArcSwap<Vec<Peer>>>,
    sent: Arc<SendCounters>,
    received: Arc<RxCounters>,
    mic: Arc<ArcSwapOption<Counters>>,
    speaker: Arc<ArcSwapOption<Counters>>,
}

impl Shared {
    fn info(&self, peer: &Peer) -> PeerInfo {
        PeerInfo {
            id: peer.id,
            name: peer.name.clone(),
            channel: peer.channel,
            addr: peer.addr,
            talking: self.mix.talking.load().contains(&peer.id),
            last_seen: peer.last_seen,
        }
    }

    fn device_counters<O: DeviceOpener>(&self, session: &AudioSession<O>) {
        self.mic.store(session.input_counters());
        self.speaker.store(session.output_counters());
    }
}

/// A running walkie-talkie. Dropping it stops every thread.
///
/// ```no_run
/// use takkie_engine::{Engine, EngineConfig, EngineEvent};
///
/// let (engine, events) = Engine::start(EngineConfig {
///     display_name: "Kitchen".into(),
///     ..EngineConfig::default()
/// })?;
/// engine.set_transmitting(true);
/// engine.set_transmitting(false);
/// for event in events.try_iter() {
///     if let EngineEvent::PeerJoined(peer) = event {
///         println!("{} joined", peer.name);
///     }
/// }
/// println!("{} peers", engine.snapshot().peers.len());
/// # Ok::<(), takkie_engine::EngineError>(())
/// ```
pub struct Engine {
    id: PeerId,
    port: u16,
    shared: Shared,
    hello: Option<HelloThread>,
    commands: Option<Sender<Command>>,
    control: Option<JoinHandle<()>>,
    send: Option<SendThread>,
    _rx: RxThread,
    _peers: PeerThread,
}

impl Engine {
    /// Opens the UDP port and the audio devices, announces us over mDNS
    /// and starts every thread. Events arrive on the returned receiver.
    ///
    /// # Errors
    /// [`EngineError`] if the port, a device, mDNS or a thread can't start.
    pub fn start(config: EngineConfig) -> Result<(Self, Receiver<EngineEvent>), EngineError> {
        let transport = UdpTransport::bind(config.port).map_err(|source| EngineError::Bind {
            port: config.port,
            source,
        })?;
        Self::start_with(
            config,
            CpalOpener,
            Arc::new(transport),
            true,
            Tuning::default(),
        )
    }

    pub(crate) fn start_with<O: DeviceOpener + 'static>(
        config: EngineConfig,
        opener: O,
        transport: Arc<dyn Transport>,
        mdns: bool,
        tuning: Tuning,
    ) -> Result<(Self, Receiver<EngineEvent>), EngineError> {
        let id = new_id();
        let port = transport.local_addr().port();
        let channel = Arc::new(AtomicU8::new(config.channel.get()));
        let targets = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let transmitting = Arc::new(AtomicBool::new(false));
        let level = Arc::new(LevelMeter::default());
        let mix = MixShared {
            transmitting: Arc::clone(&transmitting),
            ..MixShared::default()
        };
        mix.controls.set_half_duplex(config.half_duplex);

        let (packets_in, packets) = unbounded();
        let (news_in, news) = unbounded();
        let (peer_events_in, peer_events) = unbounded();
        let table = Peers::new(
            Arc::clone(&channel),
            Arc::clone(&targets),
            tuning.peer_timeout,
        );
        let peer_view = table.view();
        let peers = PeerThread::spawn(
            news,
            table,
            PeerOutputs {
                events: peer_events_in,
                mixer: packets_in.clone(),
            },
        )
        .map_err(EngineError::Thread)?;
        let rx = RxThread::spawn(
            Arc::clone(&transport),
            id,
            Arc::clone(&channel),
            RxOutputs {
                audio: packets_in,
                peers: news_in.clone(),
            },
        )
        .map_err(EngineError::Thread)?;
        let (tx_events, tx_out) = unbounded();
        let sender = Arc::new(PacketSender::new(
            transport,
            id,
            Arc::clone(&channel),
            targets,
        ));
        let send = SendThread::spawn(tx_out, Arc::clone(&sender)).map_err(EngineError::Thread)?;

        let shared = Shared {
            channel,
            transmitting,
            level,
            mix,
            peers: peer_view,
            sent: sender.counters(),
            received: rx.counters(),
            mic: Arc::default(),
            speaker: Arc::default(),
        };
        let (session, started) = AudioSession::start(
            opener,
            config.input_device.as_deref(),
            config.output_device.as_deref(),
            Wiring {
                transmitting: Arc::clone(&shared.transmitting),
                level: Arc::clone(&shared.level),
                tx_events,
                packets,
                mix: shared.mix.clone(),
            },
        )?;
        shared.device_counters(&session);
        let (events, events_out) = unbounded();
        for event in started {
            let _ = events.send(audio_event(event));
        }

        let mdns = if mdns {
            let discovery = Discovery::start(&Announcement {
                id,
                name: config.display_name.clone(),
                channel: config.channel,
                port,
            })?;
            Some((Browser::spawn(&discovery, news_in)?, discovery))
        } else {
            None
        };

        let (commands, commands_out) = unbounded();
        let control = Control {
            session,
            mdns,
            shared: shared.clone(),
            events,
            heard: Vec::new(),
        };
        let control = thread::Builder::new()
            .name("takkie-control".into())
            .spawn(move || control.run(&commands_out, peer_events))
            .map_err(EngineError::Thread)?;
        let hello = HelloThread::spawn(
            sender,
            &config.display_name,
            tuning.hello_every,
            Arc::clone(&shared.peers),
            config.static_peers,
        )
        .map_err(EngineError::Thread)?;

        tracing::info!(%id, port, channel = %config.channel, "engine started");
        let engine = Self {
            id,
            port,
            shared,
            hello: Some(hello),
            commands: Some(commands),
            control: Some(control),
            send: Some(send),
            _rx: rx,
            _peers: peers,
        };
        Ok((engine, events_out))
    }

    /// Our id on the network.
    #[must_use]
    pub fn id(&self) -> PeerId {
        self.id
    }

    /// The UDP port we listen on.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Push-to-talk. Takes effect on the next 20 ms frame.
    pub fn set_transmitting(&self, on: bool) {
        self.shared.transmitting.store(on, Relaxed);
    }

    /// Moves to another channel.
    pub fn set_channel(&self, channel: ChannelId) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::SetChannel(channel));
        }
    }

    /// Silences what we hear.
    pub fn set_muted(&self, muted: bool) {
        self.shared.mix.controls.set_muted(muted);
    }

    /// Playback volume: 1.0 is unchanged, clamped to 0.0..=2.0.
    pub fn set_volume(&self, volume: f32) {
        self.shared.mix.controls.set_volume(volume);
    }

    /// The microphones and speakers on this machine.
    ///
    /// # Errors
    /// [`EngineError::Audio`] if the audio system can't be asked.
    pub fn list_devices() -> Result<DeviceList, EngineError> {
        list_devices().map_err(|error| EngineError::Audio(error.into()))
    }

    /// What the UI draws: peers, talking, levels, buffer and stats.
    #[must_use]
    pub fn snapshot(&self) -> EngineSnapshot {
        let shared = &self.shared;
        let total = |counters: &ArcSwapOption<Counters>, pick: fn(&Counters) -> u64| {
            counters.load().as_deref().map_or(0, pick)
        };
        let mix = &shared.mix.counters;
        EngineSnapshot {
            id: self.id,
            channel: ChannelId::try_from(shared.channel.load(Relaxed)).unwrap_or(ChannelId::MIN),
            transmitting: shared.transmitting.load(Relaxed),
            muted: shared.mix.controls.is_muted(),
            volume: shared.mix.controls.volume(),
            mic: shared.level.get(),
            speaker: shared.mix.level.get(),
            buffer_ms: mix.buffer_ms.load(Relaxed),
            peers: shared
                .peers
                .load()
                .iter()
                .map(|peer| shared.info(peer))
                .collect(),
            stats: EngineStats {
                packets_sent: shared.sent.sent.load(Relaxed),
                send_errors: shared.sent.errors.load(Relaxed),
                packets_received: shared.received.received.load(Relaxed),
                packets_invalid: shared.received.invalid.load(Relaxed),
                frames_recovered: mix.recovered.load(Relaxed),
                frames_concealed: mix.concealed.load(Relaxed),
                frames_silenced: mix.silenced.load(Relaxed),
                mic_dropped: total(&shared.mic, |c| c.dropped.load(Relaxed)),
                speaker_underruns: total(&shared.speaker, |c| c.underruns.load(Relaxed)),
            },
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // No audio may follow our Bye, and Bye goes before the mDNS goodbye in the control thread.
        self.shared.transmitting.store(false, Relaxed);
        drop(self.send.take());
        drop(self.hello.take());
        drop(self.commands.take());
        if let Some(control) = self.control.take() {
            join_within(control, STOP_WITHIN + MDNS_GOODBYE);
        }
    }
}

struct Control<O: DeviceOpener> {
    session: AudioSession<O>,
    mdns: Option<(Browser, Discovery)>,
    shared: Shared,
    events: Sender<EngineEvent>,
    heard: Vec<PeerId>,
}

impl<O: DeviceOpener> Control<O> {
    fn run(mut self, commands: &Receiver<Command>, mut peer_events: Receiver<PeerEvent>) {
        let mut next_poll = Instant::now() + POLL_EVERY;
        loop {
            select! {
                recv(commands) -> command => match command {
                    Ok(Command::SetChannel(channel)) => {
                        tracing::info!(%channel, "channel changed");
                        self.shared.channel.store(channel.get(), Relaxed);
                        if let Some((_, discovery)) = &mut self.mdns
                            && let Err(error) = discovery.set_channel(channel)
                        {
                            tracing::warn!("couldn't announce channel {channel}: {error}");
                        }
                    }
                    Err(_) => break,
                },
                recv(peer_events) -> event => match event {
                    Ok(event) => self.peer(event),
                    Err(_) => peer_events = never(),
                },
                default(TALK_CHECK_EVERY) => {}
            }
            self.talk();
            let now = Instant::now();
            if now >= next_poll {
                let changes = self.session.poll(now);
                if !changes.is_empty() {
                    self.shared.device_counters(&self.session);
                }
                for change in changes {
                    self.send(audio_event(change));
                }
                next_poll = now + POLL_EVERY;
            }
        }
        drop(self.mdns.take());
    }

    fn send(&self, event: EngineEvent) {
        let _ = self.events.send(event);
    }

    fn peer(&self, event: PeerEvent) {
        let find = |id: PeerId| {
            let peers = self.shared.peers.load();
            peers
                .iter()
                .find(|peer| peer.id == id)
                .map(|peer| self.shared.info(peer))
        };
        let event = match event {
            PeerEvent::Joined(id) => find(id).map(EngineEvent::PeerJoined),
            PeerEvent::Updated(id) => find(id).map(EngineEvent::PeerUpdated),
            PeerEvent::Left(id) => Some(EngineEvent::PeerLeft(id)),
        };
        if let Some(event) = event {
            match &event {
                EngineEvent::PeerJoined(peer) => {
                    tracing::info!(id = %peer.id, name = %peer.name, channel = %peer.channel, addr = %peer.addr, "peer joined");
                }
                EngineEvent::PeerLeft(id) => tracing::info!(%id, "peer left"),
                _ => {}
            }
            self.send(event);
        }
    }

    fn talk(&mut self) {
        let talking = self.shared.mix.talking.load();
        if **talking == self.heard {
            return;
        }
        for &id in talking.iter().filter(|id| !self.heard.contains(id)) {
            self.send(EngineEvent::TalkStarted(id));
        }
        for &id in self.heard.iter().filter(|id| !talking.contains(id)) {
            self.send(EngineEvent::TalkStopped(id));
        }
        self.heard = talking.to_vec();
    }
}

fn audio_event(event: AudioEvent) -> EngineEvent {
    match event {
        AudioEvent::Started {
            direction,
            description,
        } => {
            tracing::info!("{direction} started: {description}");
            EngineEvent::DeviceStarted {
                direction,
                description,
            }
        }
        AudioEvent::Warning(warning) => {
            tracing::warn!("{warning}");
            EngineEvent::Warning(warning)
        }
        AudioEvent::DeviceLost(direction) => {
            tracing::warn!("{direction} lost, retrying the default");
            EngineEvent::DeviceLost(direction)
        }
    }
}

fn new_id() -> PeerId {
    // RandomState is seeded by the OS, so engines started together still differ.
    PeerId::new(RandomState::new().hash_one(SystemTime::now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fake::{FakeSink, FakeSource, LiveSink, LiveSource, Signal};
    use crate::audio::session::Opened;
    use crate::audio::{AudioSink, AudioSource};
    use crate::net::MemoryNetwork;

    const RATE: u32 = 48_000;
    const AMPLITUDE: f32 = 0.4;

    struct Fakes {
        mic: Signal,
        speaker: LiveSink,
    }

    impl DeviceOpener for Fakes {
        fn open_input(
            &mut self,
            _: Option<&str>,
        ) -> Result<Opened<Box<dyn AudioSource>>, SessionError> {
            Ok(Opened {
                stream: Box::new(LiveSource::new(FakeSource::new(RATE, self.mic.clone()))),
                description: "fake mic".into(),
                counters: Arc::default(),
                warning: None,
            })
        }

        fn open_output(
            &mut self,
            _: Option<&str>,
        ) -> Result<Opened<Box<dyn AudioSink>>, SessionError> {
            Ok(Opened {
                stream: Box::new(self.speaker.clone()),
                description: "fake speaker".into(),
                counters: Arc::default(),
                warning: None,
            })
        }
    }

    struct Node {
        engine: Engine,
        events: Receiver<EngineEvent>,
        speaker: LiveSink,
        addr: SocketAddr,
    }

    fn node(network: &MemoryNetwork, name: &str, static_peers: Vec<SocketAddr>) -> Node {
        node_on(network, name, 3, 440.0, static_peers)
    }

    fn node_on(
        network: &MemoryNetwork,
        name: &str,
        channel: u8,
        frequency: f32,
        static_peers: Vec<SocketAddr>,
    ) -> Node {
        let speaker = LiveSink::new(FakeSink::new(RATE, RATE as usize / 5));
        let transport = Arc::new(network.bind(0));
        let addr = transport.local_addr();
        let fakes = Fakes {
            mic: Signal::Sine {
                frequency,
                amplitude: AMPLITUDE,
            },
            speaker: speaker.clone(),
        };
        let config = EngineConfig {
            display_name: name.into(),
            channel: ChannelId::try_from(channel).unwrap(),
            static_peers,
            ..EngineConfig::default()
        };
        let fast = Tuning {
            hello_every: Duration::from_millis(100),
            peer_timeout: Duration::from_secs(1),
        };
        let (engine, events) = Engine::start_with(config, fakes, transport, false, fast).unwrap();
        Node {
            engine,
            events,
            speaker,
            addr,
        }
    }

    fn level(speaker: &LiveSink) -> f32 {
        let heard = speaker.last(RATE as usize / 5);
        (heard.iter().map(|s| s * s).sum::<f32>() / heard.len() as f32).sqrt()
    }

    fn settle(speaker: &LiveSink, done: impl Fn(f32) -> bool) -> f32 {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let heard = level(speaker);
            if done(heard) || Instant::now() >= deadline {
                return heard;
            }
            after(50);
        }
    }

    fn pitch(speaker: &LiveSink) -> f32 {
        let heard = speaker.last(RATE as usize / 5);
        let crossings = heard
            .windows(2)
            .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
            .count();
        crossings as f32 / 2.0 / (heard.len() as f32 / RATE as f32)
    }

    fn hears_only(speaker: &LiveSink, frequency: f32) -> (f32, f32) {
        let tone = AMPLITUDE / 2.0_f32.sqrt();
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let (level, pitch) = (level(speaker), pitch(speaker));
            let right =
                (level - tone).abs() / tone < 0.3 && (pitch - frequency).abs() / frequency < 0.05;
            if right || Instant::now() >= deadline {
                return (level, pitch);
            }
            after(50);
        }
    }

    fn after(ms: u64) {
        thread::sleep(Duration::from_millis(ms));
    }

    fn wait_for(
        events: &Receiver<EngineEvent>,
        want: impl Fn(&EngineEvent) -> bool,
    ) -> EngineEvent {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let event = events.recv_timeout(left).unwrap();
            if want(&event) {
                return event;
            }
        }
    }

    #[test]
    fn commands_change_what_the_other_side_hears() {
        let network = MemoryNetwork::new();
        let b = node(&network, "B", Vec::new());
        let a = node(&network, "A", vec![b.addr]);
        let tone = AMPLITUDE / 2.0_f32.sqrt();

        after(3_000);
        let heard = level(&b.speaker);
        assert!(heard < 0.01, "B heard {heard} before A talked");

        a.engine.set_transmitting(true);
        let full = settle(&b.speaker, |heard| (heard - tone).abs() / tone < 0.3);
        assert!(
            (full - tone).abs() / tone < 0.3,
            "B heard {full}, A sent {tone}"
        );

        b.engine.set_volume(0.5);
        let half = settle(&b.speaker, |heard| (heard / full - 0.5).abs() < 0.1);
        assert!(
            (half / full - 0.5).abs() < 0.1,
            "half volume gave {half} of {full}"
        );

        b.engine.set_muted(true);
        let heard = settle(&b.speaker, |heard| heard < 0.01);
        assert!(heard < 0.01, "muted B heard {heard}");
        assert!(b.engine.snapshot().speaker.rms < 0.01);

        b.engine.set_muted(false);
        b.engine.set_channel(ChannelId::try_from(4).unwrap());
        let heard = settle(&b.speaker, |heard| heard < 0.01);
        assert!(heard < 0.01, "B on channel 4 heard {heard}");

        b.engine.set_channel(ChannelId::try_from(3).unwrap());
        let heard = settle(&b.speaker, |heard| heard > 0.1);
        assert!(heard > 0.1, "B back on channel 3 heard {heard}");

        a.engine.set_transmitting(false);
        let heard = settle(&b.speaker, |heard| heard < 0.01);
        assert!(heard < 0.01, "B heard {heard} after A let go");
    }

    #[test]
    fn audio_comes_back_after_a_channel_switch_longer_than_the_peer_timeout() {
        let network = MemoryNetwork::new();
        let b = node(&network, "B", Vec::new());
        let a = node(&network, "A", vec![b.addr]);
        a.engine.set_transmitting(true);
        let heard = settle(&b.speaker, |heard| heard > 0.1);
        assert!(heard > 0.1, "B heard {heard} before switching");

        b.engine.set_channel(ChannelId::try_from(4).unwrap());
        after(3_000);
        let heard = level(&b.speaker);
        assert!(heard < 0.01, "B on channel 4 heard {heard}");
        let away = b.engine.snapshot();
        assert_eq!(away.peers.len(), 1);
        assert_eq!(away.peers[0].channel.get(), 3);

        b.engine.set_channel(ChannelId::try_from(3).unwrap());
        let heard = settle(&b.speaker, |heard| heard > 0.1);
        assert!(heard > 0.1, "B back on channel 3 heard {heard}");
    }

    #[test]
    fn two_groups_on_different_channels_never_hear_each_other() {
        let network = MemoryNetwork::new();
        let tone = AMPLITUDE / 2.0_f32.sqrt();
        let a = node_on(&network, "A", 3, 440.0, Vec::new());
        let b = node_on(&network, "B", 3, 440.0, vec![a.addr]);
        let c = node_on(&network, "C", 5, 1_000.0, vec![a.addr, b.addr]);
        let d = node_on(&network, "D", 5, 1_000.0, vec![a.addr, b.addr, c.addr]);
        let close = |(level, pitch): (f32, f32), frequency: f32| {
            (level - tone).abs() / tone < 0.3 && (pitch - frequency).abs() / frequency < 0.05
        };

        a.engine.set_transmitting(true);
        c.engine.set_transmitting(true);
        let b_heard = hears_only(&b.speaker, 440.0);
        let d_heard = hears_only(&d.speaker, 1_000.0);
        assert!(
            close(b_heard, 440.0),
            "B heard {b_heard:?}, wanted A's 440 Hz alone"
        );
        assert!(
            close(d_heard, 1_000.0),
            "D heard {d_heard:?}, wanted C's 1000 Hz alone"
        );
        assert_eq!(b.engine.snapshot().peers.len(), 3);

        d.engine.set_channel(ChannelId::try_from(3).unwrap());
        let d_heard = hears_only(&d.speaker, 440.0);
        assert!(
            close(d_heard, 440.0),
            "D on channel 3 heard {d_heard:?}, wanted A's 440 Hz"
        );
        let b_heard = hears_only(&b.speaker, 440.0);
        assert!(close(b_heard, 440.0), "B heard {b_heard:?} after D joined");
    }

    #[test]
    fn events_and_the_snapshot_follow_a_conversation() {
        let network = MemoryNetwork::new();
        let b = node(&network, "B", Vec::new());
        let a = node(&network, "A", vec![b.addr]);
        let a_id = a.engine.id();

        let started = |e: &EngineEvent| matches!(e, EngineEvent::DeviceStarted { .. });
        wait_for(&b.events, started);
        wait_for(&b.events, started);
        let EngineEvent::PeerJoined(joined) =
            wait_for(&b.events, |e| matches!(e, EngineEvent::PeerJoined(_)))
        else {
            unreachable!("waited for a join");
        };
        assert_eq!(joined.id, a_id);
        assert_eq!(joined.name, "A");
        assert!(!joined.talking);

        a.engine.set_transmitting(true);
        assert_eq!(
            wait_for(&b.events, |e| matches!(e, EngineEvent::TalkStarted(_))),
            EngineEvent::TalkStarted(a_id)
        );
        after(300);
        let heard = b.engine.snapshot();
        assert_eq!(heard.peers.len(), 1);
        assert!(heard.peers[0].talking);
        assert!(heard.buffer_ms > 0);
        assert!(heard.speaker.rms > 0.1);
        assert!(heard.stats.packets_received > 0);
        let talker = a.engine.snapshot();
        assert!(talker.transmitting);
        assert!(talker.mic.rms > 0.1);
        assert!(talker.stats.packets_sent > 0);

        a.engine.set_transmitting(false);
        assert_eq!(
            wait_for(&b.events, |e| matches!(e, EngineEvent::TalkStopped(_))),
            EngineEvent::TalkStopped(a_id)
        );

        b.engine.set_volume(0.5);
        b.engine.set_muted(true);
        let settings = b.engine.snapshot();
        assert!((settings.volume - 0.5).abs() < f32::EPSILON);
        assert!(settings.muted);
        assert_eq!(settings.channel.get(), 3);

        drop(a);
        assert_eq!(
            wait_for(&b.events, |e| matches!(e, EngineEvent::PeerLeft(_))),
            EngineEvent::PeerLeft(a_id)
        );
        assert!(b.engine.snapshot().peers.is_empty());
    }

    #[test]
    fn twenty_starts_and_stops_leave_no_thread_behind() {
        let network = MemoryNetwork::new();
        let b = node(&network, "B", Vec::new());
        for round in 0..20 {
            let a = node(&network, "A", vec![b.addr]);
            let a_id = a.engine.id();
            assert!(matches!(
                wait_for(&b.events, |e| matches!(e, EngineEvent::PeerJoined(_))),
                EngineEvent::PeerJoined(peer) if peer.id == a_id
            ));
            a.engine.set_transmitting(true);
            after(50);
            let stopping = Instant::now();
            drop(a);
            let took = stopping.elapsed();
            assert!(
                took < Duration::from_secs(1),
                "round {round}: stopping took {took:?}"
            );
            assert_eq!(
                wait_for(&b.events, |e| matches!(e, EngineEvent::PeerLeft(_))),
                EngineEvent::PeerLeft(a_id)
            );
            let back = b
                .events
                .recv_timeout(Duration::from_millis(100))
                .ok()
                .filter(|e| matches!(e, EngineEvent::PeerJoined(peer) if peer.id == a_id));
            assert_eq!(back, None, "round {round}: A came back after leaving");
        }
        assert!(b.engine.snapshot().peers.is_empty());
    }

    #[test]
    fn a_taken_port_is_a_clear_error() {
        let taken = UdpTransport::bind(0).unwrap();
        let port = taken.local_addr().port();
        let Err(error) = Engine::start(EngineConfig {
            port,
            ..EngineConfig::default()
        }) else {
            unreachable!("the port is taken");
        };
        assert!(matches!(error, EngineError::Bind { .. }));
        assert!(
            error
                .to_string()
                .starts_with(&format!("couldn't open UDP port {port}"))
        );
    }

    #[test]
    fn ids_differ_between_engines() {
        let network = MemoryNetwork::new();
        let a = node(&network, "A", Vec::new());
        let b = node(&network, "B", Vec::new());
        assert_ne!(a.engine.id(), b.engine.id());
        assert_ne!(a.engine.port(), b.engine.port());
    }
}
