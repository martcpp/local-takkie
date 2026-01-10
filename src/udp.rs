use std::net::{SocketAddr, UdpSocket};
use std::thread::{spawn,sleep};
use std::time::Duration;



pub fn udp_recv(port: u16, udp_socket: &UdpSocket) {
    println!("🎧 UDP listening on port {}", port);


    let udp_recv = udp_socket.try_clone().unwrap();
    spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            if let Ok((len, from)) = udp_recv.recv_from(&mut buf) {
                let msg = String::from_utf8_lossy(&buf[..len]);
                println!("📨 From {} → {}", from, msg);
            }
            sleep(Duration::from_millis(50));
        }
    });
}

pub fn udp_send(udp_socket: &UdpSocket, input: String, peers_snapshot: Vec<SocketAddr>, device_name: String) {
    
 
    let udp_snd = udp_socket.try_clone().unwrap();

    for peer in &peers_snapshot {
            let msg = format!("🎙 {} says {}", device_name, input.trim());
            let _ = udp_snd.send_to(msg.as_bytes(), peer);
        }
        // sleep(Duration::from_secs(3));
    

}