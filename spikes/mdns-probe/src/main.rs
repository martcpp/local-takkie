//! Laptop side of the E4.4 discovery check: announces itself under the
//! terminal app's service type and prints every peer it resolves.
//!
//! Usage: mdns-probe [name] [seconds]

use std::collections::HashMap;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

const SERVICE: &str = "_walkietalkie._udp.local.";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "laptop-probe".into());
    let seconds: u64 = args.next().map_or(Ok(120), |s| s.parse())?;

    let socket = UdpSocket::bind("0.0.0.0:0")?;
    let port = socket.local_addr()?.port();
    let daemon = ServiceDaemon::new()?;
    let host = format!("{name}.local.");
    let info = ServiceInfo::new(SERVICE, &name, &host, "", port, None::<HashMap<String, String>>)?
        .enable_addr_auto();
    let me = info.get_fullname().to_string();
    daemon.register(info)?;
    let events = daemon.browse(SERVICE)?;
    println!("announced {me} on port {port}, browsing for {seconds} s");

    let start = Instant::now();
    let deadline = start + Duration::from_secs(seconds);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Ok(event) = events.recv_timeout(left) else {
            break;
        };
        let at = start.elapsed().as_secs_f32();
        match event {
            ServiceEvent::ServiceResolved(s) if s.get_fullname() != me => {
                let addrs: Vec<String> = s.get_addresses().iter().map(ToString::to_string).collect();
                println!("{at:6.1}s resolved {} {} port {}", s.get_fullname(), addrs.join(", "), s.get_port());
            }
            ServiceEvent::ServiceRemoved(_, name) => println!("{at:6.1}s removed {name}"),
            _ => {}
        }
    }
    let _ = daemon.shutdown();
    Ok(())
}
