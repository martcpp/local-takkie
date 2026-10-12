//! What happens to the microphone's sound before it is encoded.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use takkie_core::dsp::Biquad;

use super::config::PREFERRED_RATE;

// Below this a voice has nothing, only rumble, handling noise and wind.
const LOW_CUT_HZ: f32 = 100.0;

/// Switches for the voice processing, shared with whoever sets them.
///
/// Relaxed is enough: each switch stands alone and is read once a frame.
#[derive(Debug)]
pub struct VoiceControls {
    low_cut: AtomicBool,
}

impl Default for VoiceControls {
    fn default() -> Self {
        Self {
            low_cut: AtomicBool::new(true),
        }
    }
}

impl VoiceControls {
    /// Turns the rumble filter on or off.
    pub fn set_low_cut(&self, on: bool) {
        self.low_cut.store(on, Relaxed);
    }

    /// Whether the rumble filter is on.
    #[must_use]
    pub fn low_cut(&self) -> bool {
        self.low_cut.load(Relaxed)
    }
}

/// The stages a 48 kHz mono frame goes through on its way to the encoder.
/// Nothing here allocates.
#[derive(Debug)]
pub struct VoiceChain {
    low_cut: Biquad,
}

impl Default for VoiceChain {
    fn default() -> Self {
        Self {
            low_cut: Biquad::high_pass(LOW_CUT_HZ, PREFERRED_RATE),
        }
    }
}

impl VoiceChain {
    /// Processes one frame in place with the stages `controls` has on.
    pub fn process(&mut self, frame: &mut [f32], controls: &VoiceControls) {
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

    fn run(controls: &VoiceControls) -> f32 {
        let mut chain = VoiceChain::default();
        let mut sound = hum_and_voice();
        for frame in sound.chunks_mut(FRAME) {
            chain.process(frame, controls);
        }
        Level::of(&sound[24_000..]).rms
    }

    #[test]
    fn the_low_cut_is_on_unless_switched_off() {
        let controls = VoiceControls::default();
        let voice_only = 0.1 / 2.0_f32.sqrt();
        assert!((run(&controls) / voice_only - 1.0).abs() < 0.2);

        controls.set_low_cut(false);
        assert_eq!(run(&controls), Level::of(&hum_and_voice()[24_000..]).rms);
    }
}
