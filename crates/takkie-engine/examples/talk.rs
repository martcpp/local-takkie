//! The whole engine on real devices, driven from the keyboard. Prints every
//! event and a status line every two seconds.
//!
//! `cargo run -p takkie-engine --example talk -- [name] [channel] [ip:port...]`
//!
//! Type a line and press Enter: empty toggles push-to-talk, `m` toggles
//! mute, `1`..`10` switches channel, `q` quits.

use std::io::{self, BufRead, Write};
use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use takkie_core::ChannelId;
use takkie_engine::{Engine, EngineConfig, EngineEvent};

enum Key {
    Talk,
    Mute,
    Channel(ChannelId),
    Quit,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "talk-check".into());
    let channel = ChannelId::try_from(args.next().and_then(|c| c.parse().ok()).unwrap_or(1))?;
    let static_peers = args
        .map(|peer| peer.parse::<SocketAddr>())
        .collect::<Result<Vec<_>, _>>()?;

    let (engine, events) = Engine::start(EngineConfig {
        display_name: name.clone(),
        channel,
        static_peers,
        ..EngineConfig::default()
    })?;
    let mut out = io::stdout().lock();
    writeln!(
        out,
        "{name} (id {}) on channel {channel}, UDP port {}",
        engine.id(),
        engine.port()
    )?;
    writeln!(
        out,
        "Enter: talk on/off | m: mute | 1-10: channel | q: quit"
    )?;

    let (keys_in, keys) = crossbeam_channel::unbounded();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { return };
            let key = match line.trim() {
                "" => Key::Talk,
                "m" => Key::Mute,
                "q" => Key::Quit,
                other => match other
                    .parse::<u8>()
                    .ok()
                    .and_then(|n| ChannelId::try_from(n).ok())
                {
                    Some(channel) => Key::Channel(channel),
                    None => continue,
                },
            };
            if keys_in.send(key).is_err() {
                return;
            }
        }
    });

    let start = Instant::now();
    let mut next_status = start + Duration::from_secs(2);
    let (mut talking, mut muted) = (false, false);
    loop {
        for key in keys.try_iter() {
            match key {
                Key::Talk => {
                    talking = !talking;
                    engine.set_transmitting(talking);
                    writeln!(out, "talk {}", if talking { "ON" } else { "off" })?;
                }
                Key::Mute => {
                    muted = !muted;
                    engine.set_muted(muted);
                    writeln!(out, "mute {}", if muted { "ON" } else { "off" })?;
                }
                Key::Channel(channel) => {
                    engine.set_channel(channel, None);
                    writeln!(out, "channel {channel}")?;
                }
                Key::Quit => return Ok(()),
            }
        }
        if let Ok(event) = events.recv_timeout(Duration::from_millis(50)) {
            let at = start.elapsed().as_secs_f32();
            let line = match event {
                EngineEvent::PeerJoined(peer) => format!(
                    "joined {} \"{}\" ch {} at {}",
                    peer.id, peer.name, peer.channel, peer.addr
                ),
                EngineEvent::PeerUpdated(peer) => {
                    format!("updated {} \"{}\" ch {}", peer.id, peer.name, peer.channel)
                }
                EngineEvent::PeerLeft(id) => format!("left {id}"),
                EngineEvent::TalkStarted(id) => format!("{id} is talking"),
                EngineEvent::TalkStopped(id) => format!("{id} stopped"),
                EngineEvent::DeviceStarted {
                    direction,
                    description,
                } => format!("{direction}: {description}"),
                EngineEvent::DeviceLost(direction) => format!("{direction} lost"),
                EngineEvent::Warning(warning) => format!("warning: {warning}"),
                other => format!("{other:?}"),
            };
            writeln!(out, "{at:6.1}s {line}")?;
        }
        if Instant::now() >= next_status {
            let s = engine.snapshot();
            writeln!(
                out,
                "   [ch {} | peers {} | mic {:.2} | spk {:.2} | buffer {} ms | sent {} | received {} | concealed {} | underruns {}]",
                s.channel,
                s.peers.len(),
                s.mic.rms,
                s.speaker.rms,
                s.buffer_ms,
                s.stats.packets_sent,
                s.stats.packets_received,
                s.stats.frames_concealed,
                s.stats.speaker_underruns,
            )?;
            next_status += Duration::from_secs(2);
        }
    }
}
