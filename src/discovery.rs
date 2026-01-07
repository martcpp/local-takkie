use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::thread::spawn;
use std::time::Duration;
use std::thread::sleep;
// mod announce;
// use crate::announce::Data;

pub fn discovery(service_type:  &str) {
     let mdns = ServiceDaemon::new().expect("Failed to create daemon");
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

}