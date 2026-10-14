//! Keeps audio running: owns the tx and mix threads, and when a device goes
//! away, reports it and retries the default device every few seconds.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use thiserror::Error;

use super::config::{ConfigError, input_config, output_config};
use super::devices::{DeviceError, Direction, find_device};
use super::io::{AudioSink, AudioSource};
use super::mix::{MixError, MixInput, MixShared, MixThread};
use super::stream::{Counters, CpalSink, CpalSource, OpenError};
use super::tx::{LevelMeter, TxError, TxEvent, TxThread};
use super::voice::VoiceControls;

const RETRY: Duration = Duration::from_secs(2);

/// Why audio couldn't start or restart.
#[derive(Debug, Error)]
pub enum SessionError {
    /// No such device.
    #[error(transparent)]
    Device(#[from] DeviceError),
    /// The device's configs couldn't be read.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The stream wouldn't open.
    #[error(transparent)]
    Open(#[from] OpenError),
    /// The tx thread wouldn't start.
    #[error(transparent)]
    Tx(#[from] TxError),
    /// The mix thread wouldn't start.
    #[error(transparent)]
    Mix(#[from] MixError),
}

/// A device that's open and running.
pub struct Opened<T> {
    /// The stream.
    pub stream: T,
    /// Name and settings, for the UI.
    pub description: String,
    /// Its callback counters, including the `lost` flag.
    pub counters: Arc<Counters>,
    /// Set when the wanted name fell back to the default.
    pub warning: Option<String>,
}

/// Opens devices by name; `None` means the system default.
pub trait DeviceOpener: Send {
    /// Opens a microphone.
    ///
    /// # Errors
    /// [`SessionError`] if it can't be opened.
    fn open_input(
        &mut self,
        name: Option<&str>,
    ) -> Result<Opened<Box<dyn AudioSource>>, SessionError>;

    /// Opens a speaker.
    ///
    /// # Errors
    /// [`SessionError`] if it can't be opened.
    fn open_output(
        &mut self,
        name: Option<&str>,
    ) -> Result<Opened<Box<dyn AudioSink>>, SessionError>;

    /// The name of the system's default device right now, if it can be
    /// told. The session uses it to follow a headset being plugged in.
    fn default_device(&mut self, _direction: Direction) -> Option<String> {
        None
    }
}

/// Real devices through cpal.
#[derive(Debug, Default)]
pub struct CpalOpener;

impl DeviceOpener for CpalOpener {
    fn open_input(
        &mut self,
        name: Option<&str>,
    ) -> Result<Opened<Box<dyn AudioSource>>, SessionError> {
        let chosen = find_device(Direction::Input, name)?;
        let source = CpalSource::open(&chosen.device, &input_config(&chosen.device)?)?;
        Ok(Opened {
            description: format!("{} ({})", chosen.name, source.settings()),
            counters: source.counters(),
            stream: Box::new(source),
            warning: chosen.warning,
        })
    }

    fn open_output(
        &mut self,
        name: Option<&str>,
    ) -> Result<Opened<Box<dyn AudioSink>>, SessionError> {
        let chosen = find_device(Direction::Output, name)?;
        let sink = CpalSink::open(&chosen.device, &output_config(&chosen.device)?)?;
        Ok(Opened {
            description: format!("{} ({})", chosen.name, sink.settings()),
            counters: sink.counters(),
            stream: Box::new(sink),
            warning: chosen.warning,
        })
    }

    fn default_device(&mut self, direction: Direction) -> Option<String> {
        find_device(direction, None).ok().map(|chosen| chosen.name)
    }
}

/// What the session tells the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioEvent {
    /// A device is running.
    Started {
        /// Microphone or speaker.
        direction: Direction,
        /// Name and settings.
        description: String,
    },
    /// Something worth showing, like a name that fell back to the default.
    Warning(String),
    /// A device went away; the default will be retried.
    DeviceLost(Direction),
}

/// Channels and flags that stay the same across device restarts.
pub struct Wiring {
    /// Push-to-talk, shared by both sides.
    pub transmitting: Arc<AtomicBool>,
    /// The mic level.
    pub level: Arc<LevelMeter>,
    /// Switches for the voice processing.
    pub voice: Arc<VoiceControls>,
    /// Encoded packets out.
    pub tx_events: Sender<TxEvent>,
    /// Packets in, and senders leaving.
    pub packets: Receiver<MixInput>,
    /// Volume, mute, half-duplex and lost-frame counts.
    pub mix: MixShared,
}

struct Side<T> {
    thread: Option<T>,
    counters: Option<Arc<Counters>>,
    last_try: Option<Instant>,
    // The system default's name when this was opened on it; `None` for a
    // device the user picked, which is never swapped for another.
    default: Option<String>,
}

impl<T> Side<T> {
    fn running(thread: T, counters: Arc<Counters>) -> Self {
        Self {
            thread: Some(thread),
            counters: Some(counters),
            last_try: None,
            default: None,
        }
    }

    /// Whether the system default is now a different device from the one
    /// this side runs on.
    fn left_behind(&self, default_now: Option<&str>) -> bool {
        self.thread.is_some()
            && matches!((self.default.as_deref(), default_now), (Some(was), Some(now)) if was != now)
    }

    fn close(&mut self) {
        self.thread = None;
        self.last_try = None;
    }

    fn lost(&self) -> bool {
        self.thread.is_some() && self.counters.as_ref().is_some_and(|c| c.lost.load(Relaxed))
    }

    fn due(&self, now: Instant) -> bool {
        self.thread.is_none()
            && self
                .last_try
                .is_none_or(|at| now.saturating_duration_since(at) >= RETRY)
    }
}

/// The tx and mix threads, kept running across device loss.
pub struct AudioSession<O: DeviceOpener> {
    opener: O,
    wiring: Wiring,
    input: Side<TxThread>,
    output: Side<MixThread>,
    defaults_checked: Option<Instant>,
}

/// How often to ask the system which devices are its defaults.
const DEFAULTS_EVERY: Duration = Duration::from_secs(2);

impl<O: DeviceOpener> AudioSession<O> {
    /// Opens both devices and starts the threads.
    ///
    /// # Errors
    /// [`SessionError`] if a device or thread can't start.
    pub fn start(
        mut opener: O,
        input: Option<&str>,
        output: Option<&str>,
        wiring: Wiring,
    ) -> Result<(Self, Vec<AudioEvent>), SessionError> {
        let mut events = Vec::new();
        let sink = opener.open_output(output)?;
        let mut mix = spawn_mix(&wiring, sink, &mut events)?;
        let source = opener.open_input(input)?;
        let mut tx = spawn_tx(&wiring, source, &mut events)?;
        if output.is_none() {
            mix.default = opener.default_device(Direction::Output);
        }
        if input.is_none() {
            tx.default = opener.default_device(Direction::Input);
        }
        Ok((
            Self {
                opener,
                wiring,
                input: tx,
                output: mix,
                defaults_checked: None,
            },
            events,
        ))
    }

    /// Notices lost devices and retries the default ones. Call it about
    /// once a second.
    pub fn poll(&mut self, now: Instant) -> Vec<AudioEvent> {
        let mut events = Vec::new();
        self.follow_defaults(now);
        for (side, counters) in [
            ("microphone", &self.input.counters),
            ("speaker", &self.output.counters),
        ] {
            if counters
                .as_ref()
                .is_some_and(|c| c.realtime_denied.swap(false, Relaxed))
            {
                tracing::info!("the system gave the {side} no real-time or low-latency path");
            }
        }
        if self.input.lost() {
            self.input.thread = None;
            events.push(AudioEvent::DeviceLost(Direction::Input));
        }
        if self.output.lost() {
            self.output.thread = None;
            events.push(AudioEvent::DeviceLost(Direction::Output));
        }
        if self.output.due(now) {
            self.output.last_try = Some(now);
            if let Ok(sink) = self.opener.open_output(None)
                && let Ok(side) = spawn_mix(&self.wiring, sink, &mut events)
            {
                self.output = side;
                self.output.default = self.opener.default_device(Direction::Output);
            }
        }
        if self.input.due(now) {
            self.input.last_try = Some(now);
            if let Ok(source) = self.opener.open_input(None)
                && let Ok(side) = spawn_tx(&self.wiring, source, &mut events)
            {
                self.input = side;
                self.input.default = self.opener.default_device(Direction::Input);
            }
        }
        events
    }

    // Plugging a headset in doesn't break the stream on the speakers, so
    // nothing is "lost": the system just starts preferring another device.
    fn follow_defaults(&mut self, now: Instant) {
        if self
            .defaults_checked
            .is_some_and(|at| now.saturating_duration_since(at) < DEFAULTS_EVERY)
        {
            return;
        }
        self.defaults_checked = Some(now);
        if self.output.default.is_some() {
            let default = self.opener.default_device(Direction::Output);
            if self.output.left_behind(default.as_deref()) {
                tracing::info!(?default, "the default speaker changed; moving to it");
                self.output.close();
            }
        }
        if self.input.default.is_some() {
            let default = self.opener.default_device(Direction::Input);
            if self.input.left_behind(default.as_deref()) {
                tracing::info!(?default, "the default microphone changed; moving to it");
                self.input.close();
            }
        }
    }

    /// Closes both devices, so the next [`poll`](Self::poll) opens the
    /// current defaults. For when the system moved sound somewhere else,
    /// like a headset being plugged in.
    pub fn reopen(&mut self) {
        self.input.close();
        self.output.close();
    }

    /// The running mic's counters.
    #[must_use]
    pub fn input_counters(&self) -> Option<Arc<Counters>> {
        self.input.counters.clone()
    }

    /// The running speaker's counters.
    #[must_use]
    pub fn output_counters(&self) -> Option<Arc<Counters>> {
        self.output.counters.clone()
    }
}

fn spawn_tx(
    wiring: &Wiring,
    opened: Opened<Box<dyn AudioSource>>,
    events: &mut Vec<AudioEvent>,
) -> Result<Side<TxThread>, SessionError> {
    let thread = TxThread::spawn(
        opened.stream,
        Arc::clone(&wiring.transmitting),
        Arc::clone(&wiring.level),
        Arc::clone(&wiring.voice),
        Arc::clone(&wiring.mix.tap),
        wiring.tx_events.clone(),
    )?;
    events.extend(opened.warning.map(AudioEvent::Warning));
    events.push(AudioEvent::Started {
        direction: Direction::Input,
        description: opened.description,
    });
    Ok(Side::running(thread, opened.counters))
}

fn spawn_mix(
    wiring: &Wiring,
    opened: Opened<Box<dyn AudioSink>>,
    events: &mut Vec<AudioEvent>,
) -> Result<Side<MixThread>, SessionError> {
    let thread = MixThread::spawn(wiring.packets.clone(), opened.stream, wiring.mix.clone())?;
    events.extend(opened.warning.map(AudioEvent::Warning));
    events.push(AudioEvent::Started {
        direction: Direction::Output,
        description: opened.description,
    });
    Ok(Side::running(thread, opened.counters))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crossbeam_channel::unbounded;

    use super::*;
    use crate::audio::fake::{FakeSink, FakeSource, Signal};

    type Opens = Vec<(Direction, Option<String>)>;

    #[derive(Clone, Default)]
    struct Log {
        opened: Arc<Mutex<Opens>>,
        counters: Arc<Mutex<Vec<Arc<Counters>>>>,
        failures: Arc<Mutex<u32>>,
        default_output: Arc<Mutex<Option<String>>>,
    }

    struct FakeOpener(Log);

    impl FakeOpener {
        fn open(
            &mut self,
            direction: Direction,
            name: Option<&str>,
        ) -> Result<(String, Arc<Counters>), SessionError> {
            let mut failures = self.0.failures.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                return Err(DeviceError::Missing(direction).into());
            }
            let counters = Arc::new(Counters::default());
            self.0
                .opened
                .lock()
                .unwrap()
                .push((direction, name.map(String::from)));
            self.0.counters.lock().unwrap().push(Arc::clone(&counters));
            Ok((format!("fake {direction}"), counters))
        }
    }

    impl DeviceOpener for FakeOpener {
        fn open_input(
            &mut self,
            name: Option<&str>,
        ) -> Result<Opened<Box<dyn AudioSource>>, SessionError> {
            let (description, counters) = self.open(Direction::Input, name)?;
            Ok(Opened {
                stream: Box::new(FakeSource::new(48_000, Signal::Silence)),
                description,
                counters,
                warning: None,
            })
        }

        fn open_output(
            &mut self,
            name: Option<&str>,
        ) -> Result<Opened<Box<dyn AudioSink>>, SessionError> {
            let (description, counters) = self.open(Direction::Output, name)?;
            Ok(Opened {
                stream: Box::new(FakeSink::new(48_000, 9_600)),
                description,
                counters,
                warning: None,
            })
        }

        fn default_device(&mut self, direction: Direction) -> Option<String> {
            match direction {
                Direction::Output => self.0.default_output.lock().unwrap().clone(),
                Direction::Input => None,
            }
        }
    }

    fn start(log: &Log) -> (AudioSession<FakeOpener>, Vec<AudioEvent>) {
        let (tx_events, _) = unbounded();
        let (_, packets) = unbounded();
        let wiring = Wiring {
            transmitting: Arc::default(),
            level: Arc::default(),
            voice: Arc::default(),
            tx_events,
            packets,
            mix: MixShared::default(),
        };
        AudioSession::start(
            FakeOpener(log.clone()),
            Some("headset"),
            Some("headset"),
            wiring,
        )
        .unwrap()
    }

    fn started(direction: Direction) -> AudioEvent {
        AudioEvent::Started {
            direction,
            description: format!("fake {direction}"),
        }
    }

    #[test]
    fn starts_both_devices_by_name() {
        let log = Log::default();
        let (_, events) = start(&log);
        assert_eq!(
            events,
            [started(Direction::Output), started(Direction::Input)]
        );
        let opened = log.opened.lock().unwrap().clone();
        assert_eq!(
            opened,
            [
                (Direction::Output, Some("headset".into())),
                (Direction::Input, Some("headset".into())),
            ]
        );
    }

    #[test]
    fn a_lost_mic_is_reported_then_reopened_on_the_default() {
        let log = Log::default();
        let (mut session, _) = start(&log);
        let t0 = Instant::now();
        assert!(session.poll(t0).is_empty());

        log.counters.lock().unwrap()[1].lost.store(true, Relaxed);
        let events = session.poll(t0);
        assert_eq!(
            events,
            [
                AudioEvent::DeviceLost(Direction::Input),
                started(Direction::Input)
            ]
        );
        assert_eq!(
            log.opened.lock().unwrap().last(),
            Some(&(Direction::Input, None))
        );
        assert!(session.poll(t0 + Duration::from_secs(10)).is_empty());
    }

    #[test]
    fn retries_every_two_seconds_until_a_device_is_back() {
        let log = Log::default();
        let (mut session, _) = start(&log);
        let t0 = Instant::now();
        *log.failures.lock().unwrap() = 2;
        log.counters.lock().unwrap()[0].lost.store(true, Relaxed);

        assert_eq!(
            session.poll(t0),
            [AudioEvent::DeviceLost(Direction::Output)]
        );
        assert!(session.output_counters().is_some());
        assert!(session.poll(t0 + Duration::from_secs(1)).is_empty());
        assert!(session.poll(t0 + Duration::from_secs(2)).is_empty());
        assert_eq!(
            session.poll(t0 + Duration::from_secs(4)),
            [started(Direction::Output)]
        );
        assert_eq!(log.opened.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_failed_start_is_an_error() {
        let log = Log::default();
        *log.failures.lock().unwrap() = 1;
        let (tx_events, _) = unbounded();
        let (_, packets) = unbounded();
        let wiring = Wiring {
            transmitting: Arc::default(),
            level: Arc::default(),
            voice: Arc::default(),
            tx_events,
            packets,
            mix: MixShared::default(),
        };
        assert!(AudioSession::start(FakeOpener(log), None, None, wiring).is_err());
    }

    fn wiring() -> Wiring {
        let (tx_events, _) = unbounded();
        let (_, packets) = unbounded();
        Wiring {
            transmitting: Arc::default(),
            level: Arc::default(),
            voice: Arc::default(),
            tx_events,
            packets,
            mix: MixShared::default(),
        }
    }

    fn opened(log: &Log, direction: Direction) -> usize {
        let opened = log.opened.lock().unwrap();
        opened.iter().filter(|(d, _)| *d == direction).count()
    }

    #[test]
    fn the_speaker_follows_the_system_default_to_a_headset_and_back() {
        let log = Log::default();
        *log.default_output.lock().unwrap() = Some("Speakers".into());
        let wiring = wiring();
        let (mut session, _) =
            AudioSession::start(FakeOpener(log.clone()), None, None, wiring).unwrap();
        let t0 = Instant::now();
        session.poll(t0);
        assert_eq!(opened(&log, Direction::Output), 1);

        *log.default_output.lock().unwrap() = Some("Headphones".into());
        session.poll(t0 + Duration::from_secs(3));
        assert_eq!(opened(&log, Direction::Output), 2);
        assert_eq!(opened(&log, Direction::Input), 1);

        session.poll(t0 + Duration::from_secs(6));
        assert_eq!(opened(&log, Direction::Output), 2);

        *log.default_output.lock().unwrap() = Some("Speakers".into());
        session.poll(t0 + Duration::from_secs(9));
        assert_eq!(opened(&log, Direction::Output), 3);
    }

    #[test]
    fn a_speaker_the_user_picked_is_not_swapped_for_the_default() {
        let log = Log::default();
        *log.default_output.lock().unwrap() = Some("Speakers".into());
        let wiring = wiring();
        let (mut session, _) =
            AudioSession::start(FakeOpener(log.clone()), None, Some("USB"), wiring).unwrap();
        *log.default_output.lock().unwrap() = Some("Headphones".into());
        session.poll(Instant::now() + Duration::from_secs(3));
        assert_eq!(opened(&log, Direction::Output), 1);
    }
}
