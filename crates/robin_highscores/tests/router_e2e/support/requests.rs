use super::*;

pub(crate) async fn assert_report_status(
    rig: &TestRig,
    target: AbuseReportTargetV1,
    peer: Ipv4Addr,
    expected: StatusCode,
) {
    let response = rig
        .app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/api/v1/reports",
            &AbuseReportV1 {
                schema_version: SCHEMA_VERSION_V1,
                target,
                category: AbuseReportCategoryV1::Other,
                detail: "Router E2E moderation signal".to_owned(),
            },
            peer,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), expected);
    if expected == StatusCode::ACCEPTED {
        let accepted: AbuseReportAcceptedV1 = json_body(response).await;
        accepted.validate().unwrap();
    }
}

pub(crate) fn assert_public_projection_is_redacted(value: &impl Serialize) {
    let public_value = serde_json::to_value(value).unwrap();
    assert!(public_value.get("verification_result").is_none());
    let public_json = public_value.to_string();
    for forbidden in [
        "authenticated_participant_claims",
        "participant_claims",
        "canonical_campaign_state",
        "campaign_state_requirement",
        "canonical_campaign_state_json",
        "campaign_chain_receipt",
        "expected_starting_campaign",
        "controller_public_key",
        "participant_instance_id",
        "chain_id",
    ] {
        assert!(
            !public_json.contains(forbidden),
            "public JSON leaked forbidden field {forbidden}"
        );
    }
}

pub(crate) fn assert_public_json_omits_private_participant_id(bytes: &[u8], private_id: Digest32) {
    let json = std::str::from_utf8(bytes).unwrap();
    assert!(
        !json.contains("participant_instance_id"),
        "public JSON exposed a private participant-instance field"
    );
    let sentinel = hex::encode(private_id.as_bytes());
    let sentinel_context = json.find(&sentinel).map(|position| {
        &json[position.saturating_sub(80)..(position + sentinel.len() + 80).min(json.len())]
    });
    assert!(
        sentinel_context.is_none(),
        "public JSON exposed private participant-instance sentinel {sentinel}: {sentinel_context:?}"
    );
}

pub(crate) fn assert_public_json_omits_private_chain_id(bytes: &[u8], private_chain_id: &str) {
    let json = std::str::from_utf8(bytes).unwrap();
    assert!(
        !json.contains("chain_id"),
        "public JSON exposed a private campaign-chain field"
    );
    assert!(
        !json.contains(private_chain_id),
        "public JSON exposed private campaign-chain sentinel {private_chain_id}"
    );
}

pub(crate) fn leaderboard_uri(rig: &TestRig, limit: u16, cursor: Option<&str>) -> String {
    let mut uri = format!(
        "/api/v1/leaderboards?schema_version=1&subject_kind=mission&mission_id={MISSION_ID}\
         &mission_scope=individual_level&metric=original_score\
         &content_identity_sha256={}&rules_config_sha256={}\
         &ruleset_manifest_sha256={}&limit={limit}",
        rig.content_sha256, rig.rules_config_sha256, rig.ruleset_sha256
    );
    uri.retain(|character| !character.is_ascii_whitespace());
    if let Some(cursor) = cursor {
        uri.push_str("&cursor=");
        uri.push_str(cursor);
    }
    uri
}

pub(crate) fn mission_leaderboard_uri(
    rig: &TestRig,
    mission_id: &str,
    mission_scope: &str,
    metric: &str,
) -> String {
    let content = match mission_id {
        GENESIS_MISSION_ID => rig.genesis_content_sha256.unwrap(),
        HQ_MISSION_ID => rig.terminal_content_sha256.unwrap(),
        _ => rig.content_sha256,
    };
    format!(
        "/api/v1/leaderboards?schema_version=1&subject_kind=mission&mission_id={mission_id}\
         &mission_scope={mission_scope}&metric={metric}&content_identity_sha256={content}\
         &rules_config_sha256={}&ruleset_manifest_sha256={}&limit=10",
        rig.rules_config_sha256, rig.ruleset_sha256
    )
    .chars()
    .filter(|character| !character.is_ascii_whitespace())
    .collect()
}

pub(crate) fn campaign_mission_leaderboard_uri(rig: &TestRig, mission_id: &str) -> String {
    mission_leaderboard_uri(rig, mission_id, "campaign", "original_score")
}

pub(crate) fn full_campaign_leaderboard_uri(rig: &TestRig) -> String {
    format!(
        "/api/v1/leaderboards?schema_version=1&subject_kind=full_campaign&metric=original_score\
         &content_identity_sha256={}&rules_config_sha256={}\
         &ruleset_manifest_sha256={}&limit=10",
        rig.campaign_content_sha256.unwrap(),
        rig.rules_config_sha256,
        rig.ruleset_sha256
    )
    .chars()
    .filter(|character| !character.is_ascii_whitespace())
    .collect()
}

pub(crate) fn competition_leaderboard_uri(rig: &TestRig, competition_sha256: Digest32) -> String {
    format!(
        "/api/v1/leaderboards?schema_version=1&subject_kind=mission&mission_id={MISSION_ID}\
         &mission_scope=individual_level&metric=original_score\
         &content_identity_sha256={}&rules_config_sha256={}\
         &ruleset_manifest_sha256={}&competition_manifest_sha256={competition_sha256}\
         &max_concurrent_players=1&limit=10",
        rig.content_sha256, rig.rules_config_sha256, rig.ruleset_sha256
    )
    .chars()
    .filter(|character| !character.is_ascii_whitespace())
    .collect()
}

pub(crate) fn protocol_public_key(key: &SigningKey) -> PublicKey32 {
    PublicKey32::from_bytes(key.verifying_key().to_bytes())
}

pub(crate) fn sign(key: &SigningKey, bytes: &[u8]) -> Signature64 {
    Signature64::from_bytes(key.sign(bytes).to_bytes())
}

pub(crate) fn compact_replay_fixture(label: &str) -> Vec<u8> {
    use robin_engine::replay::{REPLAY_SCHEMA_VERSION, ReplayFile, ReplayHeader};
    let replay: robin_engine::replay::ReplayData = ReplayFile {
        header: ReplayHeader {
            mission_id: label.to_owned(),
            rng_seed: 1,
            sim_config: robin_engine::engine::SimConfig::default(),
            version: REPLAY_SCHEMA_VERSION,
            total_frames: 0,
            rankability: robin_engine::replay_rankability::ReplayRankability::rankable(),
            mission_assets: robin_engine::mission_assets::MissionAssetDescriptor::built_in(
                label, label, label,
            )
            .unwrap(),
            spellforge_package: None,
            campaign: bitcode::encode(&robin_engine::campaign::Campaign::default()),
        },
        frames: Default::default(),
        hashes: Default::default(),
        save_markers: Default::default(),
        load_backs: Default::default(),
    }
    .try_into()
    .expect("valid replay fixture");
    robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
        .unwrap()
        .into_bytes()
}

pub(crate) fn replay_artifact(bytes: &[u8]) -> ReplayArtifactV1 {
    ReplayArtifactV1 {
        artifact: ArtifactRefV1 {
            sha256: Digest32::digest_bytes(bytes),
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
        },
        replay_schema_version: REPLAY_SCHEMA_VERSION,
    }
}

pub(crate) fn campaign_artifact(bytes: &[u8]) -> ArtifactRefV1 {
    ArtifactRefV1 {
        sha256: Digest32::digest_bytes(bytes),
        byte_length: u64::try_from(bytes.len()).unwrap(),
        media_type: RANKED_CAMPAIGN_MEDIA_TYPE_V1.to_owned(),
    }
}

pub(crate) fn submission_artifacts(
    replay: &[u8],
    starting_campaign: &[u8],
) -> SubmissionArtifactsV1 {
    SubmissionArtifactsV1 {
        replay: replay_artifact(replay),
        starting_campaign: campaign_artifact(starting_campaign),
    }
}

pub(crate) fn signed_submission(
    key: &SigningKey,
    offer: SubmissionOfferV1,
    replay: &[u8],
    starting_campaign: &[u8],
) -> SignedSubmissionV1 {
    let envelope = SubmissionEnvelopeV1 {
        schema_version: SCHEMA_VERSION_V1,
        replay_session_transcript: replay_session_transcript(&offer),
        offer,
        artifacts: submission_artifacts(replay, starting_campaign),
        campaign_aggregation_consent: CampaignAggregationConsentV1::NotAuthorized,
        campaign_continuation_authorization: None,
        requested_metrics: vec![BoardMetricV1::OriginalScore],
    };
    let signed = SignedSubmissionV1 {
        schema_version: SCHEMA_VERSION_V1,
        algorithm: SignatureAlgorithmV1::Ed25519,
        participant_signatures: vec![ParticipantSignatureV1 {
            public_key: protocol_public_key(key),
            signature: sign(key, &envelope.signing_bytes().unwrap()),
        }],
        submission: envelope,
    };
    signed.validate().unwrap();
    signed
}

pub(crate) fn replay_session_transcript(offer: &SubmissionOfferV1) -> ReplaySessionTranscriptV1 {
    let host = offer.session_genesis.claim.host_participant_instance_id;
    ReplaySessionTranscriptV1 {
        schema_version: SCHEMA_VERSION_V1,
        session_genesis_sha256: offer.session_genesis.canonical_digest().unwrap(),
        replay_session_id: offer.session_genesis.claim.replay_session_id,
        host_participant_instance_id: host,
        participant_instance_count: offer.participant_instance_count,
        max_concurrent_players: offer.max_concurrent_players,
        events: vec![ReplaySeatLifecycleEventV1 {
            event_ordinal: 0,
            replay_ordinal: 0,
            seat: 0,
            participant_instance_id: host,
            lifecycle: ReplaySeatLifecycleKindV1::Connected {
                connection_epoch: 0,
            },
        }],
    }
}

pub(crate) fn peer(address: Ipv4Addr) -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::new(IpAddr::V4(address), 41_000))
}

pub(crate) fn assert_dynamic_headers(response: &axum::response::Response) {
    assert_eq!(response.headers().get(CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(
        response.headers().get(X_CONTENT_TYPE_OPTIONS).unwrap(),
        "nosniff"
    );
}

pub(crate) fn json_request<T: Serialize>(
    method: Method,
    uri: &str,
    value: &T,
    address: Ipv4Addr,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(CONTENT_TYPE, "application/json")
        .extension(peer(address))
        .body(Body::from(serde_json::to_vec(value).unwrap()))
        .unwrap()
}

pub(crate) fn empty_request(method: Method, uri: &str, address: Ipv4Addr) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .extension(peer(address))
        .body(Body::empty())
        .unwrap()
}

pub(crate) fn multipart_request(
    signed: &SignedSubmissionV1,
    replay: &[u8],
    starting_campaign: &[u8],
) -> Request<Body> {
    multipart_request_with_replay_field(signed, replay, starting_campaign, "replay")
}

pub(crate) fn multipart_request_with_replay_field(
    signed: &SignedSubmissionV1,
    replay: &[u8],
    starting_campaign: &[u8],
    replay_field: &str,
) -> Request<Body> {
    multipart_request_with_replay_transport(
        signed,
        replay,
        starting_campaign,
        replay_field,
        RANKED_REPLAY_MEDIA_TYPE_V1,
    )
}

pub(crate) fn multipart_request_with_replay_transport(
    signed: &SignedSubmissionV1,
    replay: &[u8],
    starting_campaign: &[u8],
    replay_field: &str,
    replay_media_type: &str,
) -> Request<Body> {
    const BOUNDARY: &str = "robin-router-e2e-boundary";
    let metadata = serde_json::to_vec(signed).unwrap();
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"submission\"\r\nContent-Type: application/json\r\n\r\n",
    );
    body.extend_from_slice(&metadata);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{replay_field}\"; filename=\"replay.rhrec\"\r\nContent-Type: {replay_media_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(replay);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"starting_campaign\"; filename=\"starting.campaign\"\r\nContent-Type: {RANKED_CAMPAIGN_MEDIA_TYPE_V1}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(starting_campaign);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/submissions")
        .header(
            CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .extension(peer(Ipv4Addr::LOCALHOST))
        .body(Body::from(body))
        .unwrap()
}

pub(crate) fn metadata_only_multipart_request(signed: &SignedSubmissionV1) -> Request<Body> {
    const BOUNDARY: &str = "robin-router-partial-boundary";
    let metadata = serde_json::to_vec(signed).unwrap();
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"submission\"\r\nContent-Type: application/json\r\n\r\n",
    );
    body.extend_from_slice(&metadata);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    Request::builder()
        .method(Method::POST)
        .uri("/api/v1/submissions")
        .header(
            CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .extension(peer(Ipv4Addr::LOCALHOST))
        .body(Body::from(body))
        .unwrap()
}

pub(crate) async fn json_body<T: DeserializeOwned>(response: axum::response::Response) -> T {
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "could not decode {status} response as {}: {error}; body={}",
            std::any::type_name::<T>(),
            String::from_utf8_lossy(&bytes)
        )
    })
}
