//! Noticing a talker who is in the same room: their voice reaches our
//! microphone through the air before the radio plays it.
//!
//! Sound through the air is instant; the radio is at least a jitter buffer
//! late. Our own speaker can only be heard after it plays. So if the
//! microphone heard a talker's voice before we decoded it, they are in
//! earshot, and playing them as well only makes an echo.
//!
//! The delay is found with GCC-PHAT, the usual way of timing a sound between
//! two microphones: cross-correlate the two signals with every frequency
//! weighted the same, so each path shows as its own sharp peak.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// The rate everything here runs at: enough for speech, cheap to compare.
pub const RATE: usize = 8_000;
/// Samples at 48 kHz per sample here.
pub const DOWN: usize = 6;
/// How much of the talker is compared: 1.5 s.
const SPAN: usize = RATE * 3 / 2;
// How much earlier the air can bring a voice than the radio: 40 to 800 ms.
const SOONEST: usize = RATE / 25;
const LATEST: usize = RATE * 4 / 5;
// Long enough that the correlation never wraps round.
const FFT: usize = 32_768;
const _: () = assert!(FFT >= 2 * SPAN + LATEST);
// Speech band; outside it the weighting only amplifies noise.
const LOWEST_HZ: usize = 250;
const HIGHEST_HZ: usize = 3_600;
/// A peak this many times the typical correlation counts as a path.
pub const CLEAR: f32 = 7.0;
/// How much microphone sound is kept: 4 s.
pub const KEPT: usize = RATE * 4;

/// The microphone's last few seconds at [`RATE`], written by the send thread
/// and read by the mixer.
///
/// Relaxed is enough: a reader that sees a sample a frame late only shifts
/// the comparison by 20 ms, which the delay search absorbs.
#[derive(Debug)]
pub struct MicTap {
    epoch: Instant,
    samples: Box<[AtomicU32]>,
    written: AtomicU64,
    // When the newest sample was heard, in microseconds since `epoch`.
    newest_us: AtomicU64,
}

impl Default for MicTap {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            samples: (0..KEPT).map(|_| AtomicU32::new(0)).collect(),
            written: AtomicU64::new(0),
            newest_us: AtomicU64::new(0),
        }
    }
}

impl MicTap {
    /// Adds a 48 kHz frame heard at `at`.
    pub fn push(&self, frame: &[f32], at: Instant) {
        let mut written = self.written.load(Relaxed);
        for chunk in frame.as_chunks::<DOWN>().0 {
            let average = chunk.iter().sum::<f32>() / DOWN as f32;
            self.samples[(written % KEPT as u64) as usize].store(average.to_bits(), Relaxed);
            written += 1;
        }
        self.written.store(written, Relaxed);
        self.newest_us.store(self.micros(at), Relaxed);
    }

    fn micros(&self, at: Instant) -> u64 {
        u64::try_from(at.saturating_duration_since(self.epoch).as_micros()).unwrap_or(u64::MAX)
    }

    /// Copies the samples heard up to `end` into `out`, oldest first.
    /// False if they aren't all still kept, or `end` hasn't been heard yet.
    pub fn copy_until(&self, end: Instant, out: &mut [f32]) -> bool {
        let written = self.written.load(Relaxed);
        let newest = self.newest_us.load(Relaxed);
        let end = self.micros(end);
        let Some(behind) = newest.checked_sub(end) else {
            return false;
        };
        let behind = behind * RATE as u64 / 1_000_000;
        let Some(last) = written.checked_sub(behind) else {
            return false;
        };
        let Some(first) = last.checked_sub(out.len() as u64) else {
            return false;
        };
        if written - first > KEPT as u64 {
            return false;
        }
        for (i, slot) in out.iter_mut().enumerate() {
            let index = ((first + i as u64) % KEPT as u64) as usize;
            *slot = f32::from_bits(self.samples[index].load(Relaxed));
        }
        true
    }
}

/// Builds a shared tap.
#[must_use]
pub fn mic_tap() -> Arc<MicTap> {
    Arc::new(MicTap::default())
}

/// One talker's decoded voice at [`RATE`], newest last.
#[derive(Clone, Debug)]
pub struct Heard {
    samples: Vec<f32>,
    written: usize,
    newest: Option<Instant>,
}

impl Default for Heard {
    fn default() -> Self {
        Self {
            samples: vec![0.0; SPAN],
            written: 0,
            newest: None,
        }
    }
}

impl Heard {
    /// Adds a 48 kHz frame decoded at `at`.
    pub fn push(&mut self, frame: &[f32], at: Instant) {
        for chunk in frame.as_chunks::<DOWN>().0 {
            self.samples[self.written % SPAN] = chunk.iter().sum::<f32>() / DOWN as f32;
            self.written += 1;
        }
        self.newest = Some(at);
    }

    fn copy(&self, out: &mut [f32]) {
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.samples[(self.written + i) % SPAN];
        }
    }

    fn loud_enough(&self) -> bool {
        let power = self.samples.iter().map(|s| s * s).sum::<f32>() / SPAN as f32;
        self.written >= SPAN && power > 1e-5
    }
}

/// How the talker's voice reached the microphone ahead of the radio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lead {
    /// How much earlier, in milliseconds.
    pub ms: f32,
    /// How clear the peak is: its height over the typical correlation.
    pub clarity: f32,
}

/// Finds whether, and how much earlier, a talker reached the microphone.
/// Holds its buffers so a comparison allocates nothing.
pub struct Earshot {
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    talker: Vec<f32>,
    mic: Vec<f32>,
    talker_spectrum: Vec<Complex<f32>>,
    mic_spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    correlation: Vec<f32>,
    span: Vec<f32>,
    window: Vec<f32>,
}

impl std::fmt::Debug for Earshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Earshot").finish_non_exhaustive()
    }
}

impl Default for Earshot {
    fn default() -> Self {
        let mut planner = RealFftPlanner::new();
        let forward = planner.plan_fft_forward(FFT);
        let inverse = planner.plan_fft_inverse(FFT);
        let scratch_len = forward.get_scratch_len().max(inverse.get_scratch_len());
        Self {
            talker: forward.make_input_vec(),
            mic: forward.make_input_vec(),
            talker_spectrum: forward.make_output_vec(),
            mic_spectrum: forward.make_output_vec(),
            scratch: vec![Complex::default(); scratch_len],
            correlation: inverse.make_output_vec(),
            span: vec![0.0; SPAN],
            window: vec![0.0; SPAN + LATEST],
            forward,
            inverse,
        }
    }
}

impl Earshot {
    /// Compares the talker's last 1.5 s with what the microphone heard over
    /// the same time and up to 0.8 s before. `None` when there is nothing to
    /// compare: the talker was quiet, or the microphone isn't far enough on.
    pub fn compare(&mut self, heard: &Heard, mic: &MicTap) -> Option<Lead> {
        let end = heard.newest?;
        if !heard.loud_enough() {
            return None;
        }
        heard.copy(&mut self.span);
        if !mic.copy_until(end, &mut self.window) {
            return None;
        }
        self.find(SPAN + LATEST)
    }

    // Lag k in the correlation lines the talker up with the window starting
    // k samples in, so the air brought it LATEST - k samples earlier.
    fn find(&mut self, window_len: usize) -> Option<Lead> {
        self.talker.fill(0.0);
        self.mic.fill(0.0);
        self.talker[..SPAN].copy_from_slice(&self.span);
        self.mic[..window_len].copy_from_slice(&self.window[..window_len]);
        self.forward
            .process_with_scratch(
                &mut self.talker,
                &mut self.talker_spectrum,
                &mut self.scratch,
            )
            .ok()?;
        self.forward
            .process_with_scratch(&mut self.mic, &mut self.mic_spectrum, &mut self.scratch)
            .ok()?;
        let bin_hz = RATE as f32 / FFT as f32;
        for (bin, (m, t)) in self
            .mic_spectrum
            .iter_mut()
            .zip(&self.talker_spectrum)
            .enumerate()
        {
            let hz = bin as f32 * bin_hz;
            let cross = *m * t.conj();
            let size = cross.norm();
            *m = if (LOWEST_HZ as f32..=HIGHEST_HZ as f32).contains(&hz) && size > 1e-12 {
                cross / size
            } else {
                Complex::default()
            };
        }
        self.inverse
            .process_with_scratch(
                &mut self.mic_spectrum,
                &mut self.correlation,
                &mut self.scratch,
            )
            .ok()?;
        let lags = &self.correlation[..=LATEST];
        let typical = (lags.iter().map(|c| c * c).sum::<f32>() / lags.len() as f32).sqrt();
        if typical <= 0.0 {
            return None;
        }
        let (k, peak) = lags[..=LATEST - SOONEST]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))?;
        Some(Lead {
            ms: (LATEST - k) as f32 * 1_000.0 / RATE as f32,
            clarity: peak / typical,
        })
    }
}

/// Whether a talker is in the room, from one comparison a second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Nearby {
    here: bool,
    last: Option<f32>,
    matches: u8,
    misses: u8,
}

// Matches at the same delay before a talker counts as in the room, and
// comparisons without one before they count as gone again.
const TO_ARRIVE: u8 = 2;
const TO_LEAVE: u8 = 5;
const SAME_DELAY_MS: f32 = 40.0;

impl Nearby {
    /// Takes one comparison and says whether the talker is in the room.
    /// `None`, nothing to judge by, changes nothing.
    pub fn judge(&mut self, lead: Option<Lead>) -> bool {
        let Some(lead) = lead else {
            return self.here;
        };
        if lead.clarity >= CLEAR {
            let steady = self
                .last
                .is_some_and(|last| (last - lead.ms).abs() <= SAME_DELAY_MS);
            self.matches = if steady {
                self.matches.saturating_add(1)
            } else {
                1
            };
            self.last = Some(lead.ms);
            self.misses = 0;
            if self.matches >= TO_ARRIVE {
                self.here = true;
            }
        } else {
            self.matches = 0;
            self.last = None;
            self.misses = self.misses.saturating_add(1);
            if self.misses >= TO_LEAVE {
                self.here = false;
            }
        }
        self.here
    }

    /// Whether the talker is in the room.
    #[must_use]
    pub fn here(&self) -> bool {
        self.here
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::audio::codec::{VoiceDecoder, VoiceEncoder};
    use crate::audio::resample::FRAME;

    // Noisy bursts with gaps, a little like syllables, at 48 kHz.
    fn speech(seconds: f32, seed: u32) -> Vec<f32> {
        let mut state = seed;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let len = (48_000.0 * seconds) as usize;
        let mut out = Vec::with_capacity(len);
        let mut low = 0.0;
        while out.len() < len {
            let burst = 4_000 + (next() % 9_000) as usize;
            let gap = 1_500 + (next() % 6_000) as usize;
            let size = 0.05 + (next() % 1_000) as f32 / 1_000.0 * 0.2;
            for i in 0..burst {
                let white = (next() as f32 / u32::MAX as f32 - 0.5) * 2.0;
                low = 0.9 * low + 0.1 * white;
                let shape = (std::f32::consts::PI * i as f32 / burst as f32).sin();
                out.push(size * shape * (low * 3.0 + 0.3 * white));
            }
            out.extend(std::iter::repeat_n(0.0, gap));
        }
        out.truncate(len);
        out
    }

    fn through_opus(voice: &[f32]) -> Vec<f32> {
        let mut encoder = VoiceEncoder::new().unwrap();
        let mut decoder = VoiceDecoder::new().unwrap();
        let mut out = [0.0; FRAME];
        voice
            .as_chunks::<FRAME>()
            .0
            .iter()
            .flat_map(|frame| {
                let packet = encoder.encode(frame).unwrap().to_vec();
                let n = decoder.decode(&packet, &mut out).unwrap();
                out[..n].to_vec()
            })
            .collect()
    }

    fn shifted(sound: &[f32], by: isize) -> Vec<f32> {
        (0..sound.len() as isize)
            .map(|i| {
                usize::try_from(i - by)
                    .ok()
                    .and_then(|j| sound.get(j))
                    .copied()
                    .unwrap_or(0.0)
            })
            .collect()
    }

    // Feeds 4 s: the talker as decoded here and the microphone, frame by
    // frame on the same clock, then compares.
    fn compare(decoded: &[f32], mic: &[f32]) -> Option<Lead> {
        let tap = MicTap::default();
        let mut heard = Heard::default();
        let start = Instant::now();
        let mut at = start;
        for (talker, heard_here) in decoded
            .as_chunks::<FRAME>()
            .0
            .iter()
            .zip(mic.as_chunks::<FRAME>().0)
        {
            at += Duration::from_millis(20);
            tap.push(heard_here, at);
            heard.push(talker, at);
        }
        let mut earshot = Earshot::default();
        earshot.compare(&heard, &tap)
    }

    fn noise(len: usize, size: f32) -> Vec<f32> {
        let mut state = 0xDEAD_BEEF_u32;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state as f32 / u32::MAX as f32 - 0.5) * size
            })
            .collect()
    }

    #[test]
    fn a_talker_heard_through_the_air_first_is_found_at_the_right_delay() {
        let voice = speech(4.0, 7);
        let decoded = through_opus(&voice);
        for lead_ms in [80_isize, 150, 300, 600] {
            // The decoded voice is the air's sound `lead_ms` later.
            let late = shifted(&decoded, lead_ms * 48);
            let mic: Vec<f32> = voice
                .iter()
                .zip(noise(voice.len(), 0.01))
                .map(|(v, n)| v * 0.3 + n)
                .collect();
            let lead = compare(&late, &mic).unwrap();
            assert!(lead.clarity >= CLEAR, "{lead_ms} ms: {lead:?}");
            assert!(
                (lead.ms - lead_ms as f32).abs() < 25.0,
                "{lead_ms} ms: {lead:?}"
            );
        }
    }

    #[test]
    fn our_own_loud_speaker_on_top_still_lets_the_air_through() {
        let voice = speech(4.0, 7);
        let decoded = shifted(&through_opus(&voice), 150 * 48);
        let speaker = shifted(&decoded, 60 * 48);
        let mic: Vec<f32> = voice
            .iter()
            .zip(&speaker)
            .zip(noise(voice.len(), 0.01))
            .map(|((v, s), n)| v * 0.25 + s * 1.0 + n)
            .collect();
        let lead = compare(&decoded, &mic).unwrap();
        assert!(lead.clarity >= CLEAR, "{lead:?}");
        assert!((lead.ms - 150.0).abs() < 25.0, "{lead:?}");
    }

    #[test]
    fn our_own_speaker_alone_is_not_someone_in_the_room() {
        let decoded = through_opus(&speech(4.0, 7));
        for behind_ms in [20_isize, 60, 120] {
            let speaker = shifted(&decoded, behind_ms * 48);
            let mic: Vec<f32> = speaker
                .iter()
                .zip(noise(decoded.len(), 0.01))
                .map(|(s, n)| s + n)
                .collect();
            let lead = compare(&decoded, &mic).unwrap();
            assert!(lead.clarity < CLEAR, "{behind_ms} ms behind: {lead:?}");
        }
    }

    #[test]
    fn other_voices_in_our_room_are_not_the_talker() {
        let decoded = through_opus(&speech(4.0, 7));
        for seed in [3, 99, 1_234, 777] {
            let other = speech(4.0, seed);
            let lead = compare(&decoded, &other).unwrap();
            assert!(lead.clarity < CLEAR, "seed {seed}: {lead:?}");
        }
    }

    #[test]
    fn a_quiet_talker_or_a_microphone_not_far_enough_on_gives_no_answer() {
        let quiet = vec![0.0; 48_000 * 4];
        assert_eq!(compare(&quiet, &speech(4.0, 7)), None);
        let mut heard = Heard::default();
        let at = Instant::now();
        for frame in speech(2.0, 7).as_chunks::<FRAME>().0 {
            heard.push(frame, at);
        }
        assert_eq!(Earshot::default().compare(&heard, &MicTap::default()), None);
    }

    #[test]
    fn it_takes_two_steady_matches_to_arrive_and_five_misses_to_leave() {
        let clear = |ms| Some(Lead { ms, clarity: 20.0 });
        let unclear = Some(Lead {
            ms: 100.0,
            clarity: 2.0,
        });
        let mut nearby = Nearby::default();
        assert!(!nearby.judge(clear(150.0)));
        assert!(!nearby.judge(clear(400.0)), "a different delay starts over");
        assert!(nearby.judge(clear(410.0)));
        assert!(nearby.judge(None));
        for _ in 0..4 {
            assert!(nearby.judge(unclear));
        }
        assert!(!nearby.judge(unclear));
    }
}
