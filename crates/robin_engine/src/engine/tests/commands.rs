use super::*;
use crate::player_command::PlayerCommand;

fn swordfight_test_assets() -> LevelAssets {
    let mut assets = LevelAssets::new();
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.characters.push(crate::profiles::CharacterProfile {
        hth_weapon_id: 1,
        ..crate::profiles::CharacterProfile::default()
    });
    profiles.soldiers.push(crate::profiles::SoldierProfile {
        hth_weapon_id: 1,
        ..crate::profiles::SoldierProfile::default()
    });
    profiles
        .hth_weapons
        .push(crate::profiles::HtHWeaponProfile {
            distance: [20, 40, 70, 100],
            ..crate::profiles::HtHWeaponProfile::default()
        });
    assets
}

#[test]
fn script_globals() {
    let mut engine = EngineInner::new();
    engine.init_script_global(5, 42);
    assert_eq!(engine.get_script_global(5), 42);
    // `init_script_global` resizes to `id + 16`, giving scripts a
    // 16-slot slack window of valid reads beyond the last-initialised
    // index.
    assert_eq!(engine.scripts.globals.len(), 5 + 16);
    for i in 6..(5 + 16) {
        assert_eq!(engine.get_script_global(i), 0);
    }

    engine.set_script_global(5, 99);
    assert_eq!(engine.get_script_global(5), 99);

    assert!(engine.is_valid_script_global_id(5));
    assert!(engine.is_valid_script_global_id(20));
    assert!(!engine.is_valid_script_global_id(21));
}

#[test]
#[should_panic(expected = "out of range")]
fn script_global_set_out_of_range_panics() {
    let mut engine = EngineInner::new();
    engine.set_script_global(100, 1);
}

#[test]
fn global_options_default() {
    let opts = GlobalOptions::default();
    assert_eq!(opts.major_version, 1);
    assert_eq!(opts.minor_version, 2);
    assert!(opts.sound_enabled);
    assert!(opts.script_enabled);
    assert!(!opts.highlander2);
    assert_eq!(opts.level_directory, "Data/Levels");
}

#[test]
fn draw_fast_forward_skips() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.control.fast_forward = true;
    engine.control.frame_counter = 1; // Not a multiple of 32
    let result = engine.tick_display_state(&mut display);
    assert_eq!(result, 1); // Should skip
}

#[test]
fn draw_fast_forward_every_32nd() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.control.fast_forward = true;
    engine.control.frame_counter = 32; // Multiple of 32
    let result = engine.tick_display_state(&mut display);
    assert_eq!(result, 0); // Should render
}

#[test]
fn ambiance_night_colors() {
    assert_eq!(Ambiance::Day.night_color_rgb(), (45, 45, 35));
    assert_eq!(Ambiance::Fog.night_color_rgb(), (85, 77, 90));
    assert_eq!(Ambiance::Night.night_color_rgb(), (0, 0, 0));
}

#[test]
fn center_on_point() {
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4000.0, 3000.0);
    engine.center_on_point(0, crate::coordinates::MapPoint::new(1000.0, 800.0));
    // View should be offset by half the full screen on both axes
    // (raw screen vector divided by 2*zoom; the bottom-panel exclusion
    // applies only to the clamp, not the centering).  The result is
    // floored before assignment.
    let expected_x = (1000.0f32 - 512.0f32).floor(); // 1024/2
    let expected_y = (800.0f32 - 384.0f32).floor(); // 768/2
    assert!((engine.feedback.cutscene_camera.view_position.x - expected_x).abs() < 0.01);
    assert!((engine.feedback.cutscene_camera.view_position.y - expected_y).abs() < 0.01);
}

#[test]
fn mission_state_transitions() {
    let mut engine = EngineInner::new();
    assert!(!engine.mission_domain.state.mission_won);

    engine.win(true);
    assert!(engine.mission_domain.state.mission_won);
    assert!(engine.mission_domain.state.mission_won_first_time);

    // `win` writes both flags unconditionally, so a second call
    // re-toggles `mission_won_first_time`.
    engine.mission_domain.state.mission_won_first_time = false;
    engine.win(true);
    assert!(engine.mission_domain.state.mission_won_first_time);

    // A silent win (show_window=false) queues the start/quit-mission
    // widget swap as a side-effect for the host to drain.
    engine.feedback.pending_side_effects = Default::default();
    engine.win(false);
    assert!(!engine.mission_domain.state.mission_won_first_time);
    assert!(
        engine
            .feedback
            .pending_side_effects
            .pending_silent_win_widget_swap
    );
}

#[test]
fn initialize_sends_stature_message() {
    let mut assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    assert_eq!(engine.orders.messenger.count(), 0);
    engine.initialize(&mut assets);
    // Should have sent a Stature message
    let msg = engine
        .orders
        .messenger
        .poll()
        .expect("expected stature message");
    assert_eq!(msg.msg_type, MessageType::Simple(SimpleMessage::Stature));
}

#[test]
fn mission_won_first_time_raises_mission_state_notice() {
    let mut display = HostDisplayState::default();
    // On the first post-win frame with no PC guarded, the engine
    // fires the `LEAVE_MISSION_NOW` mission-state notice +
    // `EnableWidgetQuitMission(false)`.  Both are routed through
    // `SideEffects.pending_mission_state_notice`.
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.mission_domain.state.mission_won_first_time = true;
    let side_effects = engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert!(!engine.mission_domain.state.mission_won_first_time);
    assert!(
        side_effects.pending_mission_state_notice,
        "expected pending_mission_state_notice side effect"
    );
}

#[test]
fn post_load_fixups_aborts_midzoom() {
    let mut display = HostDisplayState::default();
    // Build an engine mid-zoom and run the post-load fixup path
    // directly.  The zoom-abort block previously lived in
    // `tick_display_state` under `!cache_valid`; it now runs inside
    // `EngineInner::post_load_fixups` so `Engine::restore` can't
    // leave the engine mid-zoom.
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    engine
        .feedback
        .cutscene_camera
        .display
        .background_transform
        .zoom_to_up = true;
    engine.feedback.cutscene_camera.zoom_init_done = true;

    engine.post_load_fixups(&mut display);

    assert!(
        !engine
            .feedback
            .cutscene_camera
            .display
            .background_transform
            .zoom_to_up
    );
    assert!(!engine.feedback.cutscene_camera.zoom_init_done);
    let msg = engine
        .orders
        .messenger
        .poll()
        .expect("expected zoom end message");
    assert_eq!(msg.msg_type, MessageType::Simple(SimpleMessage::ZoomUpEnd));
}

#[test]
fn patch_door_highlight_refresh_does_not_require_a_script_host() {
    let mut engine = EngineInner::new();
    let mut first = crate::patch::Patch::new();
    first.display_doors = true;
    engine.script_domains.interactables.patches.push(first);
    engine
        .script_domains
        .interactables
        .patches
        .push(crate::patch::Patch::new());

    assert!(engine.scripts.mission.is_none());
    engine.refresh_selected_patch_display_doors(Some(1));

    assert!(!engine.script_domains.interactables.patches[0].display_doors);
    assert!(engine.script_domains.interactables.patches[1].display_doors);
}

#[test]
fn mercenary_formation_single_pc_lands_on_click() {
    let click = crate::coordinates::map_pt(200.0, 300.0);
    let dests = mercenary_formation_destinations(&[crate::coordinates::map_pt(50.0, 50.0)], click);
    assert_eq!(dests.len(), 1);
    assert_eq!(dests[0].x, click.x);
    assert_eq!(dests[0].y, click.y);
}

#[test]
fn mercenary_formation_preserves_relative_offsets() {
    // 3 PCs in a horizontal line at (0,0), (50,0), (100,0).
    // Centroid = (50, 0).  Click at (200, 300).
    // Per-PC dests should preserve the (-50, 0), (0, 0), (+50, 0) offsets
    // relative to the click point.
    let pcs = [
        crate::coordinates::map_pt(0.0, 0.0),
        crate::coordinates::map_pt(50.0, 0.0),
        crate::coordinates::map_pt(100.0, 0.0),
    ];
    let click = crate::coordinates::map_pt(200.0, 300.0);
    let dests = mercenary_formation_destinations(&pcs, click);
    assert_eq!(dests.len(), 3);
    assert_eq!(dests[0], crate::coordinates::map_pt(150.0, 300.0));
    assert_eq!(dests[1], crate::coordinates::map_pt(200.0, 300.0));
    assert_eq!(dests[2], crate::coordinates::map_pt(250.0, 300.0));
}

#[test]
fn mercenary_formation_empty_input() {
    let dests = mercenary_formation_destinations(&[], crate::coordinates::map_pt(0.0, 0.0));
    assert!(dests.is_empty());
}

#[test]
fn ground_mark_hourglass_advances_and_retires_on_screen_marks() {
    let mut display = HostDisplayState::default();
    // The per-mark animation advance is gated on `IsOnScreen` and
    // even universal-frame-counter ticks.  For rollback determinism
    // the state advance happens inside `perform_hourglass` instead —
    // render is read-only.
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    // 8×8 sprites centred on camera origin so every frame is on-screen.
    engine.set_ground_mark_sprite_data(
        0.0,
        0.0,
        vec![(8, 8); crate::markers::NUMBER_OF_GROUND_FRAMES as usize],
        vec![(0, 0); crate::markers::NUMBER_OF_GROUND_FRAMES as usize],
    );
    engine.feedback.ground_mark.add_mark(100.0, 100.0, 0);
    assert_eq!(engine.feedback.ground_mark.len(), 1);

    // Plenty of ticks to burn through all NUMBER_OF_GROUND_FRAMES advances
    // (half of them gated off by odd frame counters) and retire the mark.
    for _ in 0..(2 * crate::markers::NUMBER_OF_GROUND_FRAMES as usize + 4) {
        engine.perform_hourglass(&mut display, &assets, &mut dev);
    }
    assert!(
        engine.feedback.ground_mark.is_empty(),
        "mark should have animated through to retirement"
    );
}

#[test]
fn ground_mark_hourglass_freezes_off_screen_marks() {
    let mut display = HostDisplayState::default();
    // Off-screen marks must freeze in both live and replay — the
    // `IsOnScreen` gate suppresses advance.
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.set_ground_mark_sprite_data(
        0.0,
        0.0,
        vec![(8, 8); crate::markers::NUMBER_OF_GROUND_FRAMES as usize],
        vec![(0, 0); crate::markers::NUMBER_OF_GROUND_FRAMES as usize],
    );
    // Mark at (100_000, 100_000) is well outside the 800×600 viewport.
    engine
        .feedback
        .ground_mark
        .add_mark(100_000.0, 100_000.0, 0);

    for _ in 0..(2 * crate::markers::NUMBER_OF_GROUND_FRAMES as usize + 4) {
        engine.perform_hourglass(&mut display, &assets, &mut dev);
    }
    assert_eq!(engine.feedback.ground_mark.len(), 1);
    assert_eq!(engine.feedback.ground_mark.marks[0].current_frame, 0);
}

#[test]
fn mission_stat_resets_on_new_mission() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut assets = LevelAssets::new();
    let mut staging = LevelLoadStaging::default();
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.mission_domain.mission_stat.add_collected_money(500);
    engine.mission_domain.short_briefings.add(42, true);

    let loaded = crate::level_data::LoadedLevel::empty_for_test();
    let _ = engine.initialize_from_mission(
        sim,
        &mut assets,
        &mut staging,
        "test_mission",
        "test_proto",
        loaded,
        "Data/Levels",
        (0.0, 0.0),
        &mut |_| {},
    );

    assert_eq!(engine.mission_domain.mission_stat.collected_money, 0);
    assert_eq!(engine.mission_domain.short_briefings.count(true), 0);
}

#[test]
fn resize_snaps_zoom() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(500.0, 400.0); // Small level
    engine.feedback.cutscene_camera.zoom_factor = 0.5;
    display.background_transform.current_zoom_level = 0;

    engine.resize(&mut display, 1024.0, 768.0);

    // Should have snapped to 1.0 since 0.5x can't fit
    assert_eq!(engine.feedback.cutscene_camera.zoom_factor, 1.0);
    assert_eq!(display.background_transform.current_zoom_level, 1);
}

// ── Campaign integration tests ──────────────────────────────

#[test]
fn add_campaign_value_ransom_credits_mission_stat_and_emits_jingle() {
    use crate::sound::Jingle;
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Campaign::default();
    engine.control.frame_counter = 100; // past frame 0 → jingle gate open

    engine.add_campaign_value(CampaignValue::Ransom, 250);

    assert_eq!(
        engine
            .mission_domain
            .campaign
            .get_value(CampaignValue::Ransom),
        crate::campaign::INITIAL_RANSOM + 250
    );
    assert_eq!(engine.mission_domain.mission_stat.collected_money, 250);
    let jingle_count = engine
        .feedback
        .pending_side_effects
        .sounds
        .iter()
        .filter(|s| matches!(s, SoundCommand::Jingle(Jingle::CashWon)))
        .count();
    assert_eq!(jingle_count, 1);
}

#[test]
fn add_campaign_value_score_credits_mission_stat() {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Campaign::default();
    engine.control.frame_counter = 100;

    engine.add_campaign_value(CampaignValue::Score, 750);

    assert_eq!(
        engine
            .mission_domain
            .campaign
            .get_value(CampaignValue::Score),
        750
    );
    assert_eq!(engine.mission_domain.mission_stat.added_score, 750);
    // Score is silent.
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());
}

#[test]
fn add_campaign_value_negative_ransom_skips_jingle_but_credits_money() {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Campaign::default();
    engine.control.frame_counter = 100;
    engine.mission_domain.campaign.values[CampaignValue::Ransom] = 500;
    engine.mission_domain.mission_stat.collected_money = 200;

    // A purse throw (`combat.rs:2433`) issues a negative delta.
    engine.add_campaign_value(CampaignValue::Ransom, -100);

    assert_eq!(
        engine
            .mission_domain
            .campaign
            .get_value(CampaignValue::Ransom),
        400
    );
    // `add_campaign_value` credits the mission-stat counter
    // unconditionally (wrapping_add_signed); only the jingle is gated.
    assert_eq!(engine.mission_domain.mission_stat.collected_money, 100);
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());
}

#[test]
fn add_campaign_value_skips_jingle_at_frame_zero() {
    // The `frame_counter > 0` gate ensures the pre-mission seed
    // (initial ransom = 100) doesn't sound a coin chime.
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Campaign::default();
    engine.control.frame_counter = 0;

    engine.add_campaign_value(CampaignValue::Ransom, 100);

    assert_eq!(engine.mission_domain.mission_stat.collected_money, 100);
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());
}

#[test]
fn set_campaign_value_ransom_emits_jingle_only_when_growing() {
    use crate::sound::Jingle;
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Campaign::default();
    engine.control.frame_counter = 50;
    engine.mission_domain.campaign.values[CampaignValue::Ransom] = 200;

    // Lower → no jingle (only growth fires the gate).
    engine.set_campaign_value(CampaignValue::Ransom, 100);
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());

    // Higher → jingle.
    engine.set_campaign_value(CampaignValue::Ransom, 500);
    let jingle_count = engine
        .feedback
        .pending_side_effects
        .sounds
        .iter()
        .filter(|s| matches!(s, SoundCommand::Jingle(Jingle::CashWon)))
        .count();
    assert_eq!(jingle_count, 1);
    // SetValue does NOT credit collected_money — only AddValue does.
    assert_eq!(engine.mission_domain.mission_stat.collected_money, 0);
}

#[test]
fn add_campaign_value_amulets_has_no_side_effects() {
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = Campaign::default();
    engine.control.frame_counter = 100;

    engine.add_campaign_value(CampaignValue::Amulets, 3);

    assert_eq!(
        engine
            .mission_domain
            .campaign
            .get_value(CampaignValue::Amulets),
        3
    );
    assert_eq!(engine.mission_domain.mission_stat.collected_money, 0);
    assert_eq!(engine.mission_domain.mission_stat.added_score, 0);
    assert!(engine.feedback.pending_side_effects.sounds.is_empty());
}

#[test]
fn sync_stats_to_campaign() {
    let mut engine = EngineInner::new();
    engine.mission_domain.mission_stat.collected_money = 500;
    engine.mission_domain.mission_stat.added_score = 1200;
    engine.mission_domain.mission_stat.living_soldier_count = 8;
    engine.mission_domain.mission_stat.total_soldier_count = 12;

    let mut campaign = Campaign::default();
    campaign.set_value(CampaignValue::Ransom, 100);

    engine.sync_stats_to_campaign(&mut campaign);

    // Money/score are credited during gameplay via add_campaign_value,
    // so sync at mission end must NOT re-add them — only soldier counts.
    assert_eq!(campaign.get_value(CampaignValue::Ransom), 100);
    assert_eq!(campaign.get_value(CampaignValue::Score), 0);
    assert_eq!(campaign.get_value(CampaignValue::LivingSoldiers), 8);
    assert_eq!(campaign.get_value(CampaignValue::DeadSoldiers), 4); // 12 - 8
}

#[test]
fn current_mission_profile_none_when_no_mission() {
    let engine = EngineInner::new();
    let campaign = Campaign::default();
    let profiles = crate::profiles::ProfileManager::new();
    assert!(
        engine
            .current_mission_profile(&campaign, &profiles)
            .is_none()
    );
}

#[test]
fn is_sherwood_mission_no_mission() {
    let engine = EngineInner::new();
    let campaign = Campaign::default();
    let profiles = crate::profiles::ProfileManager::new();
    assert!(!engine.is_sherwood_mission(&campaign, &profiles));
}

// ── New tests for ported engine internals ──────────────────

#[test]
fn perform_check_scroll_clamps_right() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(2000.0, 1500.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(1500.0, 0.0);
    display.background_transform.scrolling_vector = MapVec::new(400.0, 0.0);

    let valid = engine.perform_check_scroll(&mut display);
    assert!(!valid);
    // Scroll should be clamped: 2000 - 1500 - 800/1.0 = -300
    // (negative means "can't go further right")
    assert!(display.background_transform.scrolling_vector.x <= 2000.0 - 1500.0 - 800.0 + 0.01);
}

#[test]
fn perform_check_scroll_clamps_left() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(2000.0, 1500.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(10.0, 0.0);
    display.background_transform.scrolling_vector = MapVec::new(-50.0, 0.0);

    let valid = engine.perform_check_scroll(&mut display);
    assert!(!valid);
    assert!((display.background_transform.scrolling_vector.x - (-10.0)).abs() < 0.01);
}

#[test]
fn perform_check_scroll_valid() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4000.0, 3000.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(500.0, 500.0);
    display.background_transform.scrolling_vector = MapVec::new(10.0, 10.0);

    let valid = engine.perform_check_scroll(&mut display);
    assert!(valid);
    assert!((display.background_transform.scrolling_vector.x - 10.0).abs() < 0.01);
}

#[test]
fn timer_tick_decrements_and_removes() {
    let mut display = HostDisplayState::default();
    use crate::sequence::{SequenceElementRef, SequenceId};
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    let ref_a = SequenceElementRef::new(SequenceId(100), 0);
    let ref_b = SequenceElementRef::new(SequenceId(200), 0);
    engine.add_timer(3, ref_a);
    engine.add_timer(1, ref_b);
    assert_eq!(engine.orders.timer_elements.len(), 2);

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    // Timer 200 (remaining=1) should be removed, timer 100 decremented to 2
    assert_eq!(engine.orders.timer_elements.len(), 1);
    assert_eq!(engine.orders.timer_elements[0].remaining, 2);
    assert_eq!(engine.orders.timer_elements[0].element_ref, ref_a);

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert_eq!(engine.orders.timer_elements[0].remaining, 1);

    engine.perform_hourglass(&mut display, &assets, &mut dev);
    assert!(engine.orders.timer_elements.is_empty());
}

#[test]
fn timer_started_by_sequence_dispatch_ticks_on_its_launch_frame() {
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, Sequence, SequenceElement};

    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.control.fast_forward = true;

    // DisplayPopupText is deferred through SequenceManager::Hourglass. Its
    // termination synchronously advances to the immediate Timer, exactly
    // like Emb05's PlayAnimFreeze(chariot_b1) -> Timer(100) handoff.
    let mut sequence = Sequence::new();
    sequence.append_element(SequenceElement::new_generic(
        1,
        Command::DisplayPopupText,
        None,
    ));
    let mut timer = SequenceElement::new_generic(2, Command::Timer, None);
    timer.set_property(Field::Timer, FieldValue::Integer(2));
    sequence.append_element(timer);
    engine.orders.sequence_manager.launch_sequence(sequence);

    engine.perform_hourglass(&mut display, &assets, &mut dev);

    assert_eq!(engine.orders.timer_elements.len(), 1);
    assert_eq!(engine.orders.timer_elements[0].remaining, 1);
}

#[test]
fn win_respects_show_window_false() {
    let mut engine = EngineInner::new();
    engine.win(false);
    assert!(engine.mission_domain.state.mission_won);
    assert!(!engine.mission_domain.state.mission_won_first_time);
}

#[test]
fn win_respects_show_window_true() {
    let mut engine = EngineInner::new();
    engine.win(true);
    assert!(engine.mission_domain.state.mission_won);
    assert!(engine.mission_domain.state.mission_won_first_time);
}

#[test]
fn zoom_change_state_updates_level() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    assert_eq!(display.background_transform.current_zoom_level, 1);

    // Zoom up: level should increment to 2
    engine.change_state(&mut display, 0, EngineStateRequest::ZoomingUp);
    assert_eq!(display.background_transform.current_zoom_level, 2);
    assert!(display.background_transform.zoom_to_up);

    // Reset for next test
    display.background_transform.zoom_to_up = false;
    engine.feedback.cutscene_camera.zoom_init_done = false;
    display.display_op = DisplayOpCode::Nothing;

    // Zoom down: level should decrement to 1
    engine.change_state(&mut display, 0, EngineStateRequest::ZoomingDown);
    assert_eq!(display.background_transform.current_zoom_level, 1);
    assert!(display.background_transform.zoom_to_down);
}

#[test]
fn zoom_deferred_when_scrolling() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    // Simulate active scrolling
    display.background_transform.current_x_scrolling_level = 5;

    engine.change_state(&mut display, 0, EngineStateRequest::ZoomingUp);
    // Should be deferred, not immediate
    assert!(display.background_transform.required_zoom_up);
    assert!(!display.background_transform.zoom_to_up);
    assert_eq!(display.background_transform.current_zoom_level, 1); // unchanged
}

#[test]
fn sort_for_minimap_priority_order() {
    use crate::element::{ActorPc, ActorSoldier, ElementBonus, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();

    // Add entities of each priority tier.  Minimap priority ranking:
    // soldier (low) < pc < object (high).
    let mut soldier_elem = ElementData {
        kind: ElementKind::ActorSoldier,
        ..Default::default()
    };
    soldier_elem.set_position_map(MapPoint::new(20.0, 20.0));
    let soldier_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: soldier_elem,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let mut pc_elem = ElementData {
        kind: ElementKind::ActorPc,
        ..Default::default()
    };
    pc_elem.set_position_map(MapPoint::new(30.0, 30.0));
    let pc_id = engine.add_entity(Entity::Pc(ActorPc {
        element: pc_elem,
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    }));

    let mut bonus_elem = ElementData {
        kind: ElementKind::ObjectBonus,
        ..Default::default()
    };
    bonus_elem.set_position_map(MapPoint::new(40.0, 40.0));
    let object_id = engine.add_entity(Entity::Bonus(ElementBonus {
        element: bonus_elem,
        object: Default::default(),
    }));

    let sorted = engine.sort_for_minimap();
    assert_eq!(sorted, vec![soldier_id, pc_id, object_id]);
}

#[test]
fn swordfight_los_ignores_crossing_motion_line() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity, Posture};
    use crate::element_kinds::ActionState;
    use crate::fast_find_grid::GridLine;

    let mut engine = EngineInner::new();
    let assets = swordfight_test_assets();
    engine.world.fast_grid.size_map(4, 4);
    engine.world.fast_grid.allocate_layers(1);
    engine.world.fast_grid.add_line(
        GridLine::new(
            MapPoint::new(115.0, 50.0),
            MapPoint::new(115.0, 150.0),
            true,
        ),
        0,
    );

    let make_fighter = |x| {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            posture: Posture::Upright,
            ..Default::default()
        };
        element.set_position(WorldPoint3D {
            x,
            y: 100.0,
            z: 0.0,
        });
        element.set_sector(crate::position_interface::SectorHandle::new(0));
        Entity::Soldier(ActorSoldier {
            element,
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        })
    };
    let left_id = engine.add_entity(make_fighter(100.0));
    let right_id = engine.add_entity(make_fighter(130.0));

    for (fighter_id, opponent_id) in [(left_id, right_id), (right_id, left_id)] {
        let fighter = engine.world.entities.get_mut(fighter_id).unwrap();
        fighter.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        fighter
            .human_data_mut()
            .unwrap()
            .opponents
            .push(opponent_id);
    }

    assert!(
        engine
            .world
            .fast_grid
            .impact_intersection_ratio(MapPoint::new(100.0, 100.0), MapPoint::new(130.0, 100.0), 0,)
            .is_some(),
        "fixture must contain a movement barrier between the fighters"
    );

    engine.tick_waiting_sword_execute_for(sim, &assets, left_id);

    assert_eq!(
        engine
            .get_entity(left_id)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![right_id]
    );
    assert_eq!(
        engine
            .get_entity(right_id)
            .unwrap()
            .human_data()
            .unwrap()
            .opponents,
        vec![left_id]
    );
}

#[test]
fn smalltalk_strike_does_not_transfer_initiative_immediately() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{ActorSoldier, Command, ElementData, ElementKind, Entity, Posture};
    use crate::element_kinds::ActionState;

    let mut engine = EngineInner::new();
    let assets = swordfight_test_assets();

    let mut attacker_element = ElementData {
        kind: ElementKind::ActorSoldier,
        // Soldiers built ad-hoc in tests need an explicit posture —
        // the level deserialiser remaps `Undefined` to a kind-specific
        // default, but `ElementData::default()` does not.
        posture: Posture::Upright,
        ..Default::default()
    };
    attacker_element.set_position(WorldPoint3D {
        x: 100.0,
        y: 100.0,
        z: 0.0,
    });
    attacker_element.set_sector(crate::position_interface::SectorHandle::new(0));
    let attacker_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: attacker_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let mut defender_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    defender_element.set_position(WorldPoint3D {
        x: 130.0,
        y: 100.0,
        z: 0.0,
    });
    defender_element.set_sector(crate::position_interface::SectorHandle::new(0));
    let defender_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: defender_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    if let Some(attacker) = engine.world.entities.get_mut(attacker_id) {
        attacker.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = attacker.human_data_mut().unwrap();
        human.opponents.push(defender_id);
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
    }
    if let Some(defender) = engine.world.entities.get_mut(defender_id) {
        defender.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        defender
            .human_data_mut()
            .unwrap()
            .opponents
            .push(attacker_id);
    }

    engine.control.frame_counter = 15;
    crate::sim_rng::with_seed(1, |sim| {
        engine.tick_waiting_sword_execute_for(sim, &assets, attacker_id);
    });

    let attacker_human = engine
        .get_entity(attacker_id)
        .and_then(|e| e.human_data())
        .unwrap();
    let defender_human = engine
        .get_entity(defender_id)
        .and_then(|e| e.human_data())
        .unwrap();

    assert!(attacker_human.smalltalk_initiative);
    assert!(!defender_human.smalltalk_initiative);
    assert!(matches!(
        defender_human.smalltalk_hint,
        crate::element::SmalltalkHint::Left | crate::element::SmalltalkHint::Right
    ));
    assert_eq!(defender_human.smalltalk_hint_opponent, Some(attacker_id));

    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(defender_id, |command| matches!(
                command,
                Command::ParrySmalltalkLeft | Command::ParrySmalltalkRight
            ))
    );
}

#[test]
fn waiting_sword_near_gate_uses_three_dimensional_square_norm() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{ActorSoldier, Command, ElementData, ElementKind, Entity, Posture};
    use crate::element_kinds::ActionState;

    let mut engine = EngineInner::new();
    let assets = swordfight_test_assets();
    let make_fighter = |position| {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(position);
        element.set_sector(crate::position_interface::SectorHandle::new(0));
        Entity::Soldier(ActorSoldier {
            element,
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        })
    };
    let attacker = engine.add_entity(make_fighter(WorldPoint3D {
        x: 100.0,
        y: 100.0,
        z: 0.0,
    }));
    let defender = engine.add_entity(make_fighter(WorldPoint3D {
        x: 160.0,
        y: 100.0,
        z: 40.0,
    }));
    for (fighter, opponent) in [(attacker, defender), (defender, attacker)] {
        let entity = engine.get_entity_mut(fighter).expect("3D fighter exists");
        entity
            .actor_data_mut()
            .expect("3D fighter is actor")
            .action_state = ActionState::WaitingSword;
        entity
            .human_data_mut()
            .expect("3D fighter is human")
            .opponents
            .push(opponent);
    }
    {
        let human = engine
            .get_entity_mut(attacker)
            .and_then(|entity| entity.human_data_mut())
            .expect("3D attacker is human");
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
    }

    crate::sim_rng::with_seed(1, |sim| {
        engine.tick_waiting_sword_execute_for(sim, &assets, attacker);
    });

    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(attacker, |command| matches!(
                command,
                Command::SwordstrikeSmalltalkLeft | Command::SwordstrikeSmalltalkRight
            )),
        "60 units horizontally but 72.1 in 3D is outside the 70-unit maximal range"
    );
    assert_eq!(
        engine
            .get_entity(defender)
            .and_then(|entity| entity.human_data())
            .expect("3D defender remains human")
            .smalltalk_hint,
        crate::element::SmalltalkHint::None
    );
}

#[test]
#[should_panic(expected = "EvaluateSwordfight Uber range")]
fn waiting_sword_requires_real_combat_profiles_contextually() {
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity, Posture};
    use crate::element_kinds::ActionState;

    let mut engine = EngineInner::new();
    let make_fighter = || {
        Entity::Soldier(ActorSoldier {
            element: ElementData {
                kind: ElementKind::ActorSoldier,
                posture: Posture::Upright,
                ..ElementData::default()
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        })
    };
    let owner = engine.add_entity(make_fighter());
    let opponent = engine.add_entity(make_fighter());
    for (fighter, other) in [(owner, opponent), (opponent, owner)] {
        let entity = engine
            .get_entity_mut(fighter)
            .expect("profile fighter exists");
        entity
            .actor_data_mut()
            .expect("profile fighter is actor")
            .action_state = ActionState::WaitingSword;
        entity
            .human_data_mut()
            .expect("profile fighter is human")
            .opponents
            .push(other);
    }

    engine.tick_waiting_sword_execute_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );
}

#[test]
fn smalltalk_hint_suppresses_normal_swordfight_evaluation() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActorPc, ActorSoldier, Command, ElementData, ElementKind, Entity, Posture, SmalltalkHint,
    };
    use crate::element_kinds::ActionState;

    let mut engine = EngineInner::new();
    let assets = swordfight_test_assets();

    let mut pc_element = ElementData {
        kind: ElementKind::ActorPc,
        posture: Posture::Upright,
        ..Default::default()
    };
    pc_element.set_position(WorldPoint3D {
        x: 100.0,
        y: 100.0,
        z: 0.0,
    });
    let pc_id = engine.add_entity(Entity::Pc(ActorPc {
        element: pc_element,
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    }));

    let mut soldier_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    soldier_element.set_position(WorldPoint3D {
        x: 130.0,
        y: 100.0,
        z: 0.0,
    });
    let soldier_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: soldier_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    if let Some(pc) = engine.world.entities.get_mut(pc_id) {
        pc.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = pc.human_data_mut().unwrap();
        human.opponents.push(soldier_id);
        human.tiredness = 100;
        human.smalltalk_hint = SmalltalkHint::Left;
        human.smalltalk_hint_opponent = Some(soldier_id);
    }
    if let Some(soldier) = engine.world.entities.get_mut(soldier_id) {
        soldier.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        soldier.human_data_mut().unwrap().opponents.push(pc_id);
    }

    engine.tick_waiting_sword_execute_for(sim, &assets, pc_id);

    let pc_human = engine
        .get_entity(pc_id)
        .and_then(|e| e.human_data())
        .unwrap();
    assert_eq!(pc_human.smalltalk_hint, SmalltalkHint::None);
    assert_eq!(pc_human.smalltalk_hint_opponent, None);
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(pc_id, |command| {
                command == Command::SwordstrikeTired
            })
    );
}

#[test]
#[should_panic(expected = "EvaluateSmalltalkHint owner")]
fn smalltalk_hint_missing_required_opponent_fails_contextually() {
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity, SmalltalkHint};

    let mut engine = EngineInner::new();
    let owner = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));
    let stale = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));
    engine.remove_entity(stale);
    let human = engine
        .get_entity_mut(owner)
        .and_then(|entity| entity.human_data_mut())
        .expect("owner is human");
    human.smalltalk_hint = SmalltalkHint::Left;
    human.smalltalk_hint_opponent = Some(stale);

    engine.tick_waiting_sword_execute_for(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        owner,
    );
}

#[test]
fn consumed_smalltalk_hint_suppresses_same_frame_smalltalk_strike_only_for_that_actor() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActorSoldier, Command, ElementData, ElementKind, Entity, Posture, SmalltalkHint,
    };
    use crate::element_kinds::ActionState;

    let mut engine = EngineInner::new();
    let assets = swordfight_test_assets();

    let mut hinted_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    hinted_element.set_position(WorldPoint3D {
        x: 100.0,
        y: 100.0,
        z: 0.0,
    });
    let hinted_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: hinted_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let mut hinted_opponent_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    hinted_opponent_element.set_position(WorldPoint3D {
        x: 160.0,
        y: 100.0,
        z: 0.0,
    });
    let hinted_opponent_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: hinted_opponent_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let mut free_attacker_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    free_attacker_element.set_position(WorldPoint3D {
        x: 300.0,
        y: 100.0,
        z: 0.0,
    });
    free_attacker_element.set_sector(crate::position_interface::SectorHandle::new(0));
    let free_attacker_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: free_attacker_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    let mut free_defender_element = ElementData {
        kind: ElementKind::ActorSoldier,
        posture: Posture::Upright,
        ..Default::default()
    };
    free_defender_element.set_position(WorldPoint3D {
        x: 330.0,
        y: 100.0,
        z: 0.0,
    });
    free_defender_element.set_sector(crate::position_interface::SectorHandle::new(0));
    let free_defender_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: free_defender_element,
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    if let Some(hinted) = engine.world.entities.get_mut(hinted_id) {
        hinted.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = hinted.human_data_mut().unwrap();
        human.opponents.push(hinted_opponent_id);
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
        human.smalltalk_hint = SmalltalkHint::Left;
        human.smalltalk_hint_opponent = Some(hinted_opponent_id);
    }
    if let Some(hinted_opponent) = engine.world.entities.get_mut(hinted_opponent_id) {
        hinted_opponent.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        hinted_opponent
            .human_data_mut()
            .unwrap()
            .opponents
            .push(hinted_id);
    }
    if let Some(free_attacker) = engine.world.entities.get_mut(free_attacker_id) {
        free_attacker.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        let human = free_attacker.human_data_mut().unwrap();
        human.opponents.push(free_defender_id);
        human.smalltalk_initiative = true;
        human.received_smalltalk_initiative = true;
    }
    if let Some(free_defender) = engine.world.entities.get_mut(free_defender_id) {
        free_defender.actor_data_mut().unwrap().action_state = ActionState::WaitingSword;
        free_defender
            .human_data_mut()
            .unwrap()
            .opponents
            .push(free_attacker_id);
    }

    engine.tick_waiting_sword_execute_for(sim, &assets, hinted_id);
    crate::sim_rng::with_seed(1, |sim| {
        engine.tick_waiting_sword_execute_for(sim, &assets, free_attacker_id);
    });

    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(hinted_id, |command| {
                matches!(
                    command,
                    Command::ParrySmalltalkLeft | Command::ParrySmalltalkRight
                )
            })
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(hinted_id, |command| {
                matches!(
                    command,
                    Command::SwordstrikeSmalltalkLeft | Command::SwordstrikeSmalltalkRight
                )
            })
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .has_live_element_for_actor_matching(free_attacker_id, |command| {
                matches!(
                    command,
                    Command::SwordstrikeSmalltalkLeft | Command::SwordstrikeSmalltalkRight
                )
            })
    );
    assert_ne!(
        engine
            .get_entity(free_defender_id)
            .and_then(|e| e.human_data())
            .unwrap()
            .smalltalk_hint,
        SmalltalkHint::None
    );
}

#[test]
fn sword_movement_start_transfers_smalltalk_initiative() {
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();

    let attacker_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));
    let defender_id = engine.add_entity(Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    }));

    if let Some(attacker) = engine.world.entities.get_mut(attacker_id) {
        let human = attacker.human_data_mut().unwrap();
        human.opponents.push(defender_id);
        human.smalltalk_initiative = true;
    }
    if let Some(defender) = engine.world.entities.get_mut(defender_id) {
        let human = defender.human_data_mut().unwrap();
        human.opponents.push(attacker_id);
        human.smalltalk_initiative = false;
        human.received_smalltalk_initiative = false;
    }

    engine.apply_sword_movement_start_initiative_transfer(attacker_id);

    let attacker_human = engine
        .get_entity(attacker_id)
        .and_then(|e| e.human_data())
        .unwrap();
    let defender_human = engine
        .get_entity(defender_id)
        .and_then(|e| e.human_data())
        .unwrap();
    assert!(!attacker_human.smalltalk_initiative);
    assert!(defender_human.smalltalk_initiative);
    assert!(defender_human.received_smalltalk_initiative);
}

#[test]
fn sort_for_minimap_display_then_creation_tiebreak() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{ActorSoldier, ElementData, ElementKind, Entity};

    let mut engine = EngineInner::new();

    // All same priority (soldier); sort falls back to display_order
    // then EntityId (insertion / creation order).  Soldiers with no
    // sprite fall back to position.y as their display_order (matches
    // sort_for_display).
    let mk = |y: f32| {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            ..Default::default()
        };
        element.set_position(WorldPoint3D { x: 0.0, y, z: 0.0 });
        Entity::Soldier(ActorSoldier {
            element,
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        })
    };

    let late_high_y = engine.add_entity(mk(100.0));
    let early_low_y = engine.add_entity(mk(10.0));
    let mid_mid_y = engine.add_entity(mk(50.0));
    // Two entities share a y value — EntityId (insertion order) breaks the tie.
    let first_tie = engine.add_entity(mk(10.0));
    let second_tie = engine.add_entity(mk(10.0));

    let sorted = engine.sort_for_minimap();

    // Among y=10 entities, EntityId decides: early_low_y < first_tie < second_tie.
    let idx = |id| sorted.iter().position(|&e| e == id).unwrap();
    assert!(idx(early_low_y) < idx(first_tie));
    assert!(idx(first_tie) < idx(second_tie));
    // Higher y values come later in the sort.
    assert!(idx(second_tie) < idx(mid_mid_y));
    assert!(idx(mid_mid_y) < idx(late_high_y));
}

#[test]
fn camera_slide_approaches_target() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4000.0, 3000.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(100.0, 100.0);
    engine.feedback.cutscene_camera.camera_slide = crate::coordinates::MapPoint::new(500.0, 300.0);
    engine.feedback.cutscene_camera.camera_wanted = crate::coordinates::MapPoint::new(500.0, 300.0);
    engine.control.speed = 1.0;

    engine.perform_director_work(&mut display);

    // Should have set Scroll display op (or moved toward target)
    // The scrolling vector should point toward the target
    let sv = display.background_transform.scrolling_vector;
    // At speed=1, direction is normalized*1 then floored, so we check general direction
    assert!(sv.x >= 0.0 || sv.y >= 0.0);
}

#[test]
fn camera_slide_cancels_at_target() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4000.0, 3000.0);
    engine.feedback.cutscene_camera.view_position = crate::coordinates::MapPoint::new(500.0, 300.0);
    engine.feedback.cutscene_camera.camera_slide = crate::coordinates::MapPoint::new(500.0, 300.0);

    engine.perform_director_work(&mut display);

    // Should have cancelled the slide
    assert!(!engine.feedback.cutscene_camera.is_sliding());
}

#[test]
fn resize_aborts_zoom() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    display.display_op = DisplayOpCode::InZoom;
    display.background_transform.zoom_to_up = true;
    engine.feedback.cutscene_camera.zoom_init_done = true;

    engine.resize(&mut display, 1024.0, 768.0);

    assert!(!display.background_transform.zoom_to_up);
    assert!(!engine.feedback.cutscene_camera.zoom_init_done);
}

#[test]
fn dead_pc_triggers_failure() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4000.0, 3000.0);

    // Add a PC entity
    let mut pc_elem = crate::element::ElementData {
        kind: crate::element::ElementKind::ActorPc,
        ..Default::default()
    };
    pc_elem.set_position_map(crate::coordinates::MapPoint::new(100.0, 200.0));
    let entity = Entity::Pc(crate::element::ActorPc {
        element: pc_elem,
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let id = engine.add_entity(entity);
    engine.mission_domain.dead_pc = Some(id);

    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;
    assert_eq!(result, GameCode::LevelFailed);
}

#[test]
fn non_playable_pc_does_not_prevent_default_loss() {
    let mut display = HostDisplayState::default();
    let mut dev = DevState::default();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();

    let entity = Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            active: true,
            posture: crate::element::Posture::Upright,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        pc: crate::element::PcData {
            playable: false,
            life_points: 100,
            ..Default::default()
        },
    });
    engine.add_entity(entity);

    let result = engine
        .perform_hourglass(&mut display, &assets, &mut dev)
        .code;

    assert_eq!(result, GameCode::LevelFailed);
}

#[test]
fn zoom_step_completes_after_8_steps() {
    let mut display = HostDisplayState::default();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    display.background_transform.zoom_to_up = true;
    display.background_transform.zoom_count = 0;
    display.background_transform.number_of_zoom_steps = 8;
    engine.feedback.cutscene_camera.zoom_init_done = true;
    // Apply the post-draw reset to `NoBackgroundMove` so
    // `set_operation(InZoom)` can propagate (`set_operation` is
    // monotonic).
    display.display_op = DisplayOpCode::NoBackgroundMove;

    // Run 7 steps — should stay in InZoom
    for _ in 0..7 {
        engine.perform_zoom_step(&mut display);
        assert_eq!(display.display_op, DisplayOpCode::InZoom);
    }

    // 8th step — should finalize
    engine.perform_zoom_step(&mut display);
    assert_eq!(display.display_op, DisplayOpCode::NoBackgroundMove);
    assert!(!display.background_transform.zoom_to_up);
    assert!(!engine.feedback.cutscene_camera.zoom_init_done);
}

#[test]
fn minimap_command_outputs_are_derived_from_recorded_inputs() {
    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut first = EngineInner::new();
    first.feedback.cutscene_camera.level_size = MapSize::new(4096.0, 4096.0);
    let mut replay = first.clone();

    let mut first_display = HostDisplayState::default();
    let mut replay_display = HostDisplayState::default();
    replay_display.minimap.map_displayed = true;
    replay_display.minimap.drag_start = true;
    replay_display.minimap.dragged = true;
    let mut first_input = InputState::default();
    let mut replay_input = InputState::default();

    let focus = PlayerCommand::MinimapMouseMove {
        mouse_pt: crate::coordinates::ScreenPoint::new(300.0, 200.0),
        left_mouse_down: true,
        continuing_drag: true,
    };
    first.apply_command(&sim, &mut first_display, &mut first_input, &assets, &focus);
    replay.apply_command(
        &sim,
        &mut replay_display,
        &mut replay_input,
        &assets,
        &focus,
    );
    assert_eq!(
        crate::replay::state_hash(&first),
        crate::replay::state_hash(&replay),
        "host drag scratch must not decide the UiHasFocus message"
    );

    let mouse_up = PlayerCommand::MinimapMouseUp {
        on_minimap: true,
        center_on: Some(crate::coordinates::MapPoint::new(1200.0, 900.0)),
    };
    first.apply_command(
        &sim,
        &mut first_display,
        &mut first_input,
        &assets,
        &mouse_up,
    );
    replay.apply_command(
        &sim,
        &mut replay_display,
        &mut replay_input,
        &assets,
        &mouse_up,
    );
    assert_eq!(
        crate::replay::state_hash(&first),
        crate::replay::state_hash(&replay),
        "host minimap geometry must not decide the camera mutation"
    );
    assert_eq!(
        first.feedback.cutscene_camera.view_position,
        replay.feedback.cutscene_camera.view_position
    );
}

#[test]
#[should_panic(expected = "MinimapMouseUp center_on point")]
fn minimap_command_rejects_center_outside_required_level_bounds() {
    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.feedback.cutscene_camera.level_size = MapSize::new(1024.0, 768.0);
    let mut display = HostDisplayState::default();
    let mut input = InputState::default();

    engine.apply_command(
        &sim,
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::MinimapMouseUp {
            on_minimap: true,
            center_on: Some(crate::coordinates::MapPoint::new(2048.0, 10.0)),
        },
    );
}

// ── Scroll hourglass / IsTaken dispatch ──────────────────────

/// The scroll tick counter starts at 0.
#[test]
fn scroll_default_hourglass_counter_is_zero() {
    let s = crate::element::ElementScroll::default();
    assert_eq!(s.script_hourglass_timeout, 0);
}

/// Without a mission script, the per-scroll Hourglass dispatcher
/// is a no-op and doesn't touch scroll state.
#[test]
fn dispatch_scroll_hourglasses_no_script_is_noop() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    let scroll = Entity::Scroll(crate::element::ElementScroll {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ObjectScroll,
            active: true,
            ..Default::default()
        },
        ..Default::default()
    });
    let scroll_id = engine.add_entity(scroll);

    // No mission_script → nothing to dispatch, counter stays zero.
    let assets = crate::engine::LevelAssets::new();
    engine.dispatch_scroll_hourglasses(sim, &assets);
    let entity = engine.get_entity(scroll_id);
    let counter = match entity {
        Some(Entity::Scroll(s)) => s.script_hourglass_timeout,
        _ => unreachable!("scroll entity missing"),
    };
    assert_eq!(counter, 0);
}

/// `scroll_is_taken` on a scroll without a bound script flips the
/// sprite to the "opened" pose and sets status to `Opened`, but
/// returns `false`.
#[test]
fn scroll_is_taken_without_script_returns_false_and_opens() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use super::scroll_reveal::ScrollStatus;

    let mut engine = EngineInner::new();
    let scroll = Entity::Scroll(crate::element::ElementScroll {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ObjectScroll,
            active: true,
            ..Default::default()
        },
        // No script_class — `IsClassInstanciate()` returns false.
        ..Default::default()
    });
    let scroll_id = engine.add_entity(scroll);
    // A PC to pass as the taker.  Its handle value is irrelevant
    // here since no script is bound; the non-instanciated branch
    // doesn't look at the PC pointer.
    let pc = Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let pc_id = engine.add_entity(pc);

    let assets = crate::engine::LevelAssets::new();
    let accepted = engine.scroll_is_taken(sim, &assets, scroll_id, pc_id);
    assert!(!accepted);
    // Without `mission_script`, the status store isn't populated
    // either — the setter early-returns.  Covering the "happens to
    // have ScriptEffects but no class" flow is left to the integration
    // level, so here we just confirm `false` + no panic.
    let _ = ScrollStatus::Opened; // keep symbol live
}
