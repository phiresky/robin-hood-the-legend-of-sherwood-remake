//! Black-box contract tests for live-frame and timeline replay equivalence.
//!
//! Original anchors:
//! - `original-code/RHengine.cpp`, `RHEngine::PerformHourglass`: commands are
//!   resolved before the ordered simulation hourglass advances the frame.
//! - `original-code/RHgame.cpp`, `RHGame::GameLoop`: `PostInitialize` is a
//!   distinct one-shot stage after the first refresh and sound hourglass.
//!
//! The fixture therefore compares complete public frame boundaries rather
//! than inspecting the private fields that happen to implement them.

use robin_engine::campaign::Campaign;
use robin_engine::engine::{DevState, Engine, HostDisplayState, LevelAssets, SimConfig};
use robin_engine::player_command::{PlayerCommand, PlayerInput};
use robin_engine::replay::state_hash;
use robin_rs::Host;
use robin_rs::sim_timeline::{
    SimSnapshot, replay_to_frame, run_engine_tick_core, run_post_initialize_stage,
};

fn fixture_engine(assets: &mut LevelAssets) -> Engine {
    Engine::new_for_test_with_simulation(
        800.0,
        600.0,
        Campaign::default(),
        assets,
        0xD3E7_3A11_5EED_0042,
        SimConfig::default(),
    )
    .expect("construct deterministic frame-contract fixture")
}

fn command_frames() -> Vec<Vec<PlayerInput>> {
    vec![
        vec![
            PlayerInput::host(PlayerCommand::SetLockAlt(true)),
            PlayerInput::host(PlayerCommand::SetGoldenEyeMode { on: true }),
        ],
        vec![PlayerInput::host(
            PlayerCommand::SetMenToBlazonConversionMode { on: true },
        )],
        vec![PlayerInput::host(PlayerCommand::RegisterPeasantName {
            name: "deterministic fixture".into(),
        })],
        Vec::new(),
        vec![
            PlayerInput::host(PlayerCommand::SetLockAlt(false)),
            PlayerInput::host(PlayerCommand::SetGoldenEyeMode { on: false }),
        ],
    ]
}

fn advance_live_frame(
    snapshot: &mut SimSnapshot,
    host: &mut Host,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    dev: &mut DevState,
    commands: &[PlayerInput],
) {
    snapshot
        .engine
        .apply_commands(display, &mut host.input, assets, commands);
    run_engine_tick_core(host, display, assets, &mut snapshot.engine, dev);
    run_post_initialize_stage(host, display, assets, &mut snapshot.engine, dev);
    snapshot.frame += 1;
}

#[test]
fn timeline_replay_matches_the_public_live_frame_contract() {
    let mut assets = LevelAssets::new();
    let initial = fixture_engine(&mut assets);
    let frames = command_frames();

    // Deliberately use non-default presentation scratch on the live side.
    // It may change host output, but it must not change authoritative state.
    let mut live = SimSnapshot::new(0, &initial);
    let mut live_host = Host::scratch(1024.0, 768.0);
    let mut live_display = HostDisplayState::default();
    live_display.display_minimap(true, false);
    let mut live_dev = DevState::default();

    let mut checkpoint = None;
    for (frame, commands) in frames.iter().enumerate() {
        advance_live_frame(
            &mut live,
            &mut live_host,
            &mut live_display,
            &assets,
            &mut live_dev,
            commands,
        );
        if frame == 1 {
            checkpoint = Some(live.clone());
        }
    }

    let target_frame = frames.len() as u32;
    let (from_start, timing) = replay_to_frame(
        SimSnapshot::new(0, &initial),
        &assets,
        target_frame,
        |frame| frames.get(frame as usize).map(Vec::as_slice),
    )
    .expect("replay complete command journal from frame zero");

    assert_eq!(timing.replayed_frames, target_frame);
    assert_eq!(from_start.frame, live.frame);
    assert_eq!(
        from_start.engine.frame_counter(),
        live.engine.frame_counter()
    );
    assert_eq!(
        state_hash(&from_start.engine),
        state_hash(&live.engine),
        "timeline replay and the public live-frame sequence must produce identical authoritative state"
    );

    // Reconstructing only the suffix from a pre-tick checkpoint must have
    // exactly the same result as replaying the complete journal.
    let checkpoint = checkpoint.expect("captured frame-two checkpoint");
    let checkpoint_frame = checkpoint.frame;
    let (from_checkpoint, suffix_timing) =
        replay_to_frame(checkpoint, &assets, target_frame, |frame| {
            frames.get(frame as usize).map(Vec::as_slice)
        })
        .expect("replay command-journal suffix from checkpoint");

    assert_eq!(
        suffix_timing.replayed_frames,
        target_frame - checkpoint_frame
    );
    assert_eq!(
        state_hash(&from_checkpoint.engine),
        state_hash(&live.engine),
        "checkpoint replay must be equivalent to uninterrupted live advancement"
    );
}
