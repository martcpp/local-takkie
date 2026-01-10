use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::thread::spawn;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

type Peerlist = Arc<Mutex<Vec<SocketAddr>>>;
pub fn discovery(service_type: &str, self_addr: SocketAddr, peers_clone: Peerlist) {
     let mdns = ServiceDaemon::new().expect("Failed to create daemon");
        let receiver = mdns.browse(service_type)
        .expect("Failed to browse for services");

    println!(" Browsing for services... discovery");

       spawn(move || {
        while let Ok(event) = receiver.recv() {
            if let ServiceEvent::ServiceResolved(info) = event {
                if let Some(addr) = info.get_addresses().iter().next() {
                    let non = addr.to_ip_addr();
                    let peer = SocketAddr::new(non, info.get_port());
                    if peer == self_addr {
                        continue; // Skip self
                    }
                    peers_clone.lock().unwrap().push(peer);
                    println!("🔍 Found peer: {}", peer);
                }
            }
        }
    });

}