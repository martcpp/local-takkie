//! Hear yourself through the new audio pipeline: mic, 48 kHz frames, Opus,
//! jitter buffer, decoder, speaker. Use headphones, or it will howl.
//!
//! `cargo run -p takkie-engine --example loopback -- [--input NAME] [--output NAME] [--seconds N]`

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};

use crossbeam_channel::unbounded;
use takkie_core::{PeerId, Seq};
use takkie_engine::audio::config::{input_config, output_config};
use takkie_engine::audio::devices::{Direction, find_device};
use takkie_engine::audio::mix::{MixThread, RxPacket};
use takkie_engine::audio::stream::{CpalSink, CpalSource};
use takkie_engine::audio::tx::{LevelMeter, TxEvent, TxThread};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = None;
    let mut output = None;
    let mut seconds = 30_u64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => input = args.next(),
            "--output" => output = args.next(),
            "--seconds" => seconds = args.next().and_then(|s| s.parse().ok()).unwrap_or(seconds),
            other => return Err(format!("unknown argument {other}").into()),
        }
    }

    let mut out = io::stdout().lock();
    let mic = find_device(Direction::Input, input.as_deref())?;
    let speaker = find_device(Direction::Output, output.as_deref())?;
    for warning in [&mic.warning, &speaker.warning].into_iter().flatten() {
        writeln!(out, "warning: {warning}")?;
    }
    let sink = CpalSink::open(&speaker.device, &output_config(&speaker.device)?)?;
    let source = CpalSource::open(&mic.device, &input_config(&mic.device)?)?;
    writeln!(out, "mic:     {} ({})", mic.name, source.settings())?;
    writeln!(out, "speaker: {} ({})", speaker.name, sink.settings())?;
    writeln!(out, "talk now; running for {seconds} s\n")?;

    let (mic_counters, speaker_counters) = (source.counters(), sink.counters());
    let level = Arc::new(LevelMeter::default());
    let (tx_events, events) = unbounded();
    let tx = TxThread::spawn(
        Box::new(source),
        Arc::new(AtomicBool::new(true)),
        Arc::clone(&level),
        tx_events,
    )?;
    let (packets, received) = unbounded();
    let mix = MixThread::spawn(received, Box::new(sink))?;

    let start = Instant::now();
    let mut next_report = start + Duration::from_secs(1);
    let mut seq = 0_u32;
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Ok(event) = events.recv_timeout(Duration::from_millis(50)) {
            match event {
                TxEvent::Packet(packet) => {
                    packets.send(RxPacket {
                        sender: PeerId::new(1),
                        seq: Seq::new(seq),
                        payload: packet.payload,
                        end: packet.end,
                    })?;
                    seq = seq.wrapping_add(1);
                }
                TxEvent::Error(error) => writeln!(out, "encode error: {error}")?,
            }
        }
        if Instant::now() >= next_report {
            next_report += Duration::from_secs(1);
            let now = level.get();
            let bar = "#".repeat((now.peak * 40.0).min(40.0) as usize);
            writeln!(
                out,
                "{:>3}s  level {bar:<40} | packets {seq:>5} | mic dropped {} | speaker underruns {} | errors {}",
                start.elapsed().as_secs(),
                mic_counters.dropped.load(Relaxed),
                speaker_counters.underruns.load(Relaxed),
                mic_counters.errors.load(Relaxed) + speaker_counters.errors.load(Relaxed),
            )?;
        }
    }
    drop(tx);
    drop(mix);
    writeln!(out, "\ndone")?;
    Ok(())
}
