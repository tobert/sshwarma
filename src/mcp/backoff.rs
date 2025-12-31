//! Exponential backoff calculator for MCP connection retries.
//!
//! Provides classic exponential backoff with configurable base delay and cap.
//! Default configuration: 100ms base, 3s cap, yielding the sequence:
//! 100ms, 200ms, 400ms, 800ms, 1600ms, 3000ms, 3000ms...

use std::time::Duration;

/// Exponential backoff calculator.
///
/// # Example
///
/// ```
/// use sshwarma::mcp::Backoff;
/// use std::time::Duration;
///
/// let mut backoff = Backoff::new();
///
/// // First delay is 100ms
/// assert_eq!(backoff.next_delay(), Duration::from_millis(100));
///
/// // Second is 200ms
/// assert_eq!(backoff.next_delay(), Duration::from_millis(200));
///
/// // Reset after success
/// backoff.reset();
/// assert_eq!(backoff.attempt(), 0);
/// ```
#[derive(Debug, Clone)]
pub struct Backoff {
    /// Current attempt number (0-indexed)
    attempt: u32,
    /// Base delay (first retry)
    base: Duration,
    /// Maximum delay cap
    max: Duration,
}

impl Backoff {
    /// Default base delay: 100ms
    const DEFAULT_BASE: Duration = Duration::from_millis(100);
    /// Default maximum delay: 3 seconds
    const DEFAULT_MAX: Duration = Duration::from_secs(3);

    /// Create a new backoff calculator with default settings.
    ///
    /// Default: 100ms base, 3s cap
    pub fn new() -> Self {
        Self {
            attempt: 0,
            base: Self::DEFAULT_BASE,
            max: Self::DEFAULT_MAX,
        }
    }

    /// Create a backoff calculator with custom configuration.
    ///
    /// # Arguments
    ///
    /// * `base` - Initial delay (first retry)
    /// * `max` - Maximum delay cap
    pub fn with_config(base: Duration, max: Duration) -> Self {
        Self {
            attempt: 0,
            base,
            max,
        }
    }

    /// Calculate the next delay and increment the attempt counter.
    ///
    /// Returns the delay to wait before the next retry attempt.
    /// The delay doubles with each call until it hits the cap.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.current_delay();
        self.attempt = self.attempt.saturating_add(1);
        delay
    }

    /// Reset the backoff state after a successful connection.
    ///
    /// Resets the attempt counter to 0.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Get the current attempt number (0-indexed).
    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    /// Calculate what the current delay would be without incrementing.
    ///
    /// Useful for logging the delay before waiting.
    pub fn current_delay(&self) -> Duration {
        // delay = min(base * 2^attempt, max)
        // Use saturating operations to avoid overflow
        let multiplier = 2u64.saturating_pow(self.attempt);
        let delay_ms = self.base.as_millis() as u64;
        let computed = delay_ms.saturating_mul(multiplier);
        let capped = computed.min(self.max.as_millis() as u64);
        Duration::from_millis(capped)
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_defaults() {
        let backoff = Backoff::new();
        assert_eq!(backoff.attempt(), 0);
        assert_eq!(backoff.base, Duration::from_millis(100));
        assert_eq!(backoff.max, Duration::from_secs(3));
    }

    #[test]
    fn test_with_config() {
        let backoff = Backoff::with_config(
            Duration::from_millis(50),
            Duration::from_secs(1),
        );
        assert_eq!(backoff.base, Duration::from_millis(50));
        assert_eq!(backoff.max, Duration::from_secs(1));
    }

    #[test]
    fn test_first_delay_is_base() {
        let mut backoff = Backoff::new();
        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
    }

    #[test]
    fn test_exponential_sequence() {
        let mut backoff = Backoff::new();

        // 100ms, 200ms, 400ms, 800ms, 1600ms, 3000ms (capped), 3000ms...
        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
        assert_eq!(backoff.next_delay(), Duration::from_millis(200));
        assert_eq!(backoff.next_delay(), Duration::from_millis(400));
        assert_eq!(backoff.next_delay(), Duration::from_millis(800));
        assert_eq!(backoff.next_delay(), Duration::from_millis(1600));
        assert_eq!(backoff.next_delay(), Duration::from_millis(3000)); // capped
        assert_eq!(backoff.next_delay(), Duration::from_millis(3000)); // still capped
        assert_eq!(backoff.next_delay(), Duration::from_millis(3000)); // stays capped
    }

    #[test]
    fn test_cap_respected() {
        let mut backoff = Backoff::with_config(
            Duration::from_millis(100),
            Duration::from_millis(500),
        );

        // 100, 200, 400, 500 (capped), 500...
        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
        assert_eq!(backoff.next_delay(), Duration::from_millis(200));
        assert_eq!(backoff.next_delay(), Duration::from_millis(400));
        assert_eq!(backoff.next_delay(), Duration::from_millis(500)); // capped
        assert_eq!(backoff.next_delay(), Duration::from_millis(500)); // stays capped
    }

    #[test]
    fn test_reset() {
        let mut backoff = Backoff::new();

        backoff.next_delay();
        backoff.next_delay();
        backoff.next_delay();
        assert_eq!(backoff.attempt(), 3);

        backoff.reset();
        assert_eq!(backoff.attempt(), 0);
        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
    }

    #[test]
    fn test_current_delay_does_not_increment() {
        let backoff = Backoff::new();

        assert_eq!(backoff.current_delay(), Duration::from_millis(100));
        assert_eq!(backoff.current_delay(), Duration::from_millis(100));
        assert_eq!(backoff.attempt(), 0);
    }

    #[test]
    fn test_attempt_increments() {
        let mut backoff = Backoff::new();

        assert_eq!(backoff.attempt(), 0);
        backoff.next_delay();
        assert_eq!(backoff.attempt(), 1);
        backoff.next_delay();
        assert_eq!(backoff.attempt(), 2);
    }

    #[test]
    fn test_clone() {
        let mut backoff = Backoff::new();
        backoff.next_delay();
        backoff.next_delay();

        let cloned = backoff.clone();
        assert_eq!(cloned.attempt(), 2);
        assert_eq!(cloned.current_delay(), Duration::from_millis(400));
    }

    #[test]
    fn test_overflow_protection() {
        let mut backoff = Backoff::new();

        // Run many iterations to ensure no overflow panic
        for _ in 0..100 {
            let delay = backoff.next_delay();
            assert!(delay <= Duration::from_secs(3));
        }
    }
}
