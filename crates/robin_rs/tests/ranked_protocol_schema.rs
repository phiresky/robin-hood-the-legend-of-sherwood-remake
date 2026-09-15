#[test]
fn ranked_protocol_tracks_exact_runtime_schemas() {
    assert_eq!(
        robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        robin_engine::replay::REPLAY_SCHEMA_VERSION,
        "the ranking service must reject every replay schema except the exact runtime schema",
    );
    assert_eq!(
        robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
        robin_engine::multiplayer::NET_PROTOCOL_VERSION,
        "the verifier must reject replays recorded under another network protocol",
    );
}
