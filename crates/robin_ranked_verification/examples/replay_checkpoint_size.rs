//! Estimate engine-only seek checkpoint storage using a real verified replay.
//! Usage: replay_checkpoint_size RAW_DATADIR LOCALE REPLAY.jsonl
//! The experimental bundle is a size probe, not a supported replay format.

use robin_engine::engine::{Engine, HostDisplayState};
use robin_engine::game_operation::GameCode;
use robin_engine::ranked_resim::{RankedExecutionContext, resimulate_canonical_ranked_replay};
use robin_engine::replay::{ReplayData, ReplayFile, state_hash};
use robin_ranked_verification::ranked_verifier::{
    confined_official_files, load_official_profiles, prepare_ranked_replay_mission,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Serialize, Deserialize, bitcode::Encode)]
struct Checkpoint {
    ordinal: u32,
    engine: Vec<u8>,
}

fn compressed(bytes: &[u8], level: i32) -> usize {
    zstd::encode_all(bytes, level).unwrap().len()
}

fn main() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)
        .unwrap()
        .join()
        .unwrap();
}

fn run() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 4, "expected RAW_DATADIR LOCALE REPLAY.jsonl");
    let replay = ReplayData::from_file(&args[3]).expect("read replay");
    let header = replay.header();
    let policy = robin_engine::ranked_rules::ranked_policy_for_board(
        robin_run_types::BoardSimulationPolicyV1::AnyConfig,
        header.sim_config,
    )
    .unwrap();
    let files = confined_official_files(Path::new(&args[1]), &args[2]).unwrap();
    let profiles = load_official_profiles(&files).unwrap();
    let prepared = prepare_ranked_replay_mission(
        files,
        &profiles,
        &header.campaign,
        &header.mission_id,
        &Default::default(),
        policy,
        header.rng_seed,
        header.sim_config,
    )
    .unwrap();
    let (approved, assets) = prepared.into_engine_and_assets();
    let (mut engine, _, _, _) = approved.into_parts();
    let execution = RankedExecutionContext::new(policy);
    eprintln!("Verifying {} frames", replay.frame_count());
    resimulate_canonical_ranked_replay(engine.clone(), &assets, &replay, &execution)
        .expect("canonical verification must pass before measuring");

    let mut checkpoints = Vec::new();
    let mut saves = BTreeMap::new();
    let mut saved_payloads = Vec::new();
    let mut display = HostDisplayState::default();
    let mut terminal = false;
    for ordinal in 0..replay.frame_count() {
        if let Some(marker) = replay.save_marker_for_frame(ordinal) {
            assert_eq!(state_hash(&engine), marker.state_hash);
            let state = engine.capture_persisted_state().unwrap();
            saved_payloads.push(Checkpoint {
                ordinal,
                engine: bitcode::encode(&state),
            });
            saves.insert(ordinal, (state, terminal));
        }
        if let Some(load) = replay.load_back_for_frame(ordinal) {
            assert!(load.snapshot.is_none());
            let (state, saved_terminal) = saves.get(&load.to_frame).unwrap().clone();
            engine = Engine::restore_from_snapshot(
                &mut display,
                Engine::from_persisted_state(state),
                &assets,
            )
            .unwrap();
            terminal = saved_terminal;
        }
        if let Some(expected) = replay.hash_for_frame(ordinal) {
            assert_eq!(state_hash(&engine), expected, "ordinal {ordinal}");
        }
        if ordinal.is_multiple_of(250) {
            eprintln!("Capturing ordinal {ordinal}");
            checkpoints.push(Checkpoint {
                ordinal,
                engine: bitcode::encode(&*engine),
            });
        }
        if !terminal {
            let output = execution
                .advance_frame(
                    &mut engine,
                    &assets,
                    replay.frame(ordinal).unwrap().input.clone(),
                )
                .unwrap();
            terminal = output.game_code() != GameCode::LevelInProgress
                || output
                    .post_initialize_events
                    .as_ref()
                    .is_some_and(|events| events.game_code() != GameCode::LevelInProgress);
        }
    }
    let file = ReplayFile::from(&replay);
    let replay_bytes = bitcode::encode(&file);
    let compact =
        robin_replay_format::encode_compact(&replay, robin_replay_format::ENGINE_VERSION_HASH)
            .unwrap();
    let envelope_bytes = compact.len() - compressed(&replay_bytes, 19);
    let independent_level3: usize = checkpoints.iter().map(|c| compressed(&c.engine, 3)).sum();
    let independent_level19: usize = checkpoints.iter().map(|c| compressed(&c.engine, 19)).sum();
    let checkpoint_count = checkpoints.len();
    // Encode as one aggregate, allowing Zstd to match across adjacent snapshots.
    let bundle = bitcode::encode(&(file.clone(), checkpoints.clone()));
    let baseline = compact.len();
    let combined = envelope_bytes + compressed(&bundle, 19);
    // Include each saved state once, rather than duplicating the save cache
    // in every seek checkpoint. Browser host state is outside this estimate.
    let save_markers = saved_payloads.len();
    let with_saves = bitcode::encode(&(file, checkpoints, saved_payloads));
    println!(
        "{}",
        serde_json::json!({
            "mission": header.mission_id,
            "frames": replay.frame_count(),
            "interval_frames": 250,
            "checkpoint_count": checkpoint_count,
            "baseline_bytes": baseline,
            "combined_bytes": combined,
            "added_bytes": combined - baseline,
            "raw_bundle_bytes": bundle.len(),
            "save_markers": save_markers,
            "combined_with_save_states_bytes": envelope_bytes + compressed(&with_saves, 19),
            "independent_checkpoint_level3_bytes": independent_level3,
            "independent_checkpoint_level19_bytes": independent_level19,
        })
    );
}
