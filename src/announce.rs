use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use std::net::{Ipv4Addr, IpAddr};
use std::time::Duration;
use std::thread::sleep;

fn main() {
    // Create the mDNS daemon
    let mdns = ServiceDaemon::new().expect("Failed to create daemon");

    // Define service type
    let service_type = "_walkietalkie._udp.local.";
    let instance_name = "mart-device-1";
    let ip: IpAddr = Ipv4Addr::new(127, 0, 0, 1).into();
    let host_name = "mart-device-1.local.";
    let port = 9000;
    let properties = HashMap::new();

        // service_type,           // Service type
        // "mart-device-1",        // Device name
        // "",                     // Hostname (empty = auto)
        // 9000,                   // Port
        // Some(txt_records),      // TXT records  
        // None    

    // Create service info
    let service_info = ServiceInfo::new(
        service_type,
        instance_name,
        host_name,
        ip,
        port,
        Some(properties),
    ).expect("Failed to create service info");

    // Register service
    mdns.register(service_info).expect("Failed to register service");

    println!("✅ Announcing service as mart-device-1 on port 9000");
    println!("🕒 Keep this running...");

    loop {
        sleep(Duration::from_secs(60));
    }
}
