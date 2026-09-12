//! Headless verification of the rollback determinism checker.
//!
//! Loads a level, ticks the engine forward, round-trips the live warm engine
//! through native bitcode, and every frame after warm-up rewinds 25 frames +
//! re-simulates. Prints whether the replayed state matches the live state.
//!
//! This exercises the same `state_hash` + `advance_frame` path
//! the in-game rollback checker uses, but without rendering, input, or UI
//! so it can run non-interactively from CI.
//!
//! Usage:
//!   ROBINHOOD_DATA_DIR=datadirs/demo_leicester_ecoste \
//!     cargo run --example verify_rollback
#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::collections::VecDeque;
use std::path::Path;

use anyhow::Context;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::replay::state_hash;
use robin_rs::Host;

const WARMUP_FRAMES: u32 = 30;
const TOTAL_FRAMES: u32 = 100;
const WINDOW: usize = 25;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    if let Ok(dir) = std::env::var("ROBINHOOD_DATA_DIR") {
        std::env::set_current_dir(&dir).expect("chdir to ROBINHOOD_DATA_DIR");
    }
    robin_rs::main_entry::register_language_data_paths_for_tool();

    // Load the real profile pool from the legacy CPF (mirrors main_entry).
    let mut pm = robin_engine::profiles::ProfileManager::new();
    let mut cpf = robin_engine::sbfile::SbFile::open("Data/Configuration/profile.cpf")
        .expect("open profile.cpf");
    pm.load_all_legacy_cpf(&mut cpf).expect("parse profile.cpf");
    let profiles = std::sync::Arc::new(pm);

    let mut campaign = robin_engine::campaign::Campaign::new();
    campaign.reset(
        &profiles,
        robin_engine::player_profile::DifficultyLevel::Medium,
    );
    campaign.create_gang_from_pcs(
        "RJMT",
        &profiles,
        robin_engine::player_profile::DifficultyLevel::Medium,
    );
    campaign.add_all_to_mission_team();
    campaign.current_mission_idx = Some(1);

    let mut assets = LevelAssets::new();
    assets.sprite_scriptor =
        std::sync::Arc::new(robin_engine::sprite_script::SpriteScriptor::legacy_tool());
    assets.profile_manager = profiles.clone();
    let mut text_res = robin_assets::resource_manager::ResourceManager::legacy_tool();
    text_res
        .attach_resource_file("Data/Text/Level.res")
        .expect("load localized mission names");
    (assets.peasant_firstnames, assets.peasant_surnames) =
        robin_rs::game_session::load_peasant_name_pool(&mut text_res)
            .expect("decode localized peasant names");
    assets.fixed_vip_names = robin_rs::game_session::load_fixed_vip_name_map(&mut text_res)
        .expect("decode localized VIP names");
    let mut host = Host::scratch(1024.0, 768.0);
    if let Err(e) = host
        .frontend
        .resources
        .frame_holder_before_publication_mut()
        .initialize_sprite_bank(".")
    {
        tracing::warn!("sprite bank: {e}");
    }
    assets.bank_signature = host.frontend.resources.frame_holder().signature();

    // Load the mission script before level init so the level loader can
    // resolve `current_mission_idx`'s script from LevelAssets.
    let mission_name = campaign.current_mission_idx.map(|i| {
        campaign.missions[i]
            .profile(&profiles)
            .mission_filename
            .clone()
    });
    if let Some(name) = mission_name {
        let path = format!("Data/Levels/{name}.scb");
        let program = load_mission_program(Path::new(&path))?;
        let mut m = std::collections::BTreeMap::new();
        m.insert(name, std::sync::Arc::new(program));
        assets.scripts.mission_programs = std::sync::Arc::new(m);
    }

    let loaded = robin_engine::engine::level_loading::load_mission_for_campaign(
        &campaign,
        &profiles,
        "Data/Levels",
        &mut |_| {},
    )
    .expect("load mission");

    let mut engine = Engine::new(robin_engine::engine::EngineArgs {
        campaign,
        level: robin_engine::engine::LevelLoadArgs {
            assets: &mut assets,
            level_directory: "Data/Levels",
            progress: &mut |_| {},
            loaded,
            // Rollback verification doesn't need a decoded bitmap; use
            // placeholder dims large enough to pass `is_position_authorized`.
            bg_pixel_dims: (4096.0, 4096.0),
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: Vec::new(),
        rng_seed: 0,
        original_rng_replay: None,
        sim_config: robin_engine::engine::SimConfig::default(),
    })
    .expect("load level");

    let mut history: VecDeque<(Engine, LevelAssets)> = VecDeque::with_capacity(WINDOW + 1);

    let mut desyncs = 0usize;
    let mut checks = 0usize;

    for frame in 0..TOTAL_FRAMES {
        if frame == WARMUP_FRAMES {
            let encoded = engine.encode_native_snapshot();
            let restored = Engine::decode_native_snapshot(&encoded)
                .expect("decode warm mission engine snapshot");
            assert_eq!(
                state_hash(&engine),
                state_hash(&restored),
                "native bitcode changed warm mission state"
            );
            tracing::info!(
                bytes = encoded.len(),
                "native engine snapshot round-trip passed"
            );
            engine = Engine::adopt_authoritative_snapshot(restored, &assets)
                .expect("adopt warm mission engine snapshot");
        }

        // Snapshot pre-tick state.
        history.push_back((engine.clone(), assets.clone()));
        if history.len() > WINDOW {
            history.pop_front();
        }

        engine
            .advance_frame(
                &assets,
                robin_engine::engine::SimulationFrameInput::default().with_post_initialize(true),
            )
            .expect("advance live frame");

        if frame >= WARMUP_FRAMES && history.len() == WINDOW {
            // Re-simulate from the oldest snapshot forward WINDOW ticks
            // and compare the resulting state to the live engine.
            let (start_engine, start_assets) = &history[0];
            let mut sim_engine = start_engine.clone();
            let sim_assets = start_assets.clone();
            for _ in 0..WINDOW {
                sim_engine
                    .advance_frame(
                        &sim_assets,
                        robin_engine::engine::SimulationFrameInput::default()
                            .with_post_initialize(true),
                    )
                    .expect("advance reconstructed frame");
            }

            let live = state_hash(&engine);
            let replayed = state_hash(&sim_engine);
            checks += 1;
            if live != replayed {
                desyncs += 1;
                tracing::error!("DESYNC frame {frame}: live {live:016x} replayed {replayed:016x}");
                if desyncs == 1 {
                    let live_json = serde_json::to_value(&engine).unwrap();
                    let rep_json = serde_json::to_value(&sim_engine).unwrap();
                    diff_json("", &live_json, &rep_json);
                }
            }
        }
    }

    tracing::info!("checked {checks} frames, {desyncs} desyncs");
    if desyncs != 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn load_mission_program(
    path: &Path,
) -> anyhow::Result<robin_engine::script_manager::ScriptProgram> {
    let resolved =
        robin_engine::sbfile::resolve_case_insensitive(path).unwrap_or_else(|| path.to_path_buf());
    let scb = robin_assets::scb::parse_file(&resolved)
        .with_context(|| format!("load mission script {}", resolved.display()))?;
    robin_engine::script_manager::ScriptProgram::from_scb(scb)
        .with_context(|| format!("prepare mission script bytecode {}", resolved.display()))
}

fn diagnostic_preview(value: &str) -> String {
    if value.len() <= 80 {
        return value.to_owned();
    }
    let end = value.floor_char_boundary(80);
    format!("{}…", &value[..end])
}

/// Walk two JSON values in parallel and log every leaf where they
/// differ.  Used to home in on which Engine field is the source of a
/// rollback desync.
fn diff_json(path: &str, a: &serde_json::Value, b: &serde_json::Value) {
    use serde_json::Value;
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            let mut keys: Vec<&String> = am.keys().chain(bm.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                diff_json(
                    &p,
                    am.get(k).unwrap_or(&Value::Null),
                    bm.get(k).unwrap_or(&Value::Null),
                );
            }
        }
        (Value::Array(av), Value::Array(bv)) => {
            let n = av.len().max(bv.len());
            for i in 0..n {
                let p = format!("{path}[{i}]");
                diff_json(
                    &p,
                    av.get(i).unwrap_or(&Value::Null),
                    bv.get(i).unwrap_or(&Value::Null),
                );
            }
        }
        _ => {
            let sa = a.to_string();
            let sb = b.to_string();
            let sa = diagnostic_preview(&sa);
            let sb = diagnostic_preview(&sb);
            tracing::warn!("DIFF {path}: live={sa} replayed={sb}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_mission_script_reports_path_and_io_cause() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.scb");
        let error = load_mission_program(&path).unwrap_err();
        assert!(error.to_string().contains(&path.display().to_string()));
        match error.downcast_ref::<robin_assets::scb::Error>().unwrap() {
            robin_assets::scb::Error::Io(cause) => {
                assert_eq!(cause.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected file read failure, got {other:?}"),
        }
    }

    #[test]
    fn malformed_mission_script_reports_resolved_path_and_parse_cause() {
        let dir = tempfile::tempdir().unwrap();
        let actual = dir.path().join("Mission.SCB");
        std::fs::write(&actual, b"not-scb!").unwrap();
        let requested = dir.path().join("mission.scb");
        let error = load_mission_program(&requested).unwrap_err();
        assert!(error.to_string().contains(&actual.display().to_string()));
        assert!(matches!(
            error.downcast_ref::<robin_assets::scb::Error>(),
            Some(robin_assets::scb::Error::BadMagic { .. })
        ));
        assert!(format!("{error:#}").contains("not a .scb file"));
    }

    #[test]
    fn diagnostic_preview_preserves_short_text_and_ascii_byte_limit() {
        for value in ["", "Robin", "é"] {
            assert_eq!(diagnostic_preview(value), value);
        }
        let boundary = "a".repeat(80);
        assert_eq!(diagnostic_preview(&boundary), boundary);
        assert_eq!(diagnostic_preview(&"a".repeat(81)), format!("{boundary}…"));
    }

    #[test]
    fn diagnostic_preview_does_not_split_multibyte_text_at_byte_eighty() {
        let prefix = "a".repeat(79);
        assert_eq!(
            diagnostic_preview(&format!("{prefix}é!")),
            format!("{prefix}…")
        );
        // JSON adds an opening quote, putting the multibyte character across byte 80.
        let live = serde_json::json!(format!("{}é!", "a".repeat(78)));
        diff_json("text", &live, &serde_json::json!("different"));
    }
}
