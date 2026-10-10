//! Replay protection: a sender's sequence number is accepted only once.

use std::collections::HashMap;

use crate::{PeerId, Seq};

/// How far behind the newest number a late packet may still be.
pub const WINDOW: u32 = 64;

/// The numbers recently seen from one sender.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplayWindow {
    newest: Option<Seq>,
    // Bit n is set when `newest - n` was seen.
    seen: u64,
}

impl ReplayWindow {
    /// Nothing seen yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `seq` is new, recording it if so.
    ///
    /// Only call it for packets that passed authentication: a forged
    /// number would otherwise move the window and shut the real sender out.
    pub fn accept(&mut self, seq: Seq) -> bool {
        let Some(newest) = self.newest else {
            self.newest = Some(seq);
            self.seen = 1;
            return true;
        };
        if seq.is_newer_than(newest) {
            let ahead = seq.get().wrapping_sub(newest.get());
            self.seen = if ahead < WINDOW {
                (self.seen << ahead) | 1
            } else {
                1
            };
            self.newest = Some(seq);
            return true;
        }
        let behind = newest.get().wrapping_sub(seq.get());
        if behind >= WINDOW {
            return false;
        }
        let bit = 1_u64 << behind;
        let fresh = self.seen & bit == 0;
        self.seen |= bit;
        fresh
    }
}

/// A [`ReplayWindow`] for every sender.
#[derive(Clone, Debug, Default)]
pub struct Replays {
    windows: HashMap<PeerId, ReplayWindow>,
}

impl Replays {
    /// No senders yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this packet from `sender` is new. Authenticated packets only,
    /// as for [`ReplayWindow::accept`].
    pub fn accept(&mut self, sender: PeerId, seq: Seq) -> bool {
        self.windows.entry(sender).or_default().accept(seq)
    }

    /// Drops a sender who left.
    pub fn forget(&mut self, sender: PeerId) {
        self.windows.remove(&sender);
    }

    /// How many senders are tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether no sender is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use proptest::prelude::*;

    use super::*;

    fn accepted(window: &mut ReplayWindow, numbers: &[u32]) -> Vec<bool> {
        numbers
            .iter()
            .map(|&n| window.accept(Seq::new(n)))
            .collect()
    }

    #[test]
    fn in_order_packets_pass_and_repeats_do_not() {
        let mut window = ReplayWindow::new();
        assert_eq!(
            accepted(&mut window, &[10, 11, 12, 11, 12, 10, 13]),
            [true, true, true, false, false, false, true]
        );
    }

    #[test]
    fn late_packets_inside_the_window_pass_once() {
        let mut window = ReplayWindow::new();
        assert_eq!(
            accepted(&mut window, &[100, 105, 103, 101, 103, 104, 102, 102]),
            [true, true, true, true, false, true, true, false]
        );
    }

    #[test]
    fn packets_older_than_the_window_are_refused() {
        let mut window = ReplayWindow::new();
        assert!(window.accept(Seq::new(1_000)));
        assert!(window.accept(Seq::new(1_000 - WINDOW + 1)));
        assert!(!window.accept(Seq::new(1_000 - WINDOW)));
        assert!(!window.accept(Seq::new(5)));
    }

    #[test]
    fn a_big_jump_forward_forgets_the_old_numbers() {
        let mut window = ReplayWindow::new();
        assert!(window.accept(Seq::new(7)));
        assert!(window.accept(Seq::new(7 + 10 * WINDOW)));
        assert!(!window.accept(Seq::new(7)));
        assert!(window.accept(Seq::new(7 + 10 * WINDOW - 1)));
    }

    #[test]
    fn the_counter_wrapping_around_is_handled() {
        let mut window = ReplayWindow::new();
        assert_eq!(
            accepted(
                &mut window,
                &[u32::MAX - 1, u32::MAX, 0, 1, u32::MAX, 0, u32::MAX - 2]
            ),
            [true, true, true, true, false, false, true]
        );
    }

    #[test]
    fn senders_are_tracked_apart_and_can_be_forgotten() {
        let mut replays = Replays::new();
        let (a, b) = (PeerId::new(1), PeerId::new(2));
        assert!(replays.accept(a, Seq::new(5)));
        assert!(replays.accept(b, Seq::new(5)));
        assert!(!replays.accept(a, Seq::new(5)));
        assert_eq!(replays.len(), 2);
        replays.forget(a);
        assert_eq!(replays.len(), 1);
        assert!(replays.accept(a, Seq::new(5)));
        assert!(!replays.is_empty());
    }

    proptest! {
        #[test]
        fn no_number_is_ever_accepted_twice(
            start in any::<u32>(),
            steps in proptest::collection::vec(0_u32..200, 1..300),
        ) {
            let mut window = ReplayWindow::new();
            let mut passed = HashSet::new();
            for step in steps {
                let seq = start.wrapping_add(step);
                if window.accept(Seq::new(seq)) {
                    prop_assert!(passed.insert(seq), "{} was accepted twice", seq);
                }
            }
        }

        #[test]
        fn a_stream_in_order_always_passes(start in any::<u32>(), count in 1_u32..500) {
            let mut window = ReplayWindow::new();
            for n in 0..count {
                prop_assert!(window.accept(Seq::new(start.wrapping_add(n))));
            }
        }
    }
}
