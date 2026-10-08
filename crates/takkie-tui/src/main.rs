//! Terminal app for local-takkie.

use cpal::traits::StreamTrait;

use std::env;
use std::net::{SocketAddr, UdpSocket};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread::spawn;
use std::time::Duration;

use takkie_engine::{AudioBuffer, Data, audio_udp_recv, start_audio_output, start_mic_capture};
use ui::tui::{AppState, run_tui};

mod ui;

type Peerlist = Arc<Mutex<Vec<SocketAddr>>>;

const USAGE: &str = "usage: takkie [name] [port]

  name  how others see you (default: this computer's name)
  port  UDP port to use (default: any free port)";

fn main() -> ExitCode {
    // Don't initialize env_logger when using TUI
    // env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args: Vec<String> = env::args().skip(1).collect();
    let (name, requested_port) = match parse_args(&args, computer_name) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("takkie: {err}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    let instance_name = name.as_str();

    // Bind first, so a free port picked by the OS is the one we announce.
    let udp_socket = match UdpSocket::bind(("0.0.0.0", requested_port)) {
        Ok(socket) => socket,
        Err(err) => {
            eprintln!("takkie: couldn't use UDP port {requested_port}: {err}");
            return ExitCode::FAILURE;
        }
    };
    let port = match udp_socket.local_addr() {
        Ok(addr) => addr.port(),
        Err(err) => {
            eprintln!("takkie: couldn't read the UDP port: {err}");
            return ExitCode::FAILURE;
        }
    };

    let mdns = Data::new(instance_name, port);
    let local_ip = mdns.ip.to_string();
    mdns.announce();

    let peers: Peerlist = Arc::new(Mutex::new(Vec::new()));
    let audio_buffer: AudioBuffer = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let buffer_size_tracker = Arc::new(Mutex::new(0usize));

    mdns.discovery(peers.clone());

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
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Reads `[name] [port]`. Missing values fall back to `default_name()` and
/// port 0, which lets the OS pick a free one.
fn parse_args(
    args: &[String],
    default_name: impl FnOnce() -> String,
) -> Result<(String, u16), String> {
    match args {
        [] => Ok((default_name(), 0)),
        [name] => Ok((name.clone(), 0)),
        [name, port] => port
            .parse()
            .map(|port| (name.clone(), port))
            .map_err(|_| format!("`{port}` isn't a port number (0 to 65535)")),
        _ => Err("too many arguments".to_owned()),
    }
}

fn computer_name() -> String {
    let name = gethostname::gethostname()
        .to_string_lossy()
        .trim()
        .to_owned();
    if name.is_empty() {
        "takkie".to_owned()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn no_args_uses_defaults() {
        assert_eq!(
            parse_args(&[], || "pc".to_owned()),
            Ok(("pc".to_owned(), 0))
        );
    }

    #[test]
    fn name_only_picks_any_port() {
        assert_eq!(
            parse_args(&args(&["alice"]), || unreachable!()),
            Ok(("alice".to_owned(), 0))
        );
    }

    #[test]
    fn name_and_port() {
        assert_eq!(
            parse_args(&args(&["alice", "5000"]), || unreachable!()),
            Ok(("alice".to_owned(), 5000))
        );
    }

    #[test]
    fn bad_port_is_an_error() {
        assert!(parse_args(&args(&["alice", "nope"]), || unreachable!()).is_err());
        assert!(parse_args(&args(&["alice", "70000"]), || unreachable!()).is_err());
    }

    #[test]
    fn too_many_args_is_an_error() {
        assert!(parse_args(&args(&["a", "1", "x"]), || unreachable!()).is_err());
    }
}
