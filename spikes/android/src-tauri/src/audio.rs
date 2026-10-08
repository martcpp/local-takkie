//! Mic to speaker loopback through a ring buffer, plus an acoustic ping that
//! measures the round trip from speaker to mic. Runs on cpal, or on AAudio
//! directly so the two can be compared.

use std::any::Any;
use std::collections::BTreeSet;
use std::f32::consts::TAU;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, StreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};

pub(crate) const RATE: u32 = 48_000;
const RING: usize = RATE as usize / 2;
// Anything queued past this is dropped, so a slow start can't leave the
// loopback permanently behind.
const MAX_QUEUED: usize = RATE as usize / 10;
const PING_THRESHOLD: f32 = 0.2;
const PING_TIMEOUT: Duration = Duration::from_secs(1);
const PING_FAILED: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Cpal,
    AAudio,
    Voice,
}

/// Counters shared with the audio callbacks. Atomics only, so the callbacks
/// never lock.
#[derive(Default)]
pub(crate) struct Shared {
    in_callbacks: AtomicU64,
    out_callbacks: AtomicU64,
    in_frames: AtomicU32,
    out_frames: AtomicU32,
    queued: AtomicU32,
    underruns: AtomicU64,
    dropped: AtomicU64,
    errors: AtomicU64,
    pub(crate) xruns: AtomicU32,
    // f32 bits. For non-negative floats the bit patterns sort like the
    // values, so fetch_max works.
    peak: AtomicU32,
    ping_armed: AtomicBool,
    // Nanoseconds since `start` when the beep was written; 0 when idle.
    ping_sent: AtomicU64,
    ping_us: AtomicU32,
}

impl Shared {
    pub(crate) fn count_error(&self) {
        self.errors.fetch_add(1, Relaxed);
    }
}

/// What the mic callback does with each buffer.
pub(crate) struct Input {
    pub(crate) shared: Arc<Shared>,
    producer: Producer<f32>,
    start: Instant,
}

impl Input {
    pub(crate) fn process(&mut self, data: &[f32]) {
        let s = &*self.shared;
        s.in_callbacks.fetch_add(1, Relaxed);
        s.in_frames.store(data.len() as u32, Relaxed);
        let mut peak = 0.0_f32;
        let mut dropped = 0;
        for &sample in data {
            peak = peak.max(sample.abs());
            if self.producer.push(sample).is_err() {
                dropped += 1;
            }
        }
        s.peak.fetch_max(peak.to_bits(), Relaxed);
        s.dropped.fetch_add(dropped, Relaxed);
        listen_for_ping(s, data, self.start);
    }
}

/// What the speaker callback does with each buffer.
pub(crate) struct Output {
    pub(crate) shared: Arc<Shared>,
    consumer: Consumer<f32>,
    beep: Vec<f32>,
    start: Instant,
}

impl Output {
    pub(crate) fn process(&mut self, data: &mut [f32]) {
        let s = &*self.shared;
        s.out_callbacks.fetch_add(1, Relaxed);
        s.out_frames.store(data.len() as u32, Relaxed);
        let excess = self.consumer.slots().saturating_sub(MAX_QUEUED);
        if excess > 0 {
            if let Ok(chunk) = self.consumer.read_chunk(excess) {
                chunk.commit_all();
            }
            s.dropped.fetch_add(excess as u64, Relaxed);
        }
        let mut short = false;
        for slot in data.iter_mut() {
            *slot = self.consumer.pop().unwrap_or_else(|_| {
                short = true;
                0.0
            });
        }
        if short {
            s.underruns.fetch_add(1, Relaxed);
        }
        s.queued.store(self.consumer.slots() as u32, Relaxed);
        if s.ping_armed.swap(false, Relaxed) {
            let n = self.beep.len().min(data.len());
            data[..n].copy_from_slice(&self.beep[..n]);
            s.ping_sent.store(nanos_since(self.start), Relaxed);
        }
    }
}

fn halves(shared: &Arc<Shared>) -> (Input, Output) {
    let (producer, consumer) = RingBuffer::new(RING);
    let start = Instant::now();
    let beep = (0..RATE as usize / 200)
        .map(|i| 0.8 * (TAU * 2000.0 * i as f32 / RATE as f32).sin())
        .collect();
    (
        Input {
            shared: Arc::clone(shared),
            producer,
            start,
        },
        Output {
            shared: Arc::clone(shared),
            consumer,
            beep,
            start,
        },
    )
}

#[derive(Serialize)]
pub struct Stats {
    in_callbacks: u64,
    out_callbacks: u64,
    in_frames: u32,
    out_frames: u32,
    queued_ms: f32,
    underruns: u64,
    dropped: u64,
    errors: u64,
    xruns: u32,
    peak: f32,
    ping_ms: Option<f32>,
    ping_failed: bool,
}

/// A running loopback. Dropping it without `stop` leaves the streams open.
pub struct Loopback {
    shared: Arc<Shared>,
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

impl Loopback {
    /// Opens the default mic and speaker at 48 kHz mono f32 and starts both.
    /// Returns what the streams ended up with.
    ///
    /// The streams live on their own thread, so nothing here has to be `Send`.
    pub fn start(backend: Backend) -> Result<(Self, String), String> {
        let shared = Arc::new(Shared::default());
        let (stop, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (input, output) = halves(&shared);
        let thread = thread::spawn(move || {
            let mut report = String::new();
            let opened = match backend {
                Backend::Cpal => open_cpal(input, output).map(|s| Box::new(s) as Box<dyn Any>),
                Backend::AAudio => open_aaudio(input, output, false, &mut report),
                Backend::Voice => open_aaudio(input, output, true, &mut report),
            };
            match opened {
                Ok(streams) => {
                    let _ = ready_tx.send(Ok(report));
                    let _ = stop_rx.recv();
                    drop(streams);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        let report = ready_rx
            .recv()
            .map_err(|_| "audio thread exited".to_string())??;
        Ok((
            Self {
                shared,
                stop,
                thread,
            },
            report,
        ))
    }

    pub fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.thread.join();
    }

    pub fn ping(&self) {
        self.shared.ping_us.store(0, Relaxed);
        self.shared.ping_armed.store(true, Relaxed);
    }

    pub fn stats(&self) -> Stats {
        let s = &self.shared;
        let ping_us = s.ping_us.load(Relaxed);
        Stats {
            in_callbacks: s.in_callbacks.load(Relaxed),
            out_callbacks: s.out_callbacks.load(Relaxed),
            in_frames: s.in_frames.load(Relaxed),
            out_frames: s.out_frames.load(Relaxed),
            queued_ms: s.queued.load(Relaxed) as f32 * 1000.0 / RATE as f32,
            underruns: s.underruns.load(Relaxed),
            dropped: s.dropped.load(Relaxed),
            errors: s.errors.load(Relaxed),
            xruns: s.xruns.load(Relaxed),
            peak: f32::from_bits(s.peak.swap(0, Relaxed)),
            ping_ms: (ping_us != 0 && ping_us != PING_FAILED).then(|| ping_us as f32 / 1000.0),
            ping_failed: ping_us == PING_FAILED,
        }
    }
}

#[cfg(target_os = "android")]
fn open_aaudio(
    input: Input,
    output: Output,
    voice: bool,
    report: &mut String,
) -> Result<Box<dyn Any>, String> {
    crate::aaudio::open(input, output, voice, report).map(|s| Box::new(s) as Box<dyn Any>)
}

#[cfg(not(target_os = "android"))]
fn open_aaudio(_: Input, _: Output, _: bool, _: &mut String) -> Result<Box<dyn Any>, String> {
    Err("AAudio is only on Android".into())
}

struct CpalStreams {
    _mic: cpal::Stream,
    _speaker: cpal::Stream,
}

fn open_cpal(mut input: Input, mut output: Output) -> Result<CpalStreams, String> {
    let host = cpal::default_host();
    let mic = host.default_input_device().ok_or("no input device")?;
    let speaker = host.default_output_device().ok_or("no output device")?;
    let config = StreamConfig {
        channels: 1,
        sample_rate: RATE,
        buffer_size: BufferSize::Default,
    };

    let errors = Arc::clone(&input.shared);
    let mic_stream = mic
        .build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| input.process(data),
            move |_| errors.count_error(),
            None,
        )
        .map_err(|e| format!("mic: {e}"))?;

    let errors = Arc::clone(&output.shared);
    let speaker_stream = speaker
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| output.process(data),
            move |_| errors.count_error(),
            None,
        )
        .map_err(|e| format!("speaker: {e}"))?;

    mic_stream.play().map_err(|e| format!("mic: {e}"))?;
    speaker_stream.play().map_err(|e| format!("speaker: {e}"))?;
    Ok(CpalStreams {
        _mic: mic_stream,
        _speaker: speaker_stream,
    })
}

/// Rough round trip: from the output callback that wrote the beep to the mic
/// sample that first hears it. Off by up to one callback period either way.
fn listen_for_ping(s: &Shared, data: &[f32], start: Instant) {
    let sent = s.ping_sent.load(Relaxed);
    if sent == 0 {
        return;
    }
    let now = nanos_since(start);
    if let Some(i) = data.iter().position(|x| x.abs() > PING_THRESHOLD) {
        // The callback runs once the buffer is full, so sample i was
        // captured this long before now.
        let age = (data.len() - i) as u64 * 1_000_000_000 / RATE as u64;
        let us = now.saturating_sub(age).saturating_sub(sent) / 1000;
        s.ping_us.store((us as u32).max(1), Relaxed);
        s.ping_sent.store(0, Relaxed);
    } else if now.saturating_sub(sent) > PING_TIMEOUT.as_nanos() as u64 {
        s.ping_us.store(PING_FAILED, Relaxed);
        s.ping_sent.store(0, Relaxed);
    }
}

fn nanos_since(start: Instant) -> u64 {
    start.elapsed().as_nanos() as u64
}

/// What cpal reports about the host, its devices and their configs.
pub fn describe() -> String {
    let host = cpal::default_host();
    let mut out = format!("host: {:?}\n", host.id());
    for (label, device) in [
        ("default input", host.default_input_device()),
        ("default output", host.default_output_device()),
    ] {
        let Some(device) = device else {
            let _ = writeln!(out, "{label}: none");
            continue;
        };
        let config = if label.ends_with("input") {
            device.default_input_config()
        } else {
            device.default_output_config()
        };
        let _ = writeln!(out, "{label} config: {config:?}");
    }
    match host.devices() {
        Ok(devices) => {
            for device in devices {
                out.push('\n');
                describe_device(&mut out, &device);
            }
        }
        Err(e) => {
            let _ = writeln!(out, "devices: {e}");
        }
    }
    out
}

fn describe_device(out: &mut String, device: &cpal::Device) {
    match device.description() {
        Ok(d) => {
            let _ = writeln!(
                out,
                "{} ({:?}, {:?})",
                d.name(),
                d.device_type(),
                d.direction()
            );
        }
        Err(e) => {
            let _ = writeln!(out, "unnamed device: {e}");
        }
    }
    let Ok(configs) = device.supported_input_configs() else {
        return;
    };
    let mut rates = BTreeSet::new();
    let mut channels = BTreeSet::new();
    let mut formats = BTreeSet::new();
    let mut buffer = None;
    for c in configs {
        rates.insert(c.min_sample_rate());
        channels.insert(c.channels());
        formats.insert(c.sample_format().to_string());
        buffer = Some(*c.buffer_size());
    }
    let _ = writeln!(out, "  rates: {rates:?}");
    let _ = writeln!(out, "  channels: {channels:?}, formats: {formats:?}");
    let _ = writeln!(out, "  buffer: {buffer:?}");
}
