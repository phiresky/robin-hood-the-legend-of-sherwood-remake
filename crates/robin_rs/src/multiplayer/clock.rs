//! One checked Unix clock for admission and active-session scheduling.

/// Why the Unix system clock cannot schedule multiplayer.
///
/// Not serde: carries the platform clock error as its source.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ClockError {
    #[error("system clock timestamp exceeds the u64 Unix range")]
    OutOfRange,
    #[error("system clock precedes the Unix epoch: {0}")]
    BeforeEpoch(#[source] web_time::SystemTimeError),
}

pub(super) fn checked_epoch_ms(millis: u128) -> Result<u64, ClockError> {
    u64::try_from(millis).map_err(|_| ClockError::OutOfRange)
}

pub(super) fn epoch_ms_at(now: web_time::SystemTime) -> Result<u64, ClockError> {
    let duration = now
        .duration_since(web_time::UNIX_EPOCH)
        .map_err(ClockError::BeforeEpoch)?;
    checked_epoch_ms(duration.as_millis())
}

pub(super) fn try_current_epoch_ms() -> Result<u64, ClockError> {
    epoch_ms_at(web_time::SystemTime::now())
}

/// Active drivers cannot continue with an invalid scheduling clock. Setup
/// callers use the fallible function and reject admission before gameplay.
pub fn current_epoch_ms() -> u64 {
    try_current_epoch_ms().expect("multiplayer requires a valid Unix system clock")
}
