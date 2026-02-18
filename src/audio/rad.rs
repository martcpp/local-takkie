use cpal::traits::{DeviceTrait, HostTrait};
use log::{error, info};
use opus::{Channels, Decoder};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

pub type AudioBuffer = Arc<Mutex<VecDeque<u8>>>; // now holds raw Opus packets

const OPUS_SAMPLE_RATES: [u32; 5] = [48000, 24000, 16000, 12000, 8000];

fn pick_opus_output_rate(device: &cpal::Device, config: &cpal::SupportedStreamConfig) -> u32 {
    let default_rate = config.sample_rate();
    if OPUS_SAMPLE_RATES.contains(&default_rate) {
        return default_rate;
    }

    let target_channels = config.channels();
    let target_format = config.sample_format();
    let ranges: Vec<_> = device
        .supported_output_configs()
        .map(|iter| iter.collect())
        .unwrap_or_default();

    for rate in OPUS_SAMPLE_RATES {
        let supported = ranges.iter().any(|range| {
            range.channels() == target_channels
                && range.sample_format() == target_format
                && rate >= range.min_sample_rate()
                && rate <= range.max_sample_rate()
        });
        if supported {
            return rate;
        }
    }

    error!(
        "No Opus-compatible output sample rate supported; falling back to 48000 Hz"
    );
    48000
}

pub fn start_audio_output(buffer: AudioBuffer) -> cpal::Stream {
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let config = device.default_output_config().unwrap();
    let opus_rate = pick_opus_output_rate(&device, &config);
    let mut stream_config: cpal::StreamConfig = config.clone().into();
    stream_config.sample_rate = opus_rate;
    info!("Output config: {:?}", config);

    let channels = config.channels() as usize;
    let sample_rate = opus_rate;

    let opus_channels = if channels == 1 {
        Channels::Mono
    } else {
        Channels::Stereo
    };
    let decoder = Arc::new(Mutex::new(
        Decoder::new(sample_rate, opus_channels).expect("Failed to create Opus decoder"),
    ));

    // Decoded PCM samples ready to be played
    let pcm_buffer: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));

    let frame_count = Arc::new(Mutex::new(0usize));
    let frame_count_clone = frame_count.clone();

    // 20ms frame size matching the encoder
    let frame_size = (sample_rate as usize / 1000) * 20 * channels;

    device
        .build_output_stream(
            &stream_config,
            move |output: &mut [f32], _| {
                let mut count = frame_count_clone.lock().unwrap();
                *count += 1;

                // Decode any pending Opus packets into the PCM buffer
                {
                    let mut opus_buf = buffer.lock().unwrap();
                    let mut dec = decoder.lock().unwrap();
                    let mut pcm = pcm_buffer.lock().unwrap();

                    // Each packet is length-prefixed: [u16 len][packet bytes]
                    while opus_buf.len() >= 2 {
                        // Peek at the packet length prefix
                        let len = {
                            let b0 = opus_buf[0] as u16;
                            let b1 = opus_buf[1] as u16;
                            (b0 | (b1 << 8)) as usize
                        };

                        // Wait until the full packet has arrived
                        if opus_buf.len() < 2 + len {
                            break;
                        }

                        // Consume the length prefix
                        opus_buf.drain(..2);

                        // Consume the packet bytes
                        let packet: Vec<u8> = opus_buf.drain(..len).collect();

                        let mut decoded = vec![0f32; frame_size];
                        match dec.decode_float(&packet, &mut decoded, false) {
                            Ok(samples) => {
                                // samples is per-channel count; total = samples * channels
                                pcm.extend(decoded[..samples * channels].iter());
                            }
                            Err(e) => error!("Opus decode error: {e}"),
                        }
                    }
                }

                // Write decoded PCM to the output, silence if underrun
                let mut pcm = pcm_buffer.lock().unwrap();
                let mut samples_written = 0;

                for sample in output.iter_mut() {
                    if let Some(v) = pcm.pop_front() {
                        *sample = v;
                        samples_written += 1;
                    } else {
                        *sample = 0.0; // buffer underrun — fill with silence
                    }
                }

                if *count % 100 == 0 {
                    info!(
                        "Output callback #{}: wrote {} samples, pcm buffer: {} samples",
                        count,
                        samples_written,
                        pcm.len()
                    );
                }
            },
            |err| error!("Audio error: {}", err),
            None,
        )
        .unwrap()
}
