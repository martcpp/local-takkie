// use crate::network::udp::udp_send_audio;
// use cpal::traits::{DeviceTrait, HostTrait};
// use opus::{Application, Channels, Encoder};
// use std::net::{SocketAddr, UdpSocket};
// use std::sync::{
//     Arc, Mutex,
//     atomic::{AtomicBool, Ordering},
// };

// pub fn start_mic_capture(
//     udp_socket: &UdpSocket,
//     peers: Arc<Mutex<Vec<SocketAddr>>>,
//     ptt_enabled: Arc<AtomicBool>,
// ) -> cpal::Stream {
//     let host = cpal::default_host();
//     let device = host.default_input_device().expect("No input device found");

//     let config = device.default_input_config().unwrap();

//     let socket = udp_socket.try_clone().unwrap();

//     let channels = config.channels() as usize;
//     let sample_rate = config.sample_rate();

//     // Create Opus encoder outside the callback so it's reused across frames
//     let opus_channels = if channels == 1 {
//         Channels::Mono
//     } else {
//         Channels::Stereo
//     };
//     let encoder = Arc::new(Mutex::new(
//         Encoder::new(sample_rate, opus_channels, Application::Voip)
//             .expect("Failed to create Opus encoder"),
//     ));

//     // Buffer to accumulate samples until we have a full Opus frame
//     let sample_buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

//     // Opus requires exactly 2.5, 5, 10, 20, 40, or 60ms frames
//     // 20ms at 48kHz = 960 samples per channel
//     let frame_size = (sample_rate as usize / 1000) * 20 * channels; // e.g. 960 * channels

//     device
//         .build_input_stream(
//             &config.clone().into(),
//             move |input: &[f32], _| {
//                 if !ptt_enabled.load(Ordering::Relaxed) {
//                     return;
//                 }

//                 let peers_list = peers.lock().unwrap().clone();
//                 if peers_list.is_empty() {
//                     return;
//                 }

//                 // Accumulate incoming samples into the buffer
//                 let mut buffer = sample_buffer.lock().unwrap();
//                 buffer.extend_from_slice(input);

//                 // Drain complete frames from the buffer
//                 while buffer.len() >= frame_size {
//                     let frame: Vec<f32> = buffer.drain(..frame_size).collect();

//                     let mut enc = encoder.lock().unwrap();
//                     let mut encoded = vec![0u8; 4000]; // Max Opus packet size

//                     match enc.encode_float(&frame, &mut encoded) {
//                         Ok(len) => {
//                             encoded.truncate(len);
//                             udp_send_audio(&socket, &encoded, &peers_list);
//                         }
//                         Err(e) => eprintln!("Opus encode error: {e}"),
//                     }
//                 }
//             },
//             |err| eprintln!("Stream error: {err}"),
//             None,
//         )
//         .unwrap()
// }


use crate::network::udp::udp_send_audio;
use cpal::traits::{DeviceTrait, HostTrait};
use log::info;
use opus::{Application, Channels, Encoder};
use std::net::{SocketAddr, UdpSocket};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub fn start_mic_capture(
    udp_socket: &UdpSocket,
    peers: Arc<Mutex<Vec<SocketAddr>>>,
    ptt_enabled: Arc<AtomicBool>,
) -> cpal::Stream {
    let host = cpal::default_host();
    let device = host.default_input_device().expect("No input device found");
    let config = device.default_input_config().unwrap();

    let socket = udp_socket.try_clone().unwrap();

    let channels = config.channels() as usize;

    // ✅ FIX: Force 48kHz - must match decoder
    let opus_sample_rate: u32 = 48000;

    info!("Input device: channels={}, forcing 48kHz for Opus", channels);

    let opus_channels = if channels == 1 {
        Channels::Mono
    } else {
        Channels::Stereo
    };

    let encoder = Arc::new(Mutex::new(
        Encoder::new(opus_sample_rate, opus_channels, Application::Voip)
            .expect("Failed to create Opus encoder"),
    ));

    let sample_buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

    // ✅ FIX: Frame size based on Opus rate (48kHz)
    let frame_size = (opus_sample_rate as usize / 1000) * 20 * channels;

    // ✅ FIX: Force stream config to 48kHz
    let stream_config = cpal::StreamConfig {
        channels: config.channels(),
        sample_rate: opus_sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    device
        .build_input_stream(
            &stream_config,
            move |input: &[f32], _| {
                if !ptt_enabled.load(Ordering::Relaxed) {
                    return;
                }

                let peers_list = peers.lock().unwrap().clone();
                if peers_list.is_empty() {
                    return;
                }

                let mut buffer = sample_buffer.lock().unwrap();
                buffer.extend_from_slice(input);

                while buffer.len() >= frame_size {
                    let frame: Vec<f32> = buffer.drain(..frame_size).collect();

                    let mut enc = encoder.lock().unwrap();
                    let mut encoded = vec![0u8; 4000];

                    match enc.encode_float(&frame, &mut encoded) {
                        Ok(len) => {
                            encoded.truncate(len);
                            udp_send_audio(&socket, &encoded, &peers_list);
                        }
                        Err(e) => {
                            info!("Opus encode error: {e}");
                        }
                    }
                }
            },
            |err| info!("Stream error: {err}"),
            None,
        )
        .unwrap()
}