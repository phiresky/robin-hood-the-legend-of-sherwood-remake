use super::*;

pub(crate) fn protocol_public_key(key: &SigningKey) -> PublicKey32 {
    PublicKey32::from_bytes(key.verifying_key().to_bytes())
}

pub(crate) fn sign(key: &SigningKey, bytes: &[u8]) -> Signature64 {
    Signature64::from_bytes(key.sign(bytes).to_bytes())
}

/// A canonical compact replay whose bytes are unique per `label`.
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
        replay_schema_version: robin_run_protocol::CURRENT_RANKED_REPLAY_SCHEMA_VERSION_V1,
    }
}

/// An unsigned submission of `replay` to the default board and mission.
pub(crate) fn submission(
    key: &SigningKey,
    upload_challenge: UploadChallengeV1,
    replay: &[u8],
    public_disclosure: ParticipantPublicDisclosureV1,
) -> SubmissionV2 {
    SubmissionV2 {
        schema_version: SCHEMA_VERSION_V2,
        upload_challenge,
        uploader_public_key: protocol_public_key(key),
        public_disclosure,
        board_id: OpaqueId::new(BOARD_ID).unwrap(),
        mission_id: MISSION_ID.to_owned(),
        replay: replay_artifact(replay),
        requested_metrics: vec![BoardMetricV1::OriginalScore, BoardMetricV1::FastestSuccess],
    }
}

pub(crate) fn signed_submission(key: &SigningKey, submission: SubmissionV2) -> SignedSubmissionV2 {
    let signature = sign(
        key,
        &SignedSubmissionV2::signing_bytes(&submission).unwrap(),
    );
    SignedSubmissionV2 {
        schema_version: SCHEMA_VERSION_V2,
        submission,
        algorithm: SignatureAlgorithmV1::Ed25519,
        signature,
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

/// One multipart part: (field name, content type, bytes).
pub(crate) type Part<'a> = (&'a str, &'a str, &'a [u8]);

pub(crate) fn multipart_parts(parts: &[Part<'_>]) -> Request<Body> {
    const BOUNDARY: &str = "robin-router-e2e-boundary";
    let mut body = Vec::new();
    for (name, content_type, bytes) in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{name}\"\r\nContent-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
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

pub(crate) fn multipart_request(signed: &SignedSubmissionV2, replay: &[u8]) -> Request<Body> {
    let metadata = serde_json::to_vec(signed).unwrap();
    multipart_parts(&[
        ("submission", "application/json", &metadata),
        ("replay", RANKED_REPLAY_MEDIA_TYPE_V1, replay),
    ])
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

pub(crate) async fn bad_request_message(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(crate) fn leaderboard_uri(
    board_id: &str,
    metric: &str,
    limit: u16,
    cursor: Option<&str>,
) -> String {
    let mut uri = format!(
        "/api/v1/leaderboards?schema_version=2&board_id={board_id}&mission_id={MISSION_ID}\
         &metric={metric}&limit={limit}"
    );
    if let Some(cursor) = cursor {
        uri.push_str("&cursor=");
        uri.push_str(cursor);
    }
    uri
}

pub(crate) async fn assert_report_status(
    rig: &TestRig,
    target: AbuseReportTargetV1,
    address: Ipv4Addr,
    expected: StatusCode,
) {
    let response = rig
        .send(json_request(
            Method::POST,
            "/api/v1/reports",
            &AbuseReportV1 {
                schema_version: SCHEMA_VERSION_V1,
                target,
                category: AbuseReportCategoryV1::Other,
                detail: "Router E2E moderation signal".to_owned(),
            },
            address,
        ))
        .await;
    assert_eq!(response.status(), expected);
    if expected == StatusCode::ACCEPTED {
        let accepted: AbuseReportAcceptedV1 = json_body(response).await;
        accepted.validate().unwrap();
    }
}
