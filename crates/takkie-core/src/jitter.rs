//! Per-sender jitter buffer. Packets sit in a fixed ring indexed by `seq`;
//! `next` is the playout point, and anything older is late.

use std::time::{Duration, Instant};

use crate::Seq;

/// Timing for one stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Audio per packet.
    pub frame: Duration,
    /// Delay before the first playout.
    pub target: Duration,
    /// Trim back to `target` above this.
    pub max: Duration,
    /// Start over after this long with no packets.
    pub silence: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            frame: Duration::from_millis(20),
            target: Duration::from_millis(60),
            max: Duration::from_millis(200),
            silence: Duration::from_millis(300),
        }
    }
}

impl Config {
    fn frames(&self, span: Duration) -> usize {
        let frame = self.frame.as_micros().max(1);
        usize::try_from(span.as_micros() / frame).unwrap_or(usize::MAX)
    }
}

/// Reorder and playout buffer for one sender.
#[derive(Debug)]
pub struct JitterBuffer<T> {
    config: Config,
    slots: Box<[Option<(Seq, T)>]>,
    next: Option<Seq>,
    newest: Option<Seq>,
    end: Option<Seq>,
    stored: usize,
    playing: bool,
    first_arrival: Option<Instant>,
    last_arrival: Option<Instant>,
    stats: Stats,
}

/// Running totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Packets stored.
    pub received: u64,
    /// Frames played without a packet.
    pub lost: u64,
    /// Arrived after their turn.
    pub late: u64,
    /// Arrived twice.
    pub duplicates: u64,
    /// Dropped to cut the delay.
    pub trimmed: u64,
}

/// What `insert` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insert {
    /// Kept.
    Stored,
    /// Already had it.
    Duplicate,
    /// Too late.
    Late,
    /// So far ahead the buffer started over.
    Restarted,
}

/// What to play next.
#[derive(Debug, PartialEq, Eq)]
pub enum Playout<'a, T> {
    /// The packet.
    Packet {
        /// Sequence number.
        seq: Seq,
        /// Payload.
        payload: T,
    },
    /// Missing; rebuild it from the next packet's FEC data.
    Fec {
        /// Sequence number.
        seq: Seq,
        /// The next packet, still buffered.
        next: &'a T,
    },
    /// Missing; use loss concealment.
    Plc {
        /// Sequence number.
        seq: Seq,
    },
    /// Nothing to play yet.
    NotReady,
}

impl<T> JitterBuffer<T> {
    /// Default timing.
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    /// Room for twice the max delay.
    #[must_use]
    pub fn with_config(config: Config) -> Self {
        // A power of two divides 2^32, so `seq % len` stays unique across wrap-around.
        let len = config
            .frames(config.max)
            .saturating_mul(2)
            .max(1)
            .next_power_of_two();
        Self {
            config,
            slots: (0..len).map(|_| None).collect(),
            next: None,
            newest: None,
            end: None,
            stored: 0,
            playing: false,
            first_arrival: None,
            last_arrival: None,
            stats: Stats::default(),
        }
    }

    /// Totals so far.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Slots.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Packets waiting.
    #[must_use]
    pub fn len(&self) -> usize {
        self.stored
    }

    /// Whether nothing is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stored == 0
    }

    /// Stores a packet.
    pub fn insert(&mut self, seq: Seq, payload: T, now: Instant) -> Insert {
        let result = self.place(seq, payload, now);
        match result {
            Insert::Stored | Insert::Restarted => self.stats.received += 1,
            Insert::Duplicate => self.stats.duplicates += 1,
            Insert::Late => self.stats.late += 1,
        }
        result
    }

    fn place(&mut self, seq: Seq, payload: T, now: Instant) -> Insert {
        self.first_arrival.get_or_insert(now);
        self.last_arrival = Some(now);

        let (Some(next), Some(newest)) = (self.next, self.newest) else {
            self.next = Some(seq);
            self.newest = Some(seq);
            return self.store(seq, payload);
        };

        if next.is_newer_than(seq) {
            if self.playing || distance(seq, newest) >= self.capacity() {
                return Insert::Late;
            }
            self.next = Some(seq);
        } else if distance(next, seq) >= self.capacity() {
            self.reset();
            self.place(seq, payload, now);
            return Insert::Restarted;
        }

        let stored = self.store(seq, payload);
        if stored == Insert::Stored && seq.is_newer_than(newest) {
            self.newest = Some(seq);
        }
        stored
    }

    /// For the last packet of a press; the buffer starts over after it.
    pub fn insert_end(&mut self, seq: Seq, payload: T, now: Instant) -> Insert {
        let result = self.insert(seq, payload, now);
        if matches!(result, Insert::Stored | Insert::Restarted) {
            self.end = Some(seq);
        }
        result
    }

    /// The frame due at `now`.
    pub fn pop_next(&mut self, now: Instant) -> Playout<'_, T> {
        if self
            .last_arrival
            .is_some_and(|at| now.saturating_duration_since(at) >= self.config.silence)
        {
            self.reset();
            return Playout::NotReady;
        }
        if !self.playing {
            let ready = self
                .first_arrival
                .is_some_and(|at| now.saturating_duration_since(at) >= self.config.target);
            if !ready {
                return Playout::NotReady;
            }
            self.playing = true;
        }
        self.trim();

        let Some(next) = self.next else {
            return Playout::NotReady;
        };
        self.next = Some(next.next());
        let played = self.take(next);
        if self.end == Some(next) {
            self.reset();
        }
        if let Some(payload) = played {
            return Playout::Packet { seq: next, payload };
        }
        self.stats.lost += 1;
        match self.peek(next.next()) {
            Some(following) => Playout::Fec {
                seq: next,
                next: following,
            },
            None => Playout::Plc { seq: next },
        }
    }

    fn trim(&mut self) {
        let (Some(next), Some(newest)) = (self.next, self.newest) else {
            return;
        };
        if next.is_newer_than(newest) {
            return;
        }
        let buffered = distance(next, newest) + 1;
        if buffered <= self.config.frames(self.config.max) {
            return;
        }
        let keep = self.config.frames(self.config.target).max(1);
        let mut seq = next;
        for _ in 0..buffered.saturating_sub(keep) {
            if self.take(seq).is_some() {
                self.stats.trimmed += 1;
            }
            seq = seq.next();
        }
        self.next = Some(seq);
    }

    fn take(&mut self, seq: Seq) -> Option<T> {
        let index = self.index(seq);
        let slot = self.slots.get_mut(index)?;
        if slot.as_ref().is_some_and(|(held, _)| *held == seq) {
            let (_, payload) = slot.take()?;
            self.stored -= 1;
            return Some(payload);
        }
        None
    }

    fn peek(&self, seq: Seq) -> Option<&T> {
        match self.slots.get(self.index(seq)) {
            Some(Some((held, payload))) if *held == seq => Some(payload),
            _ => None,
        }
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
            None => Insert::Late,
        }
    }

    fn reset(&mut self) {
        self.slots.iter_mut().for_each(|slot| *slot = None);
        self.next = None;
        self.newest = None;
        self.end = None;
        self.stored = 0;
        self.playing = false;
        self.first_arrival = None;
        self.last_arrival = None;
    }

    fn index(&self, seq: Seq) -> usize {
        seq.get() as usize % self.slots.len()
    }
}

impl<T> Default for JitterBuffer<T> {
    fn default() -> Self {
        Self::new()
    }
}

fn distance(from: Seq, to: Seq) -> usize {
    to.get().wrapping_sub(from.get()) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[derive(Debug, PartialEq, Eq)]
    enum Out {
        Packet(u32),
        Fec(u32, u32),
        Plc(u32),
        NotReady,
    }
    use Out::*;

    fn out(playout: Playout<'_, u32>) -> Out {
        match playout {
            Playout::Packet { seq, payload } => {
                assert_eq!(seq.get(), payload);
                Packet(payload)
            }
            Playout::Fec { seq, next } => Fec(seq.get(), *next),
            Playout::Plc { seq } => Plc(seq.get()),
            Playout::NotReady => NotReady,
        }
    }

    fn fill(buffer: &mut JitterBuffer<u32>, seqs: &[u32], at: Instant) {
        for &seq in seqs {
            assert_eq!(buffer.insert(Seq::new(seq), seq, at), Insert::Stored);
        }
    }

    fn pop(buffer: &mut JitterBuffer<u32>, count: usize, at: Instant) -> Vec<Out> {
        (0..count).map(|_| out(buffer.pop_next(at))).collect()
    }

    fn pop_one(buffer: &mut JitterBuffer<u32>, at: Instant) -> Out {
        out(buffer.pop_next(at))
    }

    #[test]
    fn capacity_is_twice_the_max_delay_rounded_up() {
        assert_eq!(JitterBuffer::<u32>::new().capacity(), 32);
        let tiny = Config {
            max: Duration::ZERO,
            ..Config::default()
        };
        assert_eq!(JitterBuffer::<u32>::with_config(tiny).capacity(), 1);
    }

    #[test]
    fn in_order_packets_come_out_in_order() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[1, 2, 3], t0);
        assert_eq!(buffer.len(), 3);
        assert_eq!(
            pop(&mut buffer, 3, t0 + 60 * MS),
            [Packet(1), Packet(2), Packet(3)]
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn reordered_packets_come_out_in_order() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[3, 1, 2, 5, 4], t0);
        assert_eq!(
            pop(&mut buffer, 5, t0 + 60 * MS),
            [Packet(1), Packet(2), Packet(3), Packet(4), Packet(5)]
        );
    }

    #[test]
    fn duplicates_are_dropped() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[1, 2], t0);
        assert_eq!(buffer.insert(Seq::new(2), 99, t0), Insert::Duplicate);
        assert_eq!(buffer.len(), 2);
        assert_eq!(pop(&mut buffer, 2, t0 + 60 * MS), [Packet(1), Packet(2)]);
    }

    #[test]
    fn packets_behind_the_playout_point_are_late() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[1, 2, 3], t0);
        assert_eq!(pop(&mut buffer, 2, t0 + 60 * MS), [Packet(1), Packet(2)]);
        assert_eq!(buffer.insert(Seq::new(1), 1, t0), Insert::Late);
        assert_eq!(buffer.insert(Seq::new(2), 2, t0), Insert::Late);
        assert_eq!(pop(&mut buffer, 1, t0 + 60 * MS), [Packet(3)]);
    }

    #[test]
    fn a_gap_before_a_present_packet_uses_fec() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[1, 4], t0);
        assert_eq!(
            pop(&mut buffer, 4, t0 + 60 * MS),
            [Packet(1), Plc(2), Fec(3, 4), Packet(4)]
        );
    }

    #[test]
    fn a_late_arrival_fills_its_gap_before_playout() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[1, 3], t0);
        assert_eq!(pop(&mut buffer, 1, t0 + 60 * MS), [Packet(1)]);
        fill(&mut buffer, &[2], t0 + 60 * MS);
        assert_eq!(pop(&mut buffer, 2, t0 + 60 * MS), [Packet(2), Packet(3)]);
    }

    #[test]
    fn early_reordering_too_wide_for_the_buffer_is_late() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[10, 41], t0);
        assert_eq!(buffer.insert(Seq::new(9), 9, t0), Insert::Late);
    }

    #[test]
    fn works_across_the_seq_wrap() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[0, u32::MAX, 1, u32::MAX - 1], t0);
        assert_eq!(
            pop(&mut buffer, 4, t0 + 60 * MS),
            [Packet(u32::MAX - 1), Packet(u32::MAX), Packet(0), Packet(1)]
        );
    }

    #[test]
    fn waits_for_the_target_delay_before_playing() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        assert_eq!(pop_one(&mut buffer, t0), NotReady);
        fill(&mut buffer, &[1, 2], t0);
        assert_eq!(pop_one(&mut buffer, t0 + 59 * MS), NotReady);
        assert_eq!(pop_one(&mut buffer, t0 + 60 * MS), Packet(1));
    }

    #[test]
    fn steady_stream_plays_one_packet_per_frame() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        let mut played = Vec::new();
        for seq in 0..20_u32 {
            let now = t0 + 20 * MS * seq;
            fill(&mut buffer, &[seq], now);
            played.push(pop_one(&mut buffer, now));
        }
        played.retain(|p| *p != NotReady);
        assert_eq!(played, (0..17).map(Packet).collect::<Vec<_>>());
        assert_eq!(buffer.len(), 3);
    }

    #[test]
    fn running_dry_while_playing_uses_plc() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[1], t0);
        assert_eq!(pop_one(&mut buffer, t0 + 60 * MS), Packet(1));
        assert_eq!(pop_one(&mut buffer, t0 + 80 * MS), Plc(2));
        fill(&mut buffer, &[3], t0 + 90 * MS);
        assert_eq!(pop_one(&mut buffer, t0 + 100 * MS), Packet(3));
    }

    #[test]
    fn a_burst_over_the_max_delay_is_trimmed_to_the_target() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &(1..=15).collect::<Vec<_>>(), t0);
        assert_eq!(
            pop(&mut buffer, 3, t0 + 60 * MS),
            [Packet(13), Packet(14), Packet(15)]
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_burst_within_the_max_delay_is_kept() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &(1..=10).collect::<Vec<_>>(), t0);
        assert_eq!(pop_one(&mut buffer, t0 + 60 * MS), Packet(1));
        assert_eq!(buffer.len(), 9);
    }

    #[test]
    fn end_of_transmission_starts_over() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[1], t0);
        assert_eq!(buffer.insert_end(Seq::new(2), 2, t0), Insert::Stored);
        assert_eq!(pop(&mut buffer, 2, t0 + 60 * MS), [Packet(1), Packet(2)]);
        assert_eq!(pop_one(&mut buffer, t0 + 80 * MS), NotReady);

        fill(&mut buffer, &[50], t0 + 100 * MS);
        assert_eq!(pop_one(&mut buffer, t0 + 120 * MS), NotReady);
        assert_eq!(pop_one(&mut buffer, t0 + 160 * MS), Packet(50));
    }

    #[test]
    fn long_silence_starts_over() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[1], t0);
        assert_eq!(pop_one(&mut buffer, t0 + 60 * MS), Packet(1));
        assert_eq!(pop_one(&mut buffer, t0 + 300 * MS), NotReady);
        fill(&mut buffer, &[1], t0 + 400 * MS);
        assert_eq!(pop_one(&mut buffer, t0 + 460 * MS), Packet(1));
    }

    #[test]
    fn a_jump_past_the_buffer_starts_over_from_it() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &[1, 2], t0);
        assert_eq!(pop_one(&mut buffer, t0 + 60 * MS), Packet(1));
        assert_eq!(
            buffer.insert(Seq::new(500), 500, t0 + 70 * MS),
            Insert::Restarted
        );
        assert_eq!(buffer.len(), 1);
        assert_eq!(pop_one(&mut buffer, t0 + 100 * MS), NotReady);
        assert_eq!(pop_one(&mut buffer, t0 + 130 * MS), Packet(500));
    }

    #[test]
    fn stats_count_every_outcome() {
        let t0 = Instant::now();
        let mut buffer = JitterBuffer::new();
        fill(&mut buffer, &(1..=15).collect::<Vec<_>>(), t0);
        buffer.insert(Seq::new(15), 15, t0);
        assert_eq!(
            pop(&mut buffer, 4, t0 + 60 * MS),
            [Packet(13), Packet(14), Packet(15), Plc(16)]
        );
        buffer.insert(Seq::new(14), 14, t0 + 60 * MS);
        assert_eq!(
            buffer.stats(),
            Stats {
                received: 15,
                lost: 1,
                late: 1,
                duplicates: 1,
                trimmed: 12,
            }
        );
    }
}
