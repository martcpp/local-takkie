//! The hardware boundary: the pipeline only sees these traits, so tests can
//! run it with fakes and no sound card.

/// Mono samples from a microphone.
pub trait AudioSource: Send {
    /// Samples per second.
    fn sample_rate(&self) -> u32;

    /// Copies waiting samples into `out` and returns how many. Never blocks.
    fn read(&mut self, out: &mut [f32]) -> usize;

    /// Says the device has stopped giving samples, so whoever opened it
    /// opens it again.
    fn give_up(&mut self) {}
}

/// Mono samples for a speaker.
pub trait AudioSink: Send {
    /// Samples per second.
    fn sample_rate(&self) -> u32;

    /// Room left, in samples.
    fn free(&self) -> usize;

    /// Samples waiting to play.
    fn queued(&self) -> usize;

    /// Queues as many samples as fit and returns how many. Never blocks.
    fn write(&mut self, samples: &[f32]) -> usize;

    /// Starts or pauses playback. Queued samples stay queued while paused.
    /// A sink that can't pause ignores this.
    fn set_playing(&mut self, _playing: bool) {}

    /// Says the device has stopped taking samples, so whoever opened it
    /// opens it again.
    fn give_up(&mut self) {}

    /// The most samples the device has asked for in one go. Some phones
    /// take 80 ms at a time, and that much has to be waiting for them.
    fn largest_request(&self) -> usize {
        0
    }
}
