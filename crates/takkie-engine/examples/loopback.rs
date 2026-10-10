//! Hear yourself through the new audio pipeline: mic, 48 kHz frames, Opus,
//! jitter buffer, decoder, speaker. Use headphones, or it will howl.
//! Unplugging a device mid-run shows the recovery.
//!
//! `cargo run -p takkie-engine --example loopback -- [--input NAME] [--output NAME] [--seconds N]`

use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};

use crossbeam_channel::unbounded;
use takkie_core::{PeerId, Seq};
use takkie_engine::audio::mix::{MixInput, MixShared, RxPacket};
use takkie_engine::audio::session::{AudioEvent, AudioSession, CpalOpener, Wiring};
use takkie_engine::audio::tx::{LevelMeter, TxEvent};

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
    let (tx_events, events) = unbounded();
    let (packets, received) = unbounded();
    let level = Arc::new(LevelMeter::default());
    let mix = MixShared {
        transmitting: Arc::new(AtomicBool::new(true)),
        ..MixShared::default()
    };
    mix.controls.set_half_duplex(false);
    let wiring = Wiring {
        transmitting: Arc::clone(&mix.transmitting),
        level: Arc::clone(&level),
        tx_events,
        packets: received,
        mix: mix.clone(),
    };
    let (mut session, started) =
        AudioSession::start(CpalOpener, input.as_deref(), output.as_deref(), wiring)?;
    for event in started {
        report(&mut out, &event)?;
    }
    writeln!(out, "talk now; running for {seconds} s\n")?;

    let start = Instant::now();
    let mut next_report = start + Duration::from_secs(1);
    let mut seq = 0_u32;
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Ok(event) = events.recv_timeout(Duration::from_millis(50)) {
            match event {
                TxEvent::Packet(packet) => {
                    packets.send(MixInput::Packet(RxPacket {
                        sender: PeerId::new(1),
                        seq: Seq::new(seq),
                        payload: packet.payload,
                        end: packet.end,
                    }))?;
                    seq = seq.wrapping_add(1);
                }
                TxEvent::Error(error) => writeln!(out, "encode error: {error}")?,
            }
        }
        if Instant::now() >= next_report {
            next_report += Duration::from_secs(1);
            for event in session.poll(Instant::now()) {
                report(&mut out, &event)?;
            }
            let count =
                |c: Option<Arc<_>>, f: fn(&takkie_engine::audio::stream::Counters) -> u64| {
                    c.as_deref().map_or(0, f)
                };
            let lost = &mix.counters;
            let peak = level.get().peak;
            let bar = "#".repeat((peak * 40.0).min(40.0) as usize);
            writeln!(
                out,
                "{:>3}s  level {bar:<40} | packets {seq:>5} | lost {} | mic dropped {} | speaker underruns {}",
                start.elapsed().as_secs(),
                lost.recovered.load(Relaxed)
                    + lost.concealed.load(Relaxed)
                    + lost.silenced.load(Relaxed),
                count(session.input_counters(), |c| c.dropped.load(Relaxed)),
                count(session.output_counters(), |c| c.underruns.load(Relaxed)),
            )?;
        }
    }
    drop(session);
    writeln!(out, "\ndone")?;
    Ok(())
}

fn report(out: &mut impl Write, event: &AudioEvent) -> io::Result<()> {
    match event {
        AudioEvent::Started {
            direction,
            description,
        } => writeln!(out, "{direction}: {description}"),
        AudioEvent::Warning(warning) => writeln!(out, "warning: {warning}"),
        AudioEvent::DeviceLost(direction) => {
            writeln!(
                out,
                "*** {direction} device lost; retrying the default every 2 s"
            )
        }
    }
}
