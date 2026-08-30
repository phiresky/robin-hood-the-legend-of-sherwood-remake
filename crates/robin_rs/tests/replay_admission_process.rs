#![cfg(all(not(target_arch = "wasm32"), unix))]

use robin_engine::replay::{REPLAY_SCHEMA_VERSION, ReplayData, ReplayFile, ReplayHeader};
use sha2::Digest as _;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::process::{Command, Stdio};

fn current_replay_with_campaign(campaign: robin_engine::campaign::Campaign) -> ReplayData {
    ReplayFile {
        header: ReplayHeader {
            mission_id: "Dem_Lei_MP".to_owned(),
            rng_seed: 0x36,
            sim_config: robin_engine::engine::SimConfig::default(),
            version: REPLAY_SCHEMA_VERSION,
            total_frames: 0,
            campaign: bitcode::encode(&campaign),
        },
        frames: BTreeMap::new(),
        hashes: BTreeMap::new(),
        save_markers: BTreeMap::new(),
        load_backs: BTreeMap::new(),
    }
    .into()
}

fn empty_current_replay() -> ReplayData {
    current_replay_with_campaign(robin_engine::campaign::Campaign::default())
}

fn run_cold_worker(input: &[u8]) -> serde_json::Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_robin"))
        .arg("--internal-replay-admission-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cold replay admission worker");
    child
        .stdin
        .take()
        .expect("worker stdin")
        .write_all(input)
        .expect("write replay artifact");
    let output = child.wait_with_output().expect("wait for replay worker");
    assert!(
        output.status.success(),
        "worker failed: status={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("worker returns one bounded JSON reply")
}

#[test]
fn cold_native_worker_accepts_only_the_exact_current_compact_artifact() {
    let compact = robin_rs::replay_format::encode_compact(
        &empty_current_replay(),
        robin_rs::replay_format::ENGINE_VERSION_HASH,
    )
    .expect("encode current compact replay");
    let reply = run_cold_worker(compact.as_bytes());
    assert_eq!(reply["status"], "accepted");
    assert_eq!(
        reply["sha256"],
        hex::encode(sha2::Sha256::digest(compact.as_bytes()))
    );

    let rejected = run_cold_worker(b"{\"plausible\":\"json\"}\n");
    assert_eq!(rejected["status"], "rejected");
    assert!(rejected["sha256"].is_null());
}

#[test]
fn contained_worker_rejects_nested_campaign_allocation_amplification_and_survives() {
    let mut campaign = robin_engine::campaign::Campaign::default();
    campaign.peasant_names = vec![
        String::new();
        robin_rs::replay_format::DEFAULT_REPLAY_ADMISSION_LIMITS
            .max_typed_collection_entries
            + 1
    ];
    let compact = robin_rs::replay_format::encode_compact(
        &current_replay_with_campaign(campaign),
        robin_rs::replay_format::ENGINE_VERSION_HASH,
    )
    .expect("encode nested campaign amplification fixture");
    let rejected = run_cold_worker(compact.as_bytes());
    assert_eq!(rejected["status"], "rejected");
    assert!(
        rejected["error"]
            .as_str()
            .expect("worker error string")
            .contains("TypedCollectionEntries"),
        "unexpected rejection: {rejected}"
    );

    // The malformed campaign was confined to its one-shot process. A fresh
    // worker still admits a small canonical artifact afterwards.
    let valid = robin_rs::replay_format::encode_compact(
        &empty_current_replay(),
        robin_rs::replay_format::ENGINE_VERSION_HASH,
    )
    .expect("encode post-rejection replay");
    assert_eq!(run_cold_worker(valid.as_bytes())["status"], "accepted");
}
