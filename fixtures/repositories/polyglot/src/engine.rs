//! Retry engine used by the polyglot fixture.

use std::time::Duration;

/// Maximum attempts before the engine gives up.
pub const MAX_ATTEMPTS: usize = 5;

/// Describes how a retry backs off.
pub trait Backoff {
    /// Returns the delay for a given attempt.
    fn delay(&self, attempt: usize) -> Duration;
}

/// A fixed-interval backoff.
pub struct FixedBackoff {
    interval: Duration,
}

/// Retry outcomes reported to callers.
pub enum Outcome {
    Completed,
    Exhausted,
}

impl FixedBackoff {
    /// Builds a backoff with the given interval.
    pub fn new(interval: Duration) -> Self {
        Self { interval }
    }
}

impl Backoff for FixedBackoff {
    fn delay(&self, _attempt: usize) -> Duration {
        self.interval
    }
}

/// Runs an operation until it succeeds or attempts are exhausted.
pub fn run_with_retries(backoff: &dyn Backoff, attempts: usize) -> Outcome {
    for attempt in 0..attempts.min(MAX_ATTEMPTS) {
        let _delay = backoff.delay(attempt);
    }
    Outcome::Exhausted
}

fn unexported_helper() -> usize {
    MAX_ATTEMPTS
}
