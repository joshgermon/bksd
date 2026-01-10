//! Logging and tracing initialization for BKSD.
//!
//! This module provides structured logging using the `tracing` ecosystem.
//! It supports both pretty console output and JSON output for machine parsing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tracing::Level;
use tracing_subscriber::{
    EnvFilter,
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

/// Configuration for the logging system.
pub struct LogConfig {
    /// Output logs as JSON (for machine parsing)
    pub json: bool,
    /// Enable verbose logging (sets default level to DEBUG)
    pub verbose: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            json: false,
            verbose: false,
        }
    }
}

/// Initialize the tracing subscriber with the given configuration.
///
/// This should be called early in main(), after config is loaded.
/// The log level can be overridden at runtime via the `RUST_LOG` environment variable.
///
/// # Examples
///
/// ```ignore
/// // Basic initialization with defaults
/// bksd::logging::init(LogConfig::default());
///
/// // Verbose mode
/// bksd::logging::init(LogConfig { verbose: true, ..Default::default() });
///
/// // JSON output for log aggregation
/// bksd::logging::init(LogConfig { json: true, ..Default::default() });
/// ```
pub fn init(config: LogConfig) {
    // Determine default log level based on verbose flag
    let default_level = if config.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!("bksd={}", default_level.as_str().to_lowercase()))
    });

    if config.json {
        // JSON output for structured logging / log aggregation
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt::layer()
                    .json()
                    .with_span_events(FmtSpan::CLOSE)
                    .with_current_span(true)
                    .with_target(true),
            )
            .init();
    } else {
        // Pretty console output for human readability
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt::layer()
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_file(false)
                    .with_line_number(false),
            )
            .init();
    }
}

/// A rate limiter for throttling log messages.
///
/// Useful for progress updates that would otherwise spam the logs.
///
/// # Example
///
/// ```ignore
/// let throttle = LogThrottle::new(Duration::from_millis(500));
///
/// loop {
///     if throttle.should_log() {
///         tracing::debug!(progress = %progress, "Transfer progress");
///     }
/// }
/// ```
pub struct LogThrottle {
    interval_ms: u64,
    /// Stores the last log time in ms, or u64::MAX to indicate "never logged"
    last_log_ms: AtomicU64,
    start: Instant,
}

/// Sentinel value indicating the throttle has never logged
const NEVER_LOGGED: u64 = u64::MAX;

impl LogThrottle {
    /// Create a new throttle with the given minimum interval between logs.
    pub fn new(interval: std::time::Duration) -> Self {
        Self {
            interval_ms: interval.as_millis() as u64,
            last_log_ms: AtomicU64::new(NEVER_LOGGED),
            start: Instant::now(),
        }
    }

    /// Returns true if enough time has passed since the last log.
    ///
    /// This is thread-safe and uses atomic operations.
    pub fn should_log(&self) -> bool {
        let now_ms = self.start.elapsed().as_millis() as u64;
        let last = self.last_log_ms.load(Ordering::Relaxed);

        // First call (never logged) or enough time has passed
        let should = last == NEVER_LOGGED || now_ms.saturating_sub(last) >= self.interval_ms;

        if should {
            // Try to update; if we lose the race, another thread logged
            self.last_log_ms
                .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        } else {
            false
        }
    }

    /// Reset the throttle, allowing the next log immediately.
    pub fn reset(&self) {
        self.last_log_ms.store(NEVER_LOGGED, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn throttle_allows_first_log() {
        let throttle = LogThrottle::new(Duration::from_secs(1));
        assert!(throttle.should_log());
    }

    #[test]
    fn throttle_blocks_immediate_second_log() {
        let throttle = LogThrottle::new(Duration::from_secs(1));
        assert!(throttle.should_log());
        assert!(!throttle.should_log());
    }

    #[test]
    fn throttle_reset_allows_log() {
        let throttle = LogThrottle::new(Duration::from_secs(100));
        assert!(throttle.should_log());
        assert!(!throttle.should_log());
        throttle.reset();
        assert!(throttle.should_log());
    }
}
