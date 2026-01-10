use mdns_sd::ServiceDaemon;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread::spawn;
use std::env;
use std::io;

mod announce;
mod udp;
mod discovery;
use announce::Data;
use discovery::discovery;
use udp::{udp_recv, udp_send};

fn main() {
    // Create the mDNS daemon

    let args: Vec<String> = env::args().collect();
    println!("Args: {:?}", args.len());
    let mdns = ServiceDaemon::new().expect("Failed to create daemon");
    let instance_name = args.get(1).unwrap().as_str();
    let port = args.get(2).unwrap().parse::<u16>().unwrap();

    // Define service type

    let announcer = Data::new(instance_name, port);
    let self_addr = SocketAddr::new(announcer.ip, port);
    announcer.announce();
    let service_type = announcer.service_type.as_str();
    // discovery(service_type);


        // 1️⃣ UDP socket (for talking)
    let udp_socket = UdpSocket::bind(("0.0.0.0", port))
        .expect("Failed to bind UDP socket");
    udp_socket.set_nonblocking(true)
                .expect("Failed to set nonblocking");

    // println!("🎧 UDP listening on port {}", port);

    // Shared peer list
    let peers: Arc<Mutex<Vec<SocketAddr>>> = Arc::new(Mutex::new(Vec::new()));

    // // Register service
   


    // // Start browsing for services of this type
    // let receiver = mdns.browse(service_type)
    //     .expect("Failed to browse for services");

    // println!(" Browsing for services...");



    discovery(service_type, self_addr, peers.clone());
    // spawn(move || {
    //     loop {
    //         while let Ok(event) = receiver.recv_timeout(Duration::from_secs(1)) {
    //             match event {
    //                 ServiceEvent::ServiceResolved(info) => {
    //                     println!(
    //                         "Found: {} at {:?}:{}",
    //                         info.get_fullname(),
    //                         info.get_addresses(),
    //                         info.get_port()
    //                     );
    //                 }
    //                 _ => {}
    //             }
    //         }
    //         sleep(Duration::from_millis(200));
    //     }
    // });

    // let peers_clone = peers.clone();

    // spawn(move || {
    //     while let Ok(event) = receiver.recv() {
    //         if let ServiceEvent::ServiceResolved(info) = event {
    //             if let Some(addr) = info.get_addresses().iter().next() {
    //                 let non = addr.to_ip_addr();
    //                 let peer = SocketAddr::new(non, info.get_port());
    //                 if peer == self_addr {
    //                     continue; // Skip self
    //                 }
    //                 peers_clone.lock().unwrap().push(peer);
    //                 println!("🔍 Found peer: {}", peer);
    //             }
    //         }
    //     }
    // });

    // // 4️⃣ Receive UDP messages
    // let udp_recv = udp_socket.try_clone().unwrap();
    // spawn(move || {
    //     let mut buf = [0u8; 1024];
    //     loop {
    //         if let Ok((len, from)) = udp_recv.recv_from(&mut buf) {
    //             let msg = String::from_utf8_lossy(&buf[..len]);
    //             println!("📨 From {} → {}", from, msg);
    //         }
    //         sleep(Duration::from_millis(50));
    //     }
    // });
    udp_recv(port, &udp_socket);


// let udp_send = udp_socket.try_clone().unwrap();
let peers_ptt = peers.clone();
let device_name = instance_name.to_string();

spawn(move || {
    println!("🎤 Push-to-Talk ready. Press ENTER to speak.");

    loop {
        let mut buffer = String::new();
        io::stdin().read_line(&mut buffer).unwrap();

        let peers_snapshot = peers_ptt.lock().unwrap().clone();
        udp_send(&udp_socket, buffer.clone(), peers_snapshot, device_name.to_owned());
        // for peer in peers_snapshot {
        //     let msg = format!("🎙 {} says {}", device_name, buffer.trim());
        //     let _ = udp_send(&udp_socket, &msg, peer);
        // }
        // sleep(Duration::from_secs(3));
    }
    
});


    // 5️⃣ Send messages periodically
    // loop {
    //     let peers_snapshot = peers.lock().unwrap().clone();
    //     for peer in peers_snapshot {
    //         let msg = format!("Hello from {}", instance_name);
    //         let _ = udp_socket.send_to(msg.as_bytes(), peer);
    //     }
    //     sleep(Duration::from_secs(3));
    // }

//     // 9️⃣ Keep main thread alive (so announcement continues)
//     // loop {
//     //     sleep(Duration::from_secs(60));


    // }

    loop {
    std::thread::sleep(std::time::Duration::from_secs(60));
}
}
