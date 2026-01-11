use std::env;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread::spawn;
use std::time::Duration;

mod mdns;
mod udp;
use mdns::Data;
use udp::{udp_recv, udp_send};


type Peerlist = Arc<Mutex<Vec<SocketAddr>>>;
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: vl <instance_name> <port>");
        return;
    }

    let instance_name = args[1].as_str();
    let port: u16 = args[2]
        .parse()
        .expect("Port must be a number");

    let mdns = Data::new(instance_name, port);
    mdns.announce();

    let peers: Peerlist = Arc::new(Mutex::new(Vec::new()));
    mdns.discovery(peers.clone());

    let udp_socket = UdpSocket::bind(("0.0.0.0", port))
        .expect("Failed to bind UDP socket");
    udp_socket
        .set_nonblocking(true)
        .expect("Failed to set nonblocking");

    udp_recv(port, &udp_socket);

    let peers_for_ptt = peers.clone();
    let device_name = instance_name.to_string();
    let udp_socket_send = udp_socket
        .try_clone()
        .expect("Failed to clone UDP socket for send");

    spawn(move || {
        println!("🎤 Push-to-Talk ready. Press ENTER to speak.");

        loop {
            let mut buffer = String::new();
            if io::stdin().read_line(&mut buffer).is_err() {
                eprintln!("Failed to read input");
                continue;
            }

            let peers_snapshot = peers_for_ptt.lock().unwrap().clone();
            udp_send(
                &udp_socket_send,
                buffer,
                peers_snapshot,
                device_name.to_owned(),
            );
        }
    });

    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}
