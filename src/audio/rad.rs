use cpal::traits::{DeviceTrait, HostTrait};
use log::{error, info};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

pub type AudioBuffer = Arc<Mutex<VecDeque<f32>>>;

pub fn start_audio_output(buffer: AudioBuffer) -> cpal::Stream {
    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let config = device.default_output_config().unwrap();
    info!("Output config: {:?}", config);

    let frame_count = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let frame_count_clone = frame_count.clone();

    device
        .build_output_stream(
            &config.into(),
            move |output: &mut [f32], _| {
                let mut count = frame_count_clone.lock().unwrap();
                *count += 1;

                let mut buffer = buffer.lock().unwrap();
                let mut samples_written = 0;

                for sample in output.iter_mut() {
                    if let Some(v) = buffer.pop_front() {
                        *sample = v;
                        samples_written += 1;
                    } else {
                        *sample = 0.0; // silence
                    }
                }

                if *count % 100 == 0 {
                    // Log every 100 callbacks
                    info!(
                        "Output callback #{}: wrote {} samples, buffer now: {}",
                        count,
                        samples_written,
                        buffer.len()
                    );
                }
            },
            |err| error!("Audio error: {}", err),
            None,
        )
        .unwrap()
}
