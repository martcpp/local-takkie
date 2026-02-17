use log::{debug, info, trace};
use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread::{sleep, spawn};
use std::time::Duration;

pub type AudioBuffer = Arc<Mutex<VecDeque<u8>>>;

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

pub fn udp_send(
    udp_socket: &UdpSocket,
    input: String,
    peers_snapshot: Vec<SocketAddr>,
    device_name: String,
) {
    let udp_snd = udp_socket.try_clone().unwrap();

    for peer in &peers_snapshot {
        let msg = format!("🎙 {} says {}", device_name, input.trim());
        let _ = udp_snd.send_to(msg.as_bytes(), peer);
    }
    // sleep(Duration::from_secs(3));
}

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
