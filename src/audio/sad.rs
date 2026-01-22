use cpal::traits::{DeviceTrait, HostTrait};
use std::net::{UdpSocket, SocketAddr};
use std::sync::{Arc, Mutex};
use crate::network::udp::udp_send_audio;
use log::{info, error, debug};

pub fn start_mic_capture(
    udp_socket: &UdpSocket,
    peers: Arc<Mutex<Vec<SocketAddr>>>,
) -> cpal::Stream {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .expect("No input device found");

    let config = device.default_input_config().unwrap();
    info!("🎤 Mic config: {:?}", config);

    let socket = udp_socket.try_clone().unwrap();

    let frame_count = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let frame_count_clone = frame_count.clone();
    
    device
        .build_input_stream(
            &config.clone().into(),
            move |input: &[f32], _| {
                let mut count = frame_count_clone.lock().unwrap();
                *count += 1;
                
                if *count % 100 == 0 {  // Log every 100 callbacks to avoid spam
                    info!("🎙️ Mic callback #{}: {} samples", count, input.len());
                }
                
                // Chunk audio into ~20ms frames to avoid oversized UDP packets
                let channels = config.channels() as usize;
                let sample_rate = config.sample_rate() as usize;
                let frame_samples = (sample_rate / 50) * channels; // ~20ms

                for chunk in input.chunks(frame_samples.max(1)) {
                    let audio_bytes: &[u8] = unsafe {
                        std::slice::from_raw_parts(
                            chunk.as_ptr() as *const u8,
                            chunk.len() * 4,
                        )
                    };
                    // Use live peers discovered so far
                    let peers_list = peers.lock().unwrap().clone();
                    if *count % 100 == 0 {
                        debug!("Mic chunk: {} bytes → {} peers", audio_bytes.len(), peers_list.len());
                    }
                    if !peers_list.is_empty() {
                        udp_send_audio(&socket, audio_bytes, &peers_list);
                    }
                }
            },
            |err| error!("Mic error: {}", err),
            None,
        )
        .unwrap()
}
