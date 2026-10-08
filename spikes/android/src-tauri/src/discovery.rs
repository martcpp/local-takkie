//! Announce and browse over mDNS the way the terminal app does, to check
//! discovery on Android with and without the Wi-Fi multicast lock.

use std::collections::{BTreeMap, HashMap};
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::thread;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;

/// Same service type as `takkie-engine`, so the terminal app sees the phone.
const SERVICE: &str = "_walkietalkie._udp.local.";

#[derive(Clone, Serialize)]
pub struct Peer {
    name: String,
    addrs: Vec<String>,
    port: u16,
    /// How many times it was resolved; climbing means answers keep coming in.
    seen: u32,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    // Held so the announced port stays ours.
    _socket: UdpSocket,
    peers: Arc<Mutex<BTreeMap<String, Peer>>>,
    pub me: String,
}

impl Discovery {
    pub fn start(name: &str) -> Result<Self, String> {
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        let port = socket.local_addr().map_err(|e| e.to_string())?.port();
        let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
        let host = format!("{}.local.", name.replace(' ', "-").to_lowercase());
        let info = ServiceInfo::new(
            SERVICE,
            name,
            &host,
            "",
            port,
            None::<HashMap<String, String>>,
        )
        .map_err(|e| e.to_string())?
        .enable_addr_auto();
        let me = info.get_fullname().to_string();
        daemon.register(info).map_err(|e| e.to_string())?;
        let events = daemon.browse(SERVICE).map_err(|e| e.to_string())?;

        let peers = Arc::new(Mutex::new(BTreeMap::new()));
        let found = Arc::clone(&peers);
        let skip = me.clone();
        // Ends when the daemon shuts down and drops the sender.
        thread::spawn(move || {
            while let Ok(event) = events.recv() {
                let ServiceEvent::ServiceResolved(service) = event else {
                    continue;
                };
                let name = service.get_fullname().to_string();
                if name == skip {
                    continue;
                }
                let Ok(mut peers) = found.lock() else {
                    return;
                };
                let peer = peers.entry(name.clone()).or_insert_with(|| Peer {
                    name,
                    addrs: Vec::new(),
                    port: 0,
                    seen: 0,
                });
                peer.addrs = service
                    .get_addresses()
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                peer.port = service.get_port();
                peer.seen += 1;
            }
        });

        Ok(Self {
            daemon,
            _socket: socket,
            peers,
            me: format!("{me} on port {port}"),
        })
    }

    pub fn peers(&self) -> Vec<Peer> {
        self.peers
            .lock()
            .map(|p| p.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn stop(self) {
        let _ = self.daemon.shutdown();
    }
}
