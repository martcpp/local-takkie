//! Announces this machine over mDNS and prints every takkie it finds.
//! Run two copies (here or on two machines) to see them find each other.
//!
//! `cargo run -p takkie-engine --example discovery -- [name] [channel] [seconds]`

use std::io::{self, Write};
use std::time::{Duration, Instant};

use mdns_sd::ServiceEvent;
use takkie_core::{ChannelId, PeerId};
use takkie_engine::net::discovery::{Announcement, Discovery, SERVICE};

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
    let events = discovery.daemon().browse(SERVICE)?;
    let mut out = io::stdout().lock();
    writeln!(
        out,
        "announced {} (id {id}), browsing for {seconds} s",
        discovery.fullname()
    )?;

    let start = Instant::now();
    while let Some(left) = Duration::from_secs(seconds).checked_sub(start.elapsed()) {
        let Ok(event) = events.recv_timeout(left) else {
            break;
        };
        let at = start.elapsed().as_secs_f32();
        match event {
            ServiceEvent::ServiceResolved(service) => {
                let addrs: Vec<String> = service
                    .get_addresses()
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                let txt = |key| service.get_property_val_str(key).unwrap_or("-").to_string();
                writeln!(
                    out,
                    "{at:6.1}s resolved {} at {} port {} | v={} id={} ch={} name={}",
                    service.get_fullname(),
                    addrs.join(", "),
                    service.get_port(),
                    txt("v"),
                    txt("id"),
                    txt("ch"),
                    txt("name"),
                )?;
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                writeln!(out, "{at:6.1}s removed {fullname}")?
            }
            _ => {}
        }
    }
    Ok(())
}
