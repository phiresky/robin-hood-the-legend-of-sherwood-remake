//! Tests for the shared native transport pieces in `native.rs`: campaign
//! session ownership and handoffs, frame caps, the outgoing bridge, and the
//! epoch clock.
use super::{HostSessionContinuation, PeerOwner, checked_epoch_ms};
use crate::multiplayer::{MAX_CONTENT_FRAME_BYTES, MAX_SERVER_CONTROL_FRAME_BYTES};

#[test]
fn admission_phase_caps_fit_max_valid_offer_and_chunk() {
    let text = "x".repeat(robin_engine::multiplayer::DistributedModOffer::TEXT_BYTE_LIMIT);
    let host_id = "x"
        .repeat(robin_engine::multiplayer::DistributedModOffer::AUTHENTICATED_HOST_ID_BYTE_LIMIT);
    let offer = robin_engine::multiplayer::DistributedModOffer {
        schema_version: 1,
        full_mod_sha256: [1; 32],
        spellforge_package_sha256: None,
        spellforge_vm_abi: None,
        encoded_bytes: 1,
        mission_basename: text.clone(),
        mission_rhm_entry: text.clone(),
        map_filename: text.clone(),
        title: text.clone(),
        claimed_author: text.clone(),
        version: text.clone(),
        source_url: text.clone(),
        license: text.clone(),
        host_endpoint_id: host_id,
    };
    offer.validate().unwrap();
    let offer_bytes = super::encode_msg(&super::NetMsg::ContentOffer { offer });
    assert!(offer_bytes.len() <= MAX_SERVER_CONTROL_FRAME_BYTES);

    let chunk_bytes = super::encode_msg(&super::NetMsg::ContentChunk(
        robin_engine::multiplayer::ContentChunk {
            full_mod_sha256: [1; 32],
            offset: 0,
            total_bytes: robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT as u64,
            bytes: vec![0; robin_engine::multiplayer::DISTRIBUTED_MOD_CHUNK_LIMIT],
        },
    ));
    assert!(chunk_bytes.len() <= MAX_CONTENT_FRAME_BYTES);
    assert!(chunk_bytes.len() > MAX_SERVER_CONTROL_FRAME_BYTES);
}

#[test]
fn native_epoch_conversion_accepts_boundary_and_rejects_overflow() {
    assert_eq!(checked_epoch_ms(0).ok(), Some(0));
    assert_eq!(checked_epoch_ms(u128::from(u64::MAX)).ok(), Some(u64::MAX));
    assert!(checked_epoch_ms(u128::from(u64::MAX) + 1).is_err());
}

#[test]
fn native_clock_returns_a_real_post_epoch_timestamp() {
    assert!(super::try_current_epoch_ms().expect("native system clock") > 0);
}

#[test]
fn campaign_owners_isolate_transport_identity_and_handoffs() {
    let a = super::MultiplayerCampaignSession::default();
    let b = super::MultiplayerCampaignSession::default();
    assert_eq!(a.state().client_key.public(), a.state().client_key.public());
    assert_ne!(a.state().client_key.public(), b.state().client_key.public());
    let continuation = HostSessionContinuation {
        host_endpoint_id: iroh::SecretKey::from_bytes(&[3; 32]).public(),
        session_id: robin_engine::multiplayer::MultiplayerSessionId([4; 32]),
        expected_players: 2,
        owner_seats: std::collections::HashMap::from([(PeerOwner::Native([8; 32]), 1)]),
        relay_url: None,
    };
    super::publish_host_session_continuation(a.state(), continuation.clone());
    super::publish_host_session_continuation(b.state(), continuation.clone());
    a.discard_host_continuation().unwrap();
    drop(a);
    assert!(
        super::pending_host_session_continuation(b.state(), continuation.host_endpoint_id, 3)
            .is_err()
    );
    assert!(
        super::pending_host_session_continuation(
            b.state(),
            iroh::SecretKey::generate().public(),
            2
        )
        .is_err()
    );
    let restored =
        super::pending_host_session_continuation(b.state(), continuation.host_endpoint_id, 2)
            .unwrap()
            .unwrap();
    assert_eq!(restored.owner_seats, continuation.owner_seats);
    assert_eq!(restored.session_id, continuation.session_id);
    // Reading for replacement preparation is transactional: failure before
    // successful endpoint publication leaves the handoff intact.
    assert!(b.state().continuation.lock().is_some());
}

#[test]
fn failed_startup_cancellation_joins_bridge_with_sender_still_alive() {
    let (sender, receiver) = std::sync::mpsc::channel();
    let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (bridge, _async_receiver) = super::spawn_outgoing_bridge(
        "test-campaign-failed-startup",
        receiver,
        cancellation.clone(),
    )
    .unwrap();
    cancellation.store(true, std::sync::atomic::Ordering::Release);
    bridge.join().unwrap();
    drop(sender);
}

#[test]
fn campaign_server_lease_rejects_overlap_and_releases_failed_preparation() {
    let campaign = super::MultiplayerCampaignSession::default();
    let lease = campaign.reserve_server().unwrap();
    assert!(campaign.reserve_server().is_err());
    assert!(campaign.discard_host_continuation().is_err());
    let other = super::MultiplayerCampaignSession::default();
    let other_lease = other.reserve_server().unwrap();
    drop(lease);
    let replacement = campaign.reserve_server().unwrap();
    assert!(other.reserve_server().is_err());
    drop(other_lease);
    drop(replacement);
    assert!(campaign.reserve_server().is_ok());
}

#[test]
fn campaign_serialization_cannot_restore_transport_authority() {
    let campaign = super::MultiplayerCampaignSession::default();
    let encoded = serde_json::to_string(&campaign).unwrap();
    assert_eq!(encoded, "{}");
    let decoded: super::MultiplayerCampaignSession = serde_json::from_str(&encoded).unwrap();
    assert!(decoded.state.is_none());
}

#[test]
fn campaign_repeated_publication_merges_authenticated_seats() {
    let campaign = super::MultiplayerCampaignSession::default();
    let mut continuation = HostSessionContinuation {
        host_endpoint_id: iroh::SecretKey::from_bytes(&[3; 32]).public(),
        session_id: robin_engine::multiplayer::MultiplayerSessionId([4; 32]),
        expected_players: 3,
        owner_seats: std::collections::HashMap::from([(PeerOwner::Native([8; 32]), 1)]),
        relay_url: None,
    };
    super::publish_host_session_continuation(campaign.state(), continuation.clone());
    continuation.owner_seats = std::collections::HashMap::from([(PeerOwner::Browser([9; 32]), 2)]);
    super::publish_host_session_continuation(campaign.state(), continuation.clone());
    assert_eq!(
        super::pending_host_session_continuation(
            campaign.state(),
            continuation.host_endpoint_id,
            3
        )
        .unwrap()
        .unwrap()
        .owner_seats
        .len(),
        2
    );
}
