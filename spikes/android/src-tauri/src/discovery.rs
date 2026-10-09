//! The engine's peer stack (mDNS, UDP, Hello, expiry) on the phone, to check
//! E7 against the laptop's `discovery` example.

use std::collections::BTreeSet;
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use arc_swap::ArcSwap;
use takkie_core::peers::PeerEvent;
use takkie_core::{ChannelId, PeerId};
use takkie_engine::net::discovery::{self, Announcement, Browser};
use takkie_engine::net::hello::{HELLO_EVERY, HelloThread};
use takkie_engine::net::peers::{PEER_TIMEOUT, PeerOutputs, PeerThread, Peers};
use takkie_engine::net::rx::{RxOutputs, RxThread};
use takkie_engine::net::send::PacketSender;
use takkie_engine::net::{Transport, UdpTransport};

const CHANNEL: u8 = 2;

#[derive(Default)]
struct Seen {
    now: BTreeSet<PeerId>,
    log: Vec<String>,
}

pub struct Discovery {
    pub me: String,
    seen: Arc<Mutex<Seen>>,
    // Dropped in order: Bye, then the mDNS goodbye, then the threads.
    _running: (
        HelloThread,
        Browser,
        discovery::Discovery,
        RxThread,
        PeerThread,
    ),
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

impl Discovery {
    pub fn start(name: &str) -> Result<Self, String> {
        let me = PeerId::new(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(text)?
                .as_nanos() as u64,
        );
        let channel = ChannelId::try_from(CHANNEL).map_err(text)?;
        let transport: Arc<dyn Transport> = Arc::new(UdpTransport::bind(0).map_err(text)?);
        let port = transport.local_addr().port();
        let channel_now = Arc::new(AtomicU8::new(CHANNEL));
        let targets = Arc::new(ArcSwap::from_pointee(Vec::new()));

        let (news, news_out) = crossbeam_channel::unbounded();
        let (audio, _) = crossbeam_channel::unbounded();
        let (events, events_out) = crossbeam_channel::unbounded();
        let peers = PeerThread::spawn(
            news_out,
            Peers::new(Arc::clone(&channel_now), Arc::clone(&targets), PEER_TIMEOUT),
            PeerOutputs {
                events,
                mixer: audio.clone(),
            },
        )
        .map_err(text)?;
        let rx = RxThread::spawn(
            Arc::clone(&transport),
            me,
            Arc::clone(&channel_now),
            RxOutputs {
                audio,
                peers: news.clone(),
            },
        )
        .map_err(text)?;
        let mdns = discovery::Discovery::start(&Announcement {
            id: me,
            name: name.to_string(),
            channel,
            port,
        })
        .map_err(text)?;
        let browser = Browser::spawn(&mdns, news).map_err(text)?;
        let sender = Arc::new(PacketSender::new(transport, me, channel_now, targets));
        let hello = HelloThread::spawn(sender, name, HELLO_EVERY, Vec::new()).map_err(text)?;

        let seen = Arc::new(Mutex::new(Seen::default()));
        let recording = Arc::clone(&seen);
        let start = Instant::now();
        // Ends when the peer thread stops and drops its sender.
        thread::spawn(move || {
            while let Ok(event) = events_out.recv() {
                let at = start.elapsed().as_secs_f32();
                let Ok(mut seen) = recording.lock() else {
                    return;
                };
                let line = match event {
                    PeerEvent::Joined(peer) => {
                        seen.now.insert(peer);
                        format!("{at:6.1}s joined {peer}")
                    }
                    PeerEvent::Updated(peer) => format!("{at:6.1}s updated {peer}"),
                    PeerEvent::Left(peer) => {
                        seen.now.remove(&peer);
                        format!("{at:6.1}s left {peer}")
                    }
                };
                seen.log.push(line);
            }
        });

        Ok(Self {
            me: format!("{name} (id {me}) on channel {CHANNEL}, UDP port {port}"),
            seen,
            _running: (hello, browser, mdns, rx, peers),
        })
    }

    pub fn peers(&self) -> Vec<String> {
        let Ok(seen) = self.seen.lock() else {
            return Vec::new();
        };
        let mut lines = vec![format!("peers now: {}", seen.now.len())];
        lines.extend(seen.now.iter().map(|peer| format!("  {peer}")));
        lines.push("log:".into());
        let skip = seen.log.len().saturating_sub(20);
        lines.extend(seen.log.iter().skip(skip).cloned());
        lines
    }

    pub fn stop(self) {
        drop(self);
    }
}
