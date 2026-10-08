use log::{debug, info, trace};
use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread::{sleep, spawn};
use std::time::Duration;

/// Length-prefixed Opus packets waiting to be decoded.
pub type AudioBuffer = Arc<Mutex<VecDeque<u8>>>;

// pub fn udp_recv(port: u16, udp_socket: &UdpSocket) {
//     info!("🎧 UDP listening on port {}", port);

//     let udp_recv = udp_socket.try_clone().unwrap();
//     spawn(move || {
//         let mut buf = [0u8; 1024];
//         loop {
//             if let Ok((len, from)) = udp_recv.recv_from(&mut buf) {
//                 let msg = String::from_utf8_lossy(&buf[..len]);
//                 debug!("From {} → {}", from, msg);
//             }
//             sleep(Duration::from_millis(50));
//         }
//     });
// }

/// Spawns a thread that receives packets on `udp_socket` and queues them in
/// `audio_buffer`.
///
/// # Panics
///
/// If the socket can't be cloned.
pub fn audio_udp_recv(port: u16, udp_socket: &UdpSocket, audio_buffer: AudioBuffer) {
    info!("🎧 UDP listening on port {}", port);

    let udp_recv = udp_socket.try_clone().unwrap();

    spawn(move || {
        let mut buf = [0u8; 65535];

        loop {
            if let Ok((len, from)) = udp_recv.recv_from(&mut buf) {
                if len == 0 {
                    debug!("Empty packet from {}", from);
                } else {
                    // IMPORTANT PART
                    // Each UDP packet IS one Opus packet — no reassembly needed
                    // Push it length-prefixed into the buffer for the decoder
                    push_opus_packet(&audio_buffer, &buf[..len]);

                    trace!(
                        "From {} → received Opus packet {} bytes, buffer size: {}",
                        from,
                        len,
                        audio_buffer.lock().unwrap().len()
                    );
                }
            }

            sleep(Duration::from_millis(1));
        }
    });
}

// pub fn udp_send(
//     udp_socket: &UdpSocket,
//     input: String,
//     peers_snapshot: Vec<SocketAddr>,
//     device_name: String,
// ) {
//     let udp_snd = udp_socket.try_clone().unwrap();

//     for peer in &peers_snapshot {
//         let msg = format!("🎙 {} says {}", device_name, input.trim());
//         let _ = udp_snd.send_to(msg.as_bytes(), peer);
//     }
//     // sleep(Duration::from_secs(3));
// }

/// Sends `audio_bytes` to every peer. Send errors are logged and skipped.
///
/// # Panics
///
/// If the socket can't be cloned.
pub fn udp_send_audio(udp_socket: &UdpSocket, audio_bytes: &[u8], peers_snapshot: &[SocketAddr]) {
    use log::warn;
    if peers_snapshot.is_empty() {
        return;
    }

    let udp_snd = udp_socket.try_clone().unwrap();

    for peer in peers_snapshot {
        if let Err(e) = udp_snd.send_to(audio_bytes, peer) {
            warn!(
                "Failed to send {} bytes to {}: {}",
                audio_bytes.len(),
                peer,
                e
            );
        }
    }
}

// In your UDP receive handler, before pushing into AudioBuffer:
fn push_opus_packet(buffer: &AudioBuffer, packet: &[u8]) {
    let mut buf = buffer.lock().unwrap();
    let len = packet.len() as u16;
    buf.push_back(len as u8); // low byte
    buf.push_back((len >> 8) as u8); // high byte
    buf.extend(packet.iter().copied());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn test_push_opus_packet_empty() {
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let packet = vec![];
        push_opus_packet(&buffer, &packet);

        let buf = buffer.lock().unwrap();
        assert_eq!(buf.len(), 2); // Just the length prefix
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
    }

    #[test]
    fn test_push_opus_packet_small() {
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let packet = vec![1, 2, 3, 4, 5];
        push_opus_packet(&buffer, &packet);

        let buf = buffer.lock().unwrap();
        assert_eq!(buf.len(), 7); // 2 bytes length + 5 bytes data
        assert_eq!(buf[0], 5); // low byte of length
        assert_eq!(buf[1], 0); // high byte of length
        assert_eq!(buf[2], 1);
        assert_eq!(buf[3], 2);
        assert_eq!(buf[4], 3);
    }

    #[test]
    fn test_push_opus_packet_large() {
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let packet = vec![0xFF; 300]; // 300 bytes
        push_opus_packet(&buffer, &packet);

        let buf = buffer.lock().unwrap();
        assert_eq!(buf.len(), 302); // 2 bytes length + 300 bytes data
        assert_eq!(buf[0], 44); // 300 & 0xFF
        assert_eq!(buf[1], 1); // 300 >> 8
    }

    #[test]
    fn test_push_opus_packet_multiple() {
        let buffer = Arc::new(Mutex::new(VecDeque::new()));

        // Push first packet
        let packet1 = vec![1, 2, 3];
        push_opus_packet(&buffer, &packet1);

        // Push second packet
        let packet2 = vec![4, 5];
        push_opus_packet(&buffer, &packet2);

        let buf = buffer.lock().unwrap();
        // First packet: 2 bytes length + 3 bytes data
        // Second packet: 2 bytes length + 2 bytes data
        assert_eq!(buf.len(), 9);

        // Verify first packet
        assert_eq!(buf[0], 3); // length of first packet
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 1);
        assert_eq!(buf[3], 2);
        assert_eq!(buf[4], 3);

        // Verify second packet
        assert_eq!(buf[5], 2); // length of second packet
        assert_eq!(buf[6], 0);
        assert_eq!(buf[7], 4);
        assert_eq!(buf[8], 5);
    }

    #[test]
    fn test_udp_send_audio_empty_peers() {
        // Bind to any available port
        let socket = UdpSocket::bind("127.0.0.1:0").expect("Failed to bind socket");
        let peers = vec![];
        let audio_data = vec![1, 2, 3, 4, 5];

        // Should not panic with empty peers
        udp_send_audio(&socket, &audio_data, &peers);
    }

    #[test]
    fn test_audio_buffer_thread_safety() {
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let buffer_clone = buffer.clone();

        let handle = std::thread::spawn(move || {
            for i in 0..100 {
                let packet = vec![i as u8];
                push_opus_packet(&buffer_clone, &packet);
            }
        });

        handle.join().unwrap();

        let buf = buffer.lock().unwrap();
        // Each packet: 2 bytes length + 1 byte data = 3 bytes
        assert_eq!(buf.len(), 300);
    }
}
