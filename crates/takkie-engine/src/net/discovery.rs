//! Announcing ourselves over mDNS, so peers on the LAN can find us.

use std::collections::HashMap;

use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
use takkie_core::{ChannelId, PeerId};
use thiserror::Error;

/// The service every takkie announces.
pub const SERVICE: &str = "_takkie._udp.local.";

const MAX_NAME: usize = 64;
const MAX_LABEL: usize = 63;

/// An mDNS failure.
#[derive(Debug, Error)]
#[error("mDNS: {0}")]
pub struct DiscoveryError(#[from] mdns_sd::Error);

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

/// Our mDNS presence. One daemon, kept as long as this lives.
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
