use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::{Ipv4Addr, IpAddr};
use std::time::Duration;
use std::thread::{spawn,sleep};
use std::env;

fn main() {
    // Create the mDNS daemon

    let args: Vec<String> = env::args().collect();
    println!("Args: {:?}", args.len());
    let mdns = ServiceDaemon::new().expect("Failed to create daemon");

    // Define service type
    let service_type = "_walkietalkie._udp.local.";
    let instance_name = args.get(1).unwrap().as_str();
    let ip: IpAddr = Ipv4Addr::new(127, 0, 0, 1).into();
    let host_name = "mart-device-1.local.";
    let port = args.get(2).unwrap().parse::<u16>().unwrap();
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

    println!("Announcing service as {} on port {}", instance_name, port);
    println!("Keep this running...");



    let mdns = ServiceDaemon::new().expect("Failed to create daemon");

    // Define the type of service we want to find
    let service_type = "_walkietalkie._udp.local.";
    let _test_type = "_ftp._tcp.local.";

    // Start browsing for services of this type
    let receiver = mdns.browse(service_type)
        .expect("Failed to browse for services");

    println!(" Browsing for services...");

    spawn(move || {
        loop {
            while let Ok(event) = receiver.recv_timeout(Duration::from_secs(1)) {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        println!(
                            "Found: {} at {:?}:{}",
                            info.get_fullname(),
                            info.get_addresses(),
                            info.get_port()
                        );
                    }
                    _ => {}
                }
            }
            sleep(Duration::from_millis(200));
        }
    });

    // 9️⃣ Keep main thread alive (so announcement continues)
    loop {
        sleep(Duration::from_secs(60));
    }
}
