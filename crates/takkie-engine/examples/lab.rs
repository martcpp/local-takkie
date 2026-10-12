//! The voice lab: record a clip, then hear what the far end would get.
//!
//! `cargo run -p takkie-engine --example lab -- record clip.wav [--seconds N] [--input NAME]`
//! `cargo run -p takkie-engine --example lab -- send clip.wav heard.wav [--loss PERCENT] [--raw]`
//!
//! `--raw` leaves the voice processing out, to compare with and without.

use std::error::Error;
use std::io::{self, Write};
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use takkie_core::dsp::downmix_to_mono;
use takkie_engine::Direction;
use takkie_engine::audio::config::{PREFERRED_RATE, input_config};
use takkie_engine::audio::devices::find_device;
use takkie_engine::audio::lab::{Clip, Loss, Report, far_end, frames, process};
use takkie_engine::audio::stream::CpalSource;
use takkie_engine::audio::tx::Framer;
use takkie_engine::audio::voice::VoiceControls;

const USAGE: &str = "usage:
  lab record <out.wav> [--seconds N] [--input NAME]
  lab send <in.wav> <out.wav> [--loss PERCENT] [--raw]";

type Outcome<T> = Result<T, Box<dyn Error>>;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Outcome<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut paths = Vec::new();
    let mut seconds = 10_u32;
    let mut input = None;
    let mut loss = Loss::default();
    let mut raw = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--seconds" => seconds = value(rest.next(), "--seconds")?,
            "--input" => input = Some(rest.next().ok_or("--input needs a name")?.as_str()),
            "--loss" => {
                loss = Loss::percent(value(rest.next(), "--loss")?)
                    .ok_or("--loss is a percentage, 0 to 100")?;
            }
            "--raw" => raw = true,
            other => paths.push(other),
        }
    }
    match paths.as_slice() {
        ["record", out] => record(out, seconds, input),
        ["send", from, to] => send(from, to, loss, raw),
        _ => Err(USAGE.into()),
    }
}

fn value<T: std::str::FromStr>(arg: Option<&String>, name: &str) -> Outcome<T> {
    arg.and_then(|text| text.parse().ok())
        .ok_or_else(|| format!("{name} needs a number").into())
}

fn record(path: &str, seconds: u32, input: Option<&str>) -> Outcome<()> {
    let mut out = io::stdout().lock();
    let chosen = find_device(Direction::Input, input)?;
    if let Some(warning) = &chosen.warning {
        writeln!(out, "warning: {warning}")?;
    }
    let mut source = CpalSource::open(&chosen.device, &input_config(&chosen.device)?)?;
    let mut framer = Framer::new(source.settings().sample_rate)?;
    writeln!(out, "recording {seconds} s from {}", chosen.name)?;

    let wanted = seconds as usize * PREFERRED_RATE as usize;
    let mut voice = Vec::with_capacity(wanted);
    while voice.len() < wanted {
        match framer.next(&mut source)? {
            Some(frame) => voice.extend_from_slice(frame),
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
    drop(source);
    write_wav(path, &voice)?;
    writeln!(out, "{path}: {}", Report::of(&voice))?;
    Ok(())
}

fn send(from: &str, to: &str, loss: Loss, raw: bool) -> Outcome<()> {
    let mut out = io::stdout().lock();
    let mut voice = frames(&mut read_wav(from)?)?;
    writeln!(out, "clip   {}", Report::of(&voice))?;
    if !raw {
        process(&mut voice, &VoiceControls::default());
    }
    let heard = far_end(&voice, loss)?;
    write_wav(to, &heard)?;
    writeln!(out, "sent   {}", Report::of(&voice))?;
    writeln!(out, "heard  {}", Report::of(&heard))?;
    Ok(())
}

fn read_wav(path: &str) -> Outcome<Clip> {
    let reader = WavReader::open(path)?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels);
    if channels == 0 || spec.sample_rate == 0 {
        return Err(format!("{path}: no channels or no sample rate").into());
    }
    let samples: Vec<f32> = match spec.sample_format {
        SampleFormat::Float => reader.into_samples::<f32>().collect::<Result<_, _>>()?,
        SampleFormat::Int => {
            let full_scale = 2_f32.powi(i32::from(spec.bits_per_sample) - 1);
            reader
                .into_samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / full_scale))
                .collect::<Result<_, _>>()?
        }
    };
    let mut mono = vec![0.0; samples.len() / channels];
    let written = downmix_to_mono(&samples, channels, &mut mono);
    mono.truncate(written);
    Ok(Clip::new(spec.sample_rate, mono))
}

fn write_wav(path: &str, samples: &[f32]) -> Outcome<()> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: PREFERRED_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec)?;
    for sample in samples {
        writer.write_sample((sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}
