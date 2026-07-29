use super::*;

#[test]
#[should_panic(expected = "combat AI requires the level profile manager")]
fn required_profiles_reject_narrow_noncombat_tick_data() {
    AiPerTickData::stub().required_profile_manager();
}

#[test]
fn required_profile_manager_returns_the_supplied_level_profiles() {
    let mut profiles = crate::profiles::ProfileManager::new();
    profiles.hth_weapons.push(Default::default());
    let profiles = std::sync::Arc::new(profiles);
    let mut tick = AiPerTickData::stub();
    tick.profile_manager = Some(profiles.clone());

    assert!(std::ptr::eq(
        tick.required_profile_manager(),
        profiles.as_ref()
    ));
}

#[test]
fn substate_groups() {
    assert!(Substate::SeekingSeekpoint.is_seek_area());
    assert!(!Substate::DefaultOnPost.is_seek_area());
    assert!(Substate::AttackingSwordfight.is_any_swordfight());
    assert!(Substate::AttackingSwordfight.is_real_swordfight());
    assert!(!Substate::AttackingBowShooting.is_any_swordfight());
}

#[test]
fn ai_log_stimulus_strings_match_original_names_and_fallback() {
    assert_eq!(
        StimulusType::log_string_from_u16(StimulusType::EventView as u16),
        "EVENT-VIEW"
    );
    assert_eq!(
        StimulusType::log_string_from_u16(StimulusType::EventSeesFriendInTrouble as u16),
        "EVENT-SEESFRIENDINTROUBLE"
    );
    assert_eq!(
        StimulusType::log_string_from_u16(StimulusType::NoEvent as u16),
        "EVENT-???"
    );
    assert_eq!(StimulusType::log_string_from_u16(u16::MAX), "EVENT-???");
}

#[test]
fn ai_log_substate_strings_match_original_names_and_fallback() {
    assert_eq!(
        Substate::log_string_from_u16(Substate::DefaultGotoPost as u16),
        "SUBSTATE-DEFAULT-GOTOPOST"
    );
    assert_eq!(
        Substate::log_string_from_u16(Substate::AttackingSwordfight as u16),
        "SUBSTATE-ATTACKING-SWORDFIGHT"
    );
    assert_eq!(
        Substate::log_string_from_u16(Substate::AttackingArcherWaitOnArcheryPath as u16),
        "SUBSTATE-ATTACKING-ARCHER-WAIT-ON-ACHERY-PATH"
    );
    assert_eq!(
        Substate::log_string_from_u16(Substate::DefaultGotoChief as u16),
        "SUBSTATE-DEFAULT-GOTOCHIEF"
    );
    assert_eq!(
        Substate::log_string_from_u16(Substate::AttackingRunToAvengerOnRoof as u16),
        "SUBSTATE-???"
    );
    assert_eq!(Substate::log_string_from_u16(u16::MAX), "SUBSTATE-???");
}

#[test]
fn ai_log_decision_strings_match_original_names_and_fallback() {
    assert_eq!(
        Decision::log_string_from_u16(Decision::Fight as u16),
        "DECISION-FIGHT"
    );
    assert_eq!(
        Decision::log_string_from_u16(Decision::LookForHelp as u16),
        "DECISION-LOOK-4-HELP"
    );
    assert_eq!(
        Decision::log_string_from_u16(Decision::PredecisionOffensive as u16),
        "DECISION-???"
    );
    assert_eq!(Decision::log_string_from_u16(u16::MAX), "DECISION-???");
}

#[test]
fn ai_log_remark_strings_match_original_speech_and_fallback() {
    assert_eq!(
        Remark::log_string_from_u16(Remark::SeesBody as u16),
        "Ca va?"
    );
    assert_eq!(
        Remark::log_string_from_u16(Remark::TheSoundOfSilence as u16),
        " ........... "
    );
    assert_eq!(Remark::log_string_from_u16(u16::MAX), " ........... ");
}

#[test]
fn stimulus_similarity() {
    let a = Stimulus::new(StimulusType::EventTimer);
    let b = Stimulus::new(StimulusType::EventTimer);
    assert!(a.is_similar(&b));

    let c = Stimulus::new(StimulusType::EventDone);
    assert!(!a.is_similar(&c));

    let d = Stimulus::with_human(StimulusType::EventView, 42);
    let e = Stimulus::with_human(StimulusType::EventView, 42);
    assert!(d.is_similar(&e));

    let f = Stimulus::with_human(StimulusType::EventView, 99);
    assert!(!d.is_similar(&f));
}

#[test]
fn consideration_accumulator() {
    let mut acc = ConsiderationAccumulator::default();
    acc.consider_value(true, 80, 1, 0);
    acc.consider_value(true, 60, 1, 0);
    let result = acc.evaluate();
    assert_eq!(result, 70);
}

#[test]
fn value_between() {
    // The Windows x87 path retains the slightly-low binary32 value of
    // 0.01 through the complete expression before truncating.
    assert_eq!(AiController::value_between(0, 100, 50), 49);
    assert_eq!(AiController::value_between(0, 100, 0), 0);
    assert_eq!(AiController::value_between(0, 100, 99), 98);
    assert_eq!(AiController::value_between(0, 100, 100), 99);
    assert_eq!(AiController::value_between(10, 90, 50), 49);
    assert_eq!(AiController::value_between(10, 90, 100), 89);
    assert_eq!(AiController::value_between(90, 10, 50), 50);
}

#[test]
fn ai_controller_defaults() {
    let ai = AiController::new(1);
    assert_eq!(ai.me, 1);
    assert_eq!(ai.current_state, AiState::Default);
    assert_eq!(ai.current_substate, Substate::DefaultOnPost);
    assert_eq!(ai.attitude, Attitude::Suspicious);
    assert!(!ai.ai_is_locked());
}

#[test]
fn consider_report_preserves_pc_body_kind_in_detectable_effect() {
    let pc = crate::element::Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    let mut views = crate::ai_entity_view::AiEntityViewMap::new();
    views.insert(
        17,
        crate::ai_entity_view::entity_view_from_entity(&pc, false, None, None),
    );
    let mut report = ReconnaissanceReport::default();
    report.add_seen_body(17);
    let mut ai = AiController::new(1);

    ai.consider_report_merged(&report, 0, &views);

    assert_eq!(
        ai.outbox.actor.delete_detectable_entity,
        vec![(
            crate::element::EntityId::Pc(crate::entity_id::PcId(17)),
            crate::element::DetectableType::Body,
        )]
    );
}

#[test]
fn goto_sword_sets_force_sword_movement_flag() {
    let order = AiController::make_move_order(
        &Position {
            x: 100.0,
            y: 200.0,
            sector: None,
            level: 0,
        },
        GotoFlags::SWORD,
    );

    let flags = crate::sequence::MoveFlags::from_bits_truncate(u32::from(order.move_flags));
    assert!(flags.contains(crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT));
}

#[test]
fn goto_dont_stop_suppresses_movement_transitions() {
    let order = AiController::make_move_order(
        &Position {
            x: 100.0,
            y: 200.0,
            sector: None,
            level: 0,
        },
        GotoFlags::DONT_STOP,
    );

    let flags = crate::sequence::MoveFlags::from_bits_truncate(u32::from(order.move_flags));
    assert!(flags.contains(crate::sequence::MoveFlags::NO_TRANSITIONS));
}

#[test]
fn ordinary_goto_from_sword_state_carries_ordered_quit_prefix() {
    let mut ctx = goto_short_circuit_ctx(crate::order::OrderType::WalkingWithSword);
    ctx.self_action_state = crate::element::ActionState::MovingSword;
    let destination = Position {
        x: 200.0,
        y: 200.0,
        ..ctx.position
    };
    let mut ai = AiController::new(1);

    ai.go_to(destination, GotoFlags::RUN, &ctx);

    let orders = ai.take_pending_orders();
    assert_eq!(orders.len(), 1);
    assert!(orders[0].quit_swordfight_before_move);
    assert!(
        !ai.outbox.actor.quit_swordfight,
        "GoTo teardown belongs to the movement sequence, not the standalone relationship effect"
    );
}

#[test]
fn goto_find_accessible_and_ask_obstacle_survive_order_intent() {
    let order = AiController::make_move_order(
        &Position {
            x: 100.0,
            y: 200.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        GotoFlags::FIND_ACCESSIBLE | GotoFlags::ASK_OBSTACLE | GotoFlags::STRAIGHT,
    );

    assert!(order.find_accessible);
    assert!(order.ask_obstacle);
    assert!(!order.compute_direction);
    let move_flags = crate::sequence::MoveFlags::from_bits_truncate(u32::from(order.move_flags));
    assert!(move_flags.contains(crate::sequence::MoveFlags::STRAIGHT));
}

fn goto_short_circuit_ctx(animation: crate::order::OrderType) -> AiContext {
    AiContext {
        position: Position {
            x: 100.0,
            y: 200.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        posture: crate::element::Posture::Upright,
        self_animation: animation,
        ..AiContext::default()
    }
}

#[test]
fn goto_already_on_point_uses_original_animation_gate() {
    for animation in [
        crate::order::OrderType::WaitingUpright,
        crate::order::OrderType::WaitingAlerted,
        crate::order::OrderType::NonanimationEnd,
    ] {
        let ctx = goto_short_circuit_ctx(animation);
        let mut ai = AiController::new(1);

        ai.go_to(ctx.position, GotoFlags::empty(), &ctx);

        assert!(ai.already_on_point);
        assert!(ai.take_pending_orders().is_empty());

        let mut speed_ai = AiController::new(1);
        speed_ai.go_to_speed(ctx.position, GotoFlags::empty(), 1.5, &ctx);
        assert!(speed_ai.already_on_point);
        assert!(speed_ai.take_pending_orders().is_empty());
    }

    let ctx = goto_short_circuit_ctx(crate::order::OrderType::WaitingUprightBored);
    let mut ai = AiController::new(1);

    ai.go_to(ctx.position, GotoFlags::empty(), &ctx);

    assert!(!ai.already_on_point);
    assert_eq!(ai.take_pending_orders().len(), 1);

    let running_ctx = goto_short_circuit_ctx(crate::order::OrderType::RunningUpright);
    let mut speed_ai = AiController::new(1);
    speed_ai.go_to_speed(running_ctx.position, GotoFlags::empty(), 1.5, &running_ctx);
    assert!(!speed_ai.already_on_point);
    assert_eq!(speed_ai.take_pending_orders().len(), 1);
}

#[test]
fn run_to_map_exit_queues_running_map_movement() {
    let mut ai = AiController::new(1);
    let destination = Position {
        x: 123.0,
        y: 456.0,
        sector: SectorHandle::new(7),
        level: 0,
    };

    ai.run_to_map_exit(destination);
    let orders = ai.take_pending_orders();

    assert_eq!(orders.len(), 1);
    assert_eq!(
        orders[0].order_type,
        crate::order::OrderType::RunningUpright
    );
    let flags = crate::sequence::MoveFlags::from_bits_truncate(u32::from(orders[0].move_flags));
    assert!(flags.contains(crate::sequence::MoveFlags::MAP));
}

fn face_to_ctx(action_state: crate::element::ActionState) -> AiContext {
    AiContext {
        position: Position {
            x: 10.0,
            y: 20.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        direction: 4,
        posture: crate::element::Posture::Upright,
        self_action_state: action_state,
        ..AiContext::default()
    }
}

fn same_direction_target(ctx: &AiContext) -> Position {
    let dir = crate::shadow_polygon::sector_to_direction(ctx.direction as i16);
    Position {
        x: ctx.position.x + dir[0] * 100.0,
        y: ctx.position.y + dir[1] * 100.0,
        ..ctx.position
    }
}

fn assert_same_direction_short_circuits(mut ai: AiController, ctx: &AiContext) {
    ai.face_direction(ctx.direction, ctx);

    assert!(ai.already_turned);
    assert!(!ai.outbox.actor.halt);
    assert!(ai.take_pending_orders().is_empty());
}

#[test]
fn face_to_same_direction_waiting_short_circuits() {
    let ctx = face_to_ctx(crate::element::ActionState::Waiting);

    assert_same_direction_short_circuits(AiController::new(1), &ctx);
}

#[test]
fn face_to_same_direction_bored_short_circuits() {
    let ctx = face_to_ctx(crate::element::ActionState::Bored);
    let mut ai = AiController::new(1);
    ai.face_position_with_ctx(same_direction_target(&ctx), &ctx);

    assert!(ai.already_turned);
    assert!(!ai.outbox.actor.halt);
    assert!(ai.take_pending_orders().is_empty());
}

#[test]
fn face_position_with_context_resolves_original_sector_before_launch() {
    let ctx = face_to_ctx(crate::element::ActionState::Moving);
    let target = Position {
        x: ctx.position.x + 100.0,
        y: ctx.position.y,
        ..ctx.position
    };
    let expected = crate::position_interface::vector_to_sector_0_to_15_iso(100.0, 0.0) as i16;
    let mut ai = AiController::new(1);

    ai.face_position_with_ctx(target, &ctx);
    let orders = ai.take_pending_orders();

    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_type, crate::order::OrderType::Turning);
    assert_eq!(orders[0].explicit_direction, Some(expected));
    assert!(!orders[0].compute_direction);
}

#[test]
fn fast_face_marks_the_turn_intent_without_changing_geometry() {
    let ctx = face_to_ctx(crate::element::ActionState::Moving);
    let target = Position {
        x: ctx.position.x + 100.0,
        y: ctx.position.y,
        ..ctx.position
    };
    let mut ai = AiController::new(1);

    ai.face_position_impl(target, &ctx, 0.0, true);
    let orders = ai.take_pending_orders();

    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_type, crate::order::OrderType::Turning);
    assert!(orders[0].fast_turn);
    assert_eq!(
        orders[0].explicit_direction,
        Some(crate::position_interface::vector_to_sector_0_to_15_iso(
            100.0, 0.0
        ))
    );
}

fn assert_same_direction_queues_turn(action_state: crate::element::ActionState) {
    let mut ai = AiController::new(1);
    let ctx = face_to_ctx(action_state);

    ai.face_direction(ctx.direction, &ctx);
    let orders = ai.take_pending_orders();

    assert!(!ai.already_turned);
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_type, crate::order::OrderType::Turning);
    assert!(!orders[0].no_halt);
}

#[test]
fn face_to_same_direction_upright_moving_launches_halting_turn() {
    assert_same_direction_queues_turn(crate::element::ActionState::Moving);
}

#[test]
fn face_to_same_direction_upright_non_waiting_states_launch_halting_turn() {
    for action_state in [
        crate::element::ActionState::MovingFast,
        crate::element::ActionState::AimingWithBow,
        crate::element::ActionState::HoldingShield,
        crate::element::ActionState::Menacing,
    ] {
        assert_same_direction_queues_turn(action_state);
    }
}

#[test]
fn face_direction_preserves_all_authored_isometric_sectors() {
    for direction in 0..16 {
        let mut ai = AiController::new(1);
        let mut ctx = face_to_ctx(crate::element::ActionState::Moving);
        ctx.direction = (direction + 8) & 15;

        ai.face_direction(direction, &ctx);
        let order = ai.take_pending_orders().pop().expect("queued Turn order");
        assert_eq!(order.explicit_direction, Some(direction as i16));
        assert!(!order.compute_direction);
    }
}

#[test]
fn goto_route_arrival_launches_turn_even_when_already_facing_route() {
    use crate::ai::macro_patrol::{PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let paths = vec![RawHikingPath {
        waypoints: vec![
            RawWaypoint {
                x: 0,
                y: 0,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            },
            RawWaypoint {
                x: 10,
                y: 0,
                sector: 1,
                level: 0,
                command: WaypointCommand::Macro(vec![1]),
            },
        ],
    }];
    let mut path = PatrolPath::new(PathId::new(0).unwrap(), &paths).unwrap();
    path.advance();

    let mut ai = AiController::new(1);
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultGotoRoute;
    ai.patrol_path = Some(path);

    let route_direction = crate::position_interface::vector_to_sector_0_to_15(10.0, 0.0) as u16;
    let ctx = AiContext {
        position: Position {
            x: 10.0,
            y: 0.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        direction: route_direction,
        posture: crate::element::Posture::Upright,
        self_action_state: crate::element::ActionState::Waiting,
        hiking_paths: std::sync::Arc::new(paths),
        ..AiContext::default()
    };

    ai.think_expected_event_common_stuff(
        &crate::sim_rng::test_context(),
        &Stimulus::new(StimulusType::EventReachPoint),
        &ctx,
    );

    assert_eq!(ai.current_substate, Substate::DefaultGotoRouteTurn);
    assert!(
        ai.outbox.reentrant.self_stimuli.is_empty(),
        "route arrival uses an explicit Turn, not FaceTo's same-direction EventDone shortcut"
    );
    let orders = ai.take_pending_orders();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_type, crate::order::OrderType::Turning);
    assert_eq!(orders[0].explicit_direction, Some(route_direction as i16));
}

#[test]
fn goto_route_turn_lookup_preserves_original_endpoint_direction_flip() {
    use crate::ai::macro_patrol::{PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let paths = vec![RawHikingPath {
        waypoints: vec![
            RawWaypoint {
                x: 0,
                y: 0,
                sector: 1,
                level: 0,
                command: WaypointCommand::Macro(vec![1]),
            },
            RawWaypoint {
                x: 10,
                y: 0,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            },
        ],
    }];

    let mut ai = AiController::new(1);
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultGotoRoute;
    ai.patrol_path = PatrolPath::new(PathId::new(0).unwrap(), &paths);
    let ctx = AiContext {
        position: Position {
            x: 0.0,
            y: 0.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        direction: 0,
        posture: crate::element::Posture::Upright,
        self_action_state: crate::element::ActionState::Waiting,
        hiking_paths: std::sync::Arc::new(paths),
        ..AiContext::default()
    };

    ai.think_expected_event_common_stuff(
        &crate::sim_rng::test_context(),
        &Stimulus::new(StimulusType::EventReachPoint),
        &ctx,
    );

    let path = ai.patrol_path.as_ref().expect("patrol path");
    assert_eq!(path.current_waypoint_index, 0);
    assert!(
        !path.forward,
        "Original's live --path/++path lookup reverses traversal at waypoint zero"
    );
}

// ──────────────────────────────────────────────────────────
// init_state — initial-action gate
// ──────────────────────────────────────────────────────────

#[test]
fn init_state_waiting_upright_returns_go_to_duty() {
    // `WaitingUpright` → OnPost + bored timer + `go_to_duty = true`.
    // This is the hot path — the vast majority of NPCs are
    // authored with this action.
    crate::sim_rng::with_seed(1, |sim| {
        let mut ai = AiController::new(1);
        ai.initial_action = crate::order::OrderType::WaitingUpright as u32;
        let fx = ai.init_state(sim, &AiContext::default());

        assert!(fx.go_to_duty);
        assert!(!fx.launch_wait);
        assert_eq!(ai.current_state, AiState::Default);
        assert_eq!(ai.current_substate, Substate::DefaultOnPost);
        assert!(fx.set_posture.is_none());
        assert!(!ai.likes_to_sit_around);
        assert!(!ai.special_action);
        assert!(!ai.is_stay_at_home);
    });
}

#[test]
fn init_state_sleeping_upright_closes_eyes_and_emoticon() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // `SleepingUpright` → SleepingNapping, eyes closed, Zzz
    // emoticon, upright posture + Sleeping action state.
    // `go_to_duty = false` — the NPC stays asleep until something
    // wakes them.
    use crate::element::{ActionState, EyeStatus, Posture};
    let mut ai = AiController::new(1);
    ai.initial_action = crate::order::OrderType::SleepingUpright as u32;
    let fx = ai.init_state(sim, &AiContext::default());

    assert!(!fx.go_to_duty);
    assert_eq!(ai.current_state, AiState::Sleeping);
    assert_eq!(ai.current_substate, Substate::SleepingNapping);
    assert_eq!(ai.current_emoticon_type, EmoticonType::Zzz);
    assert_eq!(fx.set_eye_status, Some(EyeStatus::Closed));
    assert_eq!(fx.set_posture, Some(Posture::Upright));
    assert_eq!(fx.set_action_state, Some(ActionState::Sleeping));
    assert!(fx.launch_wait);
}

#[test]
fn init_state_sitting_flags_likes_to_sit_around() {
    // `Sitting` → OnPost + Sitting posture, and crucially sets
    // `likes_to_sit_around = true` so `return_to_duty_common_stuff`
    // routes back to this place with the sitting-specific posture
    // gate.
    crate::sim_rng::with_seed(1, |sim| {
        let mut ai = AiController::new(1);
        ai.initial_action = crate::order::OrderType::Sitting as u32;
        let fx = ai.init_state(sim, &AiContext::default());

        assert!(!fx.go_to_duty);
        assert_eq!(ai.current_state, AiState::Default);
        assert_eq!(ai.current_substate, Substate::DefaultOnPost);
        assert!(ai.likes_to_sit_around);
        assert_eq!(fx.set_posture, Some(crate::element::Posture::Sitting));
        assert!(fx.launch_wait);
    });
}

#[test]
fn init_state_special_flags_special_action() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // `Special` → Leisure posture, flips `special_action = true`.
    // Pairs with the corresponding branch in
    // `return_to_duty_common_stuff`.
    let mut ai = AiController::new(1);
    ai.initial_action = crate::order::OrderType::Special as u32;
    let fx = ai.init_state(sim, &AiContext::default());

    assert!(!fx.go_to_duty);
    assert!(ai.special_action);
    assert_eq!(fx.set_posture, Some(crate::element::Posture::Leisure));
    assert!(fx.launch_wait);
}

#[test]
fn init_state_being_unconscious_queues_max_concussion() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // `BeingUnconscious` → SleepingUnconscious, Lying posture,
    // concussion/unconscious side effect.
    let mut ai = AiController::new(1);
    ai.initial_action = crate::order::OrderType::BeingUnconscious as u32;
    let fx = ai.init_state(sim, &AiContext::default());

    assert!(!fx.go_to_duty);
    assert_eq!(ai.current_state, AiState::Sleeping);
    assert_eq!(ai.current_substate, Substate::SleepingUnconscious);
    assert!(fx.concussion_max_and_unconscious);
    assert_eq!(fx.set_posture, Some(crate::element::Posture::Lying));
}

#[test]
fn init_state_being_dead_zeroes_life_points() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // `BeingDead{FallenBack}` → SleepingForever,
    // `zero_life_points` side effect. Two variants differ only in
    // posture.
    for (raw, expected_posture) in [
        (
            crate::order::OrderType::BeingDead as u32,
            crate::element::Posture::Dead,
        ),
        (
            crate::order::OrderType::BeingDeadFallenBack as u32,
            crate::element::Posture::DeadBack,
        ),
    ] {
        let mut ai = AiController::new(1);
        ai.initial_action = raw;
        let fx = ai.init_state(sim, &AiContext::default());

        assert!(!fx.go_to_duty);
        assert_eq!(ai.current_substate, Substate::SleepingForever);
        assert!(fx.zero_life_points);
        assert_eq!(fx.set_posture, Some(expected_posture));
    }
}

#[test]
fn init_state_in_building_stays_at_home() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // Indoor NPCs short-circuit to `is_stay_at_home=true` +
    // DefaultHomeSweetHome, regardless of `initial_action`.
    // `go_to_duty = false`.
    let mut ai = AiController::new(1);
    ai.initial_action = crate::order::OrderType::WaitingUpright as u32;
    let ctx = AiContext {
        in_building: true,
        building_sector: SectorHandle::new(7),
        ..AiContext::default()
    };
    let fx = ai.init_state(sim, &ctx);

    assert!(!fx.go_to_duty);
    assert!(ai.is_stay_at_home);
    assert_eq!(ai.current_substate, Substate::DefaultHomeSweetHome);
}

#[test]
fn init_state_resets_flags_before_branching() {
    // Calling `init_state` repeatedly should clear stale
    // `likes_to_sit_around` / `special_action` / `is_stay_at_home`
    // flags from a prior call.  Guards against level-editor
    // authored sequences that change `initial_action` between
    // init passes (e.g. respawn via script).
    crate::sim_rng::with_seed(1, |sim| {
        let mut ai = AiController::new(1);
        ai.likes_to_sit_around = true;
        ai.special_action = true;
        ai.is_stay_at_home = true;
        ai.initial_action = crate::order::OrderType::WaitingUpright as u32;

        let fx = ai.init_state(sim, &AiContext::default());

        assert!(fx.go_to_duty);
        assert!(!ai.likes_to_sit_around);
        assert!(!ai.special_action);
        assert!(!ai.is_stay_at_home);
    });
}

#[test]
fn emoticon_transient() {
    let mut ai = AiController::new(1);
    ai.set_transient_emoticon(EmoticonType::QuestionMark, 100, 500);
    assert_eq!(ai.current_emoticon_type, EmoticonType::QuestionMark);
    assert!(ai.emoticon_has_expiration_date);
    assert_eq!(ai.emoticon_expiration_date, 600);
}

#[test]
fn recon_report() {
    let mut report = ReconnaissanceReport::default();
    assert_eq!(report.report_type, ReportType::Nothing);

    report.update(
        ReportType::Body,
        Position {
            x: 10.0,
            y: 20.0,
            sector: None,
            level: 0,
        },
    );
    assert_eq!(report.report_type, ReportType::Body);

    // Lower priority update should be ignored
    report.update(
        ReportType::Noise,
        Position {
            x: 30.0,
            y: 40.0,
            sector: None,
            level: 0,
        },
    );
    assert_eq!(report.report_type, ReportType::Body);
    assert_eq!(report.seek_position.x, 10.0);

    // Higher priority update should apply
    report.update(
        ReportType::Enemy,
        Position {
            x: 50.0,
            y: 60.0,
            sector: None,
            level: 0,
        },
    );
    assert_eq!(report.report_type, ReportType::Enemy);
    assert_eq!(report.seek_position.x, 50.0);
}

#[test]
fn position_to_point_3d_uses_waypoint_sector_layer_projection() {
    let mut bbox = crate::coordinates::MapBBox::new();
    let points = vec![
        MapPoint::new(0.0, 0.0),
        MapPoint::new(100.0, 0.0),
        MapPoint::new(100.0, 100.0),
        MapPoint::new(0.0, 100.0),
    ];
    for &point in &points {
        bbox.expand_point(point);
    }

    let sector_number = crate::sector::SectorNumber::new(7);
    let mut level = crate::fast_find_grid::LevelGrid::default();
    level.sectors.push(crate::fast_find_grid::GridSector {
        points,
        bounding_box: bbox,
        sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        layer: 2,
        sector_number,
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    });
    level.sector_number_map.insert(sector_number, 0);

    let mut obstacle = crate::sight_obstacle::SightObstacle::new(
        0,
        crate::sight_obstacle::SIGHTOBSTACLE_SOLID
            | crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
    );
    obstacle.obstacle_points = vec![
        crate::sight_obstacle::ObstaclePoint {
            x: 0.0,
            y: 0.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 100.0,
            y: 0.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 100.0,
            y: 100.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
        crate::sight_obstacle::ObstaclePoint {
            x: 0.0,
            y: 100.0,
            z_bottom: 0.0,
            z_top: 20.0,
        },
    ];
    obstacle.layer = 2;
    obstacle.sector = 7;
    obstacle.top_plane_points = [[0.0, 0.0, 20.0], [100.0, 0.0, 20.0], [0.0, 100.0, 20.0]];
    obstacle.bottom_plane_points = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]];
    obstacle.rebuild_geometry();

    let ctx = AiContext {
        fast_grid: crate::fast_find_grid::FastFindGrid {
            level: std::sync::Arc::new(level),
            line_active: Vec::new(),
            sector_active: vec![true],
            mask_active: Vec::new(),
            lift_state: std::collections::BTreeMap::new(),
            sector_type_overlay: std::collections::BTreeMap::new(),
        },
        sight_obstacles: crate::sight_obstacle::SharedSightObstacles {
            static_obstacles: std::sync::Arc::new(vec![obstacle]),
            dynamic_obstacles: std::sync::Arc::new(Vec::new()),
            static_active: std::sync::Arc::new(vec![true]),
        },
        ..AiContext::default()
    };

    let point = ctx.position_to_point_3d(Position {
        x: 50.0,
        y: 50.0,
        sector: SectorHandle::new(7),
        level: 2,
    });

    assert_eq!(point.x, 50.0);
    assert_eq!(point.y, 70.0);
    assert_eq!(point.z, 20.0);
}

#[test]
fn is_detecting_point_360_uses_current_eye_point() {
    let ctx = AiContext {
        position: Position {
            x: 0.0,
            y: 0.0,
            sector: None,
            level: 0,
        },
        direction: 4,
        posture: crate::element::Posture::LeaningOut,
        sq_standard_view_radius: 11.0 * 11.0,
        sq_self_view_radius: 11.0 * 11.0,
        ..AiContext::default()
    };

    assert!(
        ctx.is_detecting_point_360(crate::coordinates::WorldPoint3D {
            x: 50.0,
            y: 0.0,
            z: 45.0,
        })
    );
}

// ── House / building-AI tests ─────────────────────────────────

#[test]
fn house_default_values() {
    let h = House::default();
    assert_eq!(h.sector_index, 0);
    assert_eq!(h.building_index, None);
    assert!(h.door_indices.is_empty());
    assert!(h.occupant_ids.is_empty());
    assert!(!h.arrow_reserve);
}

/// Mirrors the enter / leave sequence that `execute_pass_door`
/// runs when an actor walks through a building door — a direct
/// unit-level exercise of the same Vec `push` / `retain` logic
/// used by the runtime hooks, so regressions in
/// `House::occupant_ids` semantics are caught without needing a
/// full engine fixture.
#[test]
fn house_occupant_enter_leave_cycle() {
    use crate::element::EntityId;

    let mut h = House {
        sector_index: 42,
        ..House::default()
    };
    let a = EntityId::Pc(crate::entity_id::PcId(1));
    let b = EntityId::Pc(crate::entity_id::PcId(2));

    // Enter A, then B
    if !h.occupant_ids.contains(&a) {
        h.occupant_ids.push(a);
    }
    if !h.occupant_ids.contains(&b) {
        h.occupant_ids.push(b);
    }
    assert_eq!(h.occupant_ids, vec![a, b]);

    // Dedup: re-entering A while already inside is a no-op.
    if !h.occupant_ids.contains(&a) {
        h.occupant_ids.push(a);
    }
    assert_eq!(h.occupant_ids, vec![a, b]);

    // Leave A — B stays.
    h.occupant_ids.retain(|&e| e != a);
    assert_eq!(h.occupant_ids, vec![b]);

    // Leave B — empty list, house entry still alive.
    h.occupant_ids.retain(|&e| e != b);
    assert!(h.occupant_ids.is_empty());
    assert_eq!(h.sector_index, 42);
}

#[test]
fn house_occupancy_helpers() {
    use crate::element::EntityId;
    let mut h = House::default();
    assert_eq!(h.occupant_count(), 0);
    h.occupant_ids.push(EntityId::Pc(crate::entity_id::PcId(1)));
    h.occupant_ids.push(EntityId::Pc(crate::entity_id::PcId(2)));
    assert_eq!(h.occupant_count(), 2);
    assert!(h.contains_occupant(EntityId::Pc(crate::entity_id::PcId(1))));
    assert!(!h.contains_occupant(EntityId::Pc(crate::entity_id::PcId(99))));
}

#[test]
fn ambush_point_init_lift_defaults() {
    // New AmbushPoints default to z=0 and id=0 before init runs.
    let ap = AmbushPoint {
        position: Position {
            x: 100.0,
            y: 200.0,
            sector: None,
            level: 0,
        },
        direction: 0,
        position_3d: crate::coordinates::WorldPoint3D::default(),
        id: 0,
    };
    assert_eq!(ap.position_3d.z, 0.0);
    assert_eq!(ap.id, 0);
}

#[test]
fn ai_outbox_drain_barriers_are_independent_and_serializable() {
    let mut outbox = AiOutbox::default();
    outbox.actor.halt = true;
    outbox.actor.stop_menace = true;
    outbox.actor.quit_swordfight = true;
    outbox.reentrant.self_stimuli.push(StimulusType::EventDone);
    outbox.music.instant_change = true;
    outbox.actor.archery_reservation_release = ArcheryReservationRelease {
        shooting_point: Some(ReservedShootingPoint {
            sector_index: 3,
            point_index: crate::sector::ArcheryPointIdx(4),
        }),
        release_sector: true,
    };

    let encoded = serde_json::to_string(&outbox).expect("serialize AI outbox");
    let mut decoded: AiOutbox = serde_json::from_str(&encoded).expect("deserialize AI outbox");

    assert!(decoded.actor.take_halt());
    let preemption = decoded.actor.take_movement_prefixes();
    assert!(preemption.stop_menace);
    assert!(!preemption.lower_shield);
    assert!(!decoded.actor.halt);
    assert!(decoded.actor.quit_swordfight);
    assert!(decoded.actor.archery_reservation_release.release_sector);
    assert_eq!(
        decoded.reentrant.self_stimuli,
        vec![StimulusType::EventDone]
    );
    assert!(decoded.music.instant_change);

    let core = decoded.actor.take_core();
    assert!(core.quit_swordfight);
    assert!(!decoded.actor.quit_swordfight);
    assert!(decoded.actor.archery_reservation_release.release_sector);
    assert_eq!(
        decoded.reentrant.self_stimuli,
        vec![StimulusType::EventDone]
    );
    assert!(decoded.music.instant_change);

    let archery = decoded.actor.take_archery_reservation_release();
    assert_eq!(
        archery,
        ArcheryReservationRelease {
            shooting_point: Some(ReservedShootingPoint {
                sector_index: 3,
                point_index: crate::sector::ArcheryPointIdx(4),
            }),
            release_sector: true,
        }
    );
    assert_eq!(
        decoded.actor.archery_reservation_release,
        ArcheryReservationRelease::default()
    );
    assert_eq!(
        decoded.reentrant.self_stimuli,
        vec![StimulusType::EventDone]
    );
    assert!(decoded.music.instant_change);
}

#[test]
fn owner_fifo_preserves_and_hashes_say_setstate_both_orders() {
    let state = AiOwnerWork::StateChange(AiStateChangeNotification {
        outgoing_state: AiState::Default,
        outgoing_substate: Substate::DefaultOnPost,
        incoming_state: AiState::Seeking,
        incoming_substate: Substate::SeekingHeardsteps,
        source: AiStateChangeSource::SelfActor,
        actor_effects_before_callback: Default::default(),
    });
    let speech = AiOwnerWork::Speech(AiSpeechAttempt {
        remark: Remark::Arrow,
        flags: SpeechFlags::MYTALK_1.bits(),
    });

    let mut say_then_state = AiOutbox::default();
    say_then_state.reentrant.owner_work = vec![speech.clone(), state.clone()];
    let mut state_then_say = AiOutbox::default();
    state_then_say.reentrant.owner_work = vec![state, speech];

    let encoded = serde_json::to_string(&say_then_state).expect("serialize owner FIFO");
    let decoded: AiOutbox = serde_json::from_str(&encoded).expect("deserialize owner FIFO");
    assert!(matches!(
        decoded.reentrant.owner_work.as_slice(),
        [AiOwnerWork::Speech(_), AiOwnerWork::StateChange(_)]
    ));
    assert!(matches!(
        state_then_say.reentrant.owner_work.as_slice(),
        [AiOwnerWork::StateChange(_), AiOwnerWork::Speech(_)]
    ));
    assert_ne!(
        robin_util::state_hash::compute(&say_then_state),
        robin_util::state_hash::compute(&state_then_say),
        "Say/SetState statement order must participate in deterministic state"
    );
}

#[test]
fn clear_all_pending_clears_every_outbox_barrier() {
    let mut ai = AiController::default();
    ai.outbox.patrol.direction_broadcast = Some(7);
    ai.outbox
        .detection
        .stimuli
        .push(Stimulus::new(StimulusType::EventView));
    ai.outbox.detection.mark_alerted = true;
    ai.outbox
        .reentrant
        .cross_npc_actions
        .push(CrossNpcAction::BreakPhalanx { target: 8 });
    ai.outbox
        .reentrant
        .self_stimuli
        .push(StimulusType::EventDone);
    ai.outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::StateChange(AiStateChangeNotification {
            outgoing_state: AiState::Default,
            outgoing_substate: Substate::DefaultOnPost,
            incoming_state: AiState::Seeking,
            incoming_substate: Substate::SeekingHeardsteps,
            source: AiStateChangeSource::SelfActor,
            actor_effects_before_callback: Default::default(),
        }));
    ai.outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::Speech(AiSpeechAttempt {
            remark: Remark::Arrow,
            flags: SpeechFlags::MYTALK_1.bits(),
        }));
    ai.outbox.reentrant.waypoint_script_reach_point =
        Some((PathId::new(2).expect("non-sentinel path"), 3));

    ai.outbox.actor.orders.push(AiOrderIntent::new(
        crate::order::OrderType::WaitingUpright,
        0.0,
        0.0,
    ));
    ai.outbox.actor.blink_all_enemies = true;
    ai.outbox.actor.enemy_in_house_alert = true;
    ai.outbox.actor.set_attentive_mode = Some(AttentiveModeEffect::new(true, false));
    ai.outbox.actor.set_guarded_pc = Some(GuardedPcEffect {
        old: Some(crate::entity_id::PcId(4)),
        new: Some(crate::entity_id::PcId(5)),
    });
    ai.outbox.actor.begin_panic = Some(PanicRequest {
        center: None,
        runs: 2,
        alert: AlertLevel::Yellow,
        is_new_panic: true,
    });
    ai.outbox.actor.panic_seek_fallback = true;
    ai.outbox.actor.archery_reservation_release = ArcheryReservationRelease {
        shooting_point: Some(ReservedShootingPoint {
            sector_index: 6,
            point_index: crate::sector::ArcheryPointIdx(7),
        }),
        release_sector: true,
    };

    ai.outbox.recovery.inform_resurrection = true;
    ai.outbox.recovery.set_eye_status = Some(crate::element::EyeStatus::Closed);
    ai.outbox.music.instant_change = true;

    ai.clear_all_pending();

    assert_eq!(
        serde_json::to_value(&ai.outbox).expect("serialize cleared outbox"),
        serde_json::to_value(AiOutbox::default()).expect("serialize default outbox")
    );
}

#[test]
fn every_real_substate_has_exactly_one_numeric_family() {
    use Substate::*;

    for raw in 0..NumberOfSubstates as u32 {
        let substate = Substate::try_from(raw).expect("contiguous substate discriminant");
        let is_marker = matches!(
            substate,
            StartSleepingSubstates
                | EndSleepingSubstates
                | StartDefaultSubstates
                | EndDefaultSubstates
                | StartWonderingSubstates
                | EndWonderingSubstates
                | StartSeekingSubstates
                | EndSeekingSubstates
                | StartAttackingSubstates
                | EndAttackingSubstates
                | StartMenacingSubstates
                | EndMenacingSubstates
                | StartFleeingSubstates
                | EndFleeingSubstates
                | BeginAdditionalSubstates
        );
        assert_eq!(
            substate.ai_state_family().is_none(),
            is_marker,
            "unexpected numeric family mapping for {substate:?}"
        );
    }

    assert_eq!(Substate::None.ai_state_family(), std::option::Option::None);
    assert_eq!(
        Substate::NumberOfSubstates.ai_state_family(),
        std::option::Option::None
    );
}
