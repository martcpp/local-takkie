//! The sending side: mic samples to 48 kHz frames, gated by push-to-talk,
//! on a thread of its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::Sender;

use super::io::AudioSource;
use super::resample::{FRAME, ResampleError, Resampler};

/// A 20 ms frame to send.
#[derive(Clone, Debug, PartialEq)]
pub struct TxFrame {
    /// 960 mono samples at 48 kHz.
    pub samples: Vec<f32>,
    /// Position of the first sample, in 48 kHz samples since start.
    pub timestamp: u32,
    /// Last frame of a push-to-talk press.
    pub end: bool,
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

/// Decides which frames go out while push-to-talk is held.
#[derive(Debug, Default)]
pub struct Gate {
    open: bool,
    timestamp: u32,
}

impl Gate {
    /// Passes `samples` on while `transmitting`, plus one final frame marked
    /// `end` after release. Every frame moves the timestamp on.
    pub fn pass(&mut self, samples: &[f32], transmitting: bool) -> Option<TxFrame> {
        let timestamp = self.timestamp;
        self.timestamp = self.timestamp.wrapping_add(FRAME as u32);
        let end = self.open && !transmitting;
        let send = transmitting || end;
        self.open = transmitting;
        send.then(|| TxFrame {
            samples: samples.to_vec(),
            timestamp,
            end,
        })
    }
}

/// The running tx thread. Dropping it stops the thread.
pub struct TxThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TxThread {
    /// Starts reading `source` and sending frames while `transmitting` is set.
    ///
    /// # Errors
    /// [`TxError`] for an unusable mic rate, or if the thread can't start.
    pub fn spawn(
        mut source: Box<dyn AudioSource>,
        transmitting: Arc<AtomicBool>,
        frames: Sender<TxFrame>,
    ) -> Result<Self, TxError> {
        let mut framer = Framer::new(source.sample_rate())?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("takkie-tx".into())
            .spawn(move || {
                let mut gate = Gate::default();
                while !stopping.load(Relaxed) {
                    match framer.next(source.as_mut()) {
                        Ok(Some(samples)) => {
                            if let Some(frame) = gate.pass(samples, transmitting.load(Relaxed))
                                && frames.send(frame).is_err()
                            {
                                return;
                            }
                        }
                        Ok(None) => thread::sleep(Duration::from_millis(5)),
                        Err(error) => {
                            log::error!("tx stopped: {error}");
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
            let _ = handle.join();
        }
    }
}

/// Why the tx thread couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum TxError {
    /// The mic rate can't be converted.
    #[error(transparent)]
    Resample(#[from] ResampleError),
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
        let frame = [0.1; FRAME];
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
            if let Some(out) = gate.pass(&frame, ptt.is_transmitting()) {
                sent.push((out.timestamp, out.end));
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
    fn the_thread_sends_frames_while_transmitting_and_stops_on_drop() {
        let mut source = tone(48_000);
        source.advance_ms(200);
        let transmitting = Arc::new(AtomicBool::new(true));
        let (sender, frames) = unbounded();
        let thread = TxThread::spawn(Box::new(source), Arc::clone(&transmitting), sender).unwrap();
        let received: Vec<TxFrame> = (0..10)
            .map(|_| frames.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        drop(thread);
        assert!(received.iter().all(|f| f.samples.len() == FRAME && !f.end));
        assert_eq!(received[9].timestamp, 9 * FRAME as u32);
        assert!(frames.recv_timeout(Duration::from_millis(100)).is_err());
    }
}
