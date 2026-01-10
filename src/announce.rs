use std::{collections::HashMap, net::{IpAddr, Ipv4Addr}};
use mdns_sd::{ServiceDaemon, ServiceInfo};


pub struct Data{
    pub service_type:String,
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
        let ip: IpAddr = Ipv4Addr::new(127, 0, 0, 1).into();
        let host_name = "mart-device-1.local.".to_string();
        let port = port;
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
        println!("Announcing service as {} on port {}", self.instance_name, self.port);
        println!("Keep this running... announce");
    }


    
}
