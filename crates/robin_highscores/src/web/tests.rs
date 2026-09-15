use super::*;
use crate::test_support::TestDeployment;
use axum::extract::ConnectInfo;
use http_body_util::BodyExt as _;
use std::net::SocketAddr;
use tower::ServiceExt as _;

#[tokio::test]
async fn owned_mutation_keeps_its_lease_after_response_cancellation() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let (_directory, _config, database) = TestDeployment::new().migrate().await;
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let unblock = std::sync::Arc::new(tokio::sync::Notify::new());
    let completed = std::sync::Arc::new(AtomicBool::new(false));
    let outer = tokio::spawn(run_owned_maintenance_write(
        database.clone(),
        crate::db::MaintenanceWriteClass::ApiSensitive,
        {
            let started = started.clone();
            let unblock = unblock.clone();
            let completed = completed.clone();
            async move {
                started.notify_one();
                unblock.notified().await;
                completed.store(true, Ordering::SeqCst);
            }
        },
    ));
    started.notified().await;
    outer.abort();
    let _ = outer.await;
    assert_eq!(
        database
            .active_maintenance_write_lease_count()
            .await
            .unwrap(),
        1,
        "dropping the response waiter must not release an in-flight mutation"
    );
    unblock.notify_one();
    for _ in 0..100 {
        if database
            .active_maintenance_write_lease_count()
            .await
            .unwrap()
            == 0
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(completed.load(Ordering::SeqCst));
    assert_eq!(
        database
            .active_maintenance_write_lease_count()
            .await
            .unwrap(),
        0
    );
    database.close().await;
}

#[test]
fn single_replay_submission_limit_is_checked_and_exact() {
    let mut config = ServerConfig {
        max_replay_bytes: 11,
        max_metadata_bytes: 17,
        ..Default::default()
    };
    assert_eq!(
        submission_body_limit(&config).unwrap(),
        11 + 17 + MULTIPART_ENVELOPE_OVERHEAD_BYTES
    );
    config.max_replay_bytes = u64::MAX;
    assert!(matches!(
        submission_body_limit(&config),
        Err(ApiError::Internal)
    ));
}

fn cursor() -> CursorToken {
    CursorToken {
        filter_sha256: Digest32::from_bytes([1; 32]),
        accepted_sequence_watermark: 25,
        visibility_revision: 3,
        metric_value: 12_345,
        position: 4,
        rank: 3,
        accepted_sequence: 20,
        verified_at_unix_ms: 1_234_567,
        run_id: "018f0000-0000-7000-8000-000000000000".to_owned(),
    }
}

#[test]
fn next_cursor_conversion_matches_entry_based_response_for_both_metrics() {
    for (metric, metric_value) in [
        (BoardMetricV1::OriginalScore, 12_345),
        (BoardMetricV1::FastestSuccess, 12_345),
    ] {
        let mut token = cursor();
        token.metric_value = metric_value;
        let row = BoardRow {
            rank: token.rank,
            run_id: token.run_id.clone(),
            metric_value,
            max_concurrent_players: 1,
            participant_instance_count: 1,
            uploader: None,
            replay_sha256: [3; 32],
            accepted_sequence: u64::try_from(token.accepted_sequence).unwrap(),
            verified_at_ms: token.verified_at_unix_ms,
        };
        let entry = board_entry(&row, token.position, metric).unwrap();
        let opaque_token = encode_cursor(&token, &[2; 32]).unwrap();
        let actual = leaderboard_cursor(&token, opaque_token, metric).unwrap();
        assert_eq!(actual.last, LeaderboardOrderAnchorV2::from_entry(&entry));
        let decoded = decode_cursor(&actual.opaque_token, token.filter_sha256, &[2; 32]).unwrap();
        assert_eq!(decoded.visibility_revision, token.visibility_revision);
        assert_eq!(decoded.metric_value, metric_value);
    }
    let mut invalid = cursor();
    invalid.metric_value = -1;
    assert!(leaderboard_cursor(&invalid, String::new(), BoardMetricV1::FastestSuccess).is_err());
    invalid = cursor();
    invalid.accepted_sequence = -1;
    assert!(leaderboard_cursor(&invalid, String::new(), BoardMetricV1::OriginalScore).is_err());
    invalid = cursor();
    invalid.run_id.clear();
    assert!(leaderboard_cursor(&invalid, String::new(), BoardMetricV1::OriginalScore).is_err());
}

#[test]
fn cursor_authentication_rejects_tampering_other_filters_and_other_keys() {
    let key = [2; 32];
    let encoded = encode_cursor(&cursor(), &key).unwrap();
    assert!(decode_cursor(&encoded, Digest32::from_bytes([1; 32]), &key).is_ok());
    assert!(decode_cursor(&encoded, Digest32::from_bytes([3; 32]), &key).is_err());
    let mut decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&encoded)
        .unwrap();
    decoded[10] ^= 1;
    let tampered = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(decoded);
    assert!(decode_cursor(&tampered, Digest32::from_bytes([1; 32]), &key).is_err());
    assert!(decode_cursor(&encoded, Digest32::from_bytes([1; 32]), &[4; 32]).is_err());

    let history = PlayerHistoryCursorToken {
        player_public_key: PublicKey32::from_bytes([3; 32]),
        query_sha256: Digest32::from_bytes([4; 32]),
        accepted_sequence_watermark: 25,
        visibility_revision: 3,
        accepted_sequence: 20,
        run_id: "run-1".to_owned(),
    };
    let encoded = encode_player_history_cursor(&history, &key).unwrap();
    assert!(
        decode_player_history_cursor(
            &encoded,
            history.player_public_key,
            history.query_sha256,
            &key
        )
        .is_ok()
    );
    assert!(
        decode_player_history_cursor(
            &encoded,
            PublicKey32::from_bytes([5; 32]),
            history.query_sha256,
            &key
        )
        .is_err()
    );
}

// Independent envelope construction through `ring` keeps persisted cursors
// compatible with the HMAC implementation.
#[test]
fn cursor_envelope_is_json_followed_by_hmac_sha256() {
    let key = [2; 32];
    let token = cursor();
    let json = serde_json::to_vec(&token).unwrap();
    let signature = ring::hmac::sign(&ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &key), &json);
    assert_eq!(
        encode_cursor(&token, &key).unwrap(),
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode([json.as_slice(), signature.as_ref()].concat())
    );
}

#[test]
fn trusted_proxy_must_supply_exactly_one_canonical_forwarded_address() {
    let config = ServerConfig {
        trusted_proxy_cidrs: vec!["127.0.0.1/32".to_owned()],
        ..Default::default()
    };
    let proxy: SocketAddr = "127.0.0.1:9000".parse().unwrap();
    let direct: SocketAddr = "203.0.113.9:9000".parse().unwrap();
    let header = |value: &str| HeaderValue::from_str(value).unwrap();
    assert_eq!(
        effective_client_ip(&config, direct, Some(&header("198.51.100.1"))).unwrap(),
        direct.ip(),
        "untrusted peers' forwarding headers are ignored"
    );
    assert_eq!(
        effective_client_ip(&config, proxy, Some(&header("198.51.100.1"))).unwrap(),
        "198.51.100.1".parse::<IpAddr>().unwrap()
    );
    for bad in ["198.51.100.1, 10.0.0.1", " 198.51.100.1", "not-an-ip"] {
        assert!(effective_client_ip(&config, proxy, Some(&header(bad))).is_err());
    }
    assert!(effective_client_ip(&config, proxy, None).is_err());
}

#[tokio::test]
async fn challenge_rate_limit_is_per_address_and_purpose() {
    let limiter = ChallengeRateLimiter::new(2);
    let address: IpAddr = "198.51.100.1".parse().unwrap();
    limiter
        .check(address, ChallengePurpose::Submission)
        .await
        .unwrap();
    limiter
        .check(address, ChallengePurpose::Submission)
        .await
        .unwrap();
    assert!(matches!(
        limiter.check(address, ChallengePurpose::Submission).await,
        Err(ApiError::RateLimited { .. })
    ));
    limiter
        .check(address, ChallengePurpose::Deletion)
        .await
        .unwrap();
    limiter
        .check(
            "198.51.100.2".parse().unwrap(),
            ChallengePurpose::Submission,
        )
        .await
        .unwrap();
}

#[test]
fn transport_preflight_rejects_alternate_formats_and_mismatched_identity() {
    let artifact = |bytes: &[u8]| ArtifactRefV1 {
        sha256: Digest32::digest_bytes(bytes),
        byte_length: bytes.len() as u64,
        media_type: RANKED_REPLAY_MEDIA_TYPE_V1.to_owned(),
    };
    let jsonl = b"{\"header\":{}}\n";
    assert!(preflight_ranked_replay_transport(jsonl, &artifact(jsonl)).is_err());
    let binary = [0xff_u8, 0, 1];
    assert!(preflight_ranked_replay_transport(&binary, &artifact(&binary)).is_err());
    let other = artifact(b"different bytes");
    assert!(preflight_ranked_replay_transport(jsonl, &other).is_err());
}

#[test]
fn every_rejection_code_has_a_short_public_message() {
    for code in VerificationRejectionCodeV1::ALL {
        let message = safe_rejection_message(code);
        assert!(!message.is_empty() && message.len() <= 500);
    }
}

#[tokio::test]
async fn diagnostics_accept_retry_limit_and_protect_operator_access() {
    use robin_run_protocol::diagnostics::{
        DiagnosticKindV1, DiagnosticReceiptV1, DiagnosticReportV1,
    };
    let directory = tempfile::tempdir().unwrap();
    let config = ServerConfig {
        database_path: directory.path().join("highscores.sqlite3"),
        replay_directory: directory.path().join("replays"),
        moderation_bearer_token: Some(Arc::new(b"0123456789abcdef0123456789abcdef".to_vec())),
        moderation_bearer_token_path: Some(directory.path().join("token")),
        ..Default::default()
    };
    let application = router(crate::test_support::app_state(config).await).unwrap();
    let mut report = DiagnosticReportV1 {
        schema_version: 1,
        kind: DiagnosticKindV1::Bug,
        description: "stuck".into(),
        engine_commit: "test".into(),
        platform: "test".into(),
        occurred_at_unix_ms: 0,
        backtrace: None,
        recent_log: "private log".into(),
        attachments: vec![],
        warnings: vec![],
    };
    let mut first_id = String::new();
    for index in 0_u64..12 {
        // The first request is retried byte-for-byte and must not consume quota.
        report.occurred_at_unix_ms = index.saturating_sub(1);
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/diagnostics")
                    .header("content-type", "application/json")
                    .header("content-encoding", "zstd")
                    .header("x-diagnostic-kind", "bug")
                    .header("x-diagnostic-engine-commit", "test")
                    .extension(ConnectInfo("127.0.0.1:1234".parse::<SocketAddr>().unwrap()))
                    .body(Body::from(
                        robin_run_protocol::diagnostics::compress_report(
                            &serde_json::to_vec(&report).unwrap(),
                        )
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        if index == 11 {
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            break;
        }
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let receipt: DiagnosticReceiptV1 = serde_json::from_slice(&bytes).unwrap();
        if index == 0 {
            first_id = receipt.report_id.clone();
        }
        if index == 1 {
            assert_eq!(receipt.report_id, first_id);
        }
    }
    let uri = format!("/api/v1/operator/diagnostics/{first_id}");
    let unauthorized = application
        .clone()
        .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let authorized = application
        .clone()
        .oneshot(
            Request::builder()
                .uri(&uri)
                .header("authorization", "Bearer 0123456789abcdef0123456789abcdef")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    assert!(authorized.headers().get("content-encoding").is_none());
    let deleted = application
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&uri)
                .header("authorization", "Bearer 0123456789abcdef0123456789abcdef")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
}
