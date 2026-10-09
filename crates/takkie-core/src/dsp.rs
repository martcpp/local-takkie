//! Small audio helpers for the engine's real-time paths. Samples are `f32` in
//! [-1, 1], multi-channel buffers are interleaved, and the caller owns every
//! buffer, so nothing here allocates.

/// Averages interleaved frames into mono and returns how many samples it
/// wrote. A trailing partial frame is ignored.
pub fn downmix_to_mono(input: &[f32], channels: usize, out: &mut [f32]) -> usize {
    if channels == 0 {
        return 0;
    }
    let scale = 1.0 / channels as f32;
    let mut written = 0;
    for (frame, sample) in input.chunks_exact(channels).zip(out.iter_mut()) {
        *sample = frame.iter().sum::<f32>() * scale;
        written += 1;
    }
    written
}

/// Copies each mono sample to every channel and returns how many frames it
/// wrote.
pub fn upmix_from_mono(input: &[f32], channels: usize, out: &mut [f32]) -> usize {
    if channels == 0 {
        return 0;
    }
    let mut written = 0;
    for (&sample, frame) in input.iter().zip(out.chunks_exact_mut(channels)) {
        frame.fill(sample);
        written += 1;
    }
    written
}

/// Multiplies every sample by `gain`.
pub fn apply_gain(samples: &mut [f32], gain: f32) {
    samples.iter_mut().for_each(|sample| *sample *= gain);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_downmix_is_a_copy() {
        let mut out = [0.0; 3];
        assert_eq!(downmix_to_mono(&[0.1, -0.2, 0.3], 1, &mut out), 3);
        assert_eq!(out, [0.1, -0.2, 0.3]);
    }

    #[test]
    fn stereo_downmix_averages_left_and_right() {
        let mut out = [0.0; 2];
        assert_eq!(downmix_to_mono(&[1.0, 0.0, 0.5, -0.5], 2, &mut out), 2);
        assert_eq!(out, [0.5, 0.0]);
    }

    #[test]
    fn four_and_six_channel_downmix() {
        let mut out = [0.0; 1];
        assert_eq!(downmix_to_mono(&[0.4, 0.4, 0.0, 0.0], 4, &mut out), 1);
        assert_eq!(out, [0.2]);
        assert_eq!(downmix_to_mono(&[0.6; 6], 6, &mut out), 1);
        assert!((out[0] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn downmix_stops_at_the_shorter_buffer() {
        let mut out = [9.0; 4];
        assert_eq!(downmix_to_mono(&[1.0, 1.0, 1.0], 2, &mut out), 1);
        assert_eq!(out, [1.0, 9.0, 9.0, 9.0]);

        let mut short = [0.0; 1];
        assert_eq!(downmix_to_mono(&[0.2; 8], 2, &mut short), 1);
    }

    #[test]
    fn upmix_copies_to_every_channel() {
        let mut stereo = [0.0; 4];
        assert_eq!(upmix_from_mono(&[0.1, -0.3], 2, &mut stereo), 2);
        assert_eq!(stereo, [0.1, 0.1, -0.3, -0.3]);

        let mut quad = [0.0; 4];
        assert_eq!(upmix_from_mono(&[0.5], 4, &mut quad), 1);
        assert_eq!(quad, [0.5; 4]);

        let mut six = [0.0; 12];
        assert_eq!(upmix_from_mono(&[0.2, 0.4], 6, &mut six), 2);
        assert_eq!(six[..6], [0.2; 6]);
        assert_eq!(six[6..], [0.4; 6]);
    }

    #[test]
    fn upmix_stops_at_the_shorter_buffer() {
        let mut out = [9.0; 5];
        assert_eq!(upmix_from_mono(&[0.1, 0.2, 0.3], 2, &mut out), 2);
        assert_eq!(out, [0.1, 0.1, 0.2, 0.2, 9.0]);
    }

    #[test]
    fn zero_channels_writes_nothing() {
        let mut out = [9.0; 2];
        assert_eq!(downmix_to_mono(&[1.0], 0, &mut out), 0);
        assert_eq!(upmix_from_mono(&[1.0], 0, &mut out), 0);
        assert_eq!(out, [9.0; 2]);
    }

    #[test]
    fn gain_scales_every_sample() {
        let mut samples = [0.5, -0.25, 0.0];
        apply_gain(&mut samples, 0.5);
        assert_eq!(samples, [0.25, -0.125, 0.0]);
        apply_gain(&mut samples, 0.0);
        assert_eq!(samples, [0.0, -0.0, 0.0]);
    }
}
