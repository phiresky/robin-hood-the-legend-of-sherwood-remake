//! Headless verification of the rollback determinism checker.
//!
//! Loads a level, ticks the engine forward, and every frame after
//! warm-up rewinds 25 frames + re-simulates.  Prints whether the
//! replayed state matches the live state.
//!
//! This exercises the same `state_hash` + `perform_hourglass` path
//! the in-game rollback checker uses, but without rendering, input, or UI
//! so it can run non-interactively from CI.
//!
//! Usage:
//!   ROBINHOOD_DATA_DIR=datadirs/demo_leicester_ecoste \
//!     cargo run --example verify_rollback
#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::collections::VecDeque;

use robin_engine::engine::{DevState, Engine, HostDisplayState, LevelAssets};
use robin_engine::replay::state_hash;
use robin_rs::Host;

const WARMUP_FRAMES: u32 = 30;
const TOTAL_FRAMES: u32 = 100;
const WINDOW: usize = 25;

fn main() {
    tracing_subscriber::fmt::init();

    if let Ok(dir) = std::env::var("ROBINHOOD_DATA_DIR") {
        std::env::set_current_dir(&dir).expect("chdir to ROBINHOOD_DATA_DIR");
    }

    // Load the real profile pool from the legacy CPF (mirrors main_entry).
    let mut pm = robin_engine::profiles::ProfileManager::new();
    let mut cpf = robin_engine::sbfile::SbFile::open(
        "Data/Configuration/profile.cpf",
        robin_engine::sbfile::SB_FILE_READ,
    )
    .expect("open profile.cpf");
    pm.load_all_legacy_cpf(&mut cpf).expect("parse profile.cpf");
    let profiles = std::sync::Arc::new(pm);

    let mut campaign = robin_engine::campaign::Campaign::new();
    campaign.reset(&profiles);
    campaign.create_gang_from_pcs("RJMT", &profiles);
    campaign.add_all_to_mission_team();
    campaign.current_mission_idx = Some(1);

    let mut assets = LevelAssets::new();
    assets.profile_manager = profiles.clone();
    let mut host = Host::scratch(1024.0, 768.0);
    if let Err(e) = host.frame_holder_mut().initialize_sprite_bank(".") {
        tracing::warn!("sprite bank: {e}");
    }
    assets.bank_signature = host.frame_holder.signature();

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
        let resolved = robin_engine::sbfile::resolve_case_insensitive(std::path::Path::new(&path))
            .unwrap_or_else(|| std::path::PathBuf::from(&path));
        if let Ok(b) = std::fs::read(&resolved)
            && let Ok(scb) = robin_assets::scb::parse_bytes(&b)
        {
            let mut m = std::collections::BTreeMap::new();
            m.insert(
                name,
                std::sync::Arc::new(robin_engine::script_manager::ScriptProgram::from_scb(scb)),
            );
            assets.mission_script_programs = std::sync::Arc::new(m);
        }
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
        goldeneye: false,
    })
    .expect("load level");

    let mut dev = DevState::new();
    let mut display = HostDisplayState::default();

    let mut history: VecDeque<(Engine, LevelAssets, DevState, HostDisplayState)> =
        VecDeque::with_capacity(WINDOW + 1);

    let mut desyncs = 0usize;
    let mut checks = 0usize;

    for frame in 0..TOTAL_FRAMES {
        // Snapshot pre-tick state.
        history.push_back((engine.clone(), assets.clone(), dev.clone(), display.clone()));
        if history.len() > WINDOW {
            history.pop_front();
        }

        engine.perform_hourglass(&mut display, &assets, &mut dev);
        let _ = engine.perform_post_initialize(&mut display, &assets);

        if frame >= WARMUP_FRAMES && history.len() == WINDOW {
            // Re-simulate from the oldest snapshot forward WINDOW ticks
            // and compare the resulting state to the live engine.
            let (start_engine, start_assets, start_dev, start_display) = &history[0];
            let mut sim_engine = start_engine.clone();
            let sim_assets = start_assets.clone();
            let mut sim_dev = start_dev.clone();
            let mut sim_display = start_display.clone();
            for _ in 0..WINDOW {
                sim_engine.perform_hourglass(&mut sim_display, &sim_assets, &mut sim_dev);
                let _ = sim_engine.perform_post_initialize(&mut sim_display, &sim_assets);
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
            let sa = if sa.len() > 80 {
                format!("{}…", &sa[..80])
            } else {
                sa
            };
            let sb = if sb.len() > 80 {
                format!("{}…", &sb[..80])
            } else {
                sb
            };
            tracing::warn!("DIFF {path}: live={sa} replayed={sb}");
        }
    }
}
