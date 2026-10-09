//! Property tests for the jitter buffer.

use std::time::{Duration, Instant};

use proptest::prelude::*;
use takkie_core::Seq;
use takkie_core::jitter::{Insert, JitterBuffer, Playout};

#[derive(Clone, Debug)]
enum Step {
    Arrive(u32),
    ArriveLast(u32),
    Play,
    Wait(u64),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0..48_u32).prop_map(Step::Arrive),
        1 => (0..48_u32).prop_map(Step::ArriveLast),
        4 => Just(Step::Play),
        2 => (0..120_u64).prop_map(Step::Wait),
    ]
}

proptest! {
    #[test]
    fn random_arrivals_play_in_order_within_capacity(
        base in any::<u32>(),
        steps in prop::collection::vec(step(), 0..300),
    ) {
        let mut buffer = JitterBuffer::new();
        let mut now = Instant::now();
        let mut last: Option<Seq> = None;
        let mut end: Option<Seq> = None;

        for step in steps {
            match step {
                Step::Arrive(offset) | Step::ArriveLast(offset) => {
                    let seq = Seq::new(base.wrapping_add(offset));
                    let result = if matches!(step, Step::ArriveLast(_)) {
                        buffer.insert_end(seq, seq.get(), now)
                    } else {
                        buffer.insert(seq, seq.get(), now)
                    };
                    if result == Insert::Restarted {
                        last = None;
                        end = None;
                    }
                    if matches!(step, Step::ArriveLast(_))
                        && matches!(result, Insert::Stored | Insert::Restarted)
                    {
                        end = Some(seq);
                    }
                }
                Step::Play => {
                    let played = match buffer.pop_next(now) {
                        Playout::Packet { seq, payload } => {
                            prop_assert_eq!(payload, seq.get());
                            Some(seq)
                        }
                        Playout::Fec { seq, next } => {
                            prop_assert_eq!(*next, seq.next().get());
                            Some(seq)
                        }
                        Playout::Plc { seq } => Some(seq),
                        Playout::NotReady => None,
                    };
                    match (played, last) {
                        (Some(seq), Some(before)) => prop_assert!(seq.is_newer_than(before)),
                        (None, _) => last = None,
                        _ => {}
                    }
                    if played.is_some() {
                        last = played;
                    }
                    if played.is_some() && played == end {
                        last = None;
                        end = None;
                    }
                }
                Step::Wait(ms) => now += Duration::from_millis(ms),
            }
            prop_assert!(buffer.len() <= buffer.capacity());
        }
    }
}
