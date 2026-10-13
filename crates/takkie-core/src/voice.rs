//! Shaping a voice before it is sent: pure arithmetic on 48 kHz mono frames,
//! with no allocation.

use crate::dsp::Level;

// Where speech should sit: about -20 dB, which leaves its peaks under full scale.
const TARGET_DB: f32 = -20.0;
const MOST_GAIN_DB: f32 = 30.0;
const LEAST_GAIN_DB: f32 = -15.0;
// Turning down has to be quicker than turning up, or a shout distorts.
const UP_DB_PER_SECOND: f32 = 10.0;
const DOWN_DB_PER_SECOND: f32 = 20.0;
// Speech has to be this many times louder than the background, and never
// quieter than `QUIETEST_SPEECH`.
const ABOVE_BACKGROUND: f32 = 3.0;
const QUIETEST_SPEECH: f32 = 0.002;
// Per frame, how fast the background estimate creeps up.
const CREEP: f32 = 0.002;
// How fast the speech level follows a louder and a quieter frame.
const RISE: f32 = 0.3;
const FALL: f32 = 0.03;
const CEILING: f32 = 0.9;
// Gain changes are spread over this many samples so they don't click.
const RAMP: usize = 96;

/// Brings a quiet talker up and a loud one down to the same level.
///
/// The level is only measured while someone speaks, and the gain only moves
/// then, so the background isn't turned up in the pauses.
#[derive(Clone, Debug)]
pub struct Leveller {
    sample_rate: f32,
    background: Option<f32>,
    speech_db: Option<f32>,
    gain_db: f32,
    applied: f32,
}

impl Leveller {
    /// A leveller for frames at `sample_rate`, starting with no gain.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate: sample_rate as f32,
            background: None,
            speech_db: None,
            gain_db: 0.0,
            applied: 1.0,
        }
    }

    /// The gain in use, in dB.
    #[must_use]
    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    /// Levels one frame in place.
    pub fn process(&mut self, frame: &mut [f32]) {
        let level = Level::of(frame);
        if !level.rms.is_finite() || frame.is_empty() {
            return;
        }
        if self.hears_speech(level.rms) {
            self.follow(level.rms, frame.len() as f32 / self.sample_rate);
        }
        let mut gain = 10_f32.powf(self.gain_db / 20.0);
        // A peak the gain would push past full scale gets less, just for
        // this frame.
        if level.peak * gain > CEILING {
            gain = CEILING / level.peak;
        }
        let ramp = RAMP.min(frame.len());
        let step = (gain - self.applied) / ramp as f32;
        let (start, rest) = frame.split_at_mut(ramp);
        // The old gain may still be too much for the first samples of a shout.
        for (index, sample) in start.iter_mut().enumerate() {
            let ramped = *sample * (self.applied + step * (index + 1) as f32);
            *sample = ramped.clamp(-CEILING, CEILING);
        }
        for sample in rest {
            *sample *= gain;
        }
        self.applied = gain;
    }

    fn hears_speech(&mut self, rms: f32) -> bool {
        let background = match self.background {
            None => rms,
            Some(was) if rms < was => rms,
            Some(was) => was + (rms - was) * CREEP,
        };
        self.background = Some(background);
        rms > (background * ABOVE_BACKGROUND).max(QUIETEST_SPEECH)
    }

    fn follow(&mut self, rms: f32, seconds: f32) {
        let now = 20.0 * rms.log10();
        let speech = match self.speech_db {
            None => now,
            Some(was) if now > was => was + (now - was) * RISE,
            Some(was) => was + (now - was) * FALL,
        };
        self.speech_db = Some(speech);
        let wanted = (TARGET_DB - speech).clamp(LEAST_GAIN_DB, MOST_GAIN_DB);
        let change = (wanted - self.gain_db)
            .clamp(-DOWN_DB_PER_SECOND * seconds, UP_DB_PER_SECOND * seconds);
        self.gain_db += change;
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;

    const RATE: u32 = 48_000;
    const FRAME: usize = 960;

    // Half a second of "talking" at `amplitude`, half a second of
    // background, over and over.
    fn talk(seconds: usize, amplitude: f32, background: f32) -> Vec<f32> {
        (0..RATE as usize * seconds)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let speaking = (i / (RATE as usize / 2)).is_multiple_of(2);
                let size = if speaking { amplitude } else { background };
                size * (TAU * 300.0 * t).sin()
            })
            .collect()
    }

    fn levelled(leveller: &mut Leveller, mut sound: Vec<f32>) -> Vec<f32> {
        for frame in sound.chunks_mut(FRAME) {
            leveller.process(frame);
        }
        sound
    }

    // The level of the talking half-second that starts at `second`.
    fn spoken(sound: &[f32], second: usize) -> f32 {
        let start = second * RATE as usize;
        20.0 * Level::of(&sound[start + 4_800..start + 19_200]).rms.log10()
    }

    #[test]
    fn a_quiet_talker_and_a_loud_one_end_up_alike() {
        let quiet = levelled(&mut Leveller::new(RATE), talk(12, 0.02, 0.0005));
        let loud = levelled(&mut Leveller::new(RATE), talk(12, 0.5, 0.0005));
        let (quiet, loud) = (spoken(&quiet, 10), spoken(&loud, 10));
        assert!(
            (quiet - TARGET_DB).abs() < 2.0,
            "quiet talker at {quiet} dB"
        );
        assert!((loud - TARGET_DB).abs() < 2.0, "loud talker at {loud} dB");
    }

    #[test]
    fn the_background_is_not_turned_up_in_the_pauses() {
        let mut leveller = Leveller::new(RATE);
        levelled(&mut leveller, talk(10, 0.02, 0.0005));
        let settled = leveller.gain_db();
        levelled(&mut leveller, talk(1, 0.0005, 0.0005).repeat(20));
        assert_eq!(leveller.gain_db(), settled);
    }

    #[test]
    fn the_gain_has_a_limit() {
        let mut leveller = Leveller::new(RATE);
        levelled(&mut leveller, talk(20, 0.003, 0.0001));
        assert!(leveller.gain_db() <= MOST_GAIN_DB);
        assert!(leveller.gain_db() > MOST_GAIN_DB - 1.0);
    }

    #[test]
    fn a_shout_after_a_whisper_does_not_clip() {
        let mut leveller = Leveller::new(RATE);
        levelled(&mut leveller, talk(10, 0.01, 0.0005));
        let shout = levelled(&mut leveller, talk(1, 0.9, 0.9));
        let peak = Level::of(&shout).peak;
        assert!(peak <= CEILING + 0.001, "peak {peak}");
    }

    #[test]
    fn one_bang_does_not_change_the_level_for_long() {
        let mut leveller = Leveller::new(RATE);
        levelled(&mut leveller, talk(10, 0.02, 0.0005));
        let before = leveller.gain_db();
        levelled(&mut leveller, vec![0.9; FRAME]);
        assert!((leveller.gain_db() - before).abs() < 0.5);
    }

    #[test]
    fn gain_changes_are_smooth() {
        let mut leveller = Leveller::new(RATE);
        let out = levelled(&mut leveller, vec![0.02; RATE as usize * 3]);
        let biggest_jump = out
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0, f32::max);
        assert!(biggest_jump < 0.001, "{biggest_jump}");
    }

    #[test]
    fn silence_and_broken_samples_change_nothing() {
        let mut leveller = Leveller::new(RATE);
        let mut silence = [0.0; FRAME];
        leveller.process(&mut silence);
        assert_eq!(silence, [0.0; FRAME]);
        leveller.process(&mut [f32::NAN; FRAME]);
        leveller.process(&mut []);
        assert_eq!(leveller.gain_db(), 0.0);
    }
}
