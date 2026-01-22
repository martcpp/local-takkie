use std::env;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread::spawn;
use std::time::Duration;
use cpal::traits::StreamTrait;
use env_logger::Env;
use log::{info, warn, error, debug};


mod audio;
mod network;

use network::mdns::Data;
use network::udp::{udp_recv, udp_send,audio_udp_recv, AudioBuffer};
use audio::rad::start_audio_output;
use audio::sad::start_mic_capture;


type Peerlist = Arc<Mutex<Vec<SocketAddr>>>;
fn main() {
    // Initialize logger once at startup
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        error!("Usage: vl <instance_name> <port>");
        return;
    }

    let instance_name = args[1].as_str();
    let port: u16 = args[2]
        .parse()
        .expect("Port must be a number");

    let mdns = Data::new(instance_name, port);
    mdns.announce();

    let peers: Peerlist = Arc::new(Mutex::new(Vec::new()));
    let adio_buffer: AudioBuffer = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    mdns.discovery(peers.clone());

    let udp_socket = UdpSocket::bind(("0.0.0.0", port))
        .expect("Failed to bind UDP socket");
    udp_socket
        .set_nonblocking(true)
        .expect("Failed to set nonblocking");

    info!("🎧 UDP listening on port {}", port);
    audio_udp_recv(port, &udp_socket, adio_buffer.clone());
    let stream = start_audio_output(adio_buffer.clone());
    stream.play().expect("Failed to play audio stream");
    
    // Spawn a thread to monitor buffer size
    let buf_monitor = adio_buffer.clone();
    spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let buf_size = buf_monitor.lock().unwrap().len();
            if buf_size > 0 {
                info!("📊 Audio buffer size: {} samples", buf_size);
            }
        }
    });

    let peers_for_ptt = peers.clone();
    let device_name = instance_name.to_string();
    let udp_socket_send = udp_socket
        .try_clone()
        .expect("Failed to clone UDP socket for send");

    spawn(move || {
        info!("🎤 Audio capture starting...");
        
        // Start mic capture with live peers handle
        let mic = start_mic_capture(&udp_socket, peers_for_ptt.clone());
        mic.play().expect("Failed to start mic stream");
        
        info!("🎤 Microphone stream is live. Streaming audio...");
        
        // Keep the stream alive forever by blocking here
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    });

    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}
