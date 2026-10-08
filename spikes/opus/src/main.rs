//! Compares Opus bindings for E4.3 (#54): round-trip quality, packet loss
//! with and without FEC, speed, and whether they decode each other's packets.
//!
//! Each binding is a feature. `opus03` links its own libopus, so build it on
//! its own; the others can be combined.

use std::f32::consts::TAU;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

const RATE: usize = 48_000;
const FRAME: usize = RATE / 50;
const SECONDS: usize = 4;
const BITRATE: i32 = 24_000;
const LOSS_EVERY: usize = 10;
const MAX_LAG: usize = 2_000;

type Res<T> = Result<T, String>;

trait Enc {
    fn encode(&mut self, pcm: &[f32], out: &mut [u8]) -> Res<usize>;
}

trait Dec {
    /// An empty packet means it was lost.
    fn decode(&mut self, packet: &[u8], fec: bool, out: &mut [f32]) -> Res<usize>;

    fn has_fec(&self) -> bool {
        true
    }
}

struct Binding {
    name: &'static str,
    enc: fn(fec: bool) -> Res<Box<dyn Enc>>,
    dec: fn() -> Res<Box<dyn Dec>>,
}

// opus 0.3 and 0.4 have the same API on different libopus builds.
macro_rules! opus_crate {
    ($module:ident, $krate:ident, $name:literal) => {
        mod $module {
            use super::*;
            use $krate::{Application, Bitrate, Channels, Decoder, Encoder};

            impl Enc for Encoder {
                fn encode(&mut self, pcm: &[f32], out: &mut [u8]) -> Res<usize> {
                    self.encode_float(pcm, out).map_err(|e| e.to_string())
                }
            }

            impl Dec for Decoder {
                fn decode(&mut self, packet: &[u8], fec: bool, out: &mut [f32]) -> Res<usize> {
                    self.decode_float(packet, out, fec).map_err(|e| e.to_string())
                }
            }

            pub fn binding() -> Binding {
                Binding {
                    name: $name,
                    enc: |fec| {
                        let err = |e: $krate::Error| e.to_string();
                        let mut e = Encoder::new(RATE as u32, Channels::Mono, Application::Voip)
                            .map_err(err)?;
                        e.set_bitrate(Bitrate::Bits(BITRATE)).map_err(err)?;
                        e.set_inband_fec(fec).map_err(err)?;
                        e.set_packet_loss_perc(if fec { 10 } else { 0 }).map_err(err)?;
                        Ok(Box::new(e))
                    },
                    dec: || {
                        let d = Decoder::new(RATE as u32, Channels::Mono).map_err(|e| e.to_string())?;
                        Ok(Box::new(d))
                    },
                }
            }
        }
    };
}

#[cfg(feature = "opus03")]
opus_crate!(opus03_binding, opus03, "opus 0.3");
#[cfg(feature = "opus04")]
opus_crate!(opus04_binding, opus04, "opus 0.4");

#[cfg(feature = "opusic")]
mod opusic_binding {
    use super::*;
    use opusic_c::{Application, Bitrate, Channels, Decoder, Encoder, InbandFec, SampleRate};

    impl Enc for Encoder {
        fn encode(&mut self, pcm: &[f32], out: &mut [u8]) -> Res<usize> {
            self.encode_float_to_slice(pcm, out).map_err(|e| format!("{e:?}"))
        }
    }

    impl Dec for Decoder {
        fn decode(&mut self, packet: &[u8], fec: bool, out: &mut [f32]) -> Res<usize> {
            self.decode_float_to_slice(packet, out, fec).map_err(|e| format!("{e:?}"))
        }
    }

    pub fn binding() -> Binding {
        Binding {
            name: "opusic-c",
            enc: |fec| {
                let err = |e: opusic_c::ErrorCode| format!("{e:?}");
                let mut e = Encoder::new(Channels::Mono, SampleRate::Hz48000, Application::Voip)
                    .map_err(err)?;
                e.set_bitrate(Bitrate::Value(BITRATE as u32)).map_err(err)?;
                let mode = if fec { InbandFec::Mode1 } else { InbandFec::Off };
                e.set_inband_fec(mode).map_err(err)?;
                e.set_packet_loss(if fec { 10 } else { 0 }).map_err(err)?;
                Ok(Box::new(e))
            },
            dec: || {
                let d = Decoder::new(Channels::Mono, SampleRate::Hz48000).map_err(|e| format!("{e:?}"))?;
                Ok(Box::new(d))
            },
        }
    }
}

#[cfg(feature = "pure")]
mod pure_binding {
    use super::*;
    use opus_rs::{Application, OpusDecoder, OpusEncoder};

    impl Enc for OpusEncoder {
        fn encode(&mut self, pcm: &[f32], out: &mut [u8]) -> Res<usize> {
            OpusEncoder::encode(self, pcm, pcm.len(), out).map_err(String::from)
        }
    }

    impl Dec for OpusDecoder {
        fn decode(&mut self, packet: &[u8], _fec: bool, out: &mut [f32]) -> Res<usize> {
            OpusDecoder::decode(self, packet, out.len(), out).map_err(String::from)
        }

        fn has_fec(&self) -> bool {
            false
        }
    }

    pub fn binding() -> Binding {
        Binding {
            name: "opus-rs",
            enc: |fec| {
                let mut e = OpusEncoder::new(RATE as i32, 1, Application::Voip)?;
                e.bitrate_bps = BITRATE;
                e.use_inband_fec = fec;
                e.packet_loss_perc = if fec { 10 } else { 0 };
                Ok(Box::new(e))
            },
            dec: || Ok(Box::new(OpusDecoder::new(RATE as i32, 1)?)),
        }
    }
}

fn bindings() -> Vec<Binding> {
    let mut all = Vec::new();
    #[cfg(feature = "opus03")]
    all.push(opus03_binding::binding());
    #[cfg(feature = "opus04")]
    all.push(opus04_binding::binding());
    #[cfg(feature = "opusic")]
    all.push(opusic_binding::binding());
    #[cfg(feature = "pure")]
    all.push(pure_binding::binding());
    all
}

/// A voice-like test signal: a gliding pitch with vowel-ish harmonics,
/// syllable bursts and a little breath noise.
fn speech_like() -> Vec<f32> {
    let mut out = Vec::with_capacity(RATE * SECONDS);
    let mut phase = 0.0_f32;
    let mut seed = 1_u32;
    for i in 0..RATE * SECONDS {
        let t = i as f32 / RATE as f32;
        let f0 = 140.0 + 40.0 * (TAU * 0.7 * t).sin();
        phase = (phase + f0 / RATE as f32).fract();
        let mut voice = 0.0;
        for h in 1..=20 {
            let f = f0 * h as f32;
            let formants = (-((f - 700.0) / 400.0).powi(2)).exp()
                + 0.6 * (-((f - 1800.0) / 500.0).powi(2)).exp()
                + 0.05;
            voice += formants * (TAU * phase * h as f32).sin();
        }
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (seed >> 8) as f32 / (1 << 24) as f32 - 0.5;
        let syllable = 0.5 - 0.5 * (TAU * 4.0 * t).cos();
        let pause = if t.fract() > 0.8 { 0.0 } else { 1.0 };
        out.push(0.15 * syllable * pause * (voice + 0.3 * noise));
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Clean,
    Plc,
    Fec,
}

struct Outcome {
    corr: f32,
    lag: usize,
    bytes: f32,
    enc_us: f32,
    dec_us: f32,
}

/// Like `run_inner`, but a panic inside a binding counts as a failed run.
fn run(enc: &Binding, dec: &Binding, input: &[f32], mode: Mode) -> Res<Outcome> {
    catch_unwind(AssertUnwindSafe(|| run_inner(enc, dec, input, mode)))
        .unwrap_or_else(|_| Err("panicked".to_string()))
}

fn run_inner(enc: &Binding, dec: &Binding, input: &[f32], mode: Mode) -> Res<Outcome> {
    let mut encoder = (enc.enc)(mode == Mode::Fec)?;
    let mut decoder = (dec.dec)()?;
    let frames = input.len() / FRAME;

    let mut packets = Vec::with_capacity(frames);
    let mut buf = [0_u8; 1500];
    let started = Instant::now();
    for frame in input.chunks_exact(FRAME) {
        let n = encoder.encode(frame, &mut buf)?;
        packets.push(buf[..n].to_vec());
    }
    let enc_us = started.elapsed().as_micros() as f32 / frames as f32;
    let bytes = packets.iter().map(Vec::len).sum::<usize>() as f32 / frames as f32;

    let lost = |i: usize| mode != Mode::Clean && i % LOSS_EVERY == 5;
    let mut output = vec![0.0_f32; frames * FRAME];
    let started = Instant::now();
    for (i, out) in output.chunks_exact_mut(FRAME).enumerate() {
        let n = if !lost(i) {
            decoder.decode(&packets[i], false, out)?
        } else if mode == Mode::Fec && decoder.has_fec() && i + 1 < frames {
            decoder.decode(&packets[i + 1], true, out)?
        } else {
            decoder.decode(&[], false, out)?
        };
        if n != FRAME {
            return Err(format!("frame {i}: decoded {n} samples, not {FRAME}"));
        }
    }
    let dec_us = started.elapsed().as_micros() as f32 / frames as f32;

    let lag = best_lag(input, &output);
    Ok(Outcome {
        corr: correlation(input, &output[lag..]),
        lag,
        bytes,
        enc_us,
        dec_us,
    })
}

/// The codec delay: the shift that best lines the output up with the input,
/// searched over the first second.
fn best_lag(input: &[f32], output: &[f32]) -> usize {
    (0..MAX_LAG)
        .max_by(|&a, &b| {
            let ca = correlation(&input[..RATE], &output[a..a + RATE]);
            let cb = correlation(&input[..RATE], &output[b..b + RATE]);
            ca.total_cmp(&cb)
        })
        .unwrap_or(0)
}

fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (mut ab, mut aa, mut bb) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (&x, &y) in a[..n].iter().zip(&b[..n]) {
        ab += f64::from(x) * f64::from(y);
        aa += f64::from(x) * f64::from(x);
        bb += f64::from(y) * f64::from(y);
    }
    if aa == 0.0 || bb == 0.0 {
        return 0.0;
    }
    (ab / (aa * bb).sqrt()) as f32
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let input = speech_like();
    let all = bindings();
    println!(
        "{} s at {RATE} Hz mono, 20 ms frames, {} kbps VoIP, every {LOSS_EVERY}th packet lost\n",
        SECONDS,
        BITRATE / 1000
    );
    println!(
        "{:<22} {:>7} {:>6} {:>8} {:>8} {:>7} {:>8} {:>8}",
        "encoder -> decoder", "clean", "delay", "10% PLC", "10% FEC", "bytes", "enc us", "dec us"
    );
    for enc in &all {
        for dec in &all {
            let label = format!("{} -> {}", enc.name, dec.name);
            let clean = run(enc, dec, &input, Mode::Clean);
            let plc = run(enc, dec, &input, Mode::Plc);
            let fec = run(enc, dec, &input, Mode::Fec);
            match (clean, plc) {
                (Ok(c), Ok(p)) => {
                    let fec = match fec {
                        Ok(_) if !(dec.dec)().is_ok_and(|d| d.has_fec()) => "n/a".to_string(),
                        Ok(f) => format!("{:.3}", f.corr),
                        Err(e) => e,
                    };
                    println!(
                        "{label:<22} {:>7.3} {:>4.1}ms {:>8.3} {:>8} {:>7.1} {:>8.1} {:>8.1}",
                        c.corr,
                        c.lag as f32 * 1000.0 / RATE as f32,
                        p.corr,
                        fec,
                        c.bytes,
                        c.enc_us,
                        c.dec_us
                    );
                }
                (c, p) => {
                    let err = [c.err(), p.err()].into_iter().flatten().next();
                    println!("{label:<22} error: {}", err.unwrap_or_default());
                }
            }
        }
    }
}
