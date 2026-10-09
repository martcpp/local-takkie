//! Opus with the settings from ROADMAP 3.4: mono, 48 kHz, 20 ms frames,
//! VoIP mode, 24 kbit/s, in-band FEC for 10% expected loss.

use opus::{Application, Bitrate, Channels, Decoder, Encoder};
use thiserror::Error;

/// Opus's largest packet.
const MAX_PACKET: usize = 1275;

/// An Opus failure.
#[derive(Debug, Error)]
#[error("opus: {0}")]
pub struct CodecError(#[from] opus::Error);

/// Encodes 48 kHz mono frames into Opus packets.
pub struct VoiceEncoder {
    inner: Encoder,
    packet: [u8; MAX_PACKET],
}

impl VoiceEncoder {
    /// An encoder with the voice settings.
    ///
    /// # Errors
    /// [`CodecError`] if libopus rejects a setting.
    pub fn new() -> Result<Self, CodecError> {
        let mut inner = Encoder::new(48_000, Channels::Mono, Application::Voip)?;
        inner.set_bitrate(Bitrate::Bits(24_000))?;
        inner.set_inband_fec(true)?;
        inner.set_packet_loss_perc(10)?;
        Ok(Self {
            inner,
            packet: [0; MAX_PACKET],
        })
    }

    /// Encodes one 960-sample frame; the packet is valid until the next call.
    ///
    /// # Errors
    /// [`CodecError`] if the frame has the wrong size or encoding fails.
    pub fn encode(&mut self, frame: &[f32]) -> Result<&[u8], CodecError> {
        let len = self.inner.encode_float(frame, &mut self.packet)?;
        Ok(&self.packet[..len])
    }
}

/// Decodes one sender's Opus packets into 48 kHz mono frames.
pub struct VoiceDecoder {
    inner: Decoder,
}

impl VoiceDecoder {
    /// A decoder for one sender.
    ///
    /// # Errors
    /// [`CodecError`] if libopus can't create it.
    pub fn new() -> Result<Self, CodecError> {
        Ok(Self {
            inner: Decoder::new(48_000, Channels::Mono)?,
        })
    }

    /// Decodes `packet` into `out` and returns the samples written.
    ///
    /// # Errors
    /// [`CodecError`] if the packet is corrupt or `out` is too small.
    pub fn decode(&mut self, packet: &[u8], out: &mut [f32]) -> Result<usize, CodecError> {
        Ok(self.inner.decode_float(packet, out, false)?)
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;
    use crate::audio::resample::FRAME;

    #[test]
    fn a_second_of_speech_like_tone_averages_about_60_bytes() {
        let mut encoder = VoiceEncoder::new().unwrap();
        let mut total = 0;
        for f in 0..50 {
            let frame: Vec<f32> = (0..FRAME)
                .map(|i| {
                    let t = (f * FRAME + i) as f32 / 48_000.0;
                    0.3 * (TAU * 220.0 * t).sin() + 0.1 * (TAU * 1_800.0 * t).sin()
                })
                .collect();
            total += encoder.encode(&frame).unwrap().len();
        }
        let average = total / 50;
        assert!((40..=80).contains(&average), "{average} bytes per packet");
    }

    #[test]
    fn decoding_gives_back_a_whole_frame() {
        let mut encoder = VoiceEncoder::new().unwrap();
        let mut decoder = VoiceDecoder::new().unwrap();
        let mut out = [0.0; FRAME];
        let packet = encoder.encode(&[0.1; FRAME]).unwrap().to_vec();
        assert_eq!(decoder.decode(&packet, &mut out).unwrap(), FRAME);
        assert!(decoder.decode(&[0xFF; 3], &mut out).is_err());
    }

    #[test]
    fn a_wrong_frame_size_is_an_error_not_a_panic() {
        let mut encoder = VoiceEncoder::new().unwrap();
        assert!(encoder.encode(&[0.0; 100]).is_err());
    }
}
