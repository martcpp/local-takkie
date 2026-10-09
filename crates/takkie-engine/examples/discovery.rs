//! Announces this machine over mDNS and prints every takkie that joins,
//! changes or leaves. Run two copies (here or on two machines) to see them
//! find each other.
//!
//! `cargo run -p takkie-engine --example discovery -- [name] [channel] [seconds]`

use std::io::{self, Write};
use std::time::{Duration, Instant};

use takkie_core::peers::{PeerEvent, PeerTable};
use takkie_core::{ChannelId, PeerId};
use takkie_engine::net::discovery::{Announcement, Browser, Discovery};
use takkie_engine::net::peers::apply;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "discovery-check".into());
    let channel = ChannelId::try_from(args.next().and_then(|c| c.parse().ok()).unwrap_or(1))?;
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let id = PeerId::new(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos() as u64,
    );

    let discovery = Discovery::start(&Announcement {
        id,
        name: name.clone(),
        channel,
        port: 40_000,
    })?;
    let (peers, news) = crossbeam_channel::unbounded();
    let _browser = Browser::spawn(&discovery, peers)?;
    let mut out = io::stdout().lock();
    writeln!(
        out,
        "announced {} (id {id}), browsing for {seconds} s",
        discovery.fullname()
    )?;

    let mut table = PeerTable::new();
    let start = Instant::now();
    while let Some(left) = Duration::from_secs(seconds).checked_sub(start.elapsed()) {
        let Ok(message) = news.recv_timeout(left) else {
            break;
        };
        let at = start.elapsed().as_secs_f32();
        let Some(event) = apply(&mut table, message, Instant::now()) else {
            continue;
        };
        let (what, peer) = match event {
            PeerEvent::Joined(peer) => ("joined", peer),
            PeerEvent::Updated(peer) => ("updated", peer),
            PeerEvent::Left(peer) => {
                writeln!(out, "{at:6.1}s left {peer}")?;
                continue;
            }
        };
        if let Some(peer) = table.get(peer) {
            writeln!(
                out,
                "{at:6.1}s {what} {} \"{}\" ch {} at {}",
                peer.id, peer.name, peer.channel, peer.addr
            )?;
        }
    }
    Ok(())
}
