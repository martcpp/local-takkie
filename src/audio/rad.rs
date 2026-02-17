use cpal::traits::{DeviceTrait, HostTrait};
use log::{error, info};
use opus::{Channels, Decoder};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

pub type AudioBuffer = Arc<Mutex<VecDeque<u8>>>;

pub fn start_audio_output(buffer: AudioBuffer) -> cpal::Stream {
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let config = device.default_output_config().unwrap();
    info!("Output config: {:?}", config);

    let channels = config.channels() as usize;

    // Force 48kHz — Opus native rate, must match encoder
    let opus_sample_rate: u32 = 48000;

    let opus_channels = if channels == 1 {
        Channels::Mono
    } else {
        Channels::Stereo
    };

    let decoder = Arc::new(Mutex::new(
        Decoder::new(opus_sample_rate, opus_channels)
            .expect("Failed to create Opus decoder"),
    ));

    let pcm_buffer: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
    let frame_count = Arc::new(Mutex::new(0usize));
    let frame_count_clone = frame_count.clone();

    // 20ms frame at 48kHz = 960 samples per channel
    let frame_size = (opus_sample_rate as usize / 1000) * 20 * channels;

    // Force stream to 48kHz
    let stream_config = cpal::StreamConfig {
        channels: config.channels(),
        sample_rate: opus_sample_rate, // u32 directly
        buffer_size: cpal::BufferSize::Default,
    };

    device
        .build_output_stream(
            &stream_config,
            move |output: &mut [f32], _| {
                let mut count = frame_count_clone.lock().unwrap();
                *count += 1;

                {
                    let mut opus_buf = buffer.lock().unwrap();
                    let mut dec = decoder.lock().unwrap();
                    let mut pcm = pcm_buffer.lock().unwrap();

                    // Each packet is length-prefixed: [u16 len][packet bytes]
                    while opus_buf.len() >= 2 {
                        let len = {
                            let b0 = opus_buf[0] as u16;
                            let b1 = opus_buf[1] as u16;
                            (b0 | (b1 << 8)) as usize
                        };

                        // Wait until full packet is available
                        if opus_buf.len() < 2 + len {
                            break;
                        }

                        opus_buf.drain(..2);
                        let packet: Vec<u8> = opus_buf.drain(..len).collect();

                        let mut decoded = vec![0f32; frame_size];
                        match dec.decode_float(&packet, &mut decoded, false) {
                            Ok(samples) => {
                                pcm.extend(decoded[..samples * channels].iter());
                            }
                            Err(e) => error!("Opus decode error: {e}"),
                        }
                    }
                }

                let mut pcm = pcm_buffer.lock().unwrap();
                let mut samples_written = 0;

                for sample in output.iter_mut() {
                    if let Some(v) = pcm.pop_front() {
                        *sample = v;
                        samples_written += 1;
                    } else {
                        *sample = 0.0; // silence on underrun
                    }
                }

                if *count % 100 == 0 {
                    info!(
                        "Output callback #{}: wrote {} samples, pcm buffer: {} samples",
                        count, samples_written, pcm.len()
                    );
                }
            },
            |err| error!("Audio error: {}", err),
            None,
        )
        .unwrap()
}