use std::{collections::HashMap, net::{IpAddr, Ipv4Addr}};
use mdns_sd::{ServiceDaemon, ServiceInfo, ServiceEvent};
use std::thread::spawn;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use local_ip_address::local_ip; 
use log::{info, warn};

type Peerlist = Arc<Mutex<Vec<SocketAddr>>>;


pub struct Data{
    pub service_type: String,
    instance_name: String,
    pub ip: IpAddr,
    host_name: String,
    port: u16,
    properties: HashMap<String, String>,
}

impl Data {
    pub fn new(instant_name: &str, port: u16) -> Self {
        let service_type = "_walkietalkie._udp.local.".to_string();
        let instance_name = instant_name.to_string();
        
        // Automatically get local IP instead of hardcoding
        let ip = local_ip().unwrap_or_else(|_| {
            warn!("Failed to get local IP, using localhost");
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        });
        
        info!("Using local IP: {}", ip);
        
        // Generate hostname from instance name
        let host_name = format!("{}.local.", instant_name.replace(" ", "-").to_lowercase());
        let properties = HashMap::new();

        Data {
            service_type,
            instance_name,
            ip,
            host_name,
            port,
            properties,
        }
    }

    pub fn service_info(&self) -> ServiceInfo {
        ServiceInfo::new(
            self.service_type.as_str(),
            self.instance_name.as_str(),
            self.host_name.as_str(),
            self.ip,
            self.port,
            Some(self.properties.clone()),
        ).expect("Failed to create service info")
    }

    pub fn announce(&self) {
        let mdns = ServiceDaemon::new().expect("Failed to create daemon");
        mdns.register(self.service_info()).expect("Failed to register service");
        info!("Announcing service as {} on {}:{}", 
             self.instance_name, self.ip, self.port);
        info!("Keep this running... announce");
    }

    pub fn discovery(&self, peers_clone: Peerlist) {
        let mdns = ServiceDaemon::new().expect("Failed to create daemon");
        let receiver = mdns.browse(&self.service_type)
            .expect("Failed to browse for services");
        let self_addr = SocketAddr::new(self.ip, self.port);

        info!("Browsing for services... discovery");

        spawn(move || {
            while let Ok(event) = receiver.recv() {
                if let ServiceEvent::ServiceResolved(info) = event {
                    if let Some(addr) = info.get_addresses().iter().next() {
                        let non = addr.to_ip_addr();
                        let peer = SocketAddr::new(non, info.get_port());
                        if peer == self_addr {
                            continue; // Skip self
                        }
                        let mut peers = peers_clone.lock().unwrap();
                        if !peers.contains(&peer) {
                            peers.push(peer);
                            info!("Found new peer: {}", peer);
                        }
                    }
                }
            }
        });
    }
}