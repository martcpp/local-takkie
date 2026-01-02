use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::time::Duration;
use std::thread::sleep;

fn main() {
    // Create the mDNS daemon
    let mdns = ServiceDaemon::new().expect("Failed to create daemon");

    // Define the type of service we want to find
    let _service_type = "_walkietalkie._udp.local.";
    let test_type = "_ftp._tcp.local.";

    // Start browsing for services of this type
    let receiver = mdns.browse(test_type)
        .expect("Failed to browse for services");

    println!("🔍 Browsing for services...");

      // 1️⃣ UDP socket (for talking)
    let udp_socket = UdpSocket::bind(("0.0.0.0", port))
        .expect("Failed to bind UDP socket");
    udp_socket
        .set_nonblocking(true)
        .expect("Failed to set nonblocking");

    println!("🎧 UDP listening on port {}", port);


    loop {
        // Receive events with a timeout
        while let Ok(event) = receiver.recv_timeout(Duration::from_secs(1)) {
            match event {
                // A service was resolved (we got its name, IP, and port)
                ServiceEvent::ServiceResolved(info) => {
                    // Print out the discovered device info
                    println!(
                        "🎯 Found: {} at {:?}:{}",
                        info.get_fullname(),
                        info.get_addresses(),
                        info.get_port()
                    );
                }
                _ => {
                    // Ignore other events
                }
            }
        }

        // Small pause to avoid busy-waiting
        sleep(Duration::from_millis(200));
    }
}
