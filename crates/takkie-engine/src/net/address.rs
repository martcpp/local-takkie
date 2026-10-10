//! Picking which of a peer's announced addresses to send to.

use std::net::Ipv4Addr;

/// One of our IPv4 interfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalNet {
    /// Our address on it.
    pub ip: Ipv4Addr,
    /// Its netmask.
    pub netmask: Ipv4Addr,
}

impl LocalNet {
    /// Whether `ip` is on this network.
    #[must_use]
    pub fn contains(&self, ip: Ipv4Addr) -> bool {
        let mask = self.netmask.to_bits();
        ip.to_bits() & mask == self.ip.to_bits() & mask
    }
}

/// Our IPv4 interfaces that are up, without loopback, ordinary LAN
/// addresses first.
#[must_use]
pub fn local_networks() -> Vec<LocalNet> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut nets: Vec<LocalNet> = interfaces
        .into_iter()
        .filter(|interface| interface.is_oper_up() && !interface.is_loopback())
        .filter_map(|interface| match interface.addr {
            if_addrs::IfAddr::V4(v4) => Some(LocalNet {
                ip: v4.ip,
                netmask: v4.netmask,
            }),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect();
    nets.sort_by_key(|net| net.ip.is_link_local());
    nets
}

/// The address to reach a peer on: one sharing a network with us if there
/// is one, then an ordinary address, then link-local, then loopback.
#[must_use]
pub fn best_address(
    candidates: impl IntoIterator<Item = Ipv4Addr>,
    ours: &[LocalNet],
) -> Option<Ipv4Addr> {
    candidates
        .into_iter()
        .filter(|ip| !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast())
        .max_by_key(|&ip| {
            (
                ours.iter().any(|net| net.contains(ip)),
                !ip.is_loopback(),
                !ip.is_link_local(),
                // Lowest address on a tie, so the choice doesn't flip.
                std::cmp::Reverse(ip),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> Ipv4Addr {
        text.parse().unwrap()
    }

    fn net(address: &str, mask: &str) -> LocalNet {
        LocalNet {
            ip: ip(address),
            netmask: ip(mask),
        }
    }

    fn best(candidates: &[&str], ours: &[LocalNet]) -> Option<String> {
        best_address(candidates.iter().map(|c| ip(c)), ours).map(|ip| ip.to_string())
    }

    #[test]
    fn our_subnet_wins() {
        let home = [net("192.168.1.10", "255.255.255.0")];
        assert_eq!(
            best(&["172.17.0.1", "192.168.1.20", "10.0.0.5"], &home),
            Some("192.168.1.20".into())
        );
        let wide = [net("10.20.0.4", "255.255.0.0")];
        assert_eq!(
            best(&["192.168.56.1", "10.20.9.9"], &wide),
            Some("10.20.9.9".into())
        );
    }

    #[test]
    fn link_local_and_loopback_lose_without_a_shared_subnet() {
        assert_eq!(
            best(&["127.0.0.1", "169.254.3.4", "10.1.2.3"], &[]),
            Some("10.1.2.3".into())
        );
        assert_eq!(
            best(&["127.0.0.1", "169.254.3.4"], &[]),
            Some("169.254.3.4".into())
        );
        assert_eq!(best(&["127.0.0.1"], &[]), Some("127.0.0.1".into()));
    }

    #[test]
    fn link_local_is_fine_when_we_share_it() {
        let direct = [net("169.254.3.1", "255.255.0.0")];
        assert_eq!(
            best(&["10.1.2.3", "169.254.3.4"], &direct),
            Some("169.254.3.4".into())
        );
    }

    #[test]
    fn unusable_addresses_are_never_picked() {
        assert_eq!(
            best(&["0.0.0.0", "255.255.255.255", "224.0.0.251"], &[]),
            None
        );
        assert_eq!(best(&[], &[]), None);
    }

    #[test]
    fn ties_pick_the_lowest_address_whatever_the_order() {
        let ours = [net("192.168.1.10", "255.255.255.0")];
        assert_eq!(
            best(&["192.168.1.30", "192.168.1.20"], &ours),
            Some("192.168.1.20".into())
        );
        assert_eq!(
            best(&["192.168.1.20", "192.168.1.30"], &ours),
            Some("192.168.1.20".into())
        );
    }

    #[test]
    fn the_subnet_check_uses_the_mask() {
        let ours = net("192.168.1.10", "255.255.255.0");
        assert!(ours.contains(ip("192.168.1.255")));
        assert!(!ours.contains(ip("192.168.2.1")));
        assert!(net("10.0.0.1", "0.0.0.0").contains(ip("8.8.8.8")));
    }
}
