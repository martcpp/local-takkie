use std::net::{SocketAddr, UdpSocket};
use std::thread::{spawn,sleep};
use std::time::Duration;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use log::{info, debug, trace};


pub type AudioBuffer = Arc<Mutex<VecDeque<f32>>>;

pub fn udp_recv(port: u16, udp_socket: &UdpSocket) {
    info!("🎧 UDP listening on port {}", port);


    let udp_recv = udp_socket.try_clone().unwrap();
    spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            if let Ok((len, from)) = udp_recv.recv_from(&mut buf) {
                let msg = String::from_utf8_lossy(&buf[..len]);
                debug!("From {} → {}", from, msg);
            }
            sleep(Duration::from_millis(50));
        }
    });
}

pub fn audio_udp_recv(
    port: u16,
    udp_socket: &UdpSocket,
    audio_buffer: AudioBuffer, // 👈 ADD THIS
) {
    info!("🎧 UDP listening on port {}", port);

    let udp_recv = udp_socket.try_clone().unwrap();

    spawn(move || {
        // Use a large buffer to avoid truncating UDP packets with audio payloads
        let mut buf = [0u8; 65535];

        loop {
            if let Ok((len, from)) = udp_recv.recv_from(&mut buf) {
                // IMPORTANT PART
                // Convert raw bytes → f32 samples
                if len % 4 != 0 {
                    debug!("Incomplete audio packet: {} bytes (not multiple of 4)", len);
                } else {
                    let samples = buf[..len]
                        .chunks_exact(4) // f32 = 4 bytes
                        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));

                    let mut buffer = audio_buffer.lock().unwrap();
                    for sample in samples {
                        buffer.push_back(sample);
                    }
                    trace!("From {} → received {} bytes of audio, buffer size: {}", from, len, buffer.len());
                }
            }

            // Small sleep to yield CPU without adding audible latency
            sleep(Duration::from_millis(1));
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


pub fn udp_send_audio(
    udp_socket: &UdpSocket,
    audio_bytes: &[u8],
    peers_snapshot: &[SocketAddr],
) {
    use log::warn;
    if peers_snapshot.is_empty() {
        return;
    }

    let udp_snd = udp_socket.try_clone().unwrap();

    for peer in peers_snapshot {
        if let Err(e) = udp_snd.send_to(audio_bytes, peer) {
            warn!("Failed to send {} bytes to {}: {}", audio_bytes.len(), peer, e);
        }
    }
}
