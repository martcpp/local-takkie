//! Turns a headset button's presses into push-to-talk.
//!
//! Two ways to use the one button, because not every headset can do both:
//! hold it to talk and let go to stop, or click to start and click again to
//! stop. Bluetooth headsets only ever send clicks, and with the screen off
//! Android keeps a long press for the voice assistant.

use std::time::{Duration, Instant};

/// Shorter than this is a click; longer is holding to talk.
const HOLD: Duration = Duration::from_millis(400);

/// Whether the headset button currently means "talk".
#[derive(Debug, Default)]
pub struct HeadsetTalk {
    talking: bool,
    // When the button went down, and whether that press started the talking.
    pressed: Option<(Instant, bool)>,
}

impl HeadsetTalk {
    /// The button went down. Returns whether to talk now.
    pub fn press(&mut self, now: Instant) -> bool {
        if self.pressed.is_none() {
            self.pressed = Some((now, !self.talking));
            self.talking = true;
        }
        self.talking
    }

    /// The button came up. Returns whether to talk now.
    pub fn release(&mut self, now: Instant) -> bool {
        if let Some((at, started)) = self.pressed.take() {
            let held = now.saturating_duration_since(at) >= HOLD;
            if held || !started {
                self.talking = false;
            }
        }
        self.talking
    }

    /// Forgets everything, for when something else stopped the talking.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Whether the button currently means "talk".
    pub fn is_talking(&self) -> bool {
        self.talking
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn holding_talks_until_let_go() {
        let t0 = Instant::now();
        let mut talk = HeadsetTalk::default();
        assert!(talk.press(t0));
        assert!(talk.is_talking());
        assert!(!talk.release(after(t0, 2_000)));
    }

    #[test]
    fn a_click_starts_and_another_click_stops() {
        let t0 = Instant::now();
        let mut talk = HeadsetTalk::default();
        assert!(talk.press(t0));
        assert!(talk.release(after(t0, 80)), "a click leaves it talking");
        assert!(talk.press(after(t0, 5_000)));
        assert!(!talk.release(after(t0, 5_080)), "the next click stops it");
    }

    #[test]
    fn a_long_press_also_stops_talking_started_by_a_click() {
        let t0 = Instant::now();
        let mut talk = HeadsetTalk::default();
        talk.press(t0);
        talk.release(after(t0, 80));
        talk.press(after(t0, 3_000));
        assert!(!talk.release(after(t0, 4_500)));
    }

    #[test]
    fn repeats_and_stray_releases_change_nothing() {
        let t0 = Instant::now();
        let mut talk = HeadsetTalk::default();
        assert!(!talk.release(t0), "a release with no press");
        talk.press(t0);
        assert!(talk.press(after(t0, 100)), "a repeated press");
        assert!(
            !talk.release(after(t0, 900)),
            "still one hold, from the first press"
        );
    }

    #[test]
    fn reset_forgets_a_talk_someone_else_stopped() {
        let t0 = Instant::now();
        let mut talk = HeadsetTalk::default();
        talk.press(t0);
        talk.release(after(t0, 50));
        talk.reset();
        assert!(!talk.is_talking());
        assert!(talk.press(after(t0, 1_000)));
        assert!(talk.release(after(t0, 1_050)), "a fresh click starts again");
    }
}
