//! Black-box contracts for the authoritative frame transaction and timeline
//! replay equivalence.
//!
//! Original anchors:
//! - Original-game engine update: commands are
//!   resolved before the ordered simulation hourglass advances the frame.
//! - Original-game main loop: post-initialization is a
//!   distinct one-shot stage after the first refresh and sound hourglass.
//!
//! The journal retains the complete transaction: external facts, pre/late
//! commands, the hourglass/body gates, and the PostInitialize boundary.

use robin_engine::campaign::Campaign;
use robin_engine::engine::{
    DevState, Engine, ExternalAction, ExternalActionResult, FrameConsoleResponse, HostEvent,
    LevelAssets, SimConfig, SimulationFrameInput,
};
use robin_engine::player_command::{PlayerCommand, PlayerInput};
use robin_engine::replay::state_hash;
use robin_rs::Host;
use robin_rs::sim_timeline::{
    ReplayError, ReplayFrameResult, SimSnapshot, replay_authoritative_frame,
    replay_authoritative_frame_profiled, replay_frames_to_frame, run_engine_frame_core,
};

// Low-level mutation access is checked by the compiler in Engine's doctests,
// backed by engine_facade_contract's AST allowlist (including future methods).
// Do not reinstate method-name substring scans here: a comment or an unrelated
// type's identically named method says nothing about Engine's capabilities.
#[path = "support/phase_capabilities.rs"]
mod phase_capabilities;
#[path = "support/reconstruction_contract.rs"]
mod reconstruction_contract;

#[test]
fn production_input_and_presentation_views_only_borrow_engine_queries() {
    phase_capabilities::assert_production_views_are_readonly();
}

fn fixture_engine(assets: &mut LevelAssets) -> Engine {
    Engine::new_for_test_with_simulation(
        800.0,
        600.0,
        Campaign::default(),
        assets,
        0xD3E7_3A11_5EED_0042,
        SimConfig {
            // Empty LevelAssets deliberately has no mission program. This
            // fixture tests frame admission/reconstruction, not script loading.
            script_enabled: false,
            ..SimConfig::default()
        },
    )
    .expect("construct deterministic frame-contract fixture")
}

#[test]
fn replay_boundary_is_callable_with_only_authoritative_capabilities() {
    // Real function coercions, not source spelling: neither boundary may grow
    // a hidden required host/display/input argument. The behavioral contract
    // below exercises both functions, so this is not an unused mock signature.
    type ReplayBoundary =
        fn(&mut SimSnapshot, &LevelAssets, &SimulationFrameInput) -> ReplayFrameResult;
    let boundaries: [ReplayBoundary; 2] = [
        replay_authoritative_frame,
        replay_authoritative_frame_profiled,
    ];
    let mut assets = LevelAssets::new();
    let initial = fixture_engine(&mut assets);
    let frames = [
        SimulationFrameInput::no_hourglass()
            .with_external_actions(vec![ExternalAction::SimpleMessage {
                message: robin_engine::messenger::SimpleMessage::LockAlt,
            }])
            .with_post_commands(vec![PlayerCommand::MouseRightDown.into()]),
        SimulationFrameInput::from_player_inputs(vec![PlayerInput::host(
            PlayerCommand::SetGoldenEyeMode { on: true },
        )])
        .with_post_external_actions(vec![ExternalAction::ConsoleCommand {
            command: robin_engine::console::ConsoleCommand::Goldeneye,
            selected_view_element: None,
        }])
        .with_post_commands(vec![PlayerCommand::MouseRightUp.into()])
        .with_post_initialize(true),
    ];
    for replay in boundaries {
        let mut direct = initial.clone();
        let mut reconstructed = SimSnapshot::new(0, &initial);
        for (index, frame) in frames.iter().enumerate() {
            let expected = direct
                .advance_frame(&assets, frame.clone())
                .expect("direct admission");
            let recorded = serde_json::from_slice(
                &serde_json::to_vec(frame).expect("serialize complete frame record"),
            )
            .expect("deserialize complete frame record");
            let actual = replay(&mut reconstructed, &assets, &recorded);
            // Compare the whole typed output, not merely the final state hash:
            // dropping post-boundary host events or action acknowledgements
            // must fail even if no authoritative state field changes.
            assert_eq!(
                serde_json::to_value(&actual.output).unwrap(),
                serde_json::to_value(&expected).unwrap()
            );
            assert_eq!(state_hash(&reconstructed.engine), state_hash(&direct));
            assert_eq!(reconstructed.frame, index as u32 + 1);
            assert_eq!(actual.output.external_action_results.len(), 1);
            if index == 0 {
                assert!(!actual.output.hourglass_ran);
                assert_eq!(actual.output.frame_after, actual.output.frame_before);
                assert!(
                    !actual
                        .output
                        .post_boundary_events
                        .side_effects()
                        .host_events
                        .is_empty()
                );
            } else {
                assert!(!reconstructed.engine.get_golden_eye_mode());
                // A bare synthetic engine has no mission script. Recording
                // the requested stage must not fabricate lifecycle effects.
                assert!(actual.output.post_initialize_events.is_none());
            }
        }
    }
}

#[test]
fn replay_missing_record_and_backward_target_fail_instead_of_fabricating_input() {
    let mut assets = LevelAssets::new();
    let initial = fixture_engine(&mut assets);
    let stationary = SimulationFrameInput::no_hourglass();
    let error = replay_frames_to_frame(SimSnapshot::new(0, &initial), &assets, 2, |frame| {
        (frame == 0).then_some(&stationary)
    })
    .err()
    .expect("missing record must fail after stationary frame too");
    assert_eq!(error, ReplayError::MissingCommands { frame: 1 });
    let error = replay_frames_to_frame(SimSnapshot::new(2, &initial), &assets, 1, |_| {
        panic!("invalid target must fail before consulting recorded inputs")
    })
    .err()
    .expect("cannot invent a pre-checkpoint history");
    assert_eq!(
        error,
        ReplayError::TargetBeforeCheckpoint {
            checkpoint_frame: 2,
            target_frame: 1
        }
    );
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
    let application_context = host.application_context().clone();
    let mut dev = DevState::default();
    let output = run_engine_frame_core(
        &mut host.frontend,
        &mut host.audio,
        &mut host.effects,
        &application_context,
        host.transport.local_seat(),
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
    assets: &LevelAssets,
    dev: &mut DevState,
    frame_input: SimulationFrameInput,
) {
    let application_context = host.application_context().clone();
    let output = run_engine_frame_core(
        &mut host.frontend,
        &mut host.audio,
        &mut host.effects,
        &application_context,
        host.transport.local_seat(),
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
    facade_host
        .frontend
        .presentation
        .engine_display
        .display_minimap(true, false);
    let mut facade_dev = DevState::default();

    let mut checkpoint = None;
    let mut facade_prefix_hashes = Vec::new();
    for (frame, input) in frames.iter().enumerate() {
        advance_authoritative_frame(
            &mut facade,
            &mut facade_host,
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

    let mut before_dev = DevState::default();
    advance_authoritative_frame(
        &mut before_hourglass,
        &mut before_host,
        &assets,
        &mut before_dev,
        SimulationFrameInput::from_player_inputs(vec![quit.clone()]).with_post_initialize(true),
    );

    let mut after_hourglass = SimSnapshot::new(0, &initial);
    let mut after_host = Host::default();

    let mut after_dev = DevState::default();
    advance_authoritative_frame(
        &mut after_hourglass,
        &mut after_host,
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
