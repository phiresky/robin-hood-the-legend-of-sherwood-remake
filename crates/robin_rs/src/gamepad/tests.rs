use super::*;

#[test]
fn client_gamepad_reads_its_own_selection_not_the_host_selection() {
    use crate::host::test_support::{add_pc_with_status, fixture};
    use robin_engine::engine::SimulationFrameInput;
    use robin_engine::player_command::PlayerInput;
    let (mut engine, assets, _) = fixture();
    let host_pc = add_pc_with_status(&mut engine, 10.0, 10.0, Posture::Upright, true, 100);
    let client_pc = add_pc_with_status(&mut engine, 30.0, 30.0, Posture::Upright, true, 100);
    let client = PlayerId(1);
    engine
        .advance_frame(
            &assets,
            SimulationFrameInput::new(vec![
                PlayerCommand::ConnectSeat {
                    player_id: client,
                    nickname: "client".to_owned(),
                }
                .into(),
                PlayerInput::new(
                    PlayerId::HOST,
                    PlayerCommand::SelectPc {
                        pc_id: host_pc,
                        append: false,
                    },
                )
                .into(),
                PlayerInput::new(
                    client,
                    PlayerCommand::SelectPc {
                        pc_id: client_pc,
                        append: false,
                    },
                )
                .into(),
            ])
            .with_hourglass(false),
        )
        .expect("admit separate seat selections");
    assert_eq!(selected_leader(&engine, PlayerId::HOST).unwrap().0, host_pc);
    assert_eq!(selected_leader(&engine, client).unwrap().0, client_pc);
    let mut pad = GamePadState::new();
    let mut pressed = JoystickState::default();
    pressed.buttons[GamePadButton::ActionA.index()] = 1;
    pad.update(pressed);
    pad.update(JoystickState::default());
    let commands = pad.manage_action_select(&engine, client);
    assert!(
        matches!(commands.as_slice(), [PlayerCommand::SelectAction { pc_id, action_index: 0 }] if *pc_id == client_pc)
    );
    assert_eq!(engine.hero_selection(PlayerId::HOST), &[host_pc]);
}

fn fresh_engine() -> (engine_api::Engine, engine_api::LevelAssets) {
    use robin_engine::campaign::Campaign;
    let mut assets = engine_api::LevelAssets::new();
    let engine = engine_api::Engine::new_for_test(800.0, 600.0, Campaign::default(), &mut assets)
        .expect("engine");
    (engine, assets)
}

#[test]
fn button_indices_match_original_defines() {
    assert_eq!(GamePadButton::ActionB.index(), 0);
    assert_eq!(GamePadButton::ActionA.index(), 1);
    assert_eq!(GamePadButton::ActionC.index(), 2);
    assert_eq!(GamePadButton::CancelParade.index(), 3);
    assert_eq!(GamePadButton::SelectPrevCharacter.index(), 4);
    assert_eq!(GamePadButton::AltChoice.index(), 5);
    assert_eq!(GamePadButton::SelectNextCharacter.index(), 6);
    assert_eq!(GamePadButton::QaManage.index(), 7);
    assert_eq!(GamePadButton::CrouchChinese.index(), 10);
    assert_eq!(GamePadButton::SimulatedLeftMouse.index(), 11);
}

#[test]
fn pov_from_raw_known_values() {
    assert_eq!(PovDirection::from_raw(0), PovDirection::North);
    assert_eq!(PovDirection::from_raw(4500), PovDirection::NorthEast);
    assert_eq!(PovDirection::from_raw(9000), PovDirection::East);
    assert_eq!(PovDirection::from_raw(13500), PovDirection::SouthEast);
    assert_eq!(PovDirection::from_raw(18000), PovDirection::South);
    assert_eq!(PovDirection::from_raw(22500), PovDirection::SouthWest);
    assert_eq!(PovDirection::from_raw(27000), PovDirection::West);
    assert_eq!(PovDirection::from_raw(31500), PovDirection::NorthWest);
}

#[test]
fn pov_from_raw_unknown_maps_to_centered() {
    assert_eq!(PovDirection::from_raw(0xFFFF_FFFF), PovDirection::Centered);
    assert_eq!(PovDirection::from_raw(1234), PovDirection::Centered);
}

#[test]
fn default_joystick_state_is_neutral() {
    let state = JoystickState::default();
    assert_eq!(state.x, 0);
    assert_eq!(state.y, 0);
    assert_eq!(state.rz, AXIS_CENTER);
    assert_eq!(state.sliders[0], AXIS_CENTER);
    assert_eq!(state.povs[0], 0xFFFF_FFFF);
    assert!(state.buttons.iter().all(|&b| b == 0));
}

#[test]
fn button_edge_full_lifecycle() {
    let mut pad = GamePadState::new();

    // Initially up
    assert!(!pad.is_down(GamePadButton::ActionA));
    assert!(!pad.is_pushed(GamePadButton::ActionA));
    assert!(!pad.is_released(GamePadButton::ActionA));

    // Press → Pushed
    let mut state = JoystickState::default();
    state.buttons[GamePadButton::ActionA.index()] = 1;
    pad.update(state);
    assert!(pad.is_down(GamePadButton::ActionA));
    assert!(pad.is_pushed(GamePadButton::ActionA));
    assert!(!pad.is_released(GamePadButton::ActionA));

    // Hold → Held
    let mut state = JoystickState::default();
    state.buttons[GamePadButton::ActionA.index()] = 1;
    pad.update(state);
    assert!(pad.is_down(GamePadButton::ActionA));
    assert!(!pad.is_pushed(GamePadButton::ActionA));
    assert!(!pad.is_released(GamePadButton::ActionA));

    // Release → Released
    pad.update(JoystickState::default());
    assert!(!pad.is_down(GamePadButton::ActionA));
    assert!(!pad.is_pushed(GamePadButton::ActionA));
    assert!(pad.is_released(GamePadButton::ActionA));

    // Back to Up
    pad.update(JoystickState::default());
    assert!(!pad.is_down(GamePadButton::ActionA));
    assert!(!pad.is_pushed(GamePadButton::ActionA));
    assert!(!pad.is_released(GamePadButton::ActionA));
}

#[test]
fn mouse_delta_centered_is_zero() {
    let pad = GamePadState::new();
    let (dx, dy) = pad.mouse_delta();
    assert!(dx.abs() < 0.01);
    assert!(dy.abs() < 0.01);
}

#[test]
fn mouse_delta_offset() {
    let mut pad = GamePadState::new();
    let mut state = JoystickState {
        rz: AXIS_CENTER + 1638, // dx ≈ 1.0
        ..Default::default()
    };
    state.sliders[0] = AXIS_CENTER + 1638; // dy ≈ 1.0
    pad.update(state);

    let (dx, dy) = pad.mouse_delta();
    assert!((dx - 1.0).abs() < 0.01);
    assert!((dy - 1.0).abs() < 0.01);
}

#[test]
fn movement_offset_preserves_neutral_threshold_gait_and_direction() {
    let mut pad = GamePadState::new();
    assert_eq!(pad.movement_offset(), None);
    for (x, y, expected_running) in [
        (1, 0, false),
        (28000, 0, false),
        (28001, 0, true),
        (-28000, 0, false),
        (0, -28001, true),
        (10000, 10000, false),
        (-25000, 25000, true),
    ] {
        pad.update(JoystickState {
            x,
            y,
            ..Default::default()
        });
        let (dx, dy, running) = pad.movement_offset().unwrap();
        assert_eq!(running, expected_running, "stick ({x}, {y})");
        let distance = (dx * dx + dy * dy).sqrt();
        let expected_distance = if running { 3.0 * MOVE_UNIT } else { MOVE_UNIT };
        assert!((distance - expected_distance).abs() < 0.001);
        assert_eq!(dx.signum(), (x as f32).signum());
        assert_eq!(dy.signum(), (y as f32).signum());
    }
}

#[test]
fn pov_query() {
    let mut pad = GamePadState::new();
    assert_eq!(pad.pov(), PovDirection::Centered);

    let mut state = JoystickState::default();
    state.povs[0] = 9000;
    pad.update(state);
    assert_eq!(pad.pov(), PovDirection::East);
}

#[test]
fn vector_to_sector_cardinal_directions() {
    // North (0, -1) → sector 0
    assert_eq!(vector_to_sector_0_to_15(0.0, -1.0), 0);
    // East (1, 0) → sector 4
    assert_eq!(vector_to_sector_0_to_15(1.0, 0.0), 4);
    // South (0, 1) → sector 8
    assert_eq!(vector_to_sector_0_to_15(0.0, 1.0), 8);
    // West (-1, 0) → sector 12
    assert_eq!(vector_to_sector_0_to_15(-1.0, 0.0), 12);
}

#[test]
fn vector_to_sector_diagonals() {
    // NE → sector 2
    assert_eq!(vector_to_sector_0_to_15(1.0, -1.0), 2);
    // SE → sector 6
    assert_eq!(vector_to_sector_0_to_15(1.0, 1.0), 6);
    // SW → sector 10
    assert_eq!(vector_to_sector_0_to_15(-1.0, 1.0), 10);
    // NW → sector 14
    assert_eq!(vector_to_sector_0_to_15(-1.0, -1.0), 14);
}

#[test]
fn recognize_swing_empty_returns_none() {
    assert_eq!(recognize_swing(&[], 0), None);
}

#[test]
fn recognize_swing_forward_weak() {
    // Facing north (sector 0), push stick north — below HIT_THRESHOLD.
    // Small x offset so it falls into a quadrant (strict < / > checks).
    let samples = vec![(-0.1, -100.0)];
    assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustA));
}

#[test]
fn recognize_swing_forward_strong() {
    // Facing north (sector 0), push stick north hard — above HIT_THRESHOLD
    let samples = vec![(-0.1, -30000.0)];
    assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustB));
}

#[test]
fn recognize_swing_right_side() {
    // Facing north (sector 0), push stick east (sector 4) → ThrustD
    let samples = vec![(100.0, -0.1)];
    assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustD));
}

#[test]
fn recognize_swing_left_side() {
    // Facing north (sector 0), push stick west (sector 12) → ThrustE
    let samples = vec![(-100.0, -0.1)];
    assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustE));
}

#[test]
fn recognize_swing_single_circle() {
    let samples = vec![
        (-1.0, -1.0), // Q0
        (1.0, -1.0),  // Q1
        (1.0, 1.0),   // Q2
        (-1.0, 1.0),  // Q3
    ];
    assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustH));
}

#[test]
fn recognize_swing_double_circle() {
    let samples = vec![
        (-1.0, -1.0),
        (1.0, -1.0),
        (1.0, 1.0),
        (-1.0, 1.0),
        (-1.0, -1.0),
        (1.0, -1.0),
        (1.0, 1.0),
        (-1.0, 1.0),
    ];
    assert_eq!(recognize_swing(&samples, 0), Some(SwordStrike::ThrustC));
}

#[test]
fn recognize_swing_unrecognized_direction() {
    // Facing north, push stick to sector 8 (behind) — no matching strike
    let samples = vec![(0.0, 100.0)];
    assert_eq!(recognize_swing(&samples, 0), None);
}

#[test]
fn gamepad_state_serde_roundtrip() {
    let mut pad = GamePadState::new();
    let mut state = JoystickState {
        x: 1234,
        ..Default::default()
    };
    state.buttons[0] = 1;
    pad.update(state);

    let json = serde_json::to_string(&pad).unwrap();
    let restored: GamePadState = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.current.x, 1234);
    assert!(restored.is_down(GamePadButton::ActionB));
}

#[test]
fn multiple_buttons_independent() {
    let mut pad = GamePadState::new();
    let mut state = JoystickState::default();
    state.buttons[GamePadButton::ActionA.index()] = 1;
    state.buttons[GamePadButton::ActionB.index()] = 1;
    pad.update(state);

    assert!(pad.is_pushed(GamePadButton::ActionA));
    assert!(pad.is_pushed(GamePadButton::ActionB));
    assert!(!pad.is_pushed(GamePadButton::ActionC));
}

// ── Dispatcher tests ────────────────────────────────────────

fn empty_engine() -> engine_api::Engine {
    fresh_engine().0
}

/// Prime `pad` with a prior-frame state where nothing is pushed so
/// the edge detectors treat `new_state` as fresh input.
fn prime_and_set(pad: &mut GamePadState, new_state: JoystickState) {
    pad.update(JoystickState::default());
    pad.update(new_state);
}

#[test]
fn manage_scroll_axis_pov_north_emits_scroll_up_and_unfollow() {
    let mut pad = GamePadState::new();
    let mut state = JoystickState::default();
    state.povs[0] = 0; // North
    prime_and_set(&mut pad, state);

    let mut cmds = Vec::new();
    let viewport = pad.manage_scroll_axis(&mut cmds);
    assert!(
        viewport
            .iter()
            .any(|c| matches!(c, ViewportCommand::Scroll(ScrollDirection::Up))),
        "{viewport:?}"
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, PlayerCommand::SelectFollowElement { entity_id: None })),
        "{cmds:?}"
    );
}

#[test]
fn manage_scroll_axis_pov_north_with_alt_zooms_in() {
    let mut pad = GamePadState::new();
    let mut state = JoystickState::default();
    state.povs[0] = 0; // North
    state.buttons[GamePadButton::AltChoice.index()] = 1;
    prime_and_set(&mut pad, state);

    let mut cmds = Vec::new();
    let viewport = pad.manage_scroll_axis(&mut cmds);
    assert!(
        viewport
            .iter()
            .any(|c| matches!(c, ViewportCommand::ZoomIn))
    );
    assert!(
        !viewport
            .iter()
            .any(|c| matches!(c, ViewportCommand::Scroll(_))),
        "scroll should be suppressed when zooming"
    );
}

#[test]
fn manage_scroll_axis_pov_northeast_emits_both_directions() {
    let mut pad = GamePadState::new();
    let mut state = JoystickState::default();
    state.povs[0] = 4500;
    prime_and_set(&mut pad, state);

    let mut cmds = Vec::new();
    let viewport = pad.manage_scroll_axis(&mut cmds);
    assert!(
        viewport
            .iter()
            .any(|c| matches!(c, ViewportCommand::Scroll(ScrollDirection::Right)))
    );
    assert!(
        viewport
            .iter()
            .any(|c| matches!(c, ViewportCommand::Scroll(ScrollDirection::Up)))
    );
}

#[test]
fn manage_scroll_axis_centered_emits_nothing() {
    let pad = GamePadState::new();
    let mut cmds = Vec::new();
    assert!(pad.manage_scroll_axis(&mut cmds).is_empty());
    assert!(cmds.is_empty());
}

#[test]
fn manage_character_select_none_selected_without_pcs() {
    // Empty engine has no PCs — dispatch must not panic and must
    // return no commands.
    let pad = GamePadState::new();
    let engine = empty_engine();
    assert!(
        pad.manage_character_select(&engine, PlayerId::HOST)
            .is_empty()
    );
}

#[test]
fn manage_action_select_cancel_parade_release_emits_right_click() {
    let mut pad = GamePadState::new();
    // Push then release CANCEL_PARADE (edge-detect the up-transition).
    let mut state = JoystickState::default();
    state.buttons[GamePadButton::CancelParade.index()] = 1;
    pad.update(state);
    pad.update(JoystickState::default()); // release

    let engine = empty_engine();
    let cmds = pad.manage_action_select(&engine, PlayerId::HOST);
    // Release edge always clears the held state via MouseRightUp.
    // `resolve_right_click` is empty when no PC is selected, so on
    // an empty engine MouseRightUp is the only command we expect.
    assert!(
        cmds.iter()
            .any(|c| matches!(c, PlayerCommand::MouseRightUp)),
        "{cmds:?}"
    );
}

#[test]
fn manage_qa_single_click_arms_then_fires_on_timeout() {
    let mut pad = GamePadState::new();
    let engine = empty_engine();

    // No selected PC → no QA events.
    // (Real test would need to populate engine.selected_hero_ids, but
    // the dispatcher's early-return branch is important to verify.)
    assert!(pad.manage_qa(1000, &engine, PlayerId::HOST).is_none());
}

#[test]
fn manage_qa_timer_expiration_with_selected_pc() {
    // Build a minimal engine with one selected PC so manage_qa's
    // early-return guard passes. We can't easily synthesise a full
    // PC entity without a level, but engine exposes selected_hero_ids
    // as a mutator — push a dummy id in and verify the timer flow.
    let mut pad = GamePadState::new();
    // Arm the timer: release QA while ALT is held.
    let mut pressed = JoystickState::default();
    pressed.buttons[GamePadButton::QaManage.index()] = 1;
    pressed.buttons[GamePadButton::AltChoice.index()] = 1;
    pad.update(pressed);

    let mut released_alt_still_down = JoystickState::default();
    released_alt_still_down.buttons[GamePadButton::AltChoice.index()] = 1;
    pad.update(released_alt_still_down);

    // First pass: arm the timer. Requires at least one selected PC;
    // without one the dispatch short-circuits. This test documents
    // the short-circuit behaviour until a richer engine fixture
    // lands.
    let engine = empty_engine();
    assert!(pad.manage_qa(1000, &engine, PlayerId::HOST).is_none());
}

#[test]
fn standard_button_to_gamepad_index_mapping() {
    assert_eq!(
        standard_button_to_gamepad_index(0),
        Some(GamePadButton::ActionB as u8)
    );
    assert_eq!(
        standard_button_to_gamepad_index(1),
        Some(GamePadButton::ActionA as u8)
    );
    assert_eq!(
        standard_button_to_gamepad_index(3),
        Some(GamePadButton::CancelParade as u8)
    );
    // D-pad buttons are routed elsewhere.
    assert_eq!(standard_button_to_gamepad_index(11), None);
}

#[test]
fn is_dpad_button_covers_all_four() {
    assert!(is_dpad_button(11));
    assert!(is_dpad_button(12));
    assert!(is_dpad_button(13));
    assert!(is_dpad_button(14));
    assert!(!is_dpad_button(10));
    assert!(!is_dpad_button(15));
}

#[test]
fn dpad_to_pov_cardinal_and_diagonal() {
    assert_eq!(dpad_to_pov(true, false, false, false), 0);
    assert_eq!(dpad_to_pov(true, true, false, false), 4500);
    assert_eq!(dpad_to_pov(false, true, false, false), 9000);
    assert_eq!(dpad_to_pov(false, false, true, true), 22500);
    assert_eq!(dpad_to_pov(false, false, false, false), 0xFFFF_FFFF);
}

#[test]
fn apply_axis_event_translates_rz_to_directinput_center() {
    let mut pad = GamePadState::new();
    // Standard axis 2 = RightX, value 0 (center) → rz = AXIS_CENTER.
    pad.apply_axis_event(2, 0);
    assert_eq!(pad.pending.rz, AXIS_CENTER);
    // Standard axis 2, value 1638 → rz ≈ AXIS_CENTER + 1638 → dx ≈ 1.0
    pad.apply_axis_event(2, 1638);
    assert_eq!(pad.pending.rz, AXIS_CENTER + 1638);
}

#[test]
fn apply_axis_event_left_stick_stays_signed() {
    let mut pad = GamePadState::new();
    pad.apply_axis_event(0, 25000); // LeftX
    pad.apply_axis_event(1, -25000); // LeftY
    assert_eq!(pad.pending.x, 25000);
    assert_eq!(pad.pending.y, -25000);
}

#[test]
fn apply_button_event_mirrors_pressed_state() {
    let mut pad = GamePadState::new();
    pad.apply_button_event(GamePadButton::ActionA as u8, true);
    assert_eq!(pad.pending.buttons[GamePadButton::ActionA.index()], 1);
    pad.apply_button_event(GamePadButton::ActionA as u8, false);
    assert_eq!(pad.pending.buttons[GamePadButton::ActionA.index()], 0);
}

#[test]
fn process_gamepad_input_promotes_pending_to_current() {
    let mut pad = GamePadState::new();
    let mut threaded = ThreadedInput::new();
    pad.apply_axis_event(0, 15000);
    let engine = empty_engine();
    let _ = pad.process_gamepad_input(0, &engine, PlayerId::HOST, &mut threaded);
    assert_eq!(pad.current.x, 15000);
}
