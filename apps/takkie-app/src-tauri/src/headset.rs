//! Noticing that someone who started talking with a click has gone quiet,
//! so a forgotten open microphone closes by itself.

use std::time::{Duration, Instant};

/// How long without speech before talking stops by itself.
pub const QUIET_LIMIT: Duration = Duration::from_secs(30);

// Speech has to be this many times louder than the background, plus a
// little, so that a silent room's hiss never counts.
const ABOVE_BACKGROUND: f32 = 2.5;
const AT_LEAST: f32 = 0.004;
// Per sample, how fast the background estimate creeps up. Slow, because it
// must not climb to the level of someone talking between two breaths.
const CREEP: f32 = 0.002;

/// Listens to the microphone level and keeps track of the last speech.
#[derive(Debug)]
pub struct SpeechWatch {
    background: Option<f32>,
    last_speech: Instant,
}

impl SpeechWatch {
    /// Starts counting quiet time from `now`.
    pub fn new(now: Instant) -> Self {
        Self {
            background: None,
            last_speech: now,
        }
    }

    /// Takes one microphone level, about ten a second, and says whether it
    /// was speech.
    ///
    /// The background is the quietest the room has been lately: it drops at
    /// once to any quieter sample, and otherwise creeps up slowly. Steady
    /// noise therefore becomes background, while speech, which always has
    /// gaps, doesn't.
    pub fn hear(&mut self, rms: f32, now: Instant) -> bool {
        if !rms.is_finite() {
            return false;
        }
        let background = match self.background {
            None => rms,
            Some(was) if rms < was => rms,
            Some(was) => was + (rms - was) * CREEP,
        };
        self.background = Some(background);
        let speech = rms > background * ABOVE_BACKGROUND + AT_LEAST;
        if speech {
            self.last_speech = now;
        }
        speech
    }

    /// Whether nobody has spoken for [`QUIET_LIMIT`].
    pub fn gone_quiet(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_speech) >= QUIET_LIMIT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_millis(100);

    fn feed(watch: &mut SpeechWatch, from: Instant, levels: impl Iterator<Item = f32>) -> Instant {
        let mut now = from;
        for level in levels {
            now += TICK;
            watch.hear(level, now);
        }
        now
    }

    fn quiet_room() -> impl Iterator<Item = f32> {
        std::iter::repeat(0.001)
    }

    // A second of talking: loud, with a short dip between words.
    fn talking(seconds: usize) -> impl Iterator<Item = f32> {
        (0..seconds * 10).map(|tick| if tick % 10 < 8 { 0.2 } else { 0.01 })
    }

    #[test]
    fn a_forgotten_microphone_closes_after_the_limit() {
        let t0 = Instant::now();
        let mut watch = SpeechWatch::new(t0);
        let now = feed(&mut watch, t0, quiet_room().take(290));
        assert!(!watch.gone_quiet(now), "29 s of quiet is still fine");
        let now = feed(&mut watch, now, quiet_room().take(15));
        assert!(watch.gone_quiet(now));
    }

    #[test]
    fn someone_who_keeps_talking_is_never_cut_off() {
        let t0 = Instant::now();
        let mut watch = SpeechWatch::new(t0);
        let now = feed(&mut watch, t0, talking(300));
        assert!(!watch.gone_quiet(now), "five minutes of speech");
    }

    #[test]
    fn the_limit_counts_from_the_last_word() {
        let t0 = Instant::now();
        let mut watch = SpeechWatch::new(t0);
        let now = feed(&mut watch, t0, quiet_room().take(250));
        let now = feed(&mut watch, now, talking(2));
        let now = feed(&mut watch, now, quiet_room().take(250));
        assert!(!watch.gone_quiet(now), "25 s since speaking");
        let now = feed(&mut watch, now, quiet_room().take(60));
        assert!(watch.gone_quiet(now));
    }

    #[test]
    fn steady_noise_is_not_speech_but_a_voice_over_it_is() {
        let t0 = Instant::now();
        let mut watch = SpeechWatch::new(t0);
        let street = || (0..).map(|tick| if tick % 2 == 0 { 0.05 } else { 0.06 });
        let now = feed(&mut watch, t0, street().take(320));
        assert!(watch.gone_quiet(now), "noise alone doesn't hold it open");
        assert!(
            watch.hear(0.3, now + TICK),
            "a voice over the noise is heard"
        );
        assert!(!watch.gone_quiet(now + TICK));
    }

    #[test]
    fn a_broken_level_changes_nothing() {
        let t0 = Instant::now();
        let mut watch = SpeechWatch::new(t0);
        assert!(!watch.hear(f32::NAN, t0));
        assert!(!watch.hear(f32::INFINITY, t0));
        assert!(watch.background.is_none());
    }
}
