use super::*;
use crate::ai_entity_view::{AiEntityView, AiEntityViewMap, EntityKind};
use crate::element::{Camp, Posture};
use crate::element_kinds::ObjectType;
use crate::order::OrderType;

fn object_view(object_type: ObjectType) -> AiEntityView {
    AiEntityView {
        original_creation_order: 41,
        position: Position {
            x: 10.0,
            y: 20.0,
            sector: None,
            level: 0,
        },
        detection_position: crate::coordinates::MapPoint::new(10.0, 20.0),
        detection_position_world: crate::coordinates::WorldPoint3D::new(10.0, 20.0, 0.0),
        direction: 0,
        posture: Posture::Upright,
        camp: Camp::default(),
        is_pc: false,
        is_robin: false,
        is_vip: false,
        is_beggar: false,
        is_child: false,
        kind: EntityKind::Bonus,
        is_tower_guard: false,
        is_swordfighting: false,
        is_able_to_fight: false,
        active: true,
        is_unconscious: false,
        action_state: crate::element::ActionState::Waiting,
        is_moving_map: false,
        passing_door: false,
        obstacle_idx: None,
        in_building: false,
        building_sector: None,
        script_locked: false,
        forecasted_destination: crate::ai::PreparedForecastDestination::fixed(
            Position::default(),
            0,
        ),
        ai_state: AiState::Default,
        ai_substate: Substate::DefaultOnPost,
        current_animation: OrderType::WaitingUprightBored,
        elevation: 0.0,
        object_type,
        is_dead: false,
        is_carried: false,
        is_archer: false,
        is_rider: false,
        stuck_under_net: false,
        covering_nets: Vec::new(),
        in_coma: false,
        guard: None,
        has_patrol_path: false,
        initial_position: Position::default(),
        number_of_arrows: 0,
        rank: ProfileRank::None,
        reported_to_officer: false,
        looted_after_money_fight: false,
        current_money: 0,
        macro_in_progress: false,
        path_current_waypoint_index: 0,
        path_last_waypoint_index: 0,
        path_forward_movement: true,
        patrol_hiking_path_index: None,
        interesting_object: None,
        report_type: ReportType::Nothing,
        report_seek_position: Position::default(),
        report_seen_bodies: Vec::new(),
        report_charly: None,
    }
}

#[test]
fn failed_fleeing_panic_move_uses_panic_seek_fallback() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(68);
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingPanic;
    ai.base.lasting_panic_runs = 7;
    ai.base.set_alert_status(crate::ai::AlertLevel::Red);

    ai.think_unexpected_event(
        &sim,
        &Stimulus::new(StimulusType::EventCouldntReachPoint),
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert_eq!(ai.base.view_alert_status, crate::ai::AlertLevel::Red);
    assert!(ai.base.outbox.actor.panic_seek_fallback);
}

#[test]
fn event_view_uses_owner_boundary_position_instead_of_stale_live_map() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    let mut enemy_view = object_view(ObjectType::None);
    enemy_view.kind = EntityKind::Pc;
    enemy_view.is_pc = true;
    enemy_view.position = Position {
        x: 10.0,
        y: 0.0,
        sector: None,
        level: 0,
    };
    enemy_view.detection_position = crate::coordinates::MapPoint::new(100.0, 0.0);
    enemy_view.detection_position_world = crate::coordinates::WorldPoint3D::new(100.0, 0.0, 0.0);
    let mut views = AiEntityViewMap::new();
    views.insert(12, enemy_view);
    let ctx = AiContext {
        position: Position::default(),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut tick = AiPerTickData::stub();
    tick.owner_live_position = Some(ctx.position);
    tick.enemy_detectable_positions.push((
        12,
        Position {
            x: 100.0,
            y: 0.0,
            sector: None,
            level: 0,
        },
    ));
    assert!(tick.enemy_detectable_live_world_positions.is_empty());

    ai.event_view_standard_procedure(&sim, 12, &mut AiGlobalState::default(), &ctx, &tick, None);

    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert!(
        !ai.enemy_seen_below,
        "non-optical dispatch must use the concrete entity-view geometry when no live detectable list was prepared"
    );
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingReactiontimeTurning
    );
}

#[test]
fn moving_fast_event_view_distance_uses_literal_owner_position_during_door_pass() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(134);
    let enemy_position = Position {
        x: 1648.9281,
        y: 1804.8717,
        sector: crate::position_interface::SectorHandle::new(0),
        level: 0,
    };
    let mut enemy_view = object_view(ObjectType::None);
    enemy_view.kind = EntityKind::Pc;
    enemy_view.is_pc = true;
    enemy_view.position = enemy_position;
    enemy_view.detection_position =
        crate::coordinates::MapPoint::new(enemy_position.x, enemy_position.y);
    enemy_view.detection_position_world =
        crate::coordinates::WorldPoint3D::new(enemy_position.x, enemy_position.y, 0.0);
    let mut views = AiEntityViewMap::new();
    views.insert(342, enemy_view);
    let ctx = AiContext {
        // AI Position(owner) is already forecast onto the selected
        // door's far side, but the original game's enemy distance directly reads
        // the still-interpolating element position instead.
        position: Position {
            x: 1151.0,
            y: 1817.0,
            sector: crate::position_interface::SectorHandle::new(77),
            level: 1,
        },
        self_action_state: crate::element::ActionState::MovingFast,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.owner_live_position = Some(Position {
        x: 1171.3004,
        y: 1846.5278,
        sector: crate::position_interface::SectorHandle::new(0),
        level: 0,
    });
    tick.enemy_detectable_positions.push((342, enemy_position));

    ai.event_view_standard_procedure(&sim, 342, &mut AiGlobalState::default(), &ctx, &tick, None);

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingReactiontimeRunning
    );
    assert_eq!(
        ai.base
            .outbox
            .actor
            .orders
            .last()
            .expect("moving-fast enemy sighting queues approach movement")
            .tolerance,
        161.0
    );
}

#[test]
fn moving_fast_event_view_distance_uses_stretched_world_3d_positions() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(70);
    let enemy_position = Position {
        x: 57.0,
        y: 245.0,
        sector: crate::position_interface::SectorHandle::new(0),
        level: 0,
    };
    let mut enemy_view = object_view(ObjectType::None);
    enemy_view.kind = EntityKind::Pc;
    enemy_view.is_pc = true;
    enemy_view.position = enemy_position;
    enemy_view.elevation = 36.001007;
    enemy_view.detection_position =
        crate::coordinates::MapPoint::new(enemy_position.x, enemy_position.y);
    enemy_view.detection_position_world = crate::coordinates::WorldPoint3D::new(
        enemy_position.x,
        enemy_position.y + 36.001007,
        36.001007,
    );
    let mut views = AiEntityViewMap::new();
    views.insert(132, enemy_view);
    let ctx = AiContext {
        position: Position {
            x: 277.4972,
            y: 379.12796,
            sector: crate::position_interface::SectorHandle::new(0).map(|sector| {
                sector.with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap())
            }),
            level: 0,
        },
        elevation: 1.387514,
        self_action_state: crate::element::ActionState::MovingFast,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.owner_live_position = Some(ctx.position);
    tick.enemy_detectable_positions.push((132, enemy_position));

    ai.event_view_standard_procedure(&sim, 132, &mut AiGlobalState::default(), &ctx, &tick, None);

    assert_eq!(
        ai.base
            .outbox
            .actor
            .orders
            .last()
            .expect("moving-fast enemy sighting queues approach movement")
            .tolerance,
        94.0
    );
}

#[test]
fn event_view_near_gate_uses_world_y_and_elevation() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    // Make the immediate battle-planning path terminate predictably once
    // it observes the empty visible-enemy list.
    ai.combat_trainer = true;

    let mut enemy_view = object_view(ObjectType::None);
    enemy_view.kind = EntityKind::Pc;
    enemy_view.is_pc = true;
    enemy_view.position = Position {
        x: 609.0,
        y: 2299.0,
        sector: None,
        level: 2,
    };
    enemy_view.elevation = 150.001;
    enemy_view.detection_position = crate::coordinates::MapPoint::new(609.0, 2299.0);
    enemy_view.detection_position_world =
        crate::coordinates::WorldPoint3D::new(609.0, 2449.001, 150.001);
    let mut views = AiEntityViewMap::new();
    let mut owner_view = object_view(ObjectType::None);
    owner_view.kind = EntityKind::Soldier;
    owner_view.detection_position_world =
        crate::coordinates::WorldPoint3D::new(575.6, 2465.001, 105.001);
    views.insert(1, owner_view);
    views.insert(12, enemy_view);
    let ctx = AiContext {
        position: Position {
            x: 575.6,
            y: 2360.0,
            sector: None,
            level: 1,
        },
        elevation: 105.001,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let mut tick = AiPerTickData::stub();
    tick.owner_live_position = Some(ctx.position);
    tick.enemy_detectable_positions.push((
        12,
        Position {
            x: 609.0,
            y: 2299.0,
            sector: None,
            level: 2,
        },
    ));

    ai.event_view_standard_procedure(&sim, 12, &mut AiGlobalState::default(), &ctx, &tick, None);

    // Raw map Y differs by 61 (and would take the turn branch), while
    // Original world Y differs by only 16 after adding elevation.  The
    // 45-unit elevation component keeps the 3D max norm below 50.
    assert_ne!(
        ai.base.current_substate,
        Substate::AttackingReactiontimeTurning
    );
}

#[test]
fn event_view_near_gate_uses_literal_target_position_during_door_pass() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(112);

    let mut enemy_view = object_view(ObjectType::None);
    enemy_view.kind = EntityKind::Pc;
    enemy_view.is_pc = true;
    // AI Position(enemy): the destination side of the active door pass,
    // close enough to take the immediate-battle branch if used here.
    enemy_view.position = Position {
        x: 663.75,
        y: 1421.5,
        sector: None,
        level: 2,
    };
    // Enemy world position: the still-interpolating body position read by
    // maximum-norm distance, more than 50 units from the observing soldier.
    enemy_view.detection_position = crate::coordinates::MapPoint::new(560.9536, 1422.7441);
    enemy_view.detection_position_world =
        crate::coordinates::WorldPoint3D::new(560.9536, 1552.7451, 130.001);
    enemy_view.elevation = 130.001;
    let mut views = AiEntityViewMap::new();
    views.insert(170, enemy_view);
    let ctx = AiContext {
        position: Position {
            x: 657.0,
            y: 1400.0,
            sector: None,
            level: 3,
        },
        elevation: 143.06665,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.owner_live_position = Some(Position {
        x: 654.72314,
        y: 1403.2888,
        sector: None,
        level: 3,
    });
    tick.enemy_detectable_positions.push((
        170,
        Position {
            x: 663.75,
            y: 1421.5,
            sector: None,
            level: 2,
        },
    ));

    ai.event_view_standard_procedure(&sim, 170, &mut AiGlobalState::default(), &ctx, &tick, None);

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingReactiontimeTurning
    );
    assert!(ai.base.list_us.is_empty());
}

#[test]
#[should_panic(expected = "officer 1 EVENT_SEES_SOLDIER requires target 42 in camp soldier roster")]
fn review_officer_sees_soldier_requires_target_in_live_soldier_roster() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.soldier_profile_rank = ProfileRank::Officer;
    ai.set_state(AiState::Default, Substate::DefaultOnPost);
    ai.think_unexpected_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventSeesSoldier, 42),
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
        None,
    );
}

#[test]
fn review_call_go_to_officer_preserves_original_boolean_gate() {
    let sim = crate::sim_rng::test_context();
    let stimulus = Stimulus::with_human(StimulusType::CallGoToOfficer, 42);

    let mut available = EnemyAi::new(1);
    available.soldier_profile_rank = ProfileRank::Soldier;
    assert!(available.think_unexpected_event(
        &sim,
        &stimulus,
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
        None,
    ));
    assert_eq!(
        available.base.current_substate,
        Substate::SeekingCharlySentToOfficer
    );
    assert_eq!(available.base.antagonist, Some(AiEntityHandle::new(42)));
    assert!(available.reported_to_officer);

    let mut busy = EnemyAi::new(2);
    busy.soldier_profile_rank = ProfileRank::Soldier;
    busy.set_state(AiState::Attacking, Substate::AttackingSwordfight);
    assert!(!busy.think_unexpected_event(
        &sim,
        &stimulus,
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
        None,
    ));
}

#[test]
fn found_charly_assigns_friend_only_after_speech_returns() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.soldier_profile_rank = ProfileRank::Soldier;
    ai.base.antagonist = Some(AiEntityHandle::new(90));
    ai.set_state(AiState::Seeking, Substate::SeekingGroupCalledByOfficer);
    ai.base.outbox.reentrant.owner_work.clear();

    let mut charly = object_view(ObjectType::None);
    charly.kind = EntityKind::Soldier;
    charly.rank = ProfileRank::Soldier;
    charly.ai_state = AiState::Seeking;
    charly.ai_substate = Substate::SeekingGroupCalledByOfficer;
    let mut views = AiEntityViewMap::new();
    views.insert(42, charly);
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.event_sees_charly_standard_procedure(
        &sim,
        AiEntityHandle::new(42),
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_eq!(ai.base.friend_in_trouble, None);
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [
            AiOwnerWork::StateChange(_),
            AiOwnerWork::ActorEffects(effects),
            AiOwnerWork::Speech(AiSpeechAttempt {
                remark: Remark::FoundCharly,
                ..
            }),
            AiOwnerWork::ResumeSendCharlyAfterSpeech { charly: 42 }
        ] if effects.unalert_near_charly_seekers
            == Some(CharlySeekerTarget::Npc(AiEntityHandle::new(42)))
    ));
}

#[test]
fn sync_reunion_uses_enroute_partners_last_waypoint() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(89);
    ai.set_state(AiState::Default, Substate::DefaultLookingSidewardsForCharly);
    ai.base.synchronize_charly = Some(AiEntityHandle::new(96));
    ai.base.synchronize_index = 3;
    ai.base.macro_in_progress = true;
    ai.base.macro_command = vec![MacroOpcode::Wait as u8, 100, 0];
    ai.base.macro_command_offset = 0;
    ai.base.number_of_remaining_macro_bytes = 3;

    // Enemy friend-check initialization checks *last* while an
    // actor is still SUBSTATE_DEFAULT_ENROUTE.  Being stationary at the
    // requested current waypoint is not enough: the actor has not yet
    // crossed the reach-point boundary that updates the observable path
    // state.
    let mut partner = object_view(ObjectType::None);
    partner.kind = EntityKind::Soldier;
    partner.ai_state = AiState::Default;
    partner.ai_substate = Substate::DefaultEnroute;
    partner.macro_in_progress = false;
    partner.path_current_waypoint_index = 3;
    partner.path_last_waypoint_index = 2;
    partner.is_moving_map = false;
    let mut views = AiEntityViewMap::new();
    views.insert(96, partner);
    let ctx = AiContext {
        frame: 1_072,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.event_sees_charly_standard_procedure(
        &sim,
        AiEntityHandle::new(96),
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_eq!(ai.base.current_substate, Substate::DefaultSynchronizing);
    assert_eq!(ai.base.macro_command_offset, 0);
    assert_eq!(ai.base.number_of_remaining_macro_bytes, 3);
    assert!(!ai.base.macro_timer_is_running);
    assert!(matches!(
        ai.base.outbox.reentrant.cross_npc_actions.as_slice(),
        [CrossNpcAction::RegisterSynchronizingActor {
            target: 96,
            actor: 89,
        }]
    ));
}

#[test]
fn got_hit_uses_live_swordfight_relationship_not_stale_ai_substate() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Attacking, Substate::AttackingSwordfight);

    let mut attacker = object_view(ObjectType::None);
    attacker.kind = EntityKind::Pc;
    attacker.camp = Camp::Royalists;
    attacker.position = Position::default();
    let mut views = AiEntityViewMap::new();
    views.insert(2, attacker);
    let ctx = AiContext {
        camp: Camp::Lacklandists,
        is_swordfighting: false,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 1,
            ..crate::ai_enemy::FighterSnapshot::default()
        });

    ai.think_alerting_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventGotHit, 2),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert!(
        !ai.base.outbox.actor.has_boundary_work(),
        "all actor effects authored before the got-hit event's final view-status assignment must be closed as a synchronous prefix"
    );
    assert!(matches!(
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .rev()
            .nth(1),
        Some(crate::ai::AiOwnerWork::ActorEffects(effects)) if effects.halt
    ));
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.last(),
        Some(crate::ai::AiOwnerWork::SetEyeStatus(
            crate::element::EyeStatus::DieOrGetUnconscious
        ))
    ));
}

#[test]
fn classic_apple_rule_keeps_a_swordfighter_engaged() {
    let config = crate::engine::SimConfig {
        item_gameplay: crate::gameplay_config::ItemGameplayConfig::classic(),
        ..Default::default()
    };
    let sim = crate::sim_rng::SimulationContext::with_seed_and_config(7, config);
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Attacking, Substate::AttackingSwordfight);
    let ctx = AiContext {
        is_swordfighting: true,
        frame: 50,
        ..AiContext::test_fixture()
    };

    ai.think_alerting_event(
        &sim,
        &Stimulus::with_position(StimulusType::EventApple, Position::default()),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
    assert!(!ai.base.outbox.actor.quit_swordfight);
}

#[test]
fn rebalanced_apple_interrupts_then_owns_the_fighter_state() {
    let mut config = crate::engine::SimConfig {
        item_gameplay: crate::gameplay_config::ItemGameplayConfig::classic(),
        ..Default::default()
    };
    config.item_gameplay.apple_combat_interrupt = true;
    let sim = crate::sim_rng::SimulationContext::with_seed_and_config(7, config);
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Attacking, Substate::AttackingSwordfight);
    let ctx = AiContext {
        is_swordfighting: true,
        frame: 50,
        ..AiContext::test_fixture()
    };

    ai.think_alerting_event(
        &sim,
        &Stimulus::with_position(StimulusType::EventApple, Position::default()),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Wondering);
    assert_eq!(
        ai.base.current_substate,
        Substate::WonderingAppleSauceInTheVisor
    );
    let state_change = ai
        .base
        .outbox
        .reentrant
        .owner_work
        .iter()
        .rev()
        .find_map(|work| match work {
            crate::ai::AiOwnerWork::StateChange(notification) => Some(notification),
            _ => None,
        })
        .expect("apple interrupt queues the Wondering state boundary");
    let interrupt_prefix = state_change
        .actor_effects_before_callback
        .as_ref()
        .expect("apple interruption applies its actor effects before state change");
    assert!(interrupt_prefix.halt);
    assert!(interrupt_prefix.quit_swordfight);
    assert!(ai.base.outbox.actor.slowly_open_eyes);
    assert_eq!(ai.base.when_does_timer_ring, 110);
}

#[test]
fn got_hit_while_swordfighting_requests_direct_entry_against_new_attacker() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);

    let mut attacker = object_view(ObjectType::None);
    attacker.kind = EntityKind::Soldier;
    attacker.camp = Camp::Royalists;
    let mut views = AiEntityViewMap::new();
    views.insert(2, attacker);
    let ctx = AiContext {
        camp: Camp::Lacklandists,
        is_swordfighting: true,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 1,
            opponent_handles: vec![3],
            ..crate::ai_enemy::FighterSnapshot::default()
        });

    ai.think_alerting_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventGotHit, 2),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(
        ai.base.outbox.actor.enter_swordfight,
        Some(EnterSwordfightRequest::Direct(AiEntityHandle::new(2))),
        "the original game enters swordfight directly from the hit event"
    );
}

#[test]
fn got_hit_while_menacing_sets_hit_animation_direction_goal_without_turn_order() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.set_state(AiState::Menacing, Substate::MenacingPcInComa);

    let mut attacker = object_view(ObjectType::None);
    attacker.kind = EntityKind::Soldier;
    attacker.position = Position {
        x: 716.74176,
        y: 252.32974,
        sector: None,
        level: 0,
    };
    let mut views = AiEntityViewMap::new();
    views.insert(2, attacker);
    let ctx = AiContext {
        position: Position {
            x: 756.42523,
            y: 205.49872,
            sector: None,
            level: 0,
        },
        direction: 15,
        is_swordfighting: false,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.think_alerting_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventGotHit, 2),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(
        ai.base.outbox.actor.set_direction,
        Some(9),
        "Original direction assignment faces the hitter while RECEIVE_HIT_DAMAGE remains installed"
    );
    assert!(
        ai.base.outbox.actor.orders.is_empty(),
        "direct direction assignment must not launch a standalone turn sequence"
    );
    assert_eq!(
        ai.base.outbox.actor.enter_swordfight,
        Some(EnterSwordfightRequest::RaiseSword)
    );
}

#[test]
fn got_hit_while_swordfighting_ignores_existing_opponent() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    let mut attacker = object_view(ObjectType::None);
    attacker.kind = EntityKind::Soldier;
    attacker.camp = Camp::Royalists;
    let mut views = AiEntityViewMap::new();
    views.insert(2, attacker);
    let ctx = AiContext {
        camp: Camp::Lacklandists,
        is_swordfighting: true,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 1,
            opponent_handles: vec![2],
            ..crate::ai_enemy::FighterSnapshot::default()
        });

    ai.think_alerting_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventGotHit, 2),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(ai.base.outbox.actor.enter_swordfight, None);
}

#[test]
#[should_panic(expected = "soldier 1 EVENT_GOTHIT requires attacker 2 entity view")]
fn got_hit_while_swordfighting_requires_attacker_entity_view() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    let ctx = AiContext {
        is_swordfighting: true,
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 1,
            ..crate::ai_enemy::FighterSnapshot::default()
        });

    ai.think_alerting_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventGotHit, 2),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );
}

#[test]
fn got_hit_by_friend_does_not_require_fighter_snapshot() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    let mut attacker = object_view(ObjectType::None);
    attacker.kind = EntityKind::Soldier;
    attacker.camp = Camp::Lacklandists;
    let mut views = AiEntityViewMap::new();
    views.insert(2, attacker);
    let ctx = AiContext {
        camp: Camp::Lacklandists,
        is_swordfighting: true,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.think_alerting_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventGotHit, 2),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.outbox.actor.enter_swordfight, None);
}

#[test]
#[should_panic(expected = "soldier 1 EVENT_GOTHIT requires self fighter snapshot")]
fn got_hit_while_swordfighting_requires_self_fighter_snapshot() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    let mut attacker = object_view(ObjectType::None);
    attacker.kind = EntityKind::Soldier;
    attacker.camp = Camp::Royalists;
    let mut views = AiEntityViewMap::new();
    views.insert(2, attacker);
    let ctx = AiContext {
        camp: Camp::Lacklandists,
        is_swordfighting: true,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.think_alerting_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventGotHit, 2),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );
}

#[test]
fn got_hit_can_begin_close_swordfight_from_default_state() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    assert_eq!(ai.base.current_state, AiState::Default);

    let mut attacker = object_view(ObjectType::None);
    attacker.kind = EntityKind::Pc;
    attacker.is_pc = true;
    attacker.camp = Camp::Royalists;
    attacker.position = Position::default();
    let mut views = AiEntityViewMap::new();
    views.insert(2, attacker);
    let ctx = AiContext {
        camp: Camp::Lacklandists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.fighter_registry
        .push(crate::ai_enemy::FighterSnapshot {
            handle: 1,
            ..crate::ai_enemy::FighterSnapshot::default()
        });

    ai.think_alerting_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventGotHit, 2),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    let engage = ai.base.outbox.actor.enter_swordfight.or_else(|| {
        ai.base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .find_map(|work| match work {
                crate::ai::AiOwnerWork::StateChange(notification) => notification
                    .actor_effects_before_callback
                    .as_ref()
                    .and_then(|effects| effects.enter_swordfight),
                _ => None,
            })
    });
    assert_eq!(
        engage,
        Some(EnterSwordfightRequest::Engage(AiEntityHandle::new(2)))
    );
    assert_eq!(ai.base.current_state, AiState::Attacking);
    assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
}

#[test]
fn seeing_shadow_raises_music_alert_without_accelerating_view_refresh() {
    let mut ai = EnemyAi::new(1);
    let ctx = AiContext {
        posture: Posture::Upright,
        ..AiContext::test_fixture()
    };

    ai.event_sees_shadow_standard_procedure(
        &Position {
            x: 10.0,
            y: 20.0,
            sector: None,
            level: 0,
        },
        &ctx,
        &AiPerTickData::stub(),
    );

    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
    assert_eq!(ai.base.view_alert_status, AlertLevel::Green);
    assert_eq!(ai.base.current_substate, Substate::DefaultLookingShadow);
}

fn ctx_with_object(object_type: ObjectType) -> AiContext {
    let mut views = AiEntityViewMap::new();
    views.insert(2, object_view(object_type));
    AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        posture: Posture::Upright,
        ..AiContext::test_fixture()
    }
}

#[test]
fn event_sees_runtime_money_objects_reacts_but_bonus_purse_is_ignored() {
    for object_type in [ObjectType::Purse, ObjectType::Coin] {
        let mut ai = EnemyAi::new(1);
        let ctx = ctx_with_object(object_type);

        ai.event_sees_object_standard_procedure(2, &ctx, &AiPerTickData::stub());

        assert_eq!(ai.base.current_state, AiState::Wondering);
        assert_eq!(
            ai.base.current_substate,
            Substate::WonderingMoneyReactiontime
        );
        assert_eq!(ai.base.interesting_object, Some(AiEntityHandle::new(2)));
    }

    let mut ai = EnemyAi::new(1);
    let ctx = ctx_with_object(ObjectType::BonusPurse);

    ai.event_sees_object_standard_procedure(2, &ctx, &AiPerTickData::stub());

    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
    assert_eq!(ai.base.interesting_object, None);
}

#[test]
fn event_sees_runtime_ale_reacts_but_bonus_ale_is_ignored() {
    let mut ai = EnemyAi::new(1);
    let ale_position = Position {
        x: 632.4453,
        y: 1835.14,
        sector: None,
        level: 0,
    };
    let mut ale_view = object_view(ObjectType::Ale);
    ale_view.position = ale_position;
    let mut views = AiEntityViewMap::new();
    views.insert(2, ale_view);
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        posture: Posture::Upright,
        ..AiContext::test_fixture()
    };

    ai.event_sees_object_standard_procedure(2, &ctx, &AiPerTickData::stub());

    assert_eq!(ai.base.current_state, AiState::Wondering);
    assert_eq!(ai.base.current_substate, Substate::WonderingAleReactiontime);
    assert_eq!(ai.base.interesting_object, Some(AiEntityHandle::new(2)));
    assert_eq!(ai.base.seek_position, ale_position);

    let mut ai = EnemyAi::new(1);
    let ctx = ctx_with_object(ObjectType::BonusAle);

    ai.event_sees_object_standard_procedure(2, &ctx, &AiPerTickData::stub());

    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
    assert_eq!(ai.base.interesting_object, None);
}

#[test]
fn event_sees_civilian_beggar_preserves_the_legacy_slots_entity_kind() {
    let sim_context = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSeekpoint;

    let mut beggar_view = object_view(ObjectType::None);
    beggar_view.kind = EntityKind::Civilian;
    beggar_view.is_beggar = true;
    let mut views = AiEntityViewMap::new();
    views.insert(17, beggar_view);
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.think_unexpected_event(
        &sim_context,
        &Stimulus::with_human(StimulusType::EventSeesBeggar, 17),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(
        ai.base.outbox.actor.delete_beggar_for_all_npc,
        vec![crate::element::EntityId::Civilian(
            crate::entity_id::CivilianId(17)
        )]
    );
}

#[test]
fn event_sees_current_beggar_does_not_requeue_but_still_requests_global_scrub() {
    let sim_context = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSeekpointApproachingBeggar;
    ai.beggar_to_examine = Some(AiEntityHandle::new(17));

    let mut beggar_view = object_view(ObjectType::None);
    beggar_view.kind = EntityKind::Civilian;
    beggar_view.is_beggar = true;
    let mut views = AiEntityViewMap::new();
    views.insert(17, beggar_view);
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.think_unexpected_event(
        &sim_context,
        &Stimulus::with_human(StimulusType::EventSeesBeggar, 17),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert!(ai.beggars_to_control.is_empty());
    assert!(ai.positions_of_beggars_to_control.is_empty());
    assert_eq!(
        ai.base.outbox.actor.delete_beggar_for_all_npc,
        vec![crate::element::EntityId::Civilian(
            crate::entity_id::CivilianId(17)
        )]
    );
}

#[test]
fn event_enemy_near_assigns_stimulus_target_and_begins_swordfight() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    for substate in [
        Substate::AttackingReactiontimeTurning,
        Substate::AttackingReactiontime,
        Substate::AttackingApproachToObserve,
        Substate::AttackingObserve,
    ] {
        let mut ai = EnemyAi::new(1);
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = substate;
        ai.base.primary_target = Some(AiEntityHandle::new(12));
        // The original trainer gate is exclusively on the sender.
        ai.combat_trainer = true;

        let stimulus = Stimulus::with_human(StimulusType::EventEnemyNear, 77);
        ai.think_unexpected_event(
            sim,
            &stimulus,
            &mut AiGlobalState::default(),
            &AiContext::test_fixture(),
            &AiPerTickData::stub(),
            None,
        );

        assert_eq!(
            ai.base.primary_target,
            Some(AiEntityHandle::new(77)),
            "substate {substate:?}"
        );
        // begin_swordfight raises Engage before its state change suspends
        // the actor-outbox prefix into the queued state-change owner
        // work; read the request from either place.
        let engage = ai.base.outbox.actor.enter_swordfight.or_else(|| {
            ai.base
                .outbox
                .reentrant
                .owner_work
                .iter()
                .find_map(|work| match work {
                    crate::ai::AiOwnerWork::StateChange(notification) => notification
                        .actor_effects_before_callback
                        .as_ref()
                        .and_then(|effects| effects.enter_swordfight),
                    _ => None,
                })
        });
        assert_eq!(
            engage,
            Some(EnterSwordfightRequest::Engage(AiEntityHandle::new(77))),
            "substate {substate:?}"
        );
        assert_eq!(ai.base.current_substate, Substate::AttackingSwordfight);
    }
}

#[test]
fn event_enemy_near_is_ignored_outside_original_substates() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToEnemy;
    ai.base.primary_target = Some(AiEntityHandle::new(12));

    let stimulus = Stimulus::with_human(StimulusType::EventEnemyNear, 77);
    ai.think_unexpected_event(
        sim,
        &stimulus,
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.primary_target, Some(AiEntityHandle::new(12)));
    assert_eq!(ai.base.outbox.actor.enter_swordfight, None);
    assert_eq!(ai.base.current_substate, Substate::AttackingRunningToEnemy);
}

#[test]
fn officer_call_alert_halts_actor_without_breaking_running_macro() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(44);
    ai.soldier_profile_rank = ProfileRank::Soldier;
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultInMacro;
    ai.base.macro_in_progress = true;
    ai.base.macro_timer_is_running = true;
    ai.base.when_does_macro_timer_ring = 10_054;
    ai.current_task_priority = task_priority::ALERT;
    ai.new_task_priority = task_priority::ALERT;

    let mut officer = object_view(ObjectType::None);
    officer.kind = EntityKind::Soldier;
    officer.rank = ProfileRank::Officer;
    officer.position = Position {
        x: 100.0,
        y: 20.0,
        sector: None,
        level: 0,
    };
    let mut views = AiEntityViewMap::new();
    views.insert(91, officer);
    let ctx = AiContext {
        frame: 9_768,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let accepted = ai.think_unexpected_event(
        &sim,
        &Stimulus::with_human(StimulusType::CallAlert, 91),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert!(accepted);
    let halted_before_state_change = ai.base.outbox.reentrant.owner_work.iter().any(|work| {
        matches!(
            work,
            crate::ai::AiOwnerWork::StateChange(notification)
                if notification
                    .actor_effects_before_callback
                    .as_ref()
                    .is_some_and(|effects| effects.halt)
        )
    });
    assert!(halted_before_state_change);
    assert!(ai.base.macro_in_progress);
    assert!(ai.base.macro_timer_is_running);
    assert_eq!(ai.base.when_does_macro_timer_ring, 10_054);
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingGroupCalledByOfficer
    );
}

#[test]
fn civilian_call_alert_halts_actor_without_breaking_running_macro() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(44);
    ai.soldier_profile_rank = ProfileRank::Soldier;
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultInMacro;
    ai.base.macro_in_progress = true;
    ai.base.macro_timer_is_running = true;
    ai.base.when_does_macro_timer_ring = 10_054;

    let mut civilian = object_view(ObjectType::None);
    civilian.kind = EntityKind::Civilian;
    civilian.position = Position {
        x: 100.0,
        y: 20.0,
        sector: None,
        level: 0,
    };
    let mut views = AiEntityViewMap::new();
    views.insert(91, civilian);
    let ctx = AiContext {
        frame: 9_768,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let accepted = ai.think_unexpected_event(
        &sim,
        &Stimulus::with_human(StimulusType::CallAlert, 91),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert!(accepted);
    let halted_before_state_change = ai.base.outbox.reentrant.owner_work.iter().any(|work| {
        matches!(
            work,
            crate::ai::AiOwnerWork::StateChange(notification)
                if notification
                    .actor_effects_before_callback
                    .as_ref()
                    .is_some_and(|effects| effects.halt)
        )
    });
    assert!(halted_before_state_change);
    assert!(ai.base.macro_in_progress);
    assert!(ai.base.macro_timer_is_running);
    assert_eq!(ai.base.when_does_macro_timer_ring, 10_054);
    assert_eq!(ai.base.antagonist, Some(AiEntityHandle::new(91)));
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingWaitForAlertingCivilian
    );
    assert!(ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 9_788);
}

#[test]
fn rejected_civilian_call_alert_still_replaces_antagonist() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(44);
    ai.soldier_profile_rank = ProfileRank::Soldier;
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingRunningToOfficer;
    ai.base.antagonist = Some(AiEntityHandle::new(78));

    let mut civilian = object_view(ObjectType::None);
    civilian.kind = EntityKind::Civilian;
    let mut views = AiEntityViewMap::new();
    views.insert(91, civilian);
    let ctx = AiContext {
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let accepted = ai.think_unexpected_event(
        &sim,
        &Stimulus::with_human(StimulusType::CallAlert, 91),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert!(!accepted);
    assert_eq!(ai.base.antagonist, Some(AiEntityHandle::new(91)));
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(ai.base.current_substate, Substate::SeekingRunningToOfficer);
}

#[test]
fn soldier_call_alert_halts_officer_without_breaking_running_macro() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(44);
    ai.soldier_profile_rank = ProfileRank::Officer;
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultInMacro;
    ai.base.macro_in_progress = true;
    ai.base.macro_timer_is_running = true;
    ai.base.when_does_macro_timer_ring = 10_054;

    let mut soldier = object_view(ObjectType::None);
    soldier.kind = EntityKind::Soldier;
    soldier.rank = ProfileRank::Soldier;
    soldier.position = Position {
        x: 100.0,
        y: 20.0,
        sector: None,
        level: 0,
    };
    let mut views = AiEntityViewMap::new();
    views.insert(91, soldier);
    let ctx = AiContext {
        frame: 9_768,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let accepted = ai.think_unexpected_event(
        &sim,
        &Stimulus::with_human(StimulusType::CallAlert, 91),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert!(accepted);
    let halted_before_state_change = ai.base.outbox.reentrant.owner_work.iter().any(|work| {
        matches!(
            work,
            crate::ai::AiOwnerWork::StateChange(notification)
                if notification
                    .actor_effects_before_callback
                    .as_ref()
                    .is_some_and(|effects| effects.halt)
        )
    });
    assert!(halted_before_state_change);
    assert!(ai.base.macro_in_progress);
    assert!(ai.base.macro_timer_is_running);
    assert_eq!(ai.base.when_does_macro_timer_ring, 10_054);
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingOfficerWaitForAlertingSoldier
    );
}

#[test]
fn couldnt_reach_running_enemy_enters_battle_overview() {
    let sim_context = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(1);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToEnemy;
    ai.base.list_us = vec![1, 2];

    ai.think_unexpected_event(
        &sim_context,
        &Stimulus::new(StimulusType::EventCouldntReachPoint),
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingOverviewLookLeft
    );
    assert_eq!(
        ai.base.outbox.actor.look_sidewards,
        Some(LookDirection::Left)
    );
    assert_eq!(
        ai.base.list_us,
        vec![1, 2],
        "battle-overview evaluation must not rebuild the persistent friend list"
    );
}

#[test]
fn same_frame_observe_move_failure_resumes_inline_roof_fallback() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(64);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingApproachToObserve;
    ai.base.primary_target = Some(AiEntityHandle::new(183));
    ai.base.ai_log.push(LogLine {
        line_type: LogLineType::BattleDecision,
        info: Decision::Observe as u16,
        frame: 8_103,
    });

    let target_position = Position {
        x: 2_788.0,
        y: 1_029.0,
        sector: crate::position_interface::SectorHandle::new(53),
        level: 2,
    };
    let wait_position = Position {
        x: 2_793.0,
        y: 571.0,
        sector: crate::position_interface::SectorHandle::new(53),
        level: 2,
    };
    let mut target = object_view(ObjectType::None);
    target.kind = EntityKind::Pc;
    target.is_pc = true;
    target.position = target_position;
    let mut views = AiEntityViewMap::new();
    views.insert(183, target);
    let ctx = AiContext {
        frame: 8_103,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.avenger_on_roof_wait_positions
        .push((183, wait_position));

    let mut stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
    stimulus.self_origin = crate::ai::SelfStimulusOrigin::EngineCompletion;
    ai.think_unexpected_event(
        &sim,
        &stimulus,
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunToAvengerOnRoof
    );
    assert_eq!(ai.base.seek_position, target_position);
    assert_eq!(ai.base.last_goto_destination, wait_position);
    assert!(ai.base.outbox.actor.look_sidewards.is_none());
}

#[test]
fn same_frame_fight_lift_failure_preserves_inline_roof_fallback() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(64);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToLadder;
    ai.base.primary_target = Some(AiEntityHandle::new(183));
    ai.base.last_synced_focus_target = Some(AiEntityHandle::new(183));
    ai.base.timer_is_running = true;
    ai.base.substate_at_last_timer_launch = Substate::AttackingRunningToLadder;
    ai.base.when_does_timer_ring = 8_133;

    let target_position = Position {
        x: 2_788.0,
        y: 1_029.0,
        sector: crate::position_interface::SectorHandle::new(63),
        level: 3,
    };
    let wait_position = Position {
        x: 2_793.0,
        y: 571.0,
        sector: crate::position_interface::SectorHandle::new(53),
        level: 2,
    };
    let mut target = object_view(ObjectType::None);
    target.kind = EntityKind::Pc;
    target.is_pc = true;
    target.position = target_position;
    let mut views = AiEntityViewMap::new();
    views.insert(183, target);
    let ctx = AiContext {
        frame: 8_103,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut tick = AiPerTickData::stub();
    tick.avenger_on_roof_wait_positions
        .push((183, wait_position));

    let mut stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
    stimulus.self_origin = crate::ai::SelfStimulusOrigin::EngineCompletion;
    ai.think_unexpected_event(
        &sim,
        &stimulus,
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunToAvengerOnRoof
    );
    assert_eq!(ai.base.seek_position, target_position);
    assert_eq!(ai.base.last_goto_destination, wait_position);
    assert_eq!(
        ai.base.last_synced_focus_target,
        Some(AiEntityHandle::new(183))
    );
    assert!(!ai.base.couldnt_reachpoint);
    assert!(ai.base.outbox.actor.orders.is_empty());
    let Some(crate::ai::AiOwnerWork::ActorEffects(roof_effects)) =
        ai.base.outbox.reentrant.owner_work.last()
    else {
        panic!("roof fallback must settle at a synchronous owner boundary")
    };
    assert_eq!(roof_effects.orders.len(), 1);
    assert_eq!(roof_effects.orders[0].target_x, wait_position.x);
    assert_eq!(roof_effects.orders[0].target_y, wait_position.y);
    assert!(roof_effects.look_sidewards.is_none());
    assert!(!roof_effects.unfocus);
}

#[test]
fn same_frame_roof_fallback_failure_uses_generic_overview() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(64);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunToAvengerOnRoof;
    ai.base.primary_target = Some(AiEntityHandle::new(183));
    ai.base.list_us = vec![64, 79];

    ai.think_unexpected_event(
        &sim,
        &Stimulus::new(StimulusType::EventCouldntReachPoint),
        &mut AiGlobalState::default(),
        &AiContext {
            frame: 7_938,
            ..AiContext::test_fixture()
        },
        &AiPerTickData::stub(),
        None,
    );
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingOverviewLookLeft
    );
    assert_eq!(
        ai.base.outbox.actor.look_sidewards,
        Some(LookDirection::Left)
    );
}

#[test]
fn same_frame_ladder_condolation_uses_generic_overview() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(64);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToLadder;
    ai.base.primary_target = Some(AiEntityHandle::new(183));
    ai.base.list_us = vec![64, 79];
    ai.base.timer_is_running = true;
    ai.base.substate_at_last_timer_launch = Substate::AttackingRunningToLadder;
    ai.base.when_does_timer_ring = 7_968;

    let mut stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
    stimulus.self_origin = crate::ai::SelfStimulusOrigin::Condolation;
    ai.think_unexpected_event(
        &sim,
        &stimulus,
        &mut AiGlobalState::default(),
        &AiContext {
            frame: 7_938,
            ..AiContext::test_fixture()
        },
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingOverviewLookLeft
    );
    assert_eq!(
        ai.base.outbox.actor.look_sidewards,
        Some(LookDirection::Left)
    );
}

#[test]
fn later_ladder_failure_still_uses_generic_emergency_routine() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(64);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToLadder;
    ai.base.primary_target = Some(AiEntityHandle::new(183));
    ai.base.list_us = vec![64, 79];
    ai.base.ai_log.push(LogLine {
        line_type: LogLineType::BattleDecision,
        info: Decision::Fight as u16,
        frame: 8_102,
    });

    let ctx = AiContext {
        frame: 8_103,
        ..AiContext::test_fixture()
    };
    let mut stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
    stimulus.self_origin = crate::ai::SelfStimulusOrigin::EngineCompletion;
    ai.think_unexpected_event(
        &sim,
        &stimulus,
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingOverviewLookLeft
    );
    assert_eq!(
        ai.base.outbox.actor.look_sidewards,
        Some(LookDirection::Left)
    );
}

#[test]
fn same_frame_fight_lift_failure_without_roof_wait_resumes_observe_then_overview() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(64);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToLadder;
    ai.base.primary_target = Some(AiEntityHandle::new(183));
    ai.base.last_synced_focus_target = Some(AiEntityHandle::new(183));
    ai.base.timer_is_running = true;
    ai.base.substate_at_last_timer_launch = Substate::AttackingRunningToLadder;
    ai.base.when_does_timer_ring = 7_968;

    let target_position = Position {
        x: 2_762.243,
        y: 882.6701,
        sector: crate::position_interface::SectorHandle::new(53),
        level: 2,
    };
    let mut target = object_view(ObjectType::None);
    target.kind = EntityKind::Pc;
    target.is_pc = true;
    target.position = target_position;
    let mut views = AiEntityViewMap::new();
    views.insert(183, target);
    let ctx = AiContext {
        frame: 7_938,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let tick = AiPerTickData::stub();

    let mut engine_completion = Stimulus::new(StimulusType::EventCouldntReachPoint);
    engine_completion.self_origin = crate::ai::SelfStimulusOrigin::EngineCompletion;
    ai.think_unexpected_event(
        &sim,
        &engine_completion,
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingApproachToObserve
    );
    assert_eq!(ai.base.when_does_timer_ring, 7_988);
    assert!(ai.base.timer_is_running);
    assert!(ai.base.couldnt_reachpoint);
    assert_eq!(
        ai.base.last_synced_focus_target,
        Some(AiEntityHandle::new(183))
    );

    ai.think_unexpected_event(
        &sim,
        &Stimulus::new(StimulusType::EventCouldntReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &tick,
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingOverviewLookLeft
    );
    assert_eq!(
        ai.base.outbox.actor.look_sidewards,
        Some(LookDirection::Left)
    );
}

#[test]
fn later_roof_failure_still_uses_generic_emergency_routine() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(64);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunToAvengerOnRoof;
    ai.base.primary_target = Some(AiEntityHandle::new(183));
    ai.base.list_us = vec![64, 79];
    let ctx = AiContext {
        frame: 8_104,
        ..AiContext::test_fixture()
    };
    ai.think_unexpected_event(
        &sim,
        &Stimulus::new(StimulusType::EventCouldntReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingOverviewLookLeft
    );
    assert_eq!(
        ai.base.outbox.actor.look_sidewards,
        Some(LookDirection::Left)
    );
}

#[test]
fn couldnt_reach_seeking_body_examines_queued_body_before_starting_seek_area() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(206);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBody;
    ai.other_bodies_to_examine.push(207);

    let alternate_position = Position {
        x: 658.0,
        y: 2910.0,
        sector: crate::position_interface::SectorHandle::new(18),
        level: 0,
    };
    let mut alternate_body = object_view(ObjectType::None);
    alternate_body.kind = EntityKind::Soldier;
    alternate_body.position = alternate_position;
    // Other-body examination prunes the queue by incapacitation, not
    // by combat readiness — a KO'd body is what keeps this entry
    // queued.
    alternate_body.is_unconscious = true;
    alternate_body.is_able_to_fight = false;
    let mut views = AiEntityViewMap::new();
    views.insert(207, alternate_body);
    let ctx = AiContext {
        position: Position {
            x: 792.0,
            y: 2612.0,
            sector: crate::position_interface::SectorHandle::new(44),
            level: 0,
        },
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    ai.think_unexpected_event(
        &sim,
        &Stimulus::new(StimulusType::EventCouldntReachPoint),
        &mut AiGlobalState::default(),
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.base.detected_body, Some(AiEntityHandle::new(207)));
    assert_eq!(ai.base.seek_position, alternate_position);
    assert_eq!(ai.base.current_substate, Substate::SeekingBody);
    assert!(ai.my_seek_points.is_empty());
}

/// Enemy examination of other bodies
/// prunes the queue head while the body is not out of order. That test is
/// a body-state predicate, *not* combat readiness.
/// Civilians never report able-to-fight, so proxying the two keeps a woken
/// civilian sleeper queued forever: the soldier re-examines the body it is
/// already standing next to, approach movement short-circuits to
/// `EVENT_REACHPOINT`, and the seek collapses into returning to duty.
#[test]
fn examine_other_bodies_prunes_recovered_civilian_that_cannot_fight() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(206);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBody;
    // Head of the queue: a civilian that woke up. `is_able_to_fight` is
    // false for every civilian, but incapacitation is false now.
    ai.other_bodies_to_examine.push(207);
    // Behind it: a soldier that is genuinely still down.
    ai.other_bodies_to_examine.push(208);

    let mut recovered = object_view(ObjectType::None);
    recovered.kind = EntityKind::Civilian;
    recovered.is_able_to_fight = false;

    let down_position = Position {
        x: 658.0,
        y: 2910.0,
        sector: crate::position_interface::SectorHandle::new(18),
        level: 0,
    };
    let mut still_down = object_view(ObjectType::None);
    still_down.kind = EntityKind::Soldier;
    still_down.position = down_position;
    still_down.is_able_to_fight = false;
    still_down.is_unconscious = true;

    let mut views = AiEntityViewMap::new();
    views.insert(207, recovered);
    views.insert(208, still_down);
    let ctx = AiContext {
        position: Position {
            x: 792.0,
            y: 2612.0,
            sector: crate::position_interface::SectorHandle::new(44),
            level: 0,
        },
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    assert!(ai.examine_other_bodies(&ctx, &AiPerTickData::stub()));
    assert_eq!(
        ai.base.detected_body,
        Some(AiEntityHandle::new(208)),
        "the recovered civilian must be pruned, not examined"
    );
    assert_eq!(ai.base.seek_position, down_position);
    assert!(ai.other_bodies_to_examine.is_empty());
    let _ = &sim;
}

/// The predicate itself, in both directions: incapacitation is the OR of
/// the five body states (plus PC coma) and is independent of
/// combat readiness.
#[test]
fn is_out_of_order_is_not_the_complement_of_is_able_to_fight() {
    // Civilian that is up and about: never able to fight, but in order.
    let mut civilian = object_view(ObjectType::None);
    civilian.kind = EntityKind::Civilian;
    civilian.is_able_to_fight = false;
    assert!(!civilian.is_out_of_order());

    // Netted / tied / carried / KO'd / dead all count as out of order even
    // when the combat-readiness flag says otherwise.
    for apply in [
        (|v: &mut AiEntityView| v.stuck_under_net = true) as fn(&mut AiEntityView),
        |v: &mut AiEntityView| v.posture = Posture::Tied,
        |v: &mut AiEntityView| v.posture = Posture::Carried,
        |v: &mut AiEntityView| v.is_unconscious = true,
        |v: &mut AiEntityView| v.is_dead = true,
    ] {
        let mut soldier = object_view(ObjectType::None);
        soldier.kind = EntityKind::Soldier;
        soldier.is_able_to_fight = true;
        apply(&mut soldier);
        assert!(soldier.is_out_of_order());
    }

    // The coma arm is PC-only.
    let mut comatose = object_view(ObjectType::None);
    comatose.kind = EntityKind::Pc;
    comatose.in_coma = true;
    assert!(comatose.is_out_of_order());
    let mut soldier_flagged_coma = object_view(ObjectType::None);
    soldier_flagged_coma.kind = EntityKind::Soldier;
    soldier_flagged_coma.in_coma = true;
    assert!(!soldier_flagged_coma.is_out_of_order());
}

#[test]
fn couldnt_reach_seeking_body_centers_fallback_on_actor_not_stale_body() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(206);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBody;
    ai.base.seek_position = Position {
        x: 2_000.0,
        y: 2_000.0,
        sector: crate::position_interface::SectorHandle::new(44),
        level: 0,
    };
    let actor_position = Position {
        x: 792.0,
        y: 2612.0,
        sector: crate::position_interface::SectorHandle::new(44),
        level: 0,
    };
    let actor_seek_point = Position {
        x: 652.0,
        y: 2_928.0,
        sector: crate::position_interface::SectorHandle::new(44),
        level: 0,
    };
    let stale_body_seek_point = Position {
        x: 2_020.0,
        y: 2_000.0,
        sector: crate::position_interface::SectorHandle::new(44),
        level: 0,
    };
    let point = |id, position| SeekPoint {
        position,
        frame_when_full_interest: 0,
        directions: vec![0],
        last_calculated_interest: 100,
        locked: false,
        id,
    };
    let mut global = AiGlobalState {
        seek_points: vec![point(0, actor_seek_point), point(1, stale_body_seek_point)],
        ..Default::default()
    };
    let ctx = AiContext {
        position: actor_position,
        self_is_soldier: true,
        ..AiContext::test_fixture()
    };

    ai.think_unexpected_event(
        &sim,
        &Stimulus::new(StimulusType::EventCouldntReachPoint),
        &mut global,
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.seek_center, actor_position);
    assert_eq!(ai.actual_seek_point, Some(0));
    assert_eq!(ai.base.last_goto_destination, actor_seek_point);
    assert_ne!(ai.base.last_goto_destination, stale_body_seek_point);
}

#[test]
fn event_hear_faces_noise_position_projection_not_recorded_actor_elevation() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(70);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSeekpoint;

    // Trace-shaped boundary from SuN/Profile_004/Savegame_016 r013
    // frame 895. Sector 0 has no projection area, so world-point conversion
    // leaves the noise at z=0 even though the producing PC recorded its
    // own elevation (36) on the noise record.
    let noise = Noise {
        origin: NoiseOrigin::from_position(Position {
            x: f32::from_bits(0x428b_1027),
            y: f32::from_bits(0x43af_c940),
            sector: crate::position_interface::SectorHandle::new(0).map(|sector| {
                sector.with_arena_index(crate::fast_find_grid::SectorIndex::new(0).unwrap())
            }),
            level: 0,
        }),
        noise_type: NoiseType::ZingZing,
        volume: 200,
        elevation: 36,
        element_id: 133,
    };
    let ctx = AiContext {
        position: Position {
            x: f32::from_bits(0x4326_9901),
            y: f32::from_bits(0x438f_54f0),
            sector: crate::position_interface::SectorHandle::new(0),
            level: 0,
        },
        self_body_position_world: crate::coordinates::WorldPoint3D::new(
            f32::from_bits(0x4326_9901),
            f32::from_bits(0x43a1_5511),
            f32::from_bits(0x4210_0107),
        ),
        elevation: f32::from_bits(0x4210_0107),
        direction: 4,
        ..AiContext::test_fixture_with_motion_sector(0, 0)
    };

    let scalar_dx = noise.origin.x - ctx.position.x;
    let scalar_dy = (noise.origin.y - ctx.position.y) + (noise.elevation as f32 - ctx.elevation);
    assert_eq!(
        crate::position_interface::vector_to_sector_0_to_15_with_aspect(
            scalar_dx,
            scalar_dy,
            crate::position_interface::ASPECT_RATIO,
        ),
        10,
        "the replaced scalar-elevation shortcut must select the adjacent sector"
    );

    ai.event_hear_standard_procedure(&sim, &noise, &ctx, &AiPerTickData::stub());

    let turn = ai
        .base
        .outbox
        .actor
        .orders
        .iter()
        .find(|intent| intent.order_type == OrderType::Turning)
        .expect("the seeking EventHear arm must author a Turn");
    assert_eq!(turn.explicit_direction, Some(11));
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingHeardstepsReactiontime
    );
}

/// Linux3/Profile_001/Savegame_028 r001 frame 10385: an arrow landed
/// outside every motion area, so the original game's trajectory calculation left its
/// impact position at the authored no-sector and no-layer sentinel.
/// EventHear stores that raw position and Face projects it at ground
/// level; rejecting the sentinel aborts an otherwise valid replay.
#[test]
fn event_hear_zonk_preserves_null_layer_impact_position() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(161);
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.new_task_priority = task_priority::STRANGE_THING;

    let noise = Noise {
        origin: NoiseOrigin::from_position(Position {
            x: 341.819_34,
            y: 716.628_85,
            sector: None,
            level: u16::MAX,
        }),
        noise_type: NoiseType::Zonk,
        volume: 1,
        elevation: 480,
        element_id: 0,
    };
    let ctx = AiContext {
        frame: 10_385,
        position: Position {
            x: 317.8,
            y: 716.0,
            sector: None,
            level: 8,
        },
        self_body_position_world: crate::coordinates::WorldPoint3D {
            x: 317.8,
            y: 1196.001,
            z: 480.001_04,
        },
        elevation: 480.001_04,
        direction: 4,
        self_is_active: true,
        ..AiContext::test_fixture()
    };

    ai.event_hear_standard_procedure(&sim, &noise, &ctx, &AiPerTickData::stub());

    assert_eq!(ai.base.seek_position, noise.origin.legacy_position());
    assert_eq!(ai.base.seek_position.level, u16::MAX);
    assert_eq!(ai.base.current_state, AiState::Wondering);
    assert_eq!(ai.base.current_substate, Substate::WonderingWatching);
    let turn = ai
        .base
        .outbox
        .actor
        .orders
        .iter()
        .find(|intent| intent.order_type == OrderType::Turning)
        .expect("the null-sector impact still authors Original's ground-projected Turn");
    assert_eq!(turn.explicit_direction, Some(0));
}

#[test]
fn distraction_noise_records_the_impact_and_enters_investigation() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(71);
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.current_task_priority = task_priority::NONE;
    ai.new_task_priority = task_priority::STRANGE_THING;
    let origin = Position {
        x: 140.0,
        y: 90.0,
        sector: None,
        level: 0,
    };
    let noise = Noise {
        origin: NoiseOrigin::from_position(origin),
        noise_type: NoiseType::Distraction,
        volume: crate::parameters_ai::NOISE_VOLUME_DISTRACTION as u16,
        elevation: 0,
        element_id: 0,
    };
    let ctx = AiContext {
        frame: 300,
        self_is_active: true,
        ..AiContext::test_fixture()
    };

    ai.event_hear_standard_procedure(&sim, &noise, &ctx, &AiPerTickData::stub());

    assert!(ai.investigating_distraction);
    assert_eq!(ai.base.seek_position, origin);
    assert_eq!(
        ai.base.my_reconnaissance_report.report_type,
        ReportType::Noise
    );
    assert_eq!(ai.base.my_reconnaissance_report.seek_position, origin);
    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingHeardstepsPreReactiontime
    );
    assert!(ai.base.timer_is_running);
}

/// This behavior sweeps the waiting
/// soldier's own position when the avenger it is watching for goes out of
/// view and no fighters remain. Rust used the remembered avenger position
/// seek position, which shifts the seek center and therefore the
/// near-point membership that drives the phase-4 selection draw count.
#[test]
fn avenger_roof_out_of_view_seeks_from_live_owner_position() {
    let sim = crate::sim_rng::test_context();
    let mut ai = EnemyAi::new(178);
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingWaitForAvengerOnRoof;
    ai.base.primary_target = None;
    // Keep the stale avenger center far from the live position so a
    // regression cannot accidentally pick the same seek points.
    ai.base.seek_position = Position {
        x: 240.0,
        y: 860.0,
        ..Position::default()
    };

    let live_position = Position {
        x: 1_800.0,
        y: 2_200.0,
        ..Position::default()
    };
    let mut global = AiGlobalState {
        seek_points: [(1_810.0, 2_200.0), (1_820.0, 2_200.0)]
            .into_iter()
            .enumerate()
            .map(|(id, (x, y))| crate::ai::SeekPoint {
                position: Position {
                    x,
                    y,
                    ..Position::default()
                },
                frame_when_full_interest: 0,
                directions: vec![0],
                last_calculated_interest: 100,
                locked: false,
                id: id as u16,
            })
            .collect(),
        ..Default::default()
    };

    let ctx = AiContext {
        frame: 12_345,
        position: live_position,
        ..AiContext::test_fixture()
    };

    ai.think_unexpected_event(
        &sim,
        &Stimulus::with_human(StimulusType::EventOutOfView, 42),
        &mut global,
        &ctx,
        &AiPerTickData::stub(),
        None,
    );

    assert_eq!(ai.seek_center, live_position);
    assert_ne!(ai.seek_center, ai.base.seek_position);
}
