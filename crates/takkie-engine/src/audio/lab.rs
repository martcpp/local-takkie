//! The send path run on a recording, with no devices or network, to hear
//! and measure what the far end gets.

use std::fmt;

use takkie_core::dsp::Level;
use thiserror::Error;

use super::codec::{CodecError, VoiceDecoder, VoiceEncoder};
use super::io::AudioSource;
use super::resample::{FRAME, ResampleError};
use super::tx::Framer;

/// Why a lab run failed.
#[derive(Debug, Error)]
pub enum LabError {
    /// The recording's rate can't be converted to 48 kHz.
    #[error(transparent)]
    Resample(#[from] ResampleError),
    /// Opus refused a frame or a packet.
    #[error(transparent)]
    Codec(#[from] CodecError),
}

/// A mono recording played as a microphone, all of it ready at once.
#[derive(Clone, Debug)]
pub struct Clip {
    rate: u32,
    samples: Vec<f32>,
    position: usize,
}

impl Clip {
    /// A clip of mono `samples` at `rate` samples per second.
    #[must_use]
    pub fn new(rate: u32, samples: Vec<f32>) -> Self {
        Self {
            rate,
            samples,
            position: 0,
        }
    }
}

impl AudioSource for Clip {
    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn read(&mut self, out: &mut [f32]) -> usize {
        let left = self.samples.get(self.position..).unwrap_or_default();
        let count = left.len().min(out.len());
        out[..count].copy_from_slice(&left[..count]);
        self.position += count;
        count
    }
}

/// Everything `source` has, as whole 48 kHz frames, the way the send thread
/// cuts it. A last part-frame is left out.
///
/// # Errors
/// [`LabError::Resample`] if the source's rate can't be converted.
pub fn frames(source: &mut dyn AudioSource) -> Result<Vec<f32>, LabError> {
    let mut framer = Framer::new(source.sample_rate())?;
    let mut voice = Vec::new();
    while let Some(frame) = framer.next(source)? {
        voice.extend_from_slice(frame);
    }
    Ok(voice)
}

/// How many packets in a hundred get lost on the way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Loss(u8);

impl Loss {
    /// `percent` of the packets lost; `None` above 100.
    #[must_use]
    pub fn percent(percent: u8) -> Option<Self> {
        (percent <= 100).then_some(Self(percent))
    }
}

// The same packets are lost on every run, so two runs can be compared.
struct Dice(u32);

impl Dice {
    fn lost(&mut self, loss: Loss) -> bool {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 % 100 < u32::from(loss.0)
    }
}

/// What the far end plays for `voice` (48 kHz mono): each frame through the
/// encoder and decoder, with lost packets rebuilt from FEC or guessed, as
/// the mixer does.
///
/// # Errors
/// [`LabError::Codec`] if Opus fails.
pub fn far_end(voice: &[f32], loss: Loss) -> Result<Vec<f32>, LabError> {
    let mut encoder = VoiceEncoder::new()?;
    let mut dice = Dice(0x9E37_79B9);
    let mut packets = Vec::with_capacity(voice.len() / FRAME);
    for frame in voice.as_chunks::<FRAME>().0 {
        let packet = encoder.encode(frame)?.to_vec();
        packets.push((!dice.lost(loss)).then_some(packet));
    }

    let mut decoder = VoiceDecoder::new()?;
    let mut heard = Vec::with_capacity(packets.len() * FRAME);
    let mut frame = [0.0; FRAME];
    for (index, packet) in packets.iter().enumerate() {
        let next = packets.get(index + 1).and_then(Option::as_ref);
        let written = match (packet, next) {
            (Some(packet), _) => decoder.decode(packet, &mut frame)?,
            (None, Some(next)) => decoder.decode_fec(next, &mut frame)?,
            (None, None) => decoder.conceal(&mut frame)?,
        };
        heard.extend_from_slice(&frame[..written.min(FRAME)]);
    }
    Ok(heard)
}

/// How loud a recording is, in dB below full scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Report {
    /// The largest sample.
    pub peak: f32,
    /// The average over the whole recording.
    pub level: f32,
    /// The quietest tenth of the 20 ms frames: the background.
    pub floor: f32,
}

impl Report {
    /// Measures 48 kHz mono `samples`.
    #[must_use]
    pub fn of(samples: &[f32]) -> Self {
        let whole = Level::of(samples);
        let mut frames: Vec<f32> = samples
            .as_chunks::<FRAME>()
            .0
            .iter()
            .map(|frame| Level::of(frame).rms)
            .collect();
        frames.sort_by(f32::total_cmp);
        let floor = frames.get(frames.len() / 10).copied().unwrap_or(whole.rms);
        Self {
            peak: decibels(whole.peak),
            level: decibels(whole.rms),
            floor: decibels(floor),
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "peak {:6.1} dB | level {:6.1} dB | background {:6.1} dB",
            self.peak, self.level, self.floor
        )
    }
}

const SILENCE: f32 = -120.0;

fn decibels(amplitude: f32) -> f32 {
    if amplitude > 0.0 {
        (20.0 * amplitude.log10()).max(SILENCE)
    } else {
        SILENCE
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;

    fn tone(rate: u32, seconds: f32, amplitude: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize)
            .map(|i| amplitude * (TAU * 440.0 * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn a_44k1_clip_becomes_whole_48k_frames() {
        let mut clip = Clip::new(44_100, tone(44_100, 1.0, 0.5));
        let voice = frames(&mut clip).unwrap();
        assert_eq!(voice.len() % FRAME, 0);
        assert!(
            (45..=50).contains(&(voice.len() / FRAME)),
            "{}",
            voice.len()
        );
    }

    #[test]
    fn a_clip_is_read_once_and_then_dry() {
        let mut clip = Clip::new(48_000, vec![0.25; 10]);
        let mut out = [0.0; 8];
        assert_eq!(clip.read(&mut out), 8);
        assert_eq!(clip.read(&mut out), 2);
        assert_eq!(clip.read(&mut out), 0);
    }

    #[test]
    fn the_far_end_hears_the_clip_at_about_the_same_level() {
        let voice = tone(48_000, 1.0, 0.5);
        let heard = far_end(&voice, Loss::default()).unwrap();
        assert_eq!(heard.len(), voice.len());
        let (sent, got) = (Report::of(&voice[4800..]), Report::of(&heard[4800..]));
        assert!((sent.level - got.level).abs() < 1.5, "{sent} / {got}");
    }

    #[test]
    fn lost_packets_are_filled_in_not_dropped() {
        let voice = tone(48_000, 2.0, 0.5);
        let loss = Loss::percent(20).unwrap();
        let heard = far_end(&voice, loss).unwrap();
        assert_eq!(heard.len(), voice.len());
        let clean = far_end(&voice, Loss::default()).unwrap();
        assert_ne!(heard, clean);
        assert!(Report::of(&heard).level > -20.0);
    }

    #[test]
    fn the_same_packets_are_lost_every_time() {
        let voice = tone(48_000, 1.0, 0.5);
        let loss = Loss::percent(30).unwrap();
        assert_eq!(
            far_end(&voice, loss).unwrap(),
            far_end(&voice, loss).unwrap()
        );
    }

    #[test]
    fn loss_is_a_percentage() {
        assert!(Loss::percent(100).is_some());
        assert!(Loss::percent(101).is_none());
    }

    #[test]
    fn a_report_reads_in_decibels() {
        let report = Report::of(&tone(48_000, 1.0, 0.5));
        assert!((report.peak + 6.0).abs() < 0.1, "{report}");
        assert!((report.level + 9.0).abs() < 0.1, "{report}");
        assert!((report.floor + 9.0).abs() < 0.2, "{report}");
    }

    #[test]
    fn the_background_is_the_quiet_part_between_the_words() {
        let mut samples = tone(48_000, 1.0, 0.5);
        samples.extend(tone(48_000, 1.0, 0.005));
        let report = Report::of(&samples);
        assert!((report.floor + 49.0).abs() < 0.5, "{report}");
        assert!(report.level > -15.0, "{report}");
    }

    #[test]
    fn silence_and_nothing_have_a_report_too() {
        assert_eq!(Report::of(&[]).level, SILENCE);
        assert_eq!(Report::of(&[0.0; FRAME]).floor, SILENCE);
    }
}
