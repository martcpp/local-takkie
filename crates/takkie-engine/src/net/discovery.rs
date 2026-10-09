//! mDNS: announcing ourselves and finding the other takkies on the LAN.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::Sender;
use mdns_sd::{IfKind, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo};
use takkie_core::{ChannelId, PeerId};
use thiserror::Error;

use super::peers::PeerMessage;

/// The service every takkie announces.
pub const SERVICE: &str = "_takkie._udp.local.";

const MAX_NAME: usize = 64;
const MAX_LABEL: usize = 63;
const GOODBYE_WAIT: Duration = Duration::from_secs(1);

/// An mDNS failure.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    /// The daemon refused.
    #[error("mDNS: {0}")]
    Mdns(#[from] mdns_sd::Error),
    /// The browse thread didn't start.
    #[error("couldn't start the browse thread: {0}")]
    Thread(#[from] std::io::Error),
}

/// What we tell the network about ourselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    /// Our id.
    pub id: PeerId,
    /// Display name.
    pub name: String,
    /// Our channel.
    pub channel: ChannelId,
    /// Our UDP port.
    pub port: u16,
}

/// A DNS label for `name`: a–z, 0–9 and `-`, with part of the id on the
/// end so two machines with the same name don't clash.
#[must_use]
pub fn host_label(name: &str, id: PeerId) -> String {
    let suffix = format!("-{:08x}", id.get() & 0xFFFF_FFFF);
    let mut label = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            label.push(c);
        } else if !label.is_empty() && !label.ends_with('-') {
            label.push('-');
        }
    }
    let mut base: String = label.trim_end_matches('-').to_string();
    if base.is_empty() {
        base = "takkie".into();
    }
    base.truncate(MAX_LABEL - suffix.len());
    let base = base.trim_end_matches('-');
    format!("{base}{suffix}")
}

/// The TXT record: `v`, `id`, `ch` and `name`.
#[must_use]
pub fn properties(announcement: &Announcement) -> HashMap<String, String> {
    let name: String = announcement.name.chars().take(MAX_NAME).collect();
    HashMap::from([
        ("v".to_string(), "1".to_string()),
        ("id".to_string(), announcement.id.to_string()),
        ("ch".to_string(), announcement.channel.to_string()),
        ("name".to_string(), name),
    ])
}

/// Reads a peer's TXT record and addresses. `None` for another protocol
/// version or a record we can't use.
#[must_use]
pub fn announced(
    txt: &HashMap<String, String>,
    addresses: impl IntoIterator<Item = Ipv4Addr>,
    port: u16,
) -> Option<PeerMessage> {
    if txt.get("v").map(String::as_str) != Some("1") {
        return None;
    }
    let sender = PeerId::new(u64::from_str_radix(txt.get("id")?, 16).ok()?);
    let channel = ChannelId::try_from(txt.get("ch")?.parse::<u8>().ok()?).ok()?;
    let name = txt
        .get("name")
        .map(|name| name.trim().chars().take(MAX_NAME).collect())
        .unwrap_or_default();
    let ip = addresses.into_iter().next()?;
    Some(PeerMessage::Announced {
        sender,
        name,
        channel,
        addr: SocketAddr::from((ip, port)),
    })
}

fn txt(service: &ResolvedService) -> HashMap<String, String> {
    service
        .get_properties()
        .iter()
        .map(|property| (property.key().to_string(), property.val_str().to_string()))
        .collect()
}

/// Our mDNS presence. One daemon, kept as long as this lives; dropping it
/// says goodbye so peers see us leave straight away.
pub struct Discovery {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Discovery {
    /// Starts the daemon and announces `announcement` on IPv4 LAN
    /// interfaces.
    ///
    /// # Errors
    /// [`DiscoveryError`] if the daemon can't start or register.
    pub fn start(announcement: &Announcement) -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new()?;
        // Our transport is IPv4, and a loopback address is no use to peers.
        daemon.disable_interface(vec![IfKind::IPv6, IfKind::LoopbackV4])?;
        let label = host_label(&announcement.name, announcement.id);
        let info = ServiceInfo::new(
            SERVICE,
            &label,
            &format!("{label}.local."),
            "",
            announcement.port,
            properties(announcement),
        )?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon.register(info)?;
        Ok(Self { daemon, fullname })
    }

    /// Our full service name.
    #[must_use]
    pub fn fullname(&self) -> &str {
        &self.fullname
    }

    /// The daemon, for browsing.
    #[must_use]
    pub fn daemon(&self) -> &ServiceDaemon {
        &self.daemon
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        if let Ok(done) = self.daemon.unregister(&self.fullname) {
            let _ = done.recv_timeout(GOODBYE_WAIT);
        }
        if let Ok(done) = self.daemon.shutdown() {
            let _ = done.recv_timeout(GOODBYE_WAIT);
        }
    }
}

/// Watches mDNS for other takkies and reports them as [`PeerMessage`]s.
/// Dropping it stops the thread.
pub struct Browser {
    daemon: ServiceDaemon,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Browser {
    /// Browses for [`SERVICE`] on `discovery`'s daemon.
    ///
    /// # Errors
    /// [`DiscoveryError`] if browsing or the thread can't start.
    pub fn spawn(
        discovery: &Discovery,
        peers: Sender<PeerMessage>,
    ) -> Result<Self, DiscoveryError> {
        let daemon = discovery.daemon().clone();
        let events = daemon.browse(SERVICE)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("takkie-mdns".into())
            .spawn(move || {
                // Removals only carry the service name.
                let mut ids: HashMap<String, PeerId> = HashMap::new();
                while !stopping.load(Relaxed) {
                    let message = match events.recv_timeout(Duration::from_millis(100)) {
                        Ok(ServiceEvent::ServiceResolved(service)) => {
                            let Some(message) = announced(
                                &txt(&service),
                                service.get_addresses_v4(),
                                service.get_port(),
                            ) else {
                                continue;
                            };
                            let PeerMessage::Announced { sender, .. } = &message else {
                                continue;
                            };
                            let fullname = service.get_fullname().to_string();
                            if let Some(old) = ids.insert(fullname, *sender)
                                && old != *sender
                                && peers.send(PeerMessage::Withdrawn { sender: old }).is_err()
                            {
                                return;
                            }
                            message
                        }
                        Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                            let Some(sender) = ids.remove(&fullname) else {
                                continue;
                            };
                            PeerMessage::Withdrawn { sender }
                        }
                        Ok(_) | Err(mdns_sd::RecvTimeoutError::Timeout) => continue,
                        Err(mdns_sd::RecvTimeoutError::Disconnected) => return,
                    };
                    if peers.send(message).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Self {
            daemon,
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.daemon.stop_browse(SERVICE);
        self.stop.store(true, Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: PeerId = PeerId::new(0x1234_5678_9ABC_DEF0);

    #[test]
    fn labels_keep_letters_and_digits_and_add_the_id() {
        assert_eq!(host_label("Ada's Laptop", ID), "ada-s-laptop-9abcdef0");
        assert_eq!(host_label("KITCHEN_2", ID), "kitchen-2-9abcdef0");
        assert_eq!(host_label("  --x--  ", ID), "x-9abcdef0");
    }

    #[test]
    fn labels_never_come_out_empty() {
        assert_eq!(host_label("", ID), "takkie-9abcdef0");
        assert_eq!(host_label("日本語", ID), "takkie-9abcdef0");
    }

    #[test]
    fn labels_fit_in_63_characters() {
        let label = host_label(&"a".repeat(200), ID);
        assert_eq!(label.len(), 63);
        assert!(label.ends_with("-9abcdef0"));
        let dashed = host_label(&format!("{}-b", "a".repeat(53)), ID);
        assert!(dashed.len() <= 63);
        assert!(!dashed.contains("--"));
    }

    #[test]
    fn txt_record_has_the_four_fields() {
        let txt = properties(&Announcement {
            id: ID,
            name: "Kitchen".into(),
            channel: ChannelId::try_from(4).unwrap(),
            port: 40_000,
        });
        assert_eq!(txt["v"], "1");
        assert_eq!(txt["id"], "123456789abcdef0");
        assert_eq!(txt["ch"], "4");
        assert_eq!(txt["name"], "Kitchen");
        assert_eq!(txt.len(), 4);
    }

    fn kitchen() -> Announcement {
        Announcement {
            id: ID,
            name: "Kitchen".into(),
            channel: ChannelId::try_from(4).unwrap(),
            port: 40_000,
        }
    }

    const LAN: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 20);

    #[test]
    fn our_own_txt_record_reads_back() {
        let txt = properties(&kitchen());
        assert_eq!(
            announced(&txt, [LAN], 40_000),
            Some(PeerMessage::Announced {
                sender: ID,
                name: "Kitchen".into(),
                channel: ChannelId::try_from(4).unwrap(),
                addr: SocketAddr::from((LAN, 40_000)),
            })
        );
    }

    #[test]
    fn other_versions_and_broken_records_are_ignored() {
        let good = properties(&kitchen());
        let with = |field: &str, value: &str| {
            let mut txt = good.clone();
            txt.insert(field.into(), value.into());
            txt
        };
        let without = |field: &str| {
            let mut txt = good.clone();
            txt.remove(field);
            txt
        };
        assert_eq!(announced(&with("v", "2"), [LAN], 1), None);
        assert_eq!(announced(&without("v"), [LAN], 1), None);
        assert_eq!(announced(&with("id", "kitchen"), [LAN], 1), None);
        assert_eq!(announced(&without("id"), [LAN], 1), None);
        assert_eq!(announced(&with("ch", "11"), [LAN], 1), None);
        assert_eq!(announced(&with("ch", "-1"), [LAN], 1), None);
        assert_eq!(announced(&good, [], 1), None);
    }

    #[test]
    fn a_missing_or_long_name_is_still_usable() {
        let mut txt = properties(&kitchen());
        txt.remove("name");
        let Some(PeerMessage::Announced { name, .. }) = announced(&txt, [LAN], 1) else {
            unreachable!("the rest of the record is fine");
        };
        assert_eq!(name, "");
        txt.insert("name".into(), "n".repeat(300));
        let Some(PeerMessage::Announced { name, .. }) = announced(&txt, [LAN], 1) else {
            unreachable!("the rest of the record is fine");
        };
        assert_eq!(name.chars().count(), MAX_NAME);
    }

    #[test]
    fn long_names_are_capped_in_the_txt_record() {
        let txt = properties(&Announcement {
            id: ID,
            name: "n".repeat(300),
            channel: ChannelId::MIN,
            port: 1,
        });
        assert_eq!(txt["name"].chars().count(), MAX_NAME);
    }
}
