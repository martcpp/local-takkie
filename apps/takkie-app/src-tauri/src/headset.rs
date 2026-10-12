//! Noticing that someone who started talking with a click has gone quiet,
//! so a forgotten open microphone closes by itself.

use std::time::{Duration, Instant};

/// How long without speech before talking stops by itself.
pub const QUIET_LIMIT: Duration = Duration::from_secs(30);

// Speech has to be this many times louder than the background, and never
// quieter than `AT_LEAST`. Measured on a wired earpiece in a room: the
// background sat around 0.001 to 0.002, with bumps and clicks up to 0.007.
const ABOVE_BACKGROUND: f32 = 3.0;
const AT_LEAST: f32 = 0.012;
// A voice lasts; a bump on the cable is one sample. This many loud samples
// out of the last `RECENT`, at ten a second, make it speech.
const RECENT: usize = 10;
const LOUD_ENOUGH: usize = 3;
// Per sample, how fast the background estimate creeps up. Slow, because it
// must not climb to the level of someone talking between two breaths.
const CREEP: f32 = 0.002;

/// Listens to the microphone level and keeps track of the last speech.
#[derive(Debug)]
pub struct SpeechWatch {
    background: Option<f32>,
    // The last `RECENT` samples, newest in the lowest bit: 1 for loud.
    recent: u16,
    last_speech: Instant,
}

impl SpeechWatch {
    /// Starts counting quiet time from `now`.
    pub fn new(now: Instant) -> Self {
        Self {
            background: None,
            recent: 0,
            last_speech: now,
        }
    }

    /// Takes one microphone level, about ten a second, and says whether
    /// someone is speaking.
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
        let loud = rms > (background * ABOVE_BACKGROUND).max(AT_LEAST);
        self.recent = ((self.recent << 1) | u16::from(loud)) & ((1 << RECENT) - 1);
        let speech = self.recent.count_ones() as usize >= LOUD_ENOUGH;
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
        std::iter::repeat(0.0014)
    }

    // Talking: loud, with a short dip between words.
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
    fn bumps_and_clicks_in_a_quiet_room_do_not_hold_it_open() {
        let t0 = Instant::now();
        let mut watch = SpeechWatch::new(t0);
        let room = (0..320).map(|tick| match tick % 37 {
            0 => 0.007,
            1 => 0.05,
            _ => 0.0014,
        });
        let now = feed(&mut watch, t0, room);
        assert!(
            watch.gone_quiet(now),
            "a loud sample now and then isn't a voice"
        );
    }

    #[test]
    fn someone_who_keeps_talking_is_never_cut_off() {
        let t0 = Instant::now();
        let mut watch = SpeechWatch::new(t0);
        let now = feed(&mut watch, t0, talking(300));
        assert!(!watch.gone_quiet(now), "five minutes of speech");
    }

    #[test]
    fn a_quiet_voice_still_counts() {
        let t0 = Instant::now();
        let mut watch = SpeechWatch::new(t0);
        let now = feed(&mut watch, t0, quiet_room().take(280));
        let soft = (0..20).map(|tick| if tick % 10 < 7 { 0.02 } else { 0.002 });
        let now = feed(&mut watch, now, soft);
        let now = feed(&mut watch, now, quiet_room().take(100));
        assert!(!watch.gone_quiet(now), "10 s after a soft sentence");
    }

    #[test]
    fn the_limit_counts_from_the_last_word() {
        let t0 = Instant::now();
        let mut watch = SpeechWatch::new(t0);
        let now = feed(&mut watch, t0, quiet_room().take(250));
        let now = feed(&mut watch, now, talking(2));
        let now = feed(&mut watch, now, quiet_room().take(250));
        assert!(!watch.gone_quiet(now), "25 s since speaking");
        let now = feed(&mut watch, now, quiet_room().take(70));
        assert!(watch.gone_quiet(now));
    }

    #[test]
    fn steady_noise_is_not_speech_but_a_voice_over_it_is() {
        let t0 = Instant::now();
        let mut watch = SpeechWatch::new(t0);
        let street = || (0..).map(|tick| if tick % 2 == 0 { 0.05 } else { 0.06 });
        let now = feed(&mut watch, t0, street().take(320));
        assert!(watch.gone_quiet(now), "noise alone doesn't hold it open");
        let voice = (0..10).map(|tick| if tick % 2 == 0 { 0.3 } else { 0.25 });
        let now = feed(&mut watch, now, voice);
        assert!(!watch.gone_quiet(now), "a voice over the noise is heard");
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
