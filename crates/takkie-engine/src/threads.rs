//! Joining engine threads without letting one stuck thread hang shutdown.

use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long a stopped thread gets to finish.
pub(crate) const STOP_WITHIN: Duration = Duration::from_secs(2);

/// Joins `handle` if it finishes within `timeout`; otherwise logs it and
/// lets it go. Returns whether it was joined.
pub(crate) fn join_within(handle: JoinHandle<()>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            let name = handle.thread().name().unwrap_or("unnamed").to_owned();
            tracing::warn!(thread = %name, "thread didn't stop in time, leaving it");
            return false;
        }
        thread::sleep(Duration::from_millis(2));
    }
    let _ = handle.join();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_thread_is_joined() {
        let handle = thread::spawn(|| {});
        assert!(join_within(handle, Duration::from_secs(1)));
    }

    #[test]
    fn a_stuck_thread_is_let_go_after_the_timeout() {
        let handle = thread::spawn(|| thread::sleep(Duration::from_millis(500)));
        let start = Instant::now();
        assert!(!join_within(handle, Duration::from_millis(50)));
        assert!(start.elapsed() < Duration::from_millis(400));
    }
}
