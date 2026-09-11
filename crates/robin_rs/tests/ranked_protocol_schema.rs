#[test]
fn ranked_protocol_tracks_exact_runtime_schemas() {
    assert_eq!(
        robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
        robin_engine::replay::REPLAY_SCHEMA_VERSION,
        "the ranking service must reject every replay schema except the exact runtime schema",
    );
    assert_eq!(
        robin_run_protocol::CURRENT_RANKED_SAVE_SCHEMA_VERSION_V1,
        robin_rs::save_file::SAVE_FORMAT_VERSION,
        "ranked campaign admission must track the exact runtime save schema",
    );
    assert_eq!(
        robin_run_protocol::CURRENT_RANKED_NETWORK_PROTOCOL_VERSION_V1,
        robin_engine::multiplayer::NET_PROTOCOL_VERSION,
        "ranked session admission must track the exact runtime network protocol",
    );
    assert_eq!(
        robin_engine::multiplayer::NET_PROTOCOL_VERSION,
        42,
        "authoritative AI detectable FIFO ordering requires network protocol 42",
    );
}
