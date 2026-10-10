//! The sending side: mic samples to 48 kHz frames, gated by push-to-talk and
//! encoded with Opus, on a thread of its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::Sender;
use takkie_core::dsp::Level;

use super::codec::{CodecError, VoiceEncoder};
use super::io::AudioSource;
use super::resample::{FRAME, ResampleError, Resampler};
use crate::threads::{STOP_WITHIN, join_within};

/// Where a sent frame sits in the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mark {
    /// Position of its first sample, in 48 kHz samples since start.
    pub timestamp: u32,
    /// Last frame of a push-to-talk press.
    pub end: bool,
}

/// One encoded 20 ms frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxPacket {
    /// The Opus packet.
    pub payload: Vec<u8>,
    /// Position of its first sample, in 48 kHz samples since start.
    pub timestamp: u32,
    /// Last packet of a push-to-talk press.
    pub end: bool,
}

/// What the tx thread reports.
#[derive(Debug)]
pub enum TxEvent {
    /// A packet to send.
    Packet(TxPacket),
    /// A frame that couldn't be encoded; the thread carries on.
    Error(CodecError),
}

/// Cuts mic input into whole 48 kHz frames.
pub struct Framer {
    resampler: Resampler,
    pending: Vec<f32>,
    filled: usize,
    frame: Vec<f32>,
}

impl Framer {
    /// A framer for a mic at `device_rate`.
    ///
    /// # Errors
    /// [`ResampleError`] if the rate can't be converted.
    pub fn new(device_rate: u32) -> Result<Self, ResampleError> {
        let resampler = Resampler::capture(device_rate)?;
        let pending = vec![0.0; resampler.input_max()];
        let frame = vec![0.0; resampler.output_max()];
        Ok(Self {
            resampler,
            pending,
            filled: 0,
            frame,
        })
    }

    /// The next frame, once enough input has arrived.
    ///
    /// # Errors
    /// [`ResampleError`] if resampling fails.
    pub fn next(&mut self, source: &mut dyn AudioSource) -> Result<Option<&[f32]>, ResampleError> {
        let needed = self.resampler.input_needed();
        if self.filled < needed {
            let room = &mut self.pending[self.filled..needed];
            self.filled += source.read(room);
        }
        if self.filled < needed {
            return Ok(None);
        }
        let written = self
            .resampler
            .process(&self.pending[..needed], &mut self.frame)?;
        self.filled = 0;
        Ok(Some(&self.frame[..written]))
    }
}

/// The mic level, readable from the UI without locks.
///
/// Relaxed is enough: rms and peak are independent display values, so a
/// reader mixing two frames only shows a slightly stale meter.
#[derive(Debug, Default)]
pub struct LevelMeter {
    rms: AtomicU32,
    peak: AtomicU32,
}

impl LevelMeter {
    /// Publishes a frame's level.
    pub fn set(&self, level: Level) {
        self.rms.store(level.rms.to_bits(), Relaxed);
        self.peak.store(level.peak.to_bits(), Relaxed);
    }

    /// The latest level.
    #[must_use]
    pub fn get(&self) -> Level {
        Level {
            rms: f32::from_bits(self.rms.load(Relaxed)),
            peak: f32::from_bits(self.peak.load(Relaxed)),
        }
    }
}

/// Decides which frames go out while push-to-talk is held.
#[derive(Debug, Default)]
pub struct Gate {
    open: bool,
    timestamp: u32,
}

impl Gate {
    /// Lets frames through while `transmitting`, plus one final frame marked
    /// `end` after release. Every frame moves the timestamp on.
    pub fn pass(&mut self, transmitting: bool) -> Option<Mark> {
        let timestamp = self.timestamp;
        self.timestamp = self.timestamp.wrapping_add(FRAME as u32);
        let end = self.open && !transmitting;
        self.open = transmitting;
        (transmitting || end).then_some(Mark { timestamp, end })
    }
}

/// The running tx thread. Dropping it stops the thread.
pub struct TxThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TxThread {
    /// Starts reading `source`, updating `level` every frame and sending
    /// packets while `transmitting` is set.
    ///
    /// # Errors
    /// [`TxError`] for an unusable mic rate, an encoder that won't start, or
    /// a thread that won't start.
    pub fn spawn(
        mut source: Box<dyn AudioSource>,
        transmitting: Arc<AtomicBool>,
        level: Arc<LevelMeter>,
        events: Sender<TxEvent>,
    ) -> Result<Self, TxError> {
        let mut framer = Framer::new(source.sample_rate())?;
        let mut encoder = VoiceEncoder::new()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("takkie-tx".into())
            .spawn(move || {
                let mut gate = Gate::default();
                while !stopping.load(Relaxed) {
                    match framer.next(source.as_mut()) {
                        Ok(Some(samples)) => {
                            level.set(Level::of(samples));
                            let Some(mark) = gate.pass(transmitting.load(Relaxed)) else {
                                continue;
                            };
                            let event = match encoder.encode(samples) {
                                Ok(payload) => TxEvent::Packet(TxPacket {
                                    payload: payload.to_vec(),
                                    timestamp: mark.timestamp,
                                    end: mark.end,
                                }),
                                Err(error) => TxEvent::Error(error),
                            };
                            if events.send(event).is_err() {
                                return;
                            }
                        }
                        Ok(None) => thread::sleep(Duration::from_millis(5)),
                        Err(error) => {
                            tracing::error!("tx stopped: {error}");
                            return;
                        }
                    }
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for TxThread {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(handle) = self.handle.take() {
            join_within(handle, STOP_WITHIN);
        }
    }
}

/// Why the tx thread couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum TxError {
    /// The mic rate can't be converted.
    #[error(transparent)]
    Resample(#[from] ResampleError),
    /// The encoder won't start.
    #[error(transparent)]
    Codec(#[from] CodecError),
    /// The OS wouldn't start the thread.
    #[error("couldn't start the tx thread: {0}")]
    Thread(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use crossbeam_channel::unbounded;
    use takkie_core::ptt::{PttController, PttInput, PttMode};

    use super::*;
    use crate::audio::fake::{FakeSource, Signal};

    fn tone(rate: u32) -> FakeSource {
        FakeSource::new(
            rate,
            Signal::Sine {
                frequency: 440.0,
                amplitude: 0.5,
            },
        )
    }

    #[test]
    fn framer_waits_for_a_whole_frame() {
        let mut source = tone(48_000);
        let mut framer = Framer::new(48_000).unwrap();
        source.advance(500);
        assert!(framer.next(&mut source).unwrap().is_none());
        source.advance(460);
        assert_eq!(
            framer.next(&mut source).unwrap().map(<[f32]>::len),
            Some(FRAME)
        );
        assert!(framer.next(&mut source).unwrap().is_none());
    }

    #[test]
    fn framer_resamples_44k1_into_960_sample_frames() {
        let mut source = tone(44_100);
        let mut framer = Framer::new(44_100).unwrap();
        source.advance_ms(1_000);
        let mut frames = 0;
        while let Some(frame) = framer.next(&mut source).unwrap() {
            assert_eq!(frame.len(), FRAME);
            frames += 1;
        }
        assert!((48..=50).contains(&frames));
    }

    #[test]
    fn gate_sends_only_while_transmitting_and_marks_the_end() {
        let t0 = Instant::now();
        let mut ptt = PttController::new(PttMode::Hold);
        let mut gate = Gate::default();
        let mut sent = Vec::new();
        for i in 0..10 {
            let input = match i {
                2 => Some(PttInput::Press),
                6 => Some(PttInput::Release),
                _ => None,
            };
            if let Some(input) = input {
                ptt.handle(input, t0);
            }
            if let Some(mark) = gate.pass(ptt.is_transmitting()) {
                sent.push((mark.timestamp, mark.end));
            }
        }
        let f = FRAME as u32;
        assert_eq!(
            sent,
            [
                (2 * f, false),
                (3 * f, false),
                (4 * f, false),
                (5 * f, false),
                (6 * f, true)
            ]
        );
    }

    #[test]
    fn the_thread_sends_opus_packets_while_transmitting_and_stops_on_drop() {
        let mut source = tone(48_000);
        source.advance_ms(200);
        let transmitting = Arc::new(AtomicBool::new(true));
        let (sender, events) = unbounded();
        let level = Arc::new(LevelMeter::default());
        let thread =
            TxThread::spawn(Box::new(source), Arc::clone(&transmitting), level, sender).unwrap();
        let packets: Vec<TxPacket> = (0..10)
            .map(
                |_| match events.recv_timeout(Duration::from_secs(5)).unwrap() {
                    TxEvent::Packet(packet) => Some(packet),
                    TxEvent::Error(_) => None,
                },
            )
            .collect::<Option<_>>()
            .unwrap();
        drop(thread);
        assert!(
            packets
                .iter()
                .all(|p| (2..=1275).contains(&p.payload.len()) && !p.end)
        );
        assert_eq!(packets[9].timestamp, 9 * FRAME as u32);
        assert!(events.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn meter_round_trips_a_level() {
        let meter = LevelMeter::default();
        assert_eq!(meter.get(), Level::default());
        let level = Level {
            rms: 0.25,
            peak: 0.75,
        };
        meter.set(level);
        assert_eq!(meter.get(), level);
    }

    #[test]
    fn level_follows_the_input_even_when_not_transmitting() {
        let mut source = tone(48_000);
        source.advance_ms(100);
        let (sender, events) = unbounded();
        let level = Arc::new(LevelMeter::default());
        let thread = TxThread::spawn(
            Box::new(source),
            Arc::new(AtomicBool::new(false)),
            Arc::clone(&level),
            sender,
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while level.get().peak == 0.0 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        drop(thread);
        let measured = level.get();
        assert!((measured.peak - 0.5).abs() < 0.02, "peak {}", measured.peak);
        assert!(
            (measured.rms - 0.5 / 2.0_f32.sqrt()).abs() < 0.02,
            "rms {}",
            measured.rms
        );
        assert!(events.try_recv().is_err());
    }
}
