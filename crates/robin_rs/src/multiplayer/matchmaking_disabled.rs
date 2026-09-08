//! Clock support shared by local/replay sessions when online multiplayer is
//! not compiled into the client.

/// Current Unix epoch in milliseconds.
pub fn current_epoch_ms() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .expect("system clock is before the Unix epoch")
}
