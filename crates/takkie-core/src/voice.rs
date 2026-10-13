//! Shaping a voice before it is sent: pure arithmetic on 48 kHz mono frames,
//! with no allocation.

use crate::dsp::{Biquad, Level, soft_limit};

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

    /// Whether the talker is so quiet that the gain has reached its limit.
    #[must_use]
    pub fn straining(&self) -> bool {
        self.gain_db >= MOST_GAIN_DB - 1.0
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

/// How hard a stage of the voice processing works.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Strength {
    /// Not at all.
    Off,
    /// Enough to help without being heard.
    #[default]
    Normal,
    /// As much as it can, for a loud room.
    Strong,
}

impl Strength {
    /// The word for it in settings files and on screen.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Normal => "normal",
            Self::Strong => "strong",
        }
    }

    /// The strength called `name`, if there is one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Off, Self::Normal, Self::Strong]
            .into_iter()
            .find(|strength| strength.name() == name)
    }

    /// The next one, for a key that steps through them.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Normal,
            Self::Normal => Self::Strong,
            Self::Strong => Self::Off,
        }
    }
}

/// How much the part of a voice that carries the words is lifted.
pub type Clarity = Strength;

// (lift around 3 kHz, cut below 250 Hz), in dB.
fn shape(clarity: Clarity) -> Option<(f32, f32)> {
    match clarity {
        Clarity::Off => None,
        Clarity::Normal => Some((4.0, -3.0)),
        Clarity::Strong => Some((7.0, -5.0)),
    }
}

/// What to tell the person about their microphone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MicAdvice {
    /// Nothing: it sounds fine.
    #[default]
    Fine,
    /// Too quiet even with all the gain there is: move closer.
    TooQuiet,
    /// So loud it distorts: move back or turn the input down.
    Clipping,
}

impl MicAdvice {
    /// The word for it on the wire to a frontend.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fine => "fine",
            Self::TooQuiet => "quiet",
            Self::Clipping => "clipping",
        }
    }
}

const CLIPS_AT: f32 = 0.98;
// Frames a warning stays up after its cause, so it can be read: 2 s.
const SHOWN_FOR: u32 = 100;

/// Watches the microphone for a level no processing can fix.
#[derive(Clone, Debug, Default)]
pub struct MicWatch {
    clipped: u32,
    strained: u32,
}

impl MicWatch {
    /// Takes one frame's peak, as the microphone gave it, and whether the
    /// automatic level has run out of gain.
    pub fn hear(&mut self, peak: f32, straining: bool) -> MicAdvice {
        self.clipped = if peak >= CLIPS_AT {
            SHOWN_FOR
        } else {
            self.clipped.saturating_sub(1)
        };
        self.strained = if straining {
            SHOWN_FOR
        } else {
            self.strained.saturating_sub(1)
        };
        if self.clipped > 0 {
            MicAdvice::Clipping
        } else if self.strained > 0 {
            MicAdvice::TooQuiet
        } else {
            MicAdvice::Fine
        }
    }
}

// Consonants sit between 2 and 4 kHz; a wide band around 3 kHz covers them.
const PRESENCE_HZ: f32 = 3_000.0;
const PRESENCE_Q: f32 = 0.8;
// Below this a voice is body, not words, and it uses up the loudness.
const BODY_HZ: f32 = 250.0;

/// Moves a voice's energy to where the words are, so it cuts through noise
/// at the listener's end, and keeps the result inside full scale.
#[derive(Clone, Debug)]
pub struct Shaper {
    sample_rate: u32,
    clarity: Clarity,
    presence: Biquad,
    body: Biquad,
}

impl Shaper {
    /// A shaper for frames at `sample_rate`, off until told otherwise.
    #[must_use]
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            clarity: Clarity::Off,
            presence: Biquad::peaking(PRESENCE_HZ, 0.0, PRESENCE_Q, sample_rate),
            body: Biquad::low_shelf(BODY_HZ, 0.0, sample_rate),
        }
    }

    /// Shapes one frame in place.
    pub fn process(&mut self, frame: &mut [f32], clarity: Clarity) {
        if clarity != self.clarity {
            self.clarity = clarity;
            let (lift, cut) = shape(clarity).unwrap_or_default();
            self.presence = Biquad::peaking(PRESENCE_HZ, lift, PRESENCE_Q, self.sample_rate);
            self.body = Biquad::low_shelf(BODY_HZ, cut, self.sample_rate);
        }
        if clarity == Clarity::Off {
            return;
        }
        self.presence.process(frame);
        self.body.process(frame);
        soft_limit(frame);
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

    fn tone(frequency: f32, amplitude: f32) -> Vec<f32> {
        (0..RATE as usize)
            .map(|i| amplitude * (TAU * frequency * i as f32 / RATE as f32).sin())
            .collect()
    }

    fn shaped(clarity: Clarity, mut sound: Vec<f32>) -> f32 {
        let before = Level::of(&sound[24_000..]).rms;
        let mut shaper = Shaper::new(RATE);
        for frame in sound.chunks_mut(FRAME) {
            shaper.process(frame, clarity);
        }
        20.0 * (Level::of(&sound[24_000..]).rms / before).log10()
    }

    #[test]
    fn clarity_lifts_the_consonants_and_thins_the_body() {
        assert!((shaped(Clarity::Normal, tone(3_000.0, 0.1)) - 4.0).abs() < 0.2);
        assert!((shaped(Clarity::Normal, tone(120.0, 0.1)) + 3.0).abs() < 0.5);
        assert!(shaped(Clarity::Normal, tone(700.0, 0.1)).abs() < 1.0);
        assert!((shaped(Clarity::Strong, tone(3_000.0, 0.1)) - 7.0).abs() < 0.2);
    }

    #[test]
    fn clarity_off_leaves_the_frame_alone() {
        let sound = tone(3_000.0, 0.95);
        let mut out = sound.clone();
        Shaper::new(RATE).process(&mut out, Clarity::Off);
        assert_eq!(out, sound);
    }

    #[test]
    fn a_lifted_loud_voice_stays_inside_full_scale() {
        let mut sound = tone(3_000.0, 0.9);
        let mut shaper = Shaper::new(RATE);
        for frame in sound.chunks_mut(FRAME) {
            shaper.process(frame, Clarity::Strong);
        }
        assert!(Level::of(&sound).peak <= 1.0);
    }

    #[test]
    fn changing_the_setting_mid_stream_takes_effect() {
        let mut shaper = Shaper::new(RATE);
        let mut sound = tone(3_000.0, 0.1);
        let (first, second) = sound.split_at_mut(24_000);
        for frame in first.chunks_mut(FRAME) {
            shaper.process(frame, Clarity::Normal);
        }
        for frame in second.chunks_mut(FRAME) {
            shaper.process(frame, Clarity::Strong);
        }
        let (normal, strong) = (
            Level::of(&first[12_000..]).rms,
            Level::of(&second[12_000..]).rms,
        );
        assert!((20.0 * (strong / normal).log10() - 3.0).abs() < 0.3);
    }

    #[test]
    fn strengths_have_names_and_step_round() {
        for strength in [Strength::Off, Strength::Normal, Strength::Strong] {
            assert_eq!(Strength::from_name(strength.name()), Some(strength));
        }
        assert_eq!(Strength::from_name("loud"), None);
        assert_eq!(Strength::Off.next().next().next(), Strength::Off);
    }

    #[test]
    fn a_clipping_microphone_is_reported_and_the_warning_lingers() {
        let mut watch = MicWatch::default();
        assert_eq!(watch.hear(0.5, false), MicAdvice::Fine);
        assert_eq!(watch.hear(1.0, false), MicAdvice::Clipping);
        for _ in 0..SHOWN_FOR - 1 {
            assert_eq!(watch.hear(0.1, false), MicAdvice::Clipping);
        }
        assert_eq!(watch.hear(0.1, false), MicAdvice::Fine);
    }

    #[test]
    fn a_whisper_the_gain_cannot_reach_is_reported() {
        let mut leveller = Leveller::new(RATE);
        assert!(!leveller.straining());
        levelled(&mut leveller, talk(20, 0.003, 0.0001));
        assert!(leveller.straining());
        let mut watch = MicWatch::default();
        assert_eq!(watch.hear(0.003, true), MicAdvice::TooQuiet);
        assert_eq!(watch.hear(1.0, true), MicAdvice::Clipping);
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
