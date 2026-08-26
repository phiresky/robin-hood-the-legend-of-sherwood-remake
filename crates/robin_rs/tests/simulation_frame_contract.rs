//! Black-box contracts for the authoritative frame transaction and timeline
//! replay equivalence.
//!
//! Original anchors:
//! - `original-code/RHengine.cpp`, `RHEngine::PerformHourglass`: commands are
//!   resolved before the ordered simulation hourglass advances the frame.
//! - `original-code/RHgame.cpp`, `RHGame::GameLoop`: `PostInitialize` is a
//!   distinct one-shot stage after the first refresh and sound hourglass.
//!
//! The journal retains the complete transaction: external facts, pre/late
//! commands, the hourglass/body gates, and the PostInitialize boundary.

use robin_engine::campaign::Campaign;
use robin_engine::engine::{
    DevState, Engine, ExternalAction, ExternalActionResult, FrameConsoleResponse, HostDisplayState,
    HostEvent, LevelAssets, SimConfig, SimulationFrameInput,
};
use robin_engine::player_command::{PlayerCommand, PlayerInput};
use robin_engine::replay::state_hash;
use robin_rs::Host;
use robin_rs::sim_timeline::{
    SimSnapshot, replay_authoritative_frame, replay_frames_to_frame, run_engine_frame_core,
};

#[test]
fn production_drivers_do_not_bypass_advance_frame() {
    fn visit(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read source directory") {
            let path = entry.expect("source directory entry").path();
            if path.is_dir() {
                visit(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    visit(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    visit(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples"),
        &mut files,
    );
    let forbidden = [
        ".apply_command(",
        ".apply_commands(",
        ".perform_hourglass(",
        ".perform_hourglass_with_body_gate(",
        ".perform_post_initialize(",
        ".apply_external_director_completion(",
        ".queue_replay_resolved_exclamations(",
        ".apply_replay_sound_boundary(",
        ".call_external_native_with_this(",
        ".run_cheat_string(",
        ".run_console_command(",
        ".try_ezekiel_instakill(",
        ".send_simple_message(",
        ".replace_campaign_from_console(",
        ".inject_recorded_drop_ale_route(",
    ];
    let mut violations = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path).expect("read Rust source");
        for needle in forbidden {
            if source.contains(needle) {
                violations.push(format!("{} contains {needle}", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "production simulation mutations must enter Engine::advance_frame:\n{}",
        violations.join("\n")
    );
}

#[test]
fn production_reconstruction_does_not_fabricate_host_state() {
    fn function_source<'a>(source: &'a str, name: &str) -> &'a str {
        let signature = format!("fn {name}");
        let start = source
            .find(&signature)
            .unwrap_or_else(|| panic!("missing reconstruction function {name}"));
        let body_start = source[start..]
            .find('{')
            .map(|offset| start + offset)
            .unwrap_or_else(|| panic!("missing body for reconstruction function {name}"));
        let mut depth = 0_u32;
        for (offset, byte) in source.as_bytes()[body_start..].iter().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[start..=body_start + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("unterminated body for reconstruction function {name}");
    }

    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let reconstruction_functions = [
        ("src/rewind.rs", "rewind_to"),
        ("src/sim_timeline.rs", "replay_to_frame"),
        ("src/sim_timeline.rs", "replay_frames_to_frame"),
        ("src/sim_timeline.rs", "replay_one_frame"),
        ("src/sim_timeline.rs", "replay_authoritative_frame"),
        ("src/sim_timeline.rs", "replay_authoritative_frame_profiled"),
        (
            "src/game_session/multiplayer.rs",
            "rewind_from_recent_timeline_history",
        ),
        ("src/rollback_checker.rs", "run"),
    ];
    let forbidden = [
        "Host::",
        "HostDisplayState",
        "InputState",
        "DevState",
        "run_engine_frame_core",
    ];
    let mut violations = Vec::new();
    for (relative_path, function) in reconstruction_functions {
        let source = std::fs::read_to_string(manifest.join(relative_path))
            .unwrap_or_else(|error| panic!("read {relative_path}: {error}"));
        let function_source = function_source(&source, function);
        for needle in forbidden {
            if function_source.contains(needle) {
                violations.push(format!("{relative_path}::{function} contains {needle}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "production reconstruction must not fabricate or consume host state:\n{}",
        violations.join("\n")
    );

    let timeline_source = std::fs::read_to_string(manifest.join("src/sim_timeline.rs"))
        .expect("read timeline source");
    let authoritative = function_source(&timeline_source, "replay_authoritative_frame_profiled");
    assert!(
        authoritative.contains(".advance_frame("),
        "authoritative reconstruction must advance Engine directly"
    );

    let production_paths = [
        ("src/rewind.rs", "rewind_to", "replay_authoritative_frame"),
        (
            "src/game_session/multiplayer.rs",
            "rewind_from_recent_timeline_history",
            "replay_authoritative_frame_profiled",
        ),
        ("src/rollback_checker.rs", "run", "replay_frames_to_frame"),
    ];
    for (relative_path, function, complete_frame_replay) in production_paths {
        let source = std::fs::read_to_string(manifest.join(relative_path))
            .unwrap_or_else(|error| panic!("read {relative_path}: {error}"));
        assert!(
            function_source(&source, function).contains(complete_frame_replay),
            "{relative_path}::{function} must reconstruct complete recorded SimulationFrameInput values through {complete_frame_replay}"
        );
    }
}

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

#[test]
fn no_hourglass_admission_applies_commands_without_advancing_the_engine_clock() {
    let mut assets = LevelAssets::new();
    let mut engine = fixture_engine(&mut assets);
    let before = engine.frame_counter();

    let output = engine
        .advance_frame(
            &assets,
            SimulationFrameInput::new(vec![PlayerCommand::SetGoldenEyeMode { on: true }.into()])
                .with_hourglass(false),
        )
        .expect("admit presentation-only frame");

    assert!(!output.hourglass_ran);
    assert_eq!(output.frame_before, before);
    assert_eq!(output.frame_after, before);
    assert!(engine.get_golden_eye_mode());
}

#[test]
fn reconstruction_surfaces_host_events_as_typed_output() {
    let mut assets = LevelAssets::new();
    let initial = fixture_engine(&mut assets);
    let mut snapshot = SimSnapshot::new(0, &initial);
    let frame = SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
        PlayerCommand::MouseRightDown,
    )]);

    let replayed = replay_authoritative_frame(&mut snapshot, &assets, &frame);

    assert_eq!(snapshot.frame, 1);
    assert!(
        replayed
            .output
            .events
            .side_effects()
            .host_events
            .iter()
            .any(|event| matches!(event, HostEvent::SetRightMouseDown { down: true }))
    );
}

#[test]
fn admitted_host_action_is_replayable() {
    let mut assets = LevelAssets::new();
    let initial = fixture_engine(&mut assets);
    let mut replacement_campaign = Campaign::default();
    replacement_campaign.set_ares(2);
    let frame = SimulationFrameInput::no_hourglass().with_external_actions(vec![
        ExternalAction::ConsoleCommand {
            command: robin_engine::console::ConsoleCommand::Goldeneye,
            selected_view_element: None,
        },
        ExternalAction::SimpleMessage {
            message: robin_engine::messenger::SimpleMessage::LockAlt,
        },
        ExternalAction::ReplaceCampaign {
            campaign: replacement_campaign,
        },
    ]);
    let mut live = SimSnapshot::new(0, &initial);
    let mut host = Host::default();
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let output = run_engine_frame_core(
        &mut host,
        &mut display,
        &assets,
        &mut live.engine,
        &mut dev,
        frame.clone(),
    );
    assert!(matches!(
        output.external_action_results.as_slice(),
        [
            ExternalActionResult::ConsoleCommand {
                response: FrameConsoleResponse::Ok(_),
                selected_view_element: None,
            },
            ExternalActionResult::SimpleMessage,
            ExternalActionResult::ReplaceCampaign
        ]
    ));
    assert!(live.engine.get_golden_eye_mode());
    assert_eq!(live.engine.campaign().get_ares(), 2);

    let (replayed, _) =
        replay_frames_to_frame(SimSnapshot::new(0, &initial), &assets, 1, |_| Some(&frame))
            .expect("replay host action");
    assert_eq!(state_hash(&replayed.engine), state_hash(&live.engine));
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

fn advance_authoritative_frame(
    snapshot: &mut SimSnapshot,
    host: &mut Host,
    display: &mut HostDisplayState,
    assets: &LevelAssets,
    dev: &mut DevState,
    frame_input: SimulationFrameInput,
) {
    let output = run_engine_frame_core(
        host,
        display,
        assets,
        &mut snapshot.engine,
        dev,
        frame_input,
    );
    assert_eq!(output.frame_before, snapshot.frame);
    assert!(
        output.frame_after == output.frame_before || output.frame_after == output.frame_before + 1,
        "the hourglass may either advance or close its presentation/body gate"
    );
    assert_eq!(output.frame_after, snapshot.engine.frame_counter());
    assert_eq!(output.state_hash, state_hash(&snapshot.engine));
    snapshot.frame += 1;
}

#[test]
fn timeline_replay_matches_the_supported_public_hourglass_transaction() {
    let mut assets = LevelAssets::new();
    let initial = fixture_engine(&mut assets);
    let frames = command_frames()
        .into_iter()
        .map(|commands| {
            SimulationFrameInput::from_player_inputs(commands).with_post_initialize(true)
        })
        .collect::<Vec<_>>();

    // Deliberately use non-default presentation scratch on the facade side.
    // It may change host output, but it must not change authoritative state.
    let mut facade = SimSnapshot::new(0, &initial);
    let mut facade_host = Host::scratch(1024.0, 768.0);
    let mut facade_display = HostDisplayState::default();
    facade_display.display_minimap(true, false);
    let mut facade_dev = DevState::default();

    let mut checkpoint = None;
    let mut facade_prefix_hashes = Vec::new();
    for (frame, input) in frames.iter().enumerate() {
        advance_authoritative_frame(
            &mut facade,
            &mut facade_host,
            &mut facade_display,
            &assets,
            &mut facade_dev,
            input.clone(),
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
        let (prefix, _) = replay_frames_to_frame(
            SimSnapshot::new(0, &initial),
            &assets,
            target_frame,
            |frame| frames.get(frame as usize),
        )
        .expect("replay command-journal prefix from frame zero");
        assert_eq!(
            state_hash(&prefix.engine),
            *expected_hash,
            "public hourglass transaction and replay diverged after frame {target_frame}"
        );
    }

    let target_frame = frames.len() as u32;
    let (from_start, timing) = replay_frames_to_frame(
        SimSnapshot::new(0, &initial),
        &assets,
        target_frame,
        |frame| frames.get(frame as usize),
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
        replay_frames_to_frame(checkpoint, &assets, target_frame, |frame| {
            frames.get(frame as usize)
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
    advance_authoritative_frame(
        &mut before_hourglass,
        &mut before_host,
        &mut before_display,
        &assets,
        &mut before_dev,
        SimulationFrameInput::from_player_inputs(vec![quit.clone()]).with_post_initialize(true),
    );

    let mut after_hourglass = SimSnapshot::new(0, &initial);
    let mut after_host = Host::default();
    let mut after_display = HostDisplayState::default();
    let mut after_dev = DevState::default();
    advance_authoritative_frame(
        &mut after_hourglass,
        &mut after_host,
        &mut after_display,
        &assets,
        &mut after_dev,
        SimulationFrameInput::default()
            .with_post_commands(vec![quit.clone().into()])
            .with_post_initialize(true),
    );

    assert_ne!(
        state_hash(&before_hourglass.engine),
        state_hash(&after_hourglass.engine),
        "QuitMissionRequested placement around the hourglass is authoritative"
    );

    let before_frame =
        SimulationFrameInput::from_player_inputs(vec![quit]).with_post_initialize(true);
    let (journal_replay, _) =
        replay_frames_to_frame(SimSnapshot::new(0, &initial), &assets, 1, |_| {
            Some(&before_frame)
        })
        .expect("replay an authoritative frame-zero journal");
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
