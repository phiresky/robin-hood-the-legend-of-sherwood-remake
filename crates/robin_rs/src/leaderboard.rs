//! Leaderboard client protocol, persistence and task coordination.
pub mod history;
pub mod http;
pub mod mission_end;
pub mod preferences;
pub mod receipt_watcher;
pub mod service;
pub mod signing;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod storage;
pub mod store;
pub mod task;
#[cfg(test)]
pub(crate) mod test_fixtures;

#[cfg(test)]
mod tests {
    #[test]
    fn ranked_protocol_accepts_current_engine_versions() {
        assert_eq!(
            robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
            robin_engine::replay::REPLAY_SCHEMA_VERSION,
            "ranked submissions must accept the replay format produced by this engine"
        );
        assert_eq!(
            robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
            robin_engine::multiplayer::NET_PROTOCOL_VERSION,
            "ranked verification must accept this client's network protocol"
        );
    }
}
