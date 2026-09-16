//! Capture a fresh official mission for the engine snapshot-storage benchmark.
//!
//! Usage: capture_mission_snapshot RAW_DATADIR LOCALE MISSION OUTPUT.json
//! The output contains an engine projection, not a playable game-save envelope.

use robin_engine::campaign::Campaign;
use robin_engine::engine::{CompressedEngineSnapshot, RankedSimulationPolicy};
use robin_engine::player_profile::DifficultyLevel;
use robin_ranked_verification::ranked_verifier::{
    confined_official_files, load_official_profiles, prepare_ranked_replay_mission,
};
use std::io::Write;
use std::path::Path;

fn main() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(capture)
        .expect("spawn snapshot capture")
        .join()
        .expect("snapshot capture panicked");
}

fn capture() {
    let args = std::env::args().collect::<Vec<_>>();
    assert_eq!(
        args.len(),
        5,
        "expected RAW_DATADIR LOCALE MISSION OUTPUT.json"
    );
    let files =
        confined_official_files(Path::new(&args[1]), &args[2]).expect("confine official assets");
    let profiles = load_official_profiles(&files).expect("load official profiles");
    let mission = profiles
        .missions
        .iter()
        .position(|profile| profile.mission_filename.eq_ignore_ascii_case(&args[3]))
        .expect("mission is in the official catalog");
    let policy = RankedSimulationPolicy::standard_medium();
    let config = policy.expected_config();
    let mut campaign = Campaign::from_profiles(&profiles, DifficultyLevel::Medium);
    campaign.current_mission_idx = Some(mission);
    campaign.add_all_to_mission_team();
    campaign.snapshot_with_simulation(0, config);
    let prepared = prepare_ranked_replay_mission(
        files,
        &profiles,
        &bitcode::encode(&campaign),
        &args[3],
        &Default::default(),
        policy,
        0,
        config,
    )
    .expect("prepare official mission");
    let (approved, assets) = prepared.into_engine_and_assets();
    let (engine, _, _, _) = approved.into_parts();
    let snapshot = CompressedEngineSnapshot::capture(&engine).expect("compress mission start");
    let mut restored = snapshot.restore(&assets).expect("restore mission start");
    let mut baseline = engine.clone();
    for frame in 0..=25 {
        assert_eq!(
            robin_engine::replay::state_hash(&baseline),
            robin_engine::replay::state_hash(&restored),
            "compressed restore diverged at frame {frame}",
        );
        if frame < 25 {
            baseline
                .advance_frame(&assets, Default::default())
                .expect("advance baseline");
            restored
                .advance_frame(&assets, Default::default())
                .expect("advance restored");
        }
    }
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[4])
        .expect("create new snapshot output");
    let mut output = std::io::BufWriter::new(output);
    serde_json::to_writer(
        &mut output,
        &serde_json::json!({
            "mission": args[3],
            "engine": engine,
        }),
    )
    .expect("write engine projection");
    output.flush().expect("flush engine projection");
}
