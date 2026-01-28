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
mod ui;

use network::mdns::Data;
use network::udp::{udp_recv, udp_send,audio_udp_recv, AudioBuffer};
use audio::rad::start_audio_output;
use audio::sad::start_mic_capture;
use ui::tui::{run_tui, AppState};


type Peerlist = Arc<Mutex<Vec<SocketAddr>>>;

fn main() {
    // Don't initialize env_logger when using TUI
    // env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    
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
    let local_ip = mdns.ip.to_string();
    mdns.announce();

    let peers: Peerlist = Arc::new(Mutex::new(Vec::new()));
    let audio_buffer: AudioBuffer = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let buffer_size_tracker = Arc::new(Mutex::new(0usize));
    
    mdns.discovery(peers.clone());

    let udp_socket = UdpSocket::bind(("0.0.0.0", port))
        .expect("Failed to bind UDP socket");
    udp_socket
        .set_nonblocking(true)
        .expect("Failed to set nonblocking");

    // Create app state
    let app_state = Arc::new(AppState::new(
        instance_name.to_string(),
        local_ip,
        port,
        peers.clone(),
        buffer_size_tracker.clone(),
    ));

    app_state.add_event("🎧 UDP listening started".to_string());
    
    audio_udp_recv(port, &udp_socket, audio_buffer.clone());
    let stream = start_audio_output(audio_buffer.clone());
    stream.play().expect("Failed to play audio stream");
    
    app_state.add_event("🔊 Audio output stream started".to_string());

    // Spawn a thread to monitor buffer size and update app state
    let buf_monitor = audio_buffer.clone();
    let buf_tracker = buffer_size_tracker.clone();
    spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let buf_size = buf_monitor.lock().unwrap().len();
            *buf_tracker.lock().unwrap() = buf_size;
        }
    });

    // Peer discovery event logger
    let app_state_clone = app_state.clone();
    let peers_clone = peers.clone();
    let mut known_peers = std::collections::HashSet::new();
    spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let current_peers = peers_clone.lock().unwrap().clone();
            for peer in current_peers {
                if !known_peers.contains(&peer) {
                    known_peers.insert(peer);
                    app_state_clone.add_event(format!("✅ Found new peer: {}", peer));
                }
            }
        }
    });

    let peers_for_ptt = peers.clone();
    let ptt_flag = app_state.ptt_active.clone();
    let app_state_for_mic = app_state.clone();
    
    spawn(move || {
        // Start mic capture with PTT control
        let mic = start_mic_capture(&udp_socket, peers_for_ptt.clone(), ptt_flag.clone());
        mic.play().expect("Failed to start mic stream");
        
        app_state_for_mic.add_event("🎤 Microphone stream is live".to_string());
        
        // Keep the stream alive forever
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    });

    // Run the TUI - this blocks until user quits
    if let Err(e) = run_tui(app_state.clone()) {
        eprintln!("TUI error: {}", e);
    }
}
