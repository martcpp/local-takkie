//! The API every UI uses. One [`Engine`] owns the audio, network and
//! discovery threads, and dropping it stops them all.

use std::hash::{BuildHasher, RandomState};
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use arc_swap::ArcSwap;
use crossbeam_channel::{Receiver, Sender, never, select, unbounded};
use takkie_core::peers::PeerEvent;
use takkie_core::{ChannelId, PeerId};
use thiserror::Error;

use crate::audio::devices::{DeviceList, list_devices};
use crate::audio::mix::{MixControls, MixShared};
use crate::audio::session::{
    AudioEvent, AudioSession, CpalOpener, DeviceOpener, SessionError, Wiring,
};
use crate::audio::tx::LevelMeter;
use crate::net::discovery::{Announcement, Browser, Discovery, DiscoveryError};
use crate::net::hello::{HELLO_EVERY, HelloThread};
use crate::net::peers::{PEER_TIMEOUT, PeerOutputs, PeerThread, Peers};
use crate::net::rx::{RxOutputs, RxThread};
use crate::net::send::{PacketSender, SendThread};
use crate::net::{Transport, UdpTransport};

const POLL_EVERY: Duration = Duration::from_millis(500);

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

enum Command {
    SetChannel(ChannelId),
}

/// A running walkie-talkie. Dropping it stops every thread.
///
/// ```no_run
/// use takkie_engine::{Engine, EngineConfig};
///
/// let engine = Engine::start(EngineConfig {
///     display_name: "Kitchen".into(),
///     ..EngineConfig::default()
/// })?;
/// engine.set_transmitting(true);
/// engine.set_transmitting(false);
/// # Ok::<(), takkie_engine::EngineError>(())
/// ```
pub struct Engine {
    id: PeerId,
    port: u16,
    transmitting: Arc<AtomicBool>,
    controls: Arc<MixControls>,
    hello: Option<HelloThread>,
    commands: Option<Sender<Command>>,
    control: Option<JoinHandle<()>>,
    _send: SendThread,
    _rx: RxThread,
    _peers: PeerThread,
}

impl Engine {
    /// Opens the UDP port and the audio devices, announces us over mDNS
    /// and starts every thread.
    ///
    /// # Errors
    /// [`EngineError`] if the port, a device, mDNS or a thread can't start.
    pub fn start(config: EngineConfig) -> Result<Self, EngineError> {
        let transport = UdpTransport::bind(config.port).map_err(|source| EngineError::Bind {
            port: config.port,
            source,
        })?;
        Self::start_with(config, CpalOpener, Arc::new(transport), true)
    }

    pub(crate) fn start_with<O: DeviceOpener + 'static>(
        config: EngineConfig,
        opener: O,
        transport: Arc<dyn Transport>,
        mdns: bool,
    ) -> Result<Self, EngineError> {
        let id = new_id();
        let port = transport.local_addr().port();
        let channel = Arc::new(AtomicU8::new(config.channel.get()));
        let targets = Arc::new(ArcSwap::from_pointee(Vec::new()));
        let transmitting = Arc::new(AtomicBool::new(false));
        let mix = MixShared {
            transmitting: Arc::clone(&transmitting),
            ..MixShared::default()
        };
        mix.controls.set_half_duplex(config.half_duplex);
        let controls = Arc::clone(&mix.controls);

        let (packets_in, packets) = unbounded();
        let (news_in, news) = unbounded();
        let (peer_events_in, peer_events) = unbounded();
        let peers = PeerThread::spawn(
            news,
            Peers::new(Arc::clone(&channel), Arc::clone(&targets), PEER_TIMEOUT),
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

        let (session, started) = AudioSession::start(
            opener,
            config.input_device.as_deref(),
            config.output_device.as_deref(),
            Wiring {
                transmitting: Arc::clone(&transmitting),
                level: Arc::new(LevelMeter::default()),
                tx_events,
                packets,
                mix,
            },
        )?;
        started.iter().for_each(report);

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
            channel,
        };
        let control = thread::Builder::new()
            .name("takkie-control".into())
            .spawn(move || control.run(&commands_out, peer_events))
            .map_err(EngineError::Thread)?;
        let hello = HelloThread::spawn(
            sender,
            &config.display_name,
            HELLO_EVERY,
            config.static_peers,
        )
        .map_err(EngineError::Thread)?;

        Ok(Self {
            id,
            port,
            transmitting,
            controls,
            hello: Some(hello),
            commands: Some(commands),
            control: Some(control),
            _send: send,
            _rx: rx,
            _peers: peers,
        })
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
        self.transmitting.store(on, Relaxed);
    }

    /// Moves to another channel.
    pub fn set_channel(&self, channel: ChannelId) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::SetChannel(channel));
        }
    }

    /// Silences what we hear.
    pub fn set_muted(&self, muted: bool) {
        self.controls.set_muted(muted);
    }

    /// Playback volume: 1.0 is unchanged, clamped to 0.0..=2.0.
    pub fn set_volume(&self, volume: f32) {
        self.controls.set_volume(volume);
    }

    /// The microphones and speakers on this machine.
    ///
    /// # Errors
    /// [`EngineError::Audio`] if the audio system can't be asked.
    pub fn list_devices() -> Result<DeviceList, EngineError> {
        list_devices().map_err(|error| EngineError::Audio(error.into()))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Bye has to go out before the mDNS goodbye, which the control thread owns.
        drop(self.hello.take());
        drop(self.commands.take());
        if let Some(control) = self.control.take() {
            let _ = control.join();
        }
    }
}

struct Control<O: DeviceOpener> {
    session: AudioSession<O>,
    mdns: Option<(Browser, Discovery)>,
    channel: Arc<AtomicU8>,
}

impl<O: DeviceOpener> Control<O> {
    fn run(mut self, commands: &Receiver<Command>, mut peer_events: Receiver<PeerEvent>) {
        let mut next_poll = Instant::now() + POLL_EVERY;
        loop {
            let wait = next_poll.saturating_duration_since(Instant::now());
            select! {
                recv(commands) -> command => match command {
                    Ok(Command::SetChannel(channel)) => self.channel.store(channel.get(), Relaxed),
                    Err(_) => break,
                },
                recv(peer_events) -> event => match event {
                    Ok(event) => log::debug!("{event:?}"),
                    Err(_) => peer_events = never(),
                },
                default(wait) => {}
            }
            let now = Instant::now();
            if now >= next_poll {
                self.session.poll(now).iter().for_each(report);
                next_poll = now + POLL_EVERY;
            }
        }
        drop(self.mdns.take());
    }
}

fn report(event: &AudioEvent) {
    match event {
        AudioEvent::Started {
            direction,
            description,
        } => log::info!("{direction} started: {description}"),
        AudioEvent::Warning(warning) => log::warn!("{warning}"),
        AudioEvent::DeviceLost(direction) => log::warn!("{direction} lost, retrying the default"),
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

    fn engine(
        network: &MemoryNetwork,
        static_peers: Vec<SocketAddr>,
    ) -> (Engine, LiveSink, SocketAddr) {
        let speaker = LiveSink::new(FakeSink::new(RATE, RATE as usize / 5));
        let transport = Arc::new(network.bind(0));
        let addr = transport.local_addr();
        let fakes = Fakes {
            mic: Signal::Sine {
                frequency: 440.0,
                amplitude: AMPLITUDE,
            },
            speaker: speaker.clone(),
        };
        let config = EngineConfig {
            channel: ChannelId::try_from(3).unwrap(),
            static_peers,
            ..EngineConfig::default()
        };
        let engine = Engine::start_with(config, fakes, transport, false).unwrap();
        (engine, speaker, addr)
    }

    fn level(speaker: &LiveSink) -> f32 {
        let heard = speaker.last(RATE as usize / 5);
        (heard.iter().map(|s| s * s).sum::<f32>() / heard.len() as f32).sqrt()
    }

    fn after(ms: u64) {
        thread::sleep(Duration::from_millis(ms));
    }

    #[test]
    fn commands_change_what_the_other_side_hears() {
        let network = MemoryNetwork::new();
        let (b, speaker, b_addr) = engine(&network, Vec::new());
        let (a, _, _) = engine(&network, vec![b_addr]);
        let tone = AMPLITUDE / 2.0_f32.sqrt();

        after(3_000);
        assert!(
            level(&speaker) < 0.01,
            "B heard {} before A talked",
            level(&speaker)
        );

        a.set_transmitting(true);
        after(500);
        let full = level(&speaker);
        assert!(
            (full - tone).abs() / tone < 0.3,
            "B heard {full}, A sent {tone}"
        );

        b.set_volume(0.5);
        after(400);
        let half = level(&speaker);
        assert!(
            (half / full - 0.5).abs() < 0.1,
            "half volume gave {half} of {full}"
        );

        b.set_muted(true);
        after(400);
        assert!(level(&speaker) < 0.01, "muted B heard {}", level(&speaker));

        b.set_muted(false);
        b.set_channel(ChannelId::try_from(4).unwrap());
        after(400);
        assert!(
            level(&speaker) < 0.01,
            "B on channel 4 heard {}",
            level(&speaker)
        );

        b.set_channel(ChannelId::try_from(3).unwrap());
        after(400);
        assert!(
            level(&speaker) > 0.1,
            "B back on channel 3 heard {}",
            level(&speaker)
        );

        a.set_transmitting(false);
        after(400);
        assert!(
            level(&speaker) < 0.01,
            "B heard {} after A let go",
            level(&speaker)
        );
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
        let (a, _, _) = engine(&network, Vec::new());
        let (b, _, _) = engine(&network, Vec::new());
        assert_ne!(a.id(), b.id());
        assert_ne!(a.port(), b.port());
    }
}
