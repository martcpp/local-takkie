//! Per-sender jitter buffer: turns packets that arrive late, early or out of
//! order back into one stream in `seq` order.
//!
//! Packets sit in a fixed ring of slots indexed by `seq`, so memory never
//! grows. `next` is the playout point. Before the first [`JitterBuffer::pop`]
//! it follows the oldest packet, so early reordering is fixed; after that,
//! anything older than `next` is late and dropped.

use crate::Seq;

/// Per-sender reorder buffer with a fixed number of slots.
#[derive(Debug)]
pub struct JitterBuffer<T> {
    slots: Box<[Option<(Seq, T)>]>,
    next: Option<Seq>,
    newest: Option<Seq>,
    stored: usize,
    started: bool,
}

/// What [`JitterBuffer::insert`] did with a packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insert {
    /// Kept for playout.
    Stored,
    /// Already had it.
    Duplicate,
    /// Its turn has passed.
    Late,
    /// More than a full buffer ahead of the playout point.
    TooFarAhead,
}

/// The next item in `seq` order.
#[derive(Debug, PartialEq, Eq)]
pub enum Released<T> {
    /// The packet arrived.
    Packet {
        /// Its sequence number.
        seq: Seq,
        /// Its payload.
        payload: T,
    },
    /// The packet never arrived.
    Missing {
        /// The missing sequence number.
        seq: Seq,
    },
}

impl<T> JitterBuffer<T> {
    /// A buffer holding at least `capacity` packets.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        // A power of two divides 2^32, so `seq % len` stays unique across wrap-around.
        let len = capacity.max(1).next_power_of_two();
        Self {
            slots: (0..len).map(|_| None).collect(),
            next: None,
            newest: None,
            stored: 0,
            started: false,
        }
    }

    /// How many packets fit.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// How many packets are waiting.
    #[must_use]
    pub fn len(&self) -> usize {
        self.stored
    }

    /// Whether no packets are waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stored == 0
    }

    /// Stores a packet unless it's a duplicate, late or too far ahead.
    pub fn insert(&mut self, seq: Seq, payload: T) -> Insert {
        let (Some(next), Some(newest)) = (self.next, self.newest) else {
            self.next = Some(seq);
            self.newest = Some(seq);
            return self.store(seq, payload);
        };

        if next.is_newer_than(seq) {
            if self.started || self.distance(seq, newest) >= self.capacity() {
                return Insert::Late;
            }
            self.next = Some(seq);
        } else if self.distance(next, seq) >= self.capacity() {
            return Insert::TooFarAhead;
        }

        let stored = self.store(seq, payload);
        if stored == Insert::Stored && seq.is_newer_than(newest) {
            self.newest = Some(seq);
        }
        stored
    }

    /// Releases the packet at the playout point, or reports it missing.
    /// Returns `None` while nothing is waiting.
    pub fn pop(&mut self) -> Option<Released<T>> {
        let next = self.next?;
        if self.stored == 0 {
            return None;
        }
        self.started = true;
        self.next = Some(next.next());

        let index = self.index(next);
        let slot = self.slots.get_mut(index)?;
        if slot.as_ref().is_some_and(|(seq, _)| *seq == next)
            && let Some((seq, payload)) = slot.take()
        {
            self.stored -= 1;
            return Some(Released::Packet { seq, payload });
        }
        Some(Released::Missing { seq: next })
    }

    fn store(&mut self, seq: Seq, payload: T) -> Insert {
        let index = self.index(seq);
        match self.slots.get_mut(index) {
            Some(Some((held, _))) if *held == seq => Insert::Duplicate,
            Some(slot) => {
                if slot.replace((seq, payload)).is_none() {
                    self.stored += 1;
                }
                Insert::Stored
            }
            None => Insert::TooFarAhead,
        }
    }

    fn index(&self, seq: Seq) -> usize {
        seq.get() as usize % self.slots.len()
    }

    fn distance(&self, from: Seq, to: Seq) -> usize {
        to.get().wrapping_sub(from.get()) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(seq: u32) -> Released<u32> {
        Released::Packet {
            seq: Seq::new(seq),
            payload: seq,
        }
    }

    fn missing(seq: u32) -> Released<u32> {
        Released::Missing { seq: Seq::new(seq) }
    }

    fn fill(buffer: &mut JitterBuffer<u32>, seqs: &[u32]) {
        for &seq in seqs {
            assert_eq!(buffer.insert(Seq::new(seq), seq), Insert::Stored);
        }
    }

    fn drain(buffer: &mut JitterBuffer<u32>) -> Vec<Released<u32>> {
        std::iter::from_fn(|| buffer.pop()).collect()
    }

    #[test]
    fn capacity_rounds_up_to_a_power_of_two() {
        assert_eq!(JitterBuffer::<u32>::new(10).capacity(), 16);
        assert_eq!(JitterBuffer::<u32>::new(16).capacity(), 16);
        assert_eq!(JitterBuffer::<u32>::new(0).capacity(), 1);
    }

    #[test]
    fn in_order_packets_come_out_in_order() {
        let mut buffer = JitterBuffer::new(8);
        fill(&mut buffer, &[1, 2, 3]);
        assert_eq!(buffer.len(), 3);
        assert_eq!(drain(&mut buffer), [packet(1), packet(2), packet(3)]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn reordered_packets_come_out_in_order() {
        let mut buffer = JitterBuffer::new(8);
        fill(&mut buffer, &[3, 1, 2, 5, 4]);
        assert_eq!(
            drain(&mut buffer),
            [packet(1), packet(2), packet(3), packet(4), packet(5)]
        );
    }

    #[test]
    fn duplicates_are_dropped() {
        let mut buffer = JitterBuffer::new(8);
        fill(&mut buffer, &[1, 2]);
        assert_eq!(buffer.insert(Seq::new(2), 99), Insert::Duplicate);
        assert_eq!(buffer.len(), 2);
        assert_eq!(drain(&mut buffer), [packet(1), packet(2)]);
    }

    #[test]
    fn packets_behind_the_playout_point_are_late() {
        let mut buffer = JitterBuffer::new(8);
        fill(&mut buffer, &[1, 2, 3]);
        assert_eq!(buffer.pop(), Some(packet(1)));
        assert_eq!(buffer.pop(), Some(packet(2)));
        assert_eq!(buffer.insert(Seq::new(1), 1), Insert::Late);
        assert_eq!(buffer.insert(Seq::new(2), 2), Insert::Late);
        assert_eq!(drain(&mut buffer), [packet(3)]);
    }

    #[test]
    fn gaps_come_out_as_missing() {
        let mut buffer = JitterBuffer::new(8);
        fill(&mut buffer, &[1, 4]);
        assert_eq!(
            drain(&mut buffer),
            [packet(1), missing(2), missing(3), packet(4)]
        );
    }

    #[test]
    fn a_late_arrival_fills_its_gap_before_playout() {
        let mut buffer = JitterBuffer::new(8);
        fill(&mut buffer, &[1, 3]);
        assert_eq!(buffer.pop(), Some(packet(1)));
        fill(&mut buffer, &[2]);
        assert_eq!(drain(&mut buffer), [packet(2), packet(3)]);
    }

    #[test]
    fn packets_a_full_buffer_ahead_are_dropped() {
        let mut buffer = JitterBuffer::new(4);
        fill(&mut buffer, &[10, 13]);
        assert_eq!(buffer.insert(Seq::new(14), 14), Insert::TooFarAhead);
        assert_eq!(buffer.len(), 2);
    }

    #[test]
    fn early_reordering_too_wide_for_the_buffer_is_late() {
        let mut buffer = JitterBuffer::new(4);
        fill(&mut buffer, &[10, 13]);
        assert_eq!(buffer.insert(Seq::new(9), 9), Insert::Late);
    }

    #[test]
    fn works_across_the_seq_wrap() {
        let mut buffer = JitterBuffer::new(8);
        fill(&mut buffer, &[0, u32::MAX, 1, u32::MAX - 1]);
        assert_eq!(
            drain(&mut buffer),
            [packet(u32::MAX - 1), packet(u32::MAX), packet(0), packet(1)]
        );
    }

    #[test]
    fn pop_on_an_empty_buffer_is_none() {
        let mut buffer = JitterBuffer::<u32>::new(8);
        assert_eq!(buffer.pop(), None);
        fill(&mut buffer, &[1]);
        assert_eq!(buffer.pop(), Some(packet(1)));
        assert_eq!(buffer.pop(), None);
    }
}
