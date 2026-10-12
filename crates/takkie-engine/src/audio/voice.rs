//! What happens to the microphone's sound before it is encoded.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};

use nnnoiseless::DenoiseState;
use takkie_core::dsp::Biquad;

use super::config::PREFERRED_RATE;

// Below this a voice has nothing, only rumble, handling noise and wind.
const LOW_CUT_HZ: f32 = 100.0;

/// How hard background noise is pushed down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NoiseSuppression {
    /// The microphone as it is.
    Off,
    /// Most of the noise gone, a little left so the voice sounds natural.
    #[default]
    Normal,
    /// As much as can be removed.
    Strong,
}

impl NoiseSuppression {
    // How much of the cleaned sound is used; the rest is the microphone.
    // 0.85 leaves the noise about 13 dB down, and measured as clear as 1.0.
    fn cleaned(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::Normal => 0.85,
            Self::Strong => 1.0,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Off,
            2 => Self::Strong,
            _ => Self::Normal,
        }
    }
}

/// Switches for the voice processing, shared with whoever sets them.
///
/// Relaxed is enough: each switch stands alone and is read once a frame.
#[derive(Debug)]
pub struct VoiceControls {
    low_cut: AtomicBool,
    noise: AtomicU8,
}

impl Default for VoiceControls {
    fn default() -> Self {
        Self {
            low_cut: AtomicBool::new(true),
            noise: AtomicU8::new(NoiseSuppression::default() as u8),
        }
    }
}

impl VoiceControls {
    /// Every stage off: the microphone as it is.
    #[must_use]
    pub fn raw() -> Self {
        Self {
            low_cut: AtomicBool::new(false),
            noise: AtomicU8::new(NoiseSuppression::Off as u8),
        }
    }

    /// Turns the rumble filter on or off.
    pub fn set_low_cut(&self, on: bool) {
        self.low_cut.store(on, Relaxed);
    }

    /// Whether the rumble filter is on.
    #[must_use]
    pub fn low_cut(&self) -> bool {
        self.low_cut.load(Relaxed)
    }

    /// Sets how hard noise is pushed down.
    pub fn set_noise_suppression(&self, level: NoiseSuppression) {
        self.noise.store(level as u8, Relaxed);
    }

    /// How hard noise is pushed down.
    #[must_use]
    pub fn noise_suppression(&self) -> NoiseSuppression {
        NoiseSuppression::from_u8(self.noise.load(Relaxed))
    }
}

const BLOCK: usize = DenoiseState::FRAME_SIZE;
// The denoiser wants samples on a 16-bit scale.
const SCALE: f32 = 32_768.0;

// RNNoise, 10 ms at a time. Its output is one block late, so the untouched
// sound it is blended with is held back by one block too.
struct Denoiser {
    state: Box<DenoiseState<'static>>,
    input: [f32; BLOCK],
    cleaned: [f32; BLOCK],
    before: [f32; BLOCK],
    running: bool,
}

impl Denoiser {
    fn new() -> Self {
        Self {
            state: DenoiseState::new(),
            input: [0.0; BLOCK],
            cleaned: [0.0; BLOCK],
            before: [0.0; BLOCK],
            running: false,
        }
    }

    fn process(&mut self, frame: &mut [f32], level: NoiseSuppression) {
        if level == NoiseSuppression::Off {
            self.running = false;
            return;
        }
        if !self.running {
            // It still holds the end of what it heard before it was
            // switched off. Silence pushes that out.
            self.before = [0.0; BLOCK];
            for _ in 0..2 {
                self.state.process_frame(&mut self.cleaned, &self.before);
            }
            self.running = true;
        }
        let cleaned = level.cleaned();
        // A frame that isn't whole blocks keeps its tail as it is.
        for block in frame.as_chunks_mut::<BLOCK>().0 {
            for (scaled, sample) in self.input.iter_mut().zip(block.iter()) {
                *scaled = sample * SCALE;
            }
            self.state.process_frame(&mut self.cleaned, &self.input);
            for ((sample, clean), before) in
                block.iter_mut().zip(&self.cleaned).zip(&mut self.before)
            {
                let now = *sample;
                *sample = clean / SCALE * cleaned + *before * (1.0 - cleaned);
                *before = now;
            }
        }
    }
}

impl fmt::Debug for Denoiser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Denoiser")
            .field("running", &self.running)
            .finish_non_exhaustive()
    }
}

/// The stages a 48 kHz mono frame goes through on its way to the encoder.
/// Nothing here allocates once it is built.
#[derive(Debug)]
pub struct VoiceChain {
    low_cut: Biquad,
    denoiser: Denoiser,
}

impl Default for VoiceChain {
    fn default() -> Self {
        Self {
            low_cut: Biquad::high_pass(LOW_CUT_HZ, PREFERRED_RATE),
            denoiser: Denoiser::new(),
        }
    }
}

impl VoiceChain {
    /// Processes one frame that will be sent, in place, with the stages
    /// `controls` has on.
    pub fn process(&mut self, frame: &mut [f32], controls: &VoiceControls) {
        self.cut_lows(frame, controls);
        self.denoiser.process(frame, controls.noise_suppression());
    }

    /// Processes one frame that only feeds the level meter. The costly
    /// stages rest, so an idle radio doesn't spend the battery on them.
    pub fn idle(&mut self, frame: &mut [f32], controls: &VoiceControls) {
        self.cut_lows(frame, controls);
        self.denoiser.process(frame, NoiseSuppression::Off);
    }

    fn cut_lows(&mut self, frame: &mut [f32], controls: &VoiceControls) {
        if controls.low_cut() {
            self.low_cut.process(frame);
        } else {
            self.low_cut.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use takkie_core::dsp::Level;

    use super::*;
    use crate::audio::resample::FRAME;

    fn hum_and_voice() -> Vec<f32> {
        (0..48_000)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                0.3 * (TAU * 40.0 * t).sin() + 0.1 * (TAU * 1_000.0 * t).sin()
            })
            .collect()
    }

    fn hiss(seconds: usize) -> Vec<f32> {
        let mut seed = 0x2545_F491_u32;
        (0..48_000 * seconds)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                (seed as f32 / u32::MAX as f32 - 0.5) * 0.1
            })
            .collect()
    }

    fn run(mut sound: Vec<f32>, controls: &VoiceControls) -> Vec<f32> {
        let mut chain = VoiceChain::default();
        for frame in sound.chunks_mut(FRAME) {
            chain.process(frame, controls);
        }
        sound
    }

    fn settled(sound: &[f32]) -> f32 {
        Level::of(&sound[sound.len() / 2..]).rms
    }

    fn only(low_cut: bool, noise: NoiseSuppression) -> VoiceControls {
        let controls = VoiceControls::default();
        controls.set_low_cut(low_cut);
        controls.set_noise_suppression(noise);
        controls
    }

    #[test]
    fn the_low_cut_removes_hum_under_a_voice() {
        let controls = only(true, NoiseSuppression::Off);
        let voice_only = 0.1 / 2.0_f32.sqrt();
        let out = settled(&run(hum_and_voice(), &controls));
        assert!((out / voice_only - 1.0).abs() < 0.2, "{out}");
    }

    #[test]
    fn everything_off_leaves_the_sound_alone() {
        let controls = only(false, NoiseSuppression::Off);
        assert_eq!(run(hum_and_voice(), &controls), hum_and_voice());
    }

    #[test]
    fn the_processing_is_on_unless_switched_off() {
        let controls = VoiceControls::default();
        assert!(controls.low_cut());
        assert_eq!(controls.noise_suppression(), NoiseSuppression::Normal);
    }

    #[test]
    fn hiss_is_pushed_down_and_strong_pushes_harder() {
        let raw = settled(&hiss(3));
        let normal = settled(&run(hiss(3), &only(false, NoiseSuppression::Normal)));
        let strong = settled(&run(hiss(3), &only(false, NoiseSuppression::Strong)));
        assert!(normal < raw * 0.35, "normal {normal} of {raw}");
        assert!(strong < normal * 0.6, "strong {strong}, normal {normal}");
    }

    #[test]
    fn the_cleaned_sound_stays_finite_and_in_range() {
        let out = run(hum_and_voice(), &VoiceControls::default());
        assert!(out.iter().all(|sample| sample.abs() <= 1.0));
    }

    #[test]
    fn switching_noise_suppression_on_mid_stream_does_not_replay_old_sound() {
        let controls = only(false, NoiseSuppression::Normal);
        let mut chain = VoiceChain::default();
        let mut loud: Vec<f32> = hum_and_voice()[..FRAME * 10].to_vec();
        for frame in loud.chunks_mut(FRAME) {
            chain.process(frame, &controls);
        }
        controls.set_noise_suppression(NoiseSuppression::Off);
        chain.process(&mut vec![0.0; FRAME], &controls);
        controls.set_noise_suppression(NoiseSuppression::Normal);
        let mut silence = vec![0.0; FRAME];
        chain.process(&mut silence, &controls);
        assert!(Level::of(&silence).peak < 0.01, "{:?}", Level::of(&silence));
    }

    #[test]
    fn an_idle_frame_is_not_denoised() {
        let controls = only(false, NoiseSuppression::Strong);
        let mut chain = VoiceChain::default();
        let mut frame = hiss(1)[..FRAME].to_vec();
        chain.idle(&mut frame, &controls);
        assert_eq!(frame, hiss(1)[..FRAME]);
    }

    #[test]
    fn a_level_survives_the_trip_through_the_switch() {
        let controls = VoiceControls::default();
        for level in [
            NoiseSuppression::Off,
            NoiseSuppression::Normal,
            NoiseSuppression::Strong,
        ] {
            controls.set_noise_suppression(level);
            assert_eq!(controls.noise_suppression(), level);
        }
    }
}
