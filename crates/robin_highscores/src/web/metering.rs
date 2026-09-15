//! In-process request metering: sliding-window rate limits keyed by the
//! effective client address or by the verified signing key.
//!
//! The API is a single process, so the counters live in memory and reset on
//! restart. Durable per-uploader concurrency is enforced by upload
//! reservations in SQLite instead.

use crate::error::ApiError;
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

/// Longest window any scope uses; entries idle for longer are prunable.
pub(super) const LONGEST_RATE_WINDOW: Duration = Duration::from_secs(60 * 60);
const PRUNE_ABOVE_ENTRIES: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum RateLimitScope {
    Submission,
    UsernameUpdate,
    Deletion,
    OwnerStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum RateLimitSubject {
    Address(IpAddr),
    PublicKey([u8; 32]),
}

#[derive(Clone, Default)]
pub struct RateLimiter {
    attempts:
        Arc<tokio::sync::Mutex<HashMap<(RateLimitScope, RateLimitSubject), VecDeque<Instant>>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one attempt, or reject it when `maximum` attempts already
    /// happened within `window`. Rejected attempts are not recorded.
    pub(super) async fn check(
        &self,
        scope: RateLimitScope,
        subject: RateLimitSubject,
        maximum: u32,
        window: Duration,
    ) -> Result<(), ApiError> {
        debug_assert!(window <= LONGEST_RATE_WINDOW);
        let now = Instant::now();
        let cutoff = now.checked_sub(window).unwrap_or(now);
        let mut attempts = self.attempts.lock().await;
        if attempts.len() > PRUNE_ABOVE_ENTRIES {
            let idle_cutoff = now.checked_sub(LONGEST_RATE_WINDOW).unwrap_or(now);
            attempts.retain(|_, values| values.back().is_some_and(|last| *last >= idle_cutoff));
        }
        let values = attempts.entry((scope, subject)).or_default();
        while values.front().is_some_and(|instant| *instant < cutoff) {
            values.pop_front();
        }
        let maximum = usize::try_from(maximum).map_err(|_| ApiError::Internal)?;
        if values.len() >= maximum {
            let retry_after = values.front().map_or(window, |first| {
                (*first + window).saturating_duration_since(now)
            });
            return Err(ApiError::RateLimited {
                retry_after_ms: u64::try_from(retry_after.as_millis())
                    .map_err(|_| ApiError::Internal)?
                    .max(1),
            });
        }
        values.push_back(now);
        Ok(())
    }
}
