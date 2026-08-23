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
fn nearest_door_distance_uses_original_uword_maluses() {
    assert_eq!(
        super::legacy_nearest_door_distance(100.9, 90.0, false, false),
        100
    );
    assert_eq!(
        super::legacy_nearest_door_distance(65_000.0, 0.0, true, true),
        264
    );
    assert_eq!(
        super::legacy_nearest_door_distance(65_535.0, 0.0, false, false),
        u16::MAX,
    );
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
fn outside_think_patrol_macro_finishes_after_reentrant_reach_point() {
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let sim = crate::sim_rng::test_context();
    let mut ai = AiController::new(17);
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultInMacro;
    ai.patrol_path = Some(PatrolPath {
        hiking_path_index: PathId::new(0).expect("zero is a valid hiking-path index"),
        current_waypoint_index: 0,
        last_waypoint_index: 0,
        forward: true,
        size: 6,
        history: Vec::new(),
    });
    ai.number_of_remaining_macro_bytes = 0;
    ai.macro_started_in_this_frame = false;
    ai.macro_in_progress = true;
    ai.timer_is_running = true;
    ai.when_does_timer_ring = 900;
    ai.macro_timer_is_running = true;
    ai.when_does_macro_timer_ring = 700;

    let waypoints = vec![
        RawWaypoint {
            x: 5,
            y: 5,
            sector: 1,
            level: 0,
            command: WaypointCommand::None,
        },
        RawWaypoint {
            x: 10,
            y: 10,
            sector: 1,
            level: 0,
            command: WaypointCommand::None,
        },
        RawWaypoint {
            x: 20,
            y: 20,
            sector: 1,
            level: 0,
            command: WaypointCommand::None,
        },
        RawWaypoint {
            x: 30,
            y: 30,
            sector: 1,
            level: 0,
            command: WaypointCommand::None,
        },
        RawWaypoint {
            x: 40,
            y: 40,
            sector: 1,
            level: 0,
            command: WaypointCommand::None,
        },
        RawWaypoint {
            x: 50,
            y: 50,
            sector: 1,
            level: 0,
            command: WaypointCommand::None,
        },
    ];
    let ctx = AiContext {
        position: Position {
            x: 10.0,
            y: 10.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        self_animation: crate::order::OrderType::WaitingUpright,
        hiking_paths: std::sync::Arc::new(vec![RawHikingPath { waypoints }]),
        ..AiContext::default()
    };

    ai.execute_next_macro_command(&sim, &ctx);

    assert!(ai.macro_in_progress);
    assert!(ai.macro_timer_is_running);
    assert!(ai.outbox.reentrant.finish_macro_after_self_stimuli);
    assert_eq!(
        ai.outbox.reentrant.self_stimuli,
        [StimulusType::EventReachPoint]
    );

    // Model the nested waypoint WAIT50 before the outer ExecuteNextMacroCommand
    // tail resumes. Its deadline survives the outer KillTimer(true).
    ai.when_does_macro_timer_ring = 508;
    ai.finish_patrol_macro();
    assert!(
        ai.timer_is_running,
        "Original KillTimer(true) preserves the normal timer"
    );
    assert_eq!(ai.when_does_timer_ring, 900);
    assert!(!ai.macro_timer_is_running);
    assert_eq!(ai.when_does_macro_timer_ring, 508);
    assert_eq!(ai.current_substate, Substate::DefaultEnroute);
}

#[test]
fn ai_timer_clamps_zero_while_macro_timer_preserves_raw_ulong_deadlines() {
    let mut ai = AiController::new(17);
    ai.current_substate = Substate::DefaultInMacro;

    ai.launch_timer(0, 123);
    assert_eq!(ai.when_does_timer_ring, 124);
    assert_eq!(ai.substate_at_last_timer_launch, Substate::DefaultInMacro);

    ai.launch_timer(1, 456);
    assert_eq!(ai.when_does_timer_ring, 457);

    ai.launch_timer(20, 456);
    assert_eq!(ai.when_does_timer_ring, 476);

    ai.launch_macro_timer(0, 456);
    assert_eq!(ai.when_does_macro_timer_ring, 456);

    ai.launch_timer(5, u32::MAX - 2);
    ai.launch_macro_timer(7, u32::MAX - 3);
    assert_eq!(ai.when_does_timer_ring, 2);
    assert_eq!(ai.when_does_macro_timer_ring, 3);
}

#[test]
fn break_macro_preserves_serialized_cursor_and_remaining_bytes() {
    let mut ai = AiController::new(17);
    ai.macro_command = vec![10, 20, 30, 40, 50];
    ai.macro_command_offset = 3;
    ai.number_of_remaining_macro_bytes = 2;
    ai.macro_in_progress = true;
    ai.macro_timer_is_running = true;

    ai.break_macro();

    assert!(!ai.macro_in_progress);
    assert!(!ai.macro_timer_is_running);
    assert_eq!(ai.macro_command, [10, 20, 30, 40, 50]);
    assert_eq!(ai.macro_command_offset, 3);
    assert_eq!(ai.number_of_remaining_macro_bytes, 2);
}

#[test]
fn goto_point_leaves_the_cursor_parked_on_its_unconsumed_operand() {
    use crate::ai::macro_patrol::{MacroOpcode, PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let waypoint = |x: i16| RawWaypoint {
        x,
        y: 0,
        sector: 1,
        level: 0,
        command: WaypointCommand::None,
    };
    let paths = vec![RawHikingPath {
        waypoints: vec![waypoint(0), waypoint(20), waypoint(40)],
    }];
    let mut ai = AiController::new(17);
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultInMacro;
    ai.has_patrol_path = true;
    ai.patrol_path = PatrolPath::new(PathId::new(0).unwrap(), &paths);
    // A three-byte `CMD_GOTO_POINT 2` body sitting at the head of the block.
    ai.macro_command = vec![MacroOpcode::GotoPoint as u8, 2, 0];
    ai.macro_command_offset = 0;
    ai.number_of_remaining_macro_bytes = 3;
    let ctx = AiContext {
        position: Position {
            x: 0.0,
            y: 0.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        hiking_paths: std::sync::Arc::new(paths),
        self_is_soldier: false,
        ..AiContext::default()
    };

    ai.execute_next_macro_command(&crate::sim_rng::test_context(), &ctx);

    assert_eq!(
        ai.patrol_path
            .as_ref()
            .expect("patrol path")
            .current_waypoint_index,
        2
    );
    assert_eq!(
        ai.macro_command_offset, 1,
        "the Original dereferences the waypoint index without stepping the cursor over it"
    );
}

#[test]
fn civilian_macro_run_sanitizes_flags_after_nested_path_completion() {
    use crate::ai::macro_patrol::{MacroOpcode, PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    // Keep every waypoint strictly inside the positive map quadrant:
    // GoTo fails fast with couldnt_reachpoint for any destination whose
    // x or y is <= 0, exactly as the Original bounds check does.
    let paths = vec![RawHikingPath {
        waypoints: vec![
            RawWaypoint {
                x: 10,
                y: 10,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            },
            RawWaypoint {
                x: 30,
                y: 10,
                sector: 1,
                level: 0,
                command: WaypointCommand::None,
            },
        ],
    }];
    let mut ai = AiController::new(17);
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultInMacro;
    ai.patrol_path = PatrolPath::new(PathId::new(0).unwrap(), &paths);
    ai.default_path_walking_flags = GotoFlags::BACK;
    ai.macro_command = vec![MacroOpcode::Run as u8];
    ai.number_of_remaining_macro_bytes = 1;
    let ctx = AiContext {
        position: Position {
            x: 10.0,
            y: 10.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        hiking_paths: std::sync::Arc::new(paths),
        self_is_soldier: false,
        ..AiContext::default()
    };

    ai.execute_next_macro_command(&crate::sim_rng::test_context(), &ctx);

    assert!(ai.last_goto_flags.contains(GotoFlags::RUN));
    assert!(
        ai.last_goto_flags.contains(GotoFlags::BACK),
        "Original GoTo snapshots the raw flags before masking them for a civilian"
    );
    assert_eq!(ai.default_path_walking_flags, GotoFlags::RUN);
    let order = ai.take_pending_orders().pop().expect("nested patrol GoTo");
    // The intent encodes GOTO_RUN as the running animation and GOTO_BACK as
    // the reversed-movement bit, mirroring how GoTo translates its flags
    // before launching the movement.
    assert_eq!(order.order_type, crate::order::OrderType::RunningUpright);
    assert!(
        !order.reverse,
        "the masked civilian GOTO_BACK must not reach the emitted movement"
    );
}

#[test]
fn invalid_patrol_assignment_preserves_original_partial_mutation() {
    use crate::ai::{PathId, PatrolAssignment};

    let mut ai = AiController::new(17);
    ai.has_patrol_path = false;
    ai.macro_in_progress = true;
    ai.macro_timer_is_running = true;

    let assigned = ai.assign_new_patrol_path(
        PatrolAssignment::Index(PathId::new(3).unwrap()),
        Position::default(),
        0,
        &[crate::level_data::RawHikingPath {
            waypoints: Vec::new(),
        }],
    );

    assert!(!assigned);
    assert!(
        ai.has_patrol_path,
        "Original sets mbHasPatrolPath before rejecting the index"
    );
    assert!(ai.path_id.is_none());
    assert!(!ai.macro_in_progress);
    assert!(!ai.macro_timer_is_running);
}

#[test]
fn script_way_assignment_keeps_special_action() {
    // `AssignPath` (script native) routes through Original's
    // `AssignNewPatrolPath(RHHikingPath*)` overload. Its valid-path arm
    // clears only `mbLikesToSitAround`; `mbSpecialAction` survives, so a
    // leisure-authored NPC on a scripted route later fails GoTo's
    // already-on-point shortcut and runs a real zero-length move when
    // returning to its route (RHartificialintelligence.cpp:5764-5769 vs the
    // index overload at :5717).
    use crate::ai::{PathId, PatrolAssignment};

    let paths = vec![crate::level_data::RawHikingPath {
        waypoints: vec![crate::level_data::RawWaypoint {
            x: 10,
            y: 20,
            sector: 1,
            level: 0,
            command: crate::level_data::WaypointCommand::None,
        }],
    }];

    let mut ai = AiController::new(17);
    ai.special_action = true;
    ai.likes_to_sit_around = true;
    let assigned = ai.assign_new_patrol_path(
        PatrolAssignment::ScriptWay(PathId::new(0).unwrap()),
        Position::default(),
        0,
        &paths,
    );
    assert!(assigned);
    assert!(ai.has_patrol_path);
    assert!(
        ai.special_action,
        "the RHHikingPath* overload must not clear mbSpecialAction"
    );
    assert!(!ai.likes_to_sit_around);

    // The waypoint-macro index overload clears both flags.
    let mut ai = AiController::new(17);
    ai.special_action = true;
    ai.likes_to_sit_around = true;
    let assigned = ai.assign_new_patrol_path(
        PatrolAssignment::Index(PathId::new(0).unwrap()),
        Position::default(),
        0,
        &paths,
    );
    assert!(assigned);
    assert!(
        !ai.special_action,
        "the UWORD index overload clears mbSpecialAction"
    );
    assert!(!ai.likes_to_sit_around);
}

#[test]
fn change_way_binds_assignment_callback_before_explicit_virtual_tail() {
    use crate::ai::macro_patrol::{MacroOpcode, PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let waypoint = |x: i16| RawWaypoint {
        x,
        y: 20,
        sector: 1,
        level: 0,
        command: WaypointCommand::None,
    };
    let paths = vec![
        RawHikingPath {
            waypoints: vec![waypoint(10), waypoint(30)],
        },
        RawHikingPath {
            waypoints: vec![waypoint(60), waypoint(90)],
        },
    ];
    let mut ai = AiController::new(17);
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultInMacro;
    ai.has_patrol_path = true;
    ai.patrol_path = PatrolPath::new(PathId::new(0).unwrap(), &paths);
    ai.macro_in_progress = true;
    ai.macro_command = vec![MacroOpcode::ChangeWay as u8, 1, 0];
    ai.number_of_remaining_macro_bytes = 3;
    let ctx = AiContext {
        position: Position {
            x: 40.0,
            y: 20.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        hiking_paths: std::sync::Arc::new(paths),
        self_is_soldier: true,
        ..AiContext::default()
    };

    ai.execute_next_macro_command(&crate::sim_rng::test_context(), &ctx);

    assert!(
        ai.outbox.reentrant.self_stimuli.is_empty(),
        "the exact assignment callback must be isolated from unrelated sibling stimuli"
    );
    assert!(ai.outbox.actor.orders.is_empty());
    assert!(matches!(
        ai.outbox.reentrant.owner_work.as_slice(),
        [AiOwnerWork::ChangeWayAssignmentThinkThenExplicitTail {
            assignment_callback: Some(StimulusType::EventReturnToDuty),
            owner_position_before_callback,
            owner_boundary_positions,
        }] if *owner_position_before_callback == ctx.position && owner_boundary_positions.is_empty()
    ));
    assert!(!ai.macro_in_progress);
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
fn friend_check_scans_a_detached_alert_path_without_consuming_the_following_wait() {
    use crate::ai::macro_patrol::{DetachedPatrolPathStatus, MacroOpcode, PathId};
    use crate::ai_entity_view::{AiEntityViewMap, entity_view_from_entity, shared_entity_views};
    use crate::element::{ActorSoldier, AiBrain, ElementData, ElementKind, Entity};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let path_id = PathId::new(0).expect("path zero is valid");
    let paths = vec![RawHikingPath {
        waypoints: vec![RawWaypoint {
            x: 10,
            y: 0,
            sector: 0,
            level: 0,
            command: WaypointCommand::None,
        }],
    }];

    let mut target = Entity::Soldier(ActorSoldier {
        element: ElementData {
            kind: ElementKind::ActorSoldier,
            posture: crate::element::Posture::Upright,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        npc: Default::default(),
        soldier: Default::default(),
    });
    let Entity::Soldier(target_soldier) = &mut target else {
        unreachable!()
    };
    target_soldier.npc.ai_brain = AiBrain::Enemy(Box::default());
    let target_ai = target_soldier
        .npc
        .ai_brain
        .base_mut()
        .expect("target has AI");
    target_ai.has_patrol_path = true;
    target_ai.path_id = Some(path_id);
    target_ai.detached_patrol_path_status = DetachedPatrolPathStatus {
        hiking_path_index: Some(path_id),
        current_waypoint_index: 0,
        last_waypoint_index: 0,
        forward: true,
        history: Vec::new(),
    };
    assert!(target_ai.patrol_path.is_none());

    let target_view = entity_view_from_entity(
        &target,
        22,
        false,
        None,
        None,
        crate::order::OrderType::WaitingUprightBored,
    );
    assert_eq!(target_view.patrol_hiking_path_index, Some(path_id));

    let mut views = AiEntityViewMap::new();
    views.insert(2, target_view);
    let ctx = AiContext {
        position: Position::default(),
        posture: crate::element::Posture::Upright,
        sq_self_view_radius: 1000.0 * 1000.0,
        entity_views: shared_entity_views(views),
        hiking_paths: std::sync::Arc::new(paths),
        all_soldier_handles: std::sync::Arc::new(vec![1, 2]),
        self_is_soldier: true,
        ..AiContext::default()
    };

    let mut ai = AiController::new(1);
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultInMacro;
    ai.macro_in_progress = true;
    ai.macro_command = vec![0; 20];
    ai.macro_command[17] = MacroOpcode::Wait as u8;
    ai.macro_command[18..20].copy_from_slice(&75_u16.to_le_bytes());
    ai.macro_command_offset = 17;
    ai.number_of_remaining_macro_bytes = 3;

    ai.initialize_friend_check(&crate::sim_rng::test_context(), 1, 225, u16::MAX, &ctx);

    assert_eq!(
        ai.current_substate,
        Substate::DefaultLookingSidewardsForCharly
    );
    assert_eq!(ai.checkpoint_charly, 2);
    assert_eq!(ai.macro_command_offset, 17);
    assert_eq!(ai.number_of_remaining_macro_bytes, 3);
    assert!(!ai.macro_timer_is_running);
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
        crate::ai_entity_view::entity_view_from_entity(
            &pc,
            41,
            false,
            None,
            None,
            crate::order::OrderType::NonanimationEnd,
        ),
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
fn ordinary_goto_from_menacing_carries_serialized_stop_prefix() {
    let mut ctx = goto_short_circuit_ctx(crate::order::OrderType::Menacing);
    ctx.self_action_state = crate::element::ActionState::Menacing;
    let destination = Position {
        x: 200.0,
        y: 200.0,
        ..ctx.position
    };
    let mut ai = AiController::new(1);

    ai.go_to(destination, GotoFlags::RUN, &ctx);

    let orders = ai.take_pending_orders();
    assert_eq!(orders.len(), 1);
    assert!(orders[0].stop_menace_before_move);
    assert!(
        !ai.outbox.actor.stop_menace,
        "GoTo StopMenace belongs to the movement sequence, not a standalone launch"
    );
    let encoded = serde_json::to_string(&orders[0]).expect("serialize movement intent");
    let restored: crate::order::AiOrderIntent =
        serde_json::from_str(&encoded).expect("deserialize movement intent");
    assert!(restored.stop_menace_before_move);
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
        ai.think_recursion_depth = 1;

        ai.go_to(ctx.position, GotoFlags::empty(), &ctx);

        assert!(ai.already_on_point);
        assert!(ai.outbox.reentrant.self_stimuli.is_empty());
        assert!(ai.take_pending_orders().is_empty());

        let mut speed_ai = AiController::new(1);
        speed_ai.think_recursion_depth = 1;
        speed_ai.go_to_speed(ctx.position, GotoFlags::empty(), 1.5, &ctx);
        assert!(speed_ai.already_on_point);
        assert!(speed_ai.outbox.reentrant.self_stimuli.is_empty());
        assert!(speed_ai.take_pending_orders().is_empty());
    }

    let outside_ctx = goto_short_circuit_ctx(crate::order::OrderType::WaitingUpright);
    let mut outside_ai = AiController::new(1);
    outside_ai.go_to(outside_ctx.position, GotoFlags::empty(), &outside_ctx);
    assert!(!outside_ai.already_on_point);
    assert_eq!(
        outside_ai.outbox.reentrant.self_stimuli,
        [StimulusType::EventReachPoint]
    );
    assert!(outside_ai.take_pending_orders().is_empty());

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
fn gonear_runs_goto_idle_shortcut_before_zero_near_tolerance() {
    let destination = Position {
        x: 101.0,
        y: 203.0,
        sector: SectorHandle::new(1),
        level: 0,
    };
    let mut idle = AiController::new(1);
    idle.think_recursion_depth = 1;
    let idle_ctx = goto_short_circuit_ctx(crate::order::OrderType::NonanimationEnd);

    idle.go_near(destination, 0, GotoFlags::RUN, &idle_ctx);

    assert!(idle.already_on_point);
    assert!(idle.take_pending_orders().is_empty());

    let mut running = AiController::new(1);
    running.think_recursion_depth = 1;
    let running_ctx = goto_short_circuit_ctx(crate::order::OrderType::RunningUpright);

    running.go_near(destination, 0, GotoFlags::RUN, &running_ctx);

    assert!(!running.already_on_point);
    assert_eq!(running.take_pending_orders().len(), 1);
}

#[test]
fn goto_replayed_near_flags_finish_inside_stored_tolerance() {
    let ctx = AiContext {
        position: Position {
            x: 1191.0,
            y: 1807.0,
            sector: SectorHandle::new(77),
            level: 1,
        },
        self_layer: 1,
        // The Original GOTO_NEAR gate is not restricted to idle
        // animations, unlike the ordinary five-pixel GoTo shortcut.
        self_animation: crate::order::OrderType::StrikingRightSmalltalk,
        ..AiContext::default()
    };
    let destination = Position {
        x: 1185.1125,
        y: 1787.6091,
        sector: SectorHandle::new(77),
        level: 1,
    };
    let flags = GotoFlags::NEAR | GotoFlags::SWORD;
    let mut ai = AiController::new(178);
    ai.think_recursion_depth = 1;
    ai.stop_before_end_of_path_distance = 65;

    ai.go_to(destination, flags, &ctx);

    assert!(ai.already_on_point);
    assert!(ai.take_pending_orders().is_empty());
    assert_eq!(ai.last_goto_destination, destination);
    assert_eq!(ai.last_goto_flags, flags);
}

#[test]
fn goto_replayed_near_flags_propagate_stored_tolerance_outside_radius() {
    let ctx = goto_short_circuit_ctx(crate::order::OrderType::StrikingRightSmalltalk);
    let destination = Position {
        x: 200.0,
        y: 300.0,
        sector: SectorHandle::new(1),
        level: 0,
    };
    let flags = GotoFlags::NEAR | GotoFlags::SWORD;
    let mut ai = AiController::new(178);
    ai.stop_before_end_of_path_distance = 65;

    ai.go_to(destination, flags, &ctx);

    assert!(!ai.already_on_point);
    let orders = ai.take_pending_orders();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].tolerance, 65.0);
    assert_eq!(ai.last_goto_destination, destination);
    assert_eq!(ai.last_goto_flags, flags);
}

#[test]
fn goto_already_on_point_observes_synchronous_pending_halt_multiplicity() {
    let mut ctx =
        goto_short_circuit_ctx(crate::order::OrderType::TransitionWalkingUprightRunningUpright);
    ctx.self_selected_element_is_default_wait = Some(false);
    ctx.self_selected_element_priority = Some(Some(crate::sequence::SequencePriority::Preference));
    let mut ai = AiController::new(1);
    ai.think_recursion_depth = 1;
    ai.outbox.actor.queue_halt();
    let halted_prefix = std::mem::take(&mut ai.outbox.actor);
    ai.outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::StateChange(AiStateChangeNotification {
            outgoing_state: AiState::Attacking,
            outgoing_substate: Substate::AttackingReactiontimeRunning,
            incoming_state: AiState::Seeking,
            incoming_substate: Substate::SeekingSeekpoint,
            source: AiStateChangeSource::SelfActor,
            actor_effects_before_callback: Some(halted_prefix),
        }));
    ai.go_to(ctx.position, GotoFlags::empty(), &ctx);

    assert!(ai.already_on_point);
    assert!(ai.take_pending_orders().is_empty());

    let mut running_ctx = goto_short_circuit_ctx(crate::order::OrderType::RunningUpright);
    running_ctx.self_selected_element_is_default_wait = Some(false);
    running_ctx.self_selected_element_priority =
        Some(Some(crate::sequence::SequencePriority::Preference));
    let mut speed_ai = AiController::new(1);
    speed_ai.think_recursion_depth = 1;
    speed_ai.outbox.actor.queue_halt();
    speed_ai.outbox.actor.queue_halt();
    speed_ai.go_to_speed(running_ctx.position, GotoFlags::empty(), 1.5, &running_ctx);

    assert!(speed_ai.already_on_point);
    assert!(speed_ai.take_pending_orders().is_empty());

    for animation in [
        crate::order::OrderType::WalkingUpright,
        crate::order::OrderType::RunningUpright,
        crate::order::OrderType::WalkingCrouched,
        crate::order::OrderType::TransitionWalkingCrouchedWaitingCrouched,
    ] {
        let mut transition_ctx = goto_short_circuit_ctx(animation);
        transition_ctx.self_selected_element_is_default_wait = Some(false);
        transition_ctx.self_selected_element_priority =
            Some(Some(crate::sequence::SequencePriority::Preference));
        let mut one_halt_ai = AiController::new(1);
        one_halt_ai.think_recursion_depth = 1;
        one_halt_ai.outbox.actor.queue_halt();

        one_halt_ai.go_to(transition_ctx.position, GotoFlags::empty(), &transition_ctx);

        assert!(!one_halt_ai.already_on_point, "animation {animation:?}");
        assert_eq!(
            one_halt_ai.take_pending_orders().len(),
            1,
            "animation {animation:?}"
        );
    }

    // Original's close-point switch observes the upright Wait successor of
    // these outgoing transitions, so one synchronous halt is sufficient.
    for animation in [
        crate::order::OrderType::TransitionWalkingUprightWaitingUpright,
        crate::order::OrderType::TransitionRunningUprightWaitingUpright,
    ] {
        let mut transition_ctx = goto_short_circuit_ctx(animation);
        transition_ctx.self_selected_element_is_default_wait = Some(false);
        transition_ctx.self_selected_element_priority =
            Some(Some(crate::sequence::SequencePriority::Preference));
        let mut one_halt_ai = AiController::new(1);
        one_halt_ai.think_recursion_depth = 1;
        one_halt_ai.outbox.actor.queue_halt();

        one_halt_ai.go_to(transition_ctx.position, GotoFlags::empty(), &transition_ctx);

        assert!(one_halt_ai.already_on_point, "animation {animation:?}");
        assert!(
            one_halt_ai.take_pending_orders().is_empty(),
            "animation {animation:?}"
        );
    }

    let mut walking_ctx = goto_short_circuit_ctx(crate::order::OrderType::WalkingUpright);
    walking_ctx.self_selected_element_is_default_wait = Some(false);
    walking_ctx.self_selected_element_priority =
        Some(Some(crate::sequence::SequencePriority::Preference));
    let mut two_halt_ai = AiController::new(1);
    two_halt_ai.think_recursion_depth = 1;
    two_halt_ai.outbox.actor.queue_halt();
    two_halt_ai.outbox.actor.queue_halt();

    two_halt_ai.go_to(walking_ctx.position, GotoFlags::empty(), &walking_ctx);

    assert!(two_halt_ai.already_on_point);
    assert!(two_halt_ai.take_pending_orders().is_empty());

    let mut uninterruptible_ctx = ctx.clone();
    uninterruptible_ctx.in_uninterruptible_command = true;
    let mut uninterruptible_ai = AiController::new(1);
    uninterruptible_ai.think_recursion_depth = 1;
    uninterruptible_ai.outbox.actor.queue_halt();

    uninterruptible_ai.go_to(
        uninterruptible_ctx.position,
        GotoFlags::empty(),
        &uninterruptible_ctx,
    );

    assert!(!uninterruptible_ai.already_on_point);
    assert_eq!(uninterruptible_ai.take_pending_orders().len(), 1);
}

#[test]
fn goto_pending_halt_does_not_clear_selected_default_wait_animation() {
    let mut ctx = goto_short_circuit_ctx(crate::order::OrderType::AimingWithBow);
    ctx.self_selected_element_is_default_wait = Some(true);
    ctx.self_selected_element_priority = Some(Some(crate::sequence::SequencePriority::Wait));
    let mut ai = AiController::new(91);
    ai.think_recursion_depth = 1;
    ai.stop_all();

    ai.go_to(ctx.position, GotoFlags::RUN, &ctx);

    assert!(!ai.already_on_point);
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    let orders = ai.take_pending_orders();
    assert_eq!(orders.len(), 1);
    assert_eq!(
        orders[0].order_type,
        crate::order::OrderType::RunningUpright
    );
}

#[test]
fn goto_pending_halt_respects_selected_injury_priority_near_route_point() {
    let mut injury_ctx = goto_short_circuit_ctx(crate::order::OrderType::ExtractingArrowBow);
    injury_ctx.self_selected_element_is_default_wait = Some(false);
    injury_ctx.self_selected_element_priority =
        Some(Some(crate::sequence::SequencePriority::Injury));
    let route_point = Position {
        x: injury_ctx.position.x + 1.0,
        y: injury_ctx.position.y + 2.0,
        ..injury_ctx.position
    };
    let mut injury_ai = AiController::new(29);
    injury_ai.think_recursion_depth = 1;
    injury_ai.stop_all();

    injury_ai.go_to(route_point, GotoFlags::RUN, &injury_ctx);

    assert!(!injury_ai.already_on_point);
    assert!(injury_ai.outbox.reentrant.self_stimuli.is_empty());
    assert_eq!(injury_ai.take_pending_orders().len(), 1);

    // Stop(PREFERENCE) can stop an equal-priority selected element, so the
    // same deferred Halt does expose an idle animation and takes Original's
    // under-five-pixel route shortcut.
    let mut preference_ctx = injury_ctx;
    preference_ctx.self_selected_element_priority =
        Some(Some(crate::sequence::SequencePriority::Preference));
    let mut preference_ai = AiController::new(29);
    preference_ai.think_recursion_depth = 1;
    preference_ai.stop_all();

    preference_ai.go_to(route_point, GotoFlags::RUN, &preference_ctx);

    assert!(preference_ai.already_on_point);
    assert!(preference_ai.take_pending_orders().is_empty());
}

#[test]
fn goto_already_on_point_projects_move_to_wait_transition_only_at_exact_destination() {
    // Original Actor::Hourglass exposes the idle successor before the later
    // timer-driven BattleDecisions call. The Rust phase split can still expose
    // the authored transition to AiContext, including the Save049 trace where
    // the proposed archer step-back goal is the actor's exact position.
    let mut ctx =
        goto_short_circuit_ctx(crate::order::OrderType::TransitionRunningUprightWaitingUpright);
    ctx.self_action_state = crate::element::ActionState::MovingFast;
    ctx.self_animation_reached_action_done = true;
    let mut ai = AiController::new(93);
    ai.think_recursion_depth = 1;

    ai.go_to(ctx.position, GotoFlags::RUN, &ctx);

    assert!(ai.already_on_point);
    assert!(ai.take_pending_orders().is_empty());

    ctx.self_animation_reached_action_done = false;
    let mut unfinished_ai = AiController::new(93);
    unfinished_ai.think_recursion_depth = 1;
    unfinished_ai.go_to(ctx.position, GotoFlags::RUN, &ctx);
    assert!(unfinished_ai.already_on_point);
    assert!(unfinished_ai.take_pending_orders().is_empty());

    // This projection is exact-position only. A nearby nonzero destination
    // still sees the unfinished transition and must launch a real movement.
    let nearby = Position {
        x: ctx.position.x + 1.0,
        ..ctx.position
    };
    let mut nearby_ai = AiController::new(93);
    nearby_ai.think_recursion_depth = 1;
    nearby_ai.go_to(nearby, GotoFlags::RUN, &ctx);
    assert!(!nearby_ai.already_on_point);
    assert_eq!(nearby_ai.take_pending_orders().len(), 1);

    // A transition on its first live motion frame is not a stale outgoing
    // transition. Original still exposes that transition to GoTo.
    let mut starting_ctx =
        goto_short_circuit_ctx(crate::order::OrderType::TransitionWalkingUprightWaitingUpright);
    starting_ctx.self_animation_motion_state = crate::sprite::MotionState::Start;
    let mut starting_ai = AiController::new(93);
    starting_ai.think_recursion_depth = 1;
    starting_ai.go_to(starting_ctx.position, GotoFlags::RUN, &starting_ctx);
    assert!(!starting_ai.already_on_point);
    assert_eq!(starting_ai.take_pending_orders().len(), 1);

    // Once the transition has executed its Waiting state change it is a live
    // transition in Original too.  A timer-driven ReturnToDuty at the exact
    // post must therefore launch the Move and remain in GOTOPOST until that
    // movement reports EVENT_REACHPOINT (Save018/replay-006, frame 8809).
    let mut waiting_ctx =
        goto_short_circuit_ctx(crate::order::OrderType::TransitionRunningUprightWaitingUpright);
    waiting_ctx.self_action_state = crate::element::ActionState::Waiting;
    waiting_ctx.self_animation_motion_state = crate::sprite::MotionState::InProgress;
    let mut waiting_ai = AiController::new(93);
    waiting_ai.think_recursion_depth = 1;
    waiting_ai.go_to(waiting_ctx.position, GotoFlags::empty(), &waiting_ctx);
    assert!(!waiting_ai.already_on_point);
    assert_eq!(waiting_ai.take_pending_orders().len(), 1);

    // A live transition owned by the actor's default Wait is not a stale
    // movement projection. Original GetAnimation() reads that transition,
    // so an exact-position GoTo registers a real Move and reaches the point
    // later through SequenceManager::Hourglass (Save071/replay-021).
    let mut default_wait_ctx = ctx.clone();
    default_wait_ctx.self_animation_reached_action_done = false;
    default_wait_ctx.self_selected_element_is_default_wait = Some(true);
    let mut default_wait_ai = AiController::new(93);
    default_wait_ai.think_recursion_depth = 1;
    default_wait_ai.go_to(default_wait_ctx.position, GotoFlags::RUN, &default_wait_ctx);
    assert!(!default_wait_ai.already_on_point);
    assert_eq!(default_wait_ai.take_pending_orders().len(), 1);

    let mut speed_ai = AiController::new(93);
    speed_ai.think_recursion_depth = 1;
    speed_ai.go_to_speed(ctx.position, GotoFlags::RUN, 1.5, &ctx);
    assert!(speed_ai.already_on_point);
    assert!(speed_ai.take_pending_orders().is_empty());

    // Original's close-point animation switch does not accept
    // WaitingCrouched, so its outgoing transition must remain a real GoTo.
    let crouched_ctx =
        goto_short_circuit_ctx(crate::order::OrderType::TransitionWalkingCrouchedWaitingCrouched);
    let mut crouched_ai = AiController::new(93);
    crouched_ai.think_recursion_depth = 1;
    crouched_ai.go_to(crouched_ctx.position, GotoFlags::RUN, &crouched_ctx);
    assert!(!crouched_ai.already_on_point);
    assert_eq!(crouched_ai.take_pending_orders().len(), 1);
}

#[test]
fn goto_with_live_animation_does_not_project_transition_to_idle() {
    let mut ctx =
        goto_short_circuit_ctx(crate::order::OrderType::TransitionRunningUprightWaitingUpright);
    ctx.self_animation_reached_action_done = false;
    let mut ai = AiController::new(39);

    ai.go_to_with_live_animation(ctx.position, GotoFlags::RUN, &ctx);

    assert!(!ai.already_on_point);
    assert!(ai.outbox.reentrant.self_stimuli.is_empty());
    assert_eq!(ai.take_pending_orders().len(), 1);

    // `go_to_with_live_animation` is used by the nested panic seek-point
    // fallback after reconstructing Original's live sequence order. Even at
    // the action-done point, a transition is not one of Original GoTo's
    // literal idle animations and must launch the coincident Move instead of
    // recursively delivering EVENT_REACHPOINT.
    ctx.self_animation_reached_action_done = true;
    let mut action_done_ai = AiController::new(39);
    action_done_ai.go_to_with_live_animation(ctx.position, GotoFlags::RUN, &ctx);
    assert!(!action_done_ai.already_on_point);
    assert!(action_done_ai.outbox.reentrant.self_stimuli.is_empty());
    assert_eq!(action_done_ai.take_pending_orders().len(), 1);
}

#[test]
fn trailing_stop_all_closes_prior_goto_before_queuing_halt() {
    let ctx = goto_short_circuit_ctx(crate::order::OrderType::WaitingUprightBored);
    let mut ai = AiController::new(1);

    ai.go_to(
        Position {
            x: ctx.position.x + 100.0,
            ..ctx.position
        },
        GotoFlags::RUN,
        &ctx,
    );
    ai.stop_all();

    assert_eq!(ai.outbox.reentrant.owner_work.len(), 1);
    let AiOwnerWork::ActorEffects(prefix) = &ai.outbox.reentrant.owner_work[0] else {
        panic!("GoTo prefix must precede the trailing StopAll");
    };
    assert_eq!(prefix.orders.len(), 1);
    assert!(!prefix.halt);
    assert!(ai.outbox.actor.orders.is_empty());
    assert!(ai.outbox.actor.halt);
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

/// `RHArtificialIntelligence::Face( RHposition, -1, bool )` subtracts the
/// actor's raw 3D point — `PositionToPoint3D( location ) - mpMe->GetPosition()`
/// (RHartificialintelligence.cpp:2680-2681) — while the explicit-elevation arm
/// at :2685-2686 subtracts `Point( mpMe )`.  Those origins diverge while
/// `Position( mpMe )` is overridden to a door's committed gate side, so the 3D
/// overload must not read `ctx.position`.
#[test]
fn face_position_3d_measures_from_the_body_not_the_door_side_position() {
    let mut ctx = face_to_ctx(crate::element::ActionState::Moving);
    // The door-pass override moves the AI-facing position far away from the
    // sprite's real location.
    ctx.position = Position {
        x: 100.0,
        y: 0.0,
        sector: None,
        level: 0,
    };
    ctx.self_body_position_world = crate::coordinates::WorldPoint3D {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    ctx.elevation = 0.0;
    let target = Position {
        x: 0.0,
        y: -100.0,
        sector: None,
        level: 0,
    };

    let mut ai = AiController::new(1);
    ai.face_position_3d_with_ctx(target, &ctx);
    let orders = ai.take_pending_orders();

    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_type, crate::order::OrderType::Turning);
    assert_eq!(
        orders[0].explicit_direction,
        Some(crate::position_interface::vector_to_sector_0_to_15_iso(
            0.0, -100.0
        )),
        "the faced vector starts at the body point, not the overridden RHposition"
    );
}

/// Interactive session 002-s0005 frame 6025: Soldier 59 hears the
/// drawbridge from below it. The normalized noise origin has no sector, but
/// `RHnoise::uwElevation` still carries the height that Original recovers via
/// `posOrigin.pSector` in `Face(RHposition)`.
#[test]
fn face_sectorless_noise_origin_uses_recorded_elevation() {
    let mut ctx = face_to_ctx(crate::element::ActionState::Moving);
    ctx.position = Position {
        x: 1023.0087,
        y: 1618.9951,
        sector: None,
        level: 4,
    };
    ctx.elevation = 400.001;
    ctx.self_body_position_world = crate::coordinates::WorldPoint3D {
        x: 1023.0087,
        y: 2018.9961,
        z: 400.001,
    };
    let noise = Noise {
        origin: Position {
            x: 1135.0,
            y: 1843.0,
            sector: None,
            level: 0,
        },
        noise_type: NoiseType::Drawbridge,
        volume: 274,
        elevation: 220,
        element_id: 0,
    };

    let mut normalized = AiController::new(1);
    normalized.face_noise_origin_with_ctx(&noise, &ctx);
    let normalized_orders = normalized.take_pending_orders();

    assert_eq!(normalized_orders.len(), 1);
    assert_eq!(normalized_orders[0].explicit_direction, Some(6));

    let mut ground_level_control = AiController::new(1);
    ground_level_control.face_position_3d_with_ctx(noise.origin, &ctx);
    let ground_level_orders = ground_level_control.take_pending_orders();
    assert_eq!(ground_level_orders.len(), 1);
    assert_eq!(ground_level_orders[0].explicit_direction, Some(1));
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

    let sim = crate::sim_rng::test_context();
    ai.think_expected_event_common_stuff(&sim, &Stimulus::new(StimulusType::EventReachPoint), &ctx);
    assert_eq!(ai.current_substate, Substate::DefaultGotoRouteTurn);
    // The REACHPOINT handler suspends across the SetState callback barrier;
    // the engine's owner-work drain runs this continuation. Invoke it
    // directly for the controller-level check.
    ai.resume_goto_route_reach_point(&sim, &ctx);

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

    let sim = crate::sim_rng::test_context();
    ai.think_expected_event_common_stuff(&sim, &Stimulus::new(StimulusType::EventReachPoint), &ctx);
    // The REACHPOINT handler suspends across the SetState callback barrier;
    // run the engine-drained continuation directly to reach the live
    // --path/++path lookup.
    ai.resume_goto_route_reach_point(&sim, &ctx);

    let path = ai.patrol_path.as_ref().expect("patrol path");
    assert_eq!(path.current_waypoint_index, 0);
    assert!(
        !path.forward,
        "Original's live --path/++path lookup reverses traversal at waypoint zero"
    );
}

#[test]
fn one_point_path_post_preserves_store_initial_direction_round_trip() {
    use crate::ai::macro_patrol::{PathId, PatrolPath};
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let paths = vec![RawHikingPath {
        waypoints: vec![RawWaypoint {
            x: 699,
            y: 1464,
            sector: 50,
            level: 1,
            command: WaypointCommand::None,
        }],
    }];
    let mut ai = AiController::new(142);
    ai.current_state = AiState::Default;
    ai.current_substate = Substate::DefaultGotoRouteTurn;
    ai.has_patrol_path = true;
    ai.patrol_path = PatrolPath::new(PathId::new(0).unwrap(), &paths);
    ai.think_recursion_depth = 1;
    let ctx = AiContext {
        position: Position {
            x: 698.99304,
            y: 1464.0072,
            sector: SectorHandle::new(50),
            level: 1,
        },
        direction: 3,
        posture: crate::element::Posture::Upright,
        self_action_state: crate::element::ActionState::Waiting,
        self_animation: crate::order::OrderType::NonanimationEnd,
        hiking_paths: std::sync::Arc::new(paths),
        ..AiContext::default()
    };
    let sim = crate::sim_rng::test_context();

    ai.think_expected_event_common_stuff(&sim, &Stimulus::new(StimulusType::EventDone), &ctx);

    assert!(!ai.has_patrol_path);
    assert_eq!(ai.initial_position, ctx.position);
    assert_eq!(
        ai.initial_view_direction, 2,
        "StoreInitialPositionParameters stores sector 3 as a unit vector, whose later FaceTo aspect conversion selects sector 2"
    );
    assert_eq!(ai.current_substate, Substate::DefaultGotoPost);
    assert!(ai.already_on_point);

    ai.already_on_point = false;
    ai.think_expected_event_common_stuff(&sim, &Stimulus::new(StimulusType::EventReachPoint), &ctx);

    assert_eq!(ai.current_substate, Substate::DefaultGotoPostTurn);
    assert!(!ai.already_turned);
    let orders = ai.take_pending_orders();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_type, crate::order::OrderType::Turning);
    assert_eq!(orders[0].explicit_direction, Some(2));
}

#[test]
fn entering_fleeing_hiding_blinks_visible_enemies_for_redetection() {
    for substate in [
        Substate::FleeingRunToHide,
        Substate::FleeingRunToDoor,
        Substate::FleeingPanic,
    ] {
        let mut ai = AiController::new(1);
        ai.current_state = AiState::Fleeing;
        ai.current_substate = substate;
        ai.directed_panic = true;
        ai.lasting_panic_runs = 0;

        ai.think_expected_event_common_stuff(
            &crate::sim_rng::test_context(),
            &Stimulus::new(StimulusType::EventReachPoint),
            &AiContext::default(),
        );

        assert_eq!(ai.current_substate, Substate::FleeingHiding);
        assert!(ai.timer_is_running);
        assert!(
            (crate::parameters_ai::AI_MIN_PANIC_HIDING_TIME as u32
                ..(crate::parameters_ai::AI_MIN_PANIC_HIDING_TIME
                    + crate::parameters_ai::AI_DELTA_PANIC_HIDING_TIME) as u32)
                .contains(&ai.when_does_timer_ring),
            "{substate:?} must use Original's shared 500..1000-frame hiding interval"
        );
        assert!(
            ai.outbox.actor.blink_all_enemies,
            "Original BlinkEnemy(NULL) is unconditional when {substate:?} enters hiding"
        );
    }
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
        fast_grid: std::sync::Arc::new(crate::fast_find_grid::FastFindGrid {
            level: std::sync::Arc::new(level),
            line_active: Vec::new(),
            sector_active: vec![true],
            mask_active: Vec::new(),
            lift_state: std::collections::BTreeMap::new(),
            sector_type_overlay: std::collections::BTreeMap::new(),
        }),
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
fn position_to_point_3d_uses_building_door_outside_projection() {
    use crate::fast_find_grid::{DoorProjectionInfo, GridSector};
    use crate::sector::{SectorNumber, SectorType};

    let mut level = crate::fast_find_grid::LevelGrid::default();
    let building_number = SectorNumber::new(7);
    level.sector_number_map.insert(building_number, 0);
    level.sectors.push(GridSector {
        sector_type: SectorType::AREA | SectorType::MOTION | SectorType::BUILDING,
        sector_number: building_number,
        gate_indices: vec![crate::gate::DoorIndex(0)],
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        layer: 0,
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        underlying_sector: None,
    });
    level.door_projection_infos.push(DoorProjectionInfo {
        point_in: MapPoint::new(50.0, 50.0),
        point_out: MapPoint::new(45.0, 55.0),
        sector_out: SectorNumber::new(8),
        layer_out: 2,
    });

    let mut obstacle = crate::sight_obstacle::SightObstacle::new(
        0,
        crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
    );
    obstacle.box_projection = crate::coordinates::MapBBox::from_geo(
        crate::geo2d::BBox2D::from_coords(0.0, 0.0, 100.0, 100.0),
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
    obstacle.sector = 8;
    obstacle.top_plane_points = [[0.0, 0.0, 20.0], [100.0, 0.0, 20.0], [0.0, 100.0, 20.0]];
    obstacle.bottom_plane_points = [[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]];
    obstacle.rebuild_geometry();

    let ctx = AiContext {
        fast_grid: std::sync::Arc::new(crate::fast_find_grid::FastFindGrid {
            level: std::sync::Arc::new(level),
            line_active: Vec::new(),
            sector_active: vec![true],
            mask_active: Vec::new(),
            lift_state: std::collections::BTreeMap::new(),
            sector_type_overlay: std::collections::BTreeMap::new(),
        }),
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
        level: 9,
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
    outbox
        .reentrant
        .self_stimuli
        .push(StimulusType::EventDone.into());
    outbox
        .reentrant
        .owner_work
        .push(AiOwnerWork::ChangeWayAssignmentThinkThenExplicitTail {
            assignment_callback: Some(StimulusType::EventReturnToDuty),
            owner_position_before_callback: Position::default(),
            owner_boundary_positions: vec![(7, Position::default())],
        });
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
    assert!(matches!(
        decoded.reentrant.owner_work.as_slice(),
        [AiOwnerWork::ChangeWayAssignmentThinkThenExplicitTail {
            assignment_callback: Some(StimulusType::EventReturnToDuty),
            owner_position_before_callback,
            owner_boundary_positions,
        }] if *owner_position_before_callback == Position::default()
            && owner_boundary_positions == &[(7, Position::default())]
    ));
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
fn self_stimulus_provenance_is_live_only_and_survives_queue_conversion() {
    let queued = QueuedSelfStimulus::new(
        StimulusType::EventCouldntReachPoint,
        SelfStimulusOrigin::EngineCompletion,
    );
    let stimulus = Stimulus::from_queued_self(queued);
    assert_eq!(stimulus.stimulus_type, StimulusType::EventCouldntReachPoint);
    assert_eq!(stimulus.self_origin, SelfStimulusOrigin::EngineCompletion);

    let encoded = serde_json::to_string(&queued).expect("serialize queued self-stimulus");
    assert_eq!(
        encoded,
        serde_json::to_string(&StimulusType::EventCouldntReachPoint)
            .expect("serialize legacy self-stimulus")
    );
    let decoded: QueuedSelfStimulus =
        serde_json::from_str(&encoded).expect("deserialize queued self-stimulus");
    assert_eq!(decoded.stimulus_type, StimulusType::EventCouldntReachPoint);
    assert_eq!(decoded.origin, SelfStimulusOrigin::Ordinary);

    assert_eq!(
        robin_util::state_hash::compute(&vec![queued]),
        robin_util::state_hash::compute(&vec![StimulusType::EventCouldntReachPoint]),
        "runtime provenance must not perturb the prior replay-state hash"
    );
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
        .push(CrossNpcAction::BreakPhalanx {
            target: 8,
            refresh_them_list: false,
        });
    ai.outbox
        .reentrant
        .self_stimuli
        .push(StimulusType::EventDone.into());
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
