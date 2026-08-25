//! Black-box contract tests for the current command + hourglass transaction
//! and timeline replay equivalence.
//!
//! Original anchors:
//! - `original-code/RHengine.cpp`, `RHEngine::PerformHourglass`: commands are
//!   resolved before the ordered simulation hourglass advances the frame.
//! - `original-code/RHgame.cpp`, `RHGame::GameLoop`: `PostInitialize` is a
//!   distinct one-shot stage after the first refresh and sound hourglass.
//!
//! This is deliberately not a complete Original host-frame model. The current
//! journal cannot represent fact-only/paused host iterations, a closed body
//! gate, or commands dispatched after `PerformHourglass` by nested refresh or
//! popup work. The fixture covers only the command-first, body-allowed
//! transaction that both public APIs currently support, plus the one-shot
//! post-initialize normalization required by replay.

use robin_engine::campaign::Campaign;
use robin_engine::engine::{
    DevState, Engine, HostDisplayState, LevelAssets, SimConfig, SimulationFrameInput,
};
use robin_engine::player_command::{PlayerCommand, PlayerInput};
use robin_engine::replay::state_hash;
use robin_rs::Host;
use robin_rs::sim_timeline::{SimSnapshot, replay_to_frame, run_post_initialize_stage};

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
            // A non-commuting pair makes command order observable at this
            // prefix even though later frames change the flag again.
            PlayerInput::host(PlayerCommand::SetLockAlt(false)),
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

fn advance_supported_facade_frame(
    snapshot: &mut SimSnapshot,
    host: &mut Host,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    dev: &mut DevState,
    commands: &[PlayerInput],
) {
    // TODO(architecture): replace this command-only construction once the
    // timeline journal can retain external facts, the body-gate decision, and
    // post-hourglass commands from nested host work.
    let frame_input = SimulationFrameInput::from_player_inputs(commands.to_vec());
    assert!(frame_input.external_facts.is_empty());
    assert!(frame_input.simulation_body_allowed);
    let output = snapshot
        .engine
        .advance_frame(display, &mut host.input, assets, dev, frame_input)
        .expect("advance through the public command + hourglass transaction");
    assert_eq!(output.frame_before, snapshot.frame);
    assert!(
        output.frame_after == output.frame_before || output.frame_after == output.frame_before + 1,
        "the hourglass may either advance or close its presentation/body gate"
    );
    assert_eq!(output.frame_after, snapshot.engine.frame_counter());
    assert_eq!(output.state_hash, state_hash(&snapshot.engine));
    host.apply_side_effects(output.events.into_side_effects());
    run_post_initialize_stage(host, display, assets, &mut snapshot.engine, dev);
    snapshot.frame += 1;
}

#[test]
fn timeline_replay_matches_the_supported_public_hourglass_transaction() {
    let mut assets = LevelAssets::new();
    let initial = fixture_engine(&mut assets);
    let frames = command_frames();

    // Deliberately use non-default presentation scratch on the facade side.
    // It may change host output, but it must not change authoritative state.
    let mut facade = SimSnapshot::new(0, &initial);
    let mut facade_host = Host::scratch(1024.0, 768.0);
    let mut facade_display = HostDisplayState::default();
    facade_display.display_minimap(true, false);
    let mut facade_dev = DevState::default();

    let mut checkpoint = None;
    let mut facade_prefix_hashes = Vec::new();
    for (frame, commands) in frames.iter().enumerate() {
        advance_supported_facade_frame(
            &mut facade,
            &mut facade_host,
            &mut facade_display,
            &assets,
            &mut facade_dev,
            commands,
        );
        if frame == 1 {
            checkpoint = Some(facade.clone());
        }
        facade_prefix_hashes.push(state_hash(&facade.engine));
    }

    // Check every prefix, not only the final state: several fixture commands
    // are deliberately reversed by later frames and a final-only comparison
    // would let an early ordering regression cancel itself out.
    for (index, expected_hash) in facade_prefix_hashes.iter().enumerate() {
        let target_frame = index as u32 + 1;
        let (prefix, _) = replay_to_frame(
            SimSnapshot::new(0, &initial),
            &assets,
            target_frame,
            |frame| frames.get(frame as usize).map(Vec::as_slice),
        )
        .expect("replay command-journal prefix from frame zero");
        assert_eq!(
            state_hash(&prefix.engine),
            *expected_hash,
            "public hourglass transaction and replay diverged after frame {target_frame}"
        );
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
    assert_eq!(from_start.frame, facade.frame);
    assert_eq!(
        from_start.engine.frame_counter(),
        facade.engine.frame_counter()
    );
    assert_eq!(
        state_hash(&from_start.engine),
        state_hash(&facade.engine),
        "timeline replay and the supported public hourglass transaction must produce identical authoritative state"
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
        state_hash(&facade.engine),
        "checkpoint replay must be equivalent to uninterrupted supported transactions"
    );
}

#[test]
fn post_hourglass_quit_command_cannot_be_replayed_as_a_pre_hourglass_command() {
    let mut assets = LevelAssets::new();
    let initial = fixture_engine(&mut assets);
    let quit = PlayerInput::host(PlayerCommand::QuitMissionRequested);

    let mut before_hourglass = SimSnapshot::new(0, &initial);
    let mut before_host = Host::default();
    let mut before_display = HostDisplayState::default();
    let mut before_dev = DevState::default();
    advance_supported_facade_frame(
        &mut before_hourglass,
        &mut before_host,
        &mut before_display,
        &assets,
        &mut before_dev,
        std::slice::from_ref(&quit),
    );

    let mut after_hourglass = SimSnapshot::new(0, &initial);
    let mut after_host = Host::default();
    let mut after_display = HostDisplayState::default();
    let mut after_dev = DevState::default();
    advance_supported_facade_frame(
        &mut after_hourglass,
        &mut after_host,
        &mut after_display,
        &assets,
        &mut after_dev,
        &[],
    );
    after_hourglass.engine.apply_commands(
        &mut after_display,
        &mut after_host.input,
        &assets,
        std::slice::from_ref(&quit),
    );

    assert_ne!(
        state_hash(&before_hourglass.engine),
        state_hash(&after_hourglass.engine),
        "QuitMissionRequested placement around the hourglass is authoritative"
    );

    let (journal_replay, _) = replay_to_frame(SimSnapshot::new(0, &initial), &assets, 1, |_| {
        Some(std::slice::from_ref(&quit))
    })
    .expect("replay a command-only frame-zero journal");
    assert_eq!(
        state_hash(&journal_replay.engine),
        state_hash(&before_hourglass.engine)
    );
    assert_ne!(
        state_hash(&journal_replay.engine),
        state_hash(&after_hourglass.engine),
        "a post-hourglass command needs an explicit journal phase instead of being folded into frame-zero inputs"
    );
}
