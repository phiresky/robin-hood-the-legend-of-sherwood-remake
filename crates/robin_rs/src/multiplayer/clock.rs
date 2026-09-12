//! One checked Unix clock for admission and active-session scheduling.

pub(super) fn checked_epoch_ms(millis: u128) -> Result<u64, String> {
    u64::try_from(millis)
        .map_err(|_| "system clock timestamp exceeds the u64 Unix range".to_owned())
}

pub(super) fn epoch_ms_at(now: web_time::SystemTime) -> Result<u64, String> {
    let duration = now
        .duration_since(web_time::UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes the Unix epoch: {error}"))?;
    checked_epoch_ms(duration.as_millis())
}

pub(super) fn try_current_epoch_ms() -> Result<u64, String> {
    epoch_ms_at(web_time::SystemTime::now())
}

/// Active drivers cannot continue with an invalid scheduling clock. Setup
/// callers use the fallible function and reject admission before gameplay.
pub fn current_epoch_ms() -> u64 {
    try_current_epoch_ms().expect("multiplayer requires a valid Unix system clock")
}
