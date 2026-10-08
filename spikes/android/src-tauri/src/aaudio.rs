//! The same loopback opened straight on AAudio in low-latency mode, which
//! cpal never asks for. `voice` also tags the streams as a voice call: on
//! Xiaomi that skips the MiSound effect that blocks the fast output path, and
//! turns on echo cancellation.

use std::fmt::Write as _;
use std::sync::atomic::Ordering::Relaxed;

use ndk::audio::{
    AudioCallbackResult, AudioContentType, AudioDirection, AudioFormat, AudioInputPreset,
    AudioPerformanceMode, AudioSharingMode, AudioStream, AudioStreamBuilder, AudioUsage,
};

use crate::audio::{Input, Output, RATE};

pub struct Streams {
    _mic: AudioStream,
    _speaker: AudioStream,
}

pub fn open(
    mut input: Input,
    mut output: Output,
    voice: bool,
    report: &mut String,
) -> Result<Streams, String> {
    let errors = input.shared.clone();
    let mic = builder(AudioDirection::Input, voice)?
        .data_callback(Box::new(move |stream, data, frames| {
            // SAFETY: checked below that the stream is mono f32, and AAudio
            // passes `frames` of them.
            let data = unsafe { std::slice::from_raw_parts(data as *const f32, frames as usize) };
            input.process(data);
            input
                .shared
                .xruns
                .store(stream.x_run_count() as u32, Relaxed);
            AudioCallbackResult::Continue
        }))
        .error_callback(Box::new(move |_, _| errors.count_error()))
        .open_stream()
        .map_err(|e| format!("mic: {e}"))?;

    let errors = output.shared.clone();
    let speaker = builder(AudioDirection::Output, voice)?
        .data_callback(Box::new(move |_, data, frames| {
            // SAFETY: as for the mic.
            let data = unsafe { std::slice::from_raw_parts_mut(data as *mut f32, frames as usize) };
            output.process(data);
            AudioCallbackResult::Continue
        }))
        .error_callback(Box::new(move |_, _| errors.count_error()))
        .open_stream()
        .map_err(|e| format!("speaker: {e}"))?;

    check(&mic, "mic")?;
    check(&speaker, "speaker")?;
    // Two bursts is Google's suggestion for the lowest output latency that
    // still survives scheduling jitter.
    let _ = speaker.set_buffer_size_in_frames(speaker.frames_per_burst() * 2);
    describe(report, "mic", &mic);
    describe(report, "speaker", &speaker);

    mic.request_start().map_err(|e| format!("mic: {e}"))?;
    speaker
        .request_start()
        .map_err(|e| format!("speaker: {e}"))?;
    Ok(Streams {
        _mic: mic,
        _speaker: speaker,
    })
}

fn builder(direction: AudioDirection, voice: bool) -> Result<AudioStreamBuilder, String> {
    let builder = AudioStreamBuilder::new()
        .map_err(|e| e.to_string())?
        .direction(direction)
        .sample_rate(RATE as i32)
        .channel_count(1)
        .format(AudioFormat::PCM_Float)
        .performance_mode(AudioPerformanceMode::LowLatency)
        .sharing_mode(AudioSharingMode::Exclusive);
    Ok(if voice {
        builder
            .usage(AudioUsage::VoiceCommunication)
            .content_type(AudioContentType::Speech)
            .input_preset(AudioInputPreset::VoiceCommunication)
    } else {
        builder
    })
}

/// The callbacks read raw buffers as mono f32, so anything else is unsafe.
fn check(stream: &AudioStream, name: &str) -> Result<(), String> {
    if stream.format() != AudioFormat::PCM_Float || stream.channel_count() != 1 {
        return Err(format!(
            "{name}: got {:?} with {} channels, not mono f32",
            stream.format(),
            stream.channel_count()
        ));
    }
    Ok(())
}

fn describe(out: &mut String, name: &str, s: &AudioStream) {
    let _ = writeln!(
        out,
        "{name}: {} Hz, {:?}, {:?}, {:?}, burst {} frames, buffer {} of {} frames",
        s.sample_rate(),
        s.usage(),
        s.performance_mode(),
        s.sharing_mode(),
        s.frames_per_burst(),
        s.buffer_size_in_frames(),
        s.buffer_capacity_in_frames(),
    );
}
