//! Push-to-talk logic shared by every UI, so a key behaves the same
//! everywhere.
//!
//! Terminals report a held key differently. Windows sends repeated presses
//! and then a release. Most Linux terminals send the repeated presses but
//! never a release, which is what [`PttMode::HoldWithTimeout`] is for. The
//! kitty keyboard protocol sends [`PttInput::Repeat`] instead of repeated
//! presses. Time is passed in, so tests can replay real key timings.

use std::time::{Duration, Instant};

/// How the push-to-talk key behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PttMode {
    /// Transmit while the key is down; needs release events.
    Hold,
    /// Each tap turns transmitting on or off.
    Toggle,
    /// Transmit while presses keep coming, for terminals that never report a
    /// release. Stops `timeout` after the last one.
    HoldWithTimeout {
        /// Must be longer than the keyboard's repeat delay, or it flickers.
        timeout: Duration,
    },
}

/// A key event or a clock tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PttInput {
    /// The key went down, or a terminal's auto-repeat sent another press.
    Press,
    /// An explicit auto-repeat event.
    Repeat,
    /// The key came up.
    Release,
    /// Time passed with no key event.
    Tick,
}

/// A change in whether we're transmitting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PttChange {
    /// Started transmitting.
    Started,
    /// Stopped transmitting.
    Stopped,
}

/// A press this soon after the last key event is auto-repeat, not a new tap.
/// Longer than usual repeat delays (250 to 660 ms).
const REPEAT_GAP: Duration = Duration::from_millis(700);

/// Turns key events into transmit on and off.
#[derive(Clone, Debug)]
pub struct PttController {
    mode: PttMode,
    transmitting: bool,
    key_down: bool,
    last_key: Option<Instant>,
}

impl PttController {
    /// A controller that isn't transmitting yet.
    #[must_use]
    pub fn new(mode: PttMode) -> Self {
        Self {
            mode,
            transmitting: false,
            key_down: false,
            last_key: None,
        }
    }

    /// Whether we're transmitting now.
    #[must_use]
    pub fn is_transmitting(&self) -> bool {
        self.transmitting
    }

    /// Feeds one input that happened at `now`.
    pub fn handle(&mut self, input: PttInput, now: Instant) -> Option<PttChange> {
        let repeat = input == PttInput::Repeat
            || (input == PttInput::Press
                && self.key_down
                && self
                    .last_key
                    .is_some_and(|at| now.saturating_duration_since(at) < REPEAT_GAP));
        if input != PttInput::Tick {
            self.last_key = Some(now);
        }

        match (self.mode, input) {
            (_, PttInput::Release) => {
                self.key_down = false;
                match self.mode {
                    PttMode::Toggle => None,
                    _ => self.set(false),
                }
            }
            (PttMode::Toggle, PttInput::Press) if !repeat => {
                self.key_down = true;
                self.set(!self.transmitting)
            }
            (PttMode::Toggle, _) => None,
            (_, PttInput::Press | PttInput::Repeat) => {
                self.key_down = true;
                self.set(true)
            }
            (PttMode::HoldWithTimeout { timeout }, PttInput::Tick) => {
                let quiet = self
                    .last_key
                    .is_none_or(|at| now.saturating_duration_since(at) >= timeout);
                if quiet {
                    self.key_down = false;
                    self.set(false)
                } else {
                    None
                }
            }
            (PttMode::Hold, PttInput::Tick) => None,
        }
    }

    fn set(&mut self, on: bool) -> Option<PttChange> {
        if self.transmitting == on {
            return None;
        }
        self.transmitting = on;
        Some(if on {
            PttChange::Started
        } else {
            PttChange::Stopped
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PttChange::{Started, Stopped};
    use PttInput::{Press, Release, Repeat, Tick};

    fn replay(mode: PttMode, events: &[(u64, PttInput)], until: u64) -> Vec<(u64, PttChange)> {
        let t0 = Instant::now();
        let mut ptt = PttController::new(mode);
        let mut changes = Vec::new();
        let mut events = events.iter().peekable();
        for ms in 0..=until {
            while let Some(&&(at, input)) = events.peek() {
                if at != ms {
                    break;
                }
                if let Some(change) = ptt.handle(input, t0 + Duration::from_millis(ms)) {
                    changes.push((ms, change));
                }
                events.next();
            }
            if ms % 20 == 0
                && let Some(change) = ptt.handle(Tick, t0 + Duration::from_millis(ms))
            {
                changes.push((ms, change));
            }
        }
        changes
    }

    fn held(start: u64, end: u64, delay: u64, repeat: PttInput) -> Vec<(u64, PttInput)> {
        let mut events = vec![(start, Press)];
        let mut at = start + delay;
        while at < end {
            events.push((at, repeat));
            at += 33;
        }
        events
    }

    fn with_release(mut events: Vec<(u64, PttInput)>, at: u64) -> Vec<(u64, PttInput)> {
        events.push((at, Release));
        events
    }

    const TIMEOUT: PttMode = PttMode::HoldWithTimeout {
        timeout: Duration::from_millis(700),
    };

    #[test]
    fn hold_on_windows_starts_once_and_stops_on_release() {
        let events = with_release(held(100, 2_000, 500, Press), 2_000);
        assert_eq!(
            replay(PttMode::Hold, &events, 3_000),
            [(100, Started), (2_000, Stopped)]
        );
    }

    #[test]
    fn hold_with_explicit_repeat_events() {
        let events = with_release(held(100, 1_500, 300, Repeat), 1_500);
        assert_eq!(
            replay(PttMode::Hold, &events, 2_000),
            [(100, Started), (1_500, Stopped)]
        );
    }

    #[test]
    fn hold_ignores_ticks_and_stray_releases() {
        assert_eq!(replay(PttMode::Hold, &[(50, Release)], 1_000), []);
    }

    #[test]
    fn timeout_mode_on_linux_never_flickers_during_the_repeat_delay() {
        let events = held(100, 2_000, 660, Press);
        let last_repeat = events.last().map_or(0, |&(at, _)| at);
        let changes = replay(TIMEOUT, &events, 4_000);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0], (100, Started));
        let (stopped_at, change) = changes[1];
        assert_eq!(change, Stopped);
        assert!((last_repeat + 700..=last_repeat + 720).contains(&stopped_at));
    }

    #[test]
    fn timeout_mode_stops_at_once_when_a_release_arrives() {
        let events = with_release(held(100, 1_000, 500, Press), 1_000);
        assert_eq!(
            replay(TIMEOUT, &events, 3_000),
            [(100, Started), (1_000, Stopped)]
        );
    }

    #[test]
    fn timeout_mode_quick_tap_still_transmits_for_the_timeout() {
        assert_eq!(
            replay(TIMEOUT, &[(100, Press)], 2_000),
            [(100, Started), (800, Stopped)]
        );
    }

    #[test]
    fn toggle_taps_turn_transmitting_on_and_off() {
        let events = [
            (100, Press),
            (180, Release),
            (1_500, Press),
            (1_580, Release),
        ];
        assert_eq!(
            replay(PttMode::Toggle, &events, 2_000),
            [(100, Started), (1_500, Stopped)]
        );
    }

    #[test]
    fn toggle_held_on_windows_toggles_once() {
        let events = with_release(held(100, 2_000, 500, Press), 2_000);
        assert_eq!(replay(PttMode::Toggle, &events, 3_000), [(100, Started)]);
    }

    #[test]
    fn toggle_held_on_linux_toggles_once_then_a_later_tap_stops() {
        let mut events = held(100, 2_000, 660, Press);
        events.push((3_000, Press));
        assert_eq!(
            replay(PttMode::Toggle, &events, 4_000),
            [(100, Started), (3_000, Stopped)]
        );
    }

    #[test]
    fn toggle_quick_taps_with_releases_all_count() {
        let events = [(100, Press), (150, Release), (300, Press), (350, Release)];
        assert_eq!(
            replay(PttMode::Toggle, &events, 1_000),
            [(100, Started), (300, Stopped)]
        );
    }

    #[test]
    fn is_transmitting_follows_the_changes() {
        let t0 = Instant::now();
        let mut ptt = PttController::new(PttMode::Hold);
        assert!(!ptt.is_transmitting());
        ptt.handle(Press, t0);
        assert!(ptt.is_transmitting());
        ptt.handle(Release, t0);
        assert!(!ptt.is_transmitting());
    }
}
