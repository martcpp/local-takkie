//! Runs the whole peer side of the engine without audio: mDNS, UDP, Hello
//! and expiry. Prints every takkie that joins, changes or leaves. Run two
//! copies (here or on two machines) to see them find each other; quit one
//! cleanly by letting it run out, or kill it with Ctrl+C.
//!
//! `cargo run -p takkie-engine --example discovery -- [name] [channel] [seconds] [ip:port...]`
//!
//! Any `ip:port` after the seconds is a static peer.

use std::io::{self, Write};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU8;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use takkie_core::peers::PeerEvent;
use takkie_core::{ChannelId, PeerId};
use takkie_engine::net::discovery::{Announcement, Browser, Discovery};
use takkie_engine::net::hello::{HELLO_EVERY, HelloThread};
use takkie_engine::net::peers::{PEER_TIMEOUT, PeerOutputs, PeerThread, Peers};
use takkie_engine::net::rx::{RxOutputs, RxThread};
use takkie_engine::net::send::PacketSender;
use takkie_engine::net::{Transport, UdpTransport};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "discovery-check".into());
    let channel = ChannelId::try_from(args.next().and_then(|c| c.parse().ok()).unwrap_or(1))?;
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let static_peers = args
        .map(|peer| peer.parse::<SocketAddr>())
        .collect::<Result<Vec<_>, _>>()?;
    let me = PeerId::new(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos() as u64,
    );

    let transport: Arc<dyn Transport> = Arc::new(UdpTransport::bind(0)?);
    let port = transport.local_addr().port();
    let channel_now = Arc::new(AtomicU8::new(channel.get()));
    let targets = Arc::new(ArcSwap::from_pointee(Vec::new()));

    let (news, news_out) = crossbeam_channel::unbounded();
    let (audio, _) = crossbeam_channel::unbounded();
    let (events, events_out) = crossbeam_channel::unbounded();
    let table = Peers::new(Arc::clone(&channel_now), Arc::clone(&targets), PEER_TIMEOUT);
    let known = table.view();
    let _peers = PeerThread::spawn(
        news_out,
        table,
        PeerOutputs {
            events,
            mixer: audio.clone(),
        },
    )?;
    let _rx = RxThread::spawn(
        Arc::clone(&transport),
        me,
        Arc::clone(&channel_now),
        RxOutputs {
            audio,
            peers: news.clone(),
        },
    )?;
    let discovery = Discovery::start(&Announcement {
        id: me,
        name: name.clone(),
        channel,
        port,
    })?;
    let _browser = Browser::spawn(&discovery, news)?;
    let sender = Arc::new(PacketSender::new(transport, me, channel_now, targets));
    let _hello = HelloThread::spawn(sender, &name, HELLO_EVERY, known, static_peers)?;

    let mut out = io::stdout().lock();
    writeln!(
        out,
        "{name} (id {me}) on channel {channel}, UDP port {port}, running for {seconds} s"
    )?;
    let start = Instant::now();
    while let Some(left) = Duration::from_secs(seconds).checked_sub(start.elapsed()) {
        let Ok(event) = events_out.recv_timeout(left) else {
            break;
        };
        let at = start.elapsed().as_secs_f32();
        let (what, peer) = match event {
            PeerEvent::Joined(peer) => ("joined", peer),
            PeerEvent::Updated(peer) => ("updated", peer),
            PeerEvent::Left(peer) => ("left", peer),
        };
        writeln!(out, "{at:6.1}s {what} {peer}")?;
    }
    writeln!(out, "quitting cleanly")?;
    Ok(())
}
