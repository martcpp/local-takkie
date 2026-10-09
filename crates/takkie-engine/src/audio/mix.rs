//! The receiving side: one jitter buffer and Opus decoder per sender, mixed
//! into 20 ms frames.

use std::collections::HashMap;
use std::time::Instant;

use takkie_core::dsp::mix_into;
use takkie_core::jitter::{JitterBuffer, Playout};
use takkie_core::{PeerId, Seq};

use super::codec::{CodecError, VoiceDecoder};
use super::resample::FRAME;

/// An audio packet from the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RxPacket {
    /// Who sent it.
    pub sender: PeerId,
    /// Its sequence number.
    pub seq: Seq,
    /// The Opus packet.
    pub payload: Vec<u8>,
    /// Last packet of a push-to-talk press.
    pub end: bool,
}

struct Talker {
    jitter: JitterBuffer<Vec<u8>>,
    decoder: VoiceDecoder,
}

/// Every sender's buffer and decoder, mixed one frame at a time.
pub struct Mixer {
    talkers: HashMap<PeerId, Talker>,
    frame: Vec<f32>,
}

impl Mixer {
    /// A mixer with no senders yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            talkers: HashMap::new(),
            frame: vec![0.0; FRAME],
        }
    }

    /// Queues a packet that arrived at `now`, adding its sender on their
    /// first packet.
    ///
    /// # Errors
    /// [`CodecError`] if a new sender's decoder can't be created.
    pub fn receive(&mut self, packet: RxPacket, now: Instant) -> Result<(), CodecError> {
        let talker = match self.talkers.entry(packet.sender) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => entry.insert(Talker {
                jitter: JitterBuffer::new(),
                decoder: VoiceDecoder::new()?,
            }),
        };
        if packet.end {
            talker.jitter.insert_end(packet.seq, packet.payload, now);
        } else {
            talker.jitter.insert(packet.seq, packet.payload, now);
        }
        Ok(())
    }

    /// Forgets a sender who left.
    pub fn remove(&mut self, sender: PeerId) {
        self.talkers.remove(&sender);
    }

    /// How many senders have a decoder.
    #[must_use]
    pub fn senders(&self) -> usize {
        self.talkers.len()
    }

    /// Mixes the next 20 ms from every sender into `out`.
    pub fn tick(&mut self, now: Instant, out: &mut [f32]) {
        out.fill(0.0);
        for talker in self.talkers.values_mut() {
            if let Playout::Packet { payload, .. } = talker.jitter.pop_next(now)
                && let Ok(FRAME) = talker.decoder.decode(&payload, &mut self.frame)
            {
                mix_into(out, &self.frame);
            }
        }
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;
    use std::time::Duration;

    use super::*;
    use crate::audio::codec::VoiceEncoder;

    fn packets(frequency: f32, count: usize) -> Vec<Vec<u8>> {
        let mut encoder = VoiceEncoder::new().unwrap();
        (0..count)
            .map(|f| {
                let frame: Vec<f32> = (0..FRAME)
                    .map(|i| 0.3 * (TAU * frequency * (f * FRAME + i) as f32 / 48_000.0).sin())
                    .collect();
                encoder.encode(&frame).unwrap().to_vec()
            })
            .collect()
    }

    fn decoded(packets: &[Vec<u8>]) -> Vec<f32> {
        let mut decoder = VoiceDecoder::new().unwrap();
        let mut out = vec![0.0; FRAME];
        packets
            .iter()
            .flat_map(|packet| {
                decoder.decode(packet, &mut out).unwrap();
                out.clone()
            })
            .collect()
    }

    fn rx(sender: u64, seq: usize, payload: &[u8]) -> RxPacket {
        RxPacket {
            sender: PeerId::new(sender),
            seq: Seq::new(seq as u32),
            payload: payload.to_vec(),
            end: false,
        }
    }

    #[test]
    fn two_senders_mix_to_the_sum_of_each_decoded_alone() {
        let a = packets(440.0, 10);
        let b = packets(1_200.0, 10);
        let t0 = Instant::now();
        let mut mixer = Mixer::new();
        for (seq, (pa, pb)) in a.iter().zip(&b).enumerate() {
            mixer.receive(rx(1, seq, pa), t0).unwrap();
            mixer.receive(rx(2, seq, pb), t0).unwrap();
        }
        assert_eq!(mixer.senders(), 2);

        let mut mixed = Vec::new();
        let mut out = vec![0.0; FRAME];
        for _ in 0..10 {
            mixer.tick(t0 + Duration::from_millis(60), &mut out);
            mixed.extend_from_slice(&out);
        }
        let expected: Vec<f32> = decoded(&a)
            .iter()
            .zip(decoded(&b))
            .map(|(x, y)| x + y)
            .collect();
        assert_eq!(mixed.len(), expected.len());
        assert!(
            mixed
                .iter()
                .zip(&expected)
                .all(|(m, e)| (m - e).abs() < 1e-6)
        );
    }

    #[test]
    fn nothing_plays_before_the_jitter_delay_or_without_senders() {
        let t0 = Instant::now();
        let mut mixer = Mixer::new();
        let mut out = vec![9.0; FRAME];
        mixer.tick(t0, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        mixer.receive(rx(1, 0, &packets(440.0, 1)[0]), t0).unwrap();
        out.fill(9.0);
        mixer.tick(t0 + Duration::from_millis(10), &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_sender_who_leaves_is_dropped() {
        let t0 = Instant::now();
        let mut mixer = Mixer::new();
        mixer.receive(rx(7, 0, &packets(440.0, 1)[0]), t0).unwrap();
        mixer.remove(PeerId::new(7));
        assert_eq!(mixer.senders(), 0);
    }
}
