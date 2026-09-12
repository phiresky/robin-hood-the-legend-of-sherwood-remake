use super::*;

#[test]
fn friendly_ai_defaults() {
    let ai = FriendlyAi::new(99);
    assert_eq!(ai.base.me, 99);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.beggar_dont_talk_counter, 0);
}

#[test]
fn civilian_start_think_distinguishes_static_and_ailock_freeze() {
    let ctx = AiContext::test_fixture();
    let stimulus = Stimulus::new(StimulusType::EventTimer);

    let mut static_frozen = FriendlyAi::new(1);
    assert!(!static_frozen.start_think(&stimulus, &ctx, true));
    assert!(static_frozen.base.stimulus_queue.is_empty());

    let mut ai_locked = FriendlyAi::new(2);
    ai_locked.base.locks_flag_field = AiLockFlags::FREEZE;
    assert!(!ai_locked.start_think(&stimulus, &ctx, false));
    assert_eq!(ai_locked.base.stimulus_queue.len(), 1);
    assert_eq!(
        ai_locked.base.stimulus_queue[0].stimulus_type,
        StimulusType::EventTimer
    );
}

#[test]
fn civilian_set_state() {
    let mut ai = FriendlyAi::new(1);
    ai.set_state(AiState::Fleeing, Substate::FleeingPanic);
    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
}

#[test]
fn civilian_set_state_alert_levels() {
    let mut ai = FriendlyAi::new(1);

    ai.set_state(AiState::Default, Substate::DefaultOnPost);
    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);

    ai.set_state(AiState::Wondering, Substate::WonderingCivilianAdmiringHero);
    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);

    ai.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);
    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);

    ai.set_state(AiState::Fleeing, Substate::FleeingPanic);
    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
}

#[test]
fn patrol_coordinate_uses_friendly_virtual_state_before_walk_and_run() {
    for (distance, expected_substate, expected_order) in [
        (
            45.0,
            Substate::DefaultPatrolEnroute,
            crate::order::OrderType::WalkingUpright,
        ),
        (
            60.0,
            Substate::DefaultPatrolEnrouteRunning,
            crate::order::OrderType::RunningUpright,
        ),
    ] {
        let mut ai = FriendlyAi::new(1);
        ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
            crate::entity_id::SoldierId(2),
        ));
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
        ai.base.current_music_alert_status = AlertLevel::Yellow;
        ai.base.view_alert_status = AlertLevel::Yellow;

        let ctx = AiContext {
            position: Position {
                x: 100.0,
                y: 100.0,
                sector: SectorHandle::new(1),
                level: 0,
            },
            ..AiContext::test_fixture()
        };
        let target = Position {
            x: ctx.position.x + distance,
            ..ctx.position
        };
        ai.coordinate_patrol(
            &StimulusInfo::Position(target),
            &ctx,
            Position {
                x: ctx.position.x + 100.0,
                ..ctx.position
            },
        );

        assert_eq!(ai.base.current_state, AiState::Default);
        assert_eq!(ai.base.current_substate, expected_substate);
        assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);
        assert_eq!(ai.base.view_alert_status, AlertLevel::Green);
        let [AiOwnerWork::StateChange(notification)] =
            ai.base.outbox.reentrant.owner_work.as_slice()
        else {
            panic!("patrol coordinate must trigger friendly state change");
        };
        let prefix = notification
            .actor_effects_before_callback
            .as_ref()
            .expect("StopAll must precede the friendly state callback");
        assert!(prefix.halt);
        let [order] = ai.base.outbox.actor.orders.as_slice() else {
            panic!("patrol coordinate must queue one replacement movement");
        };
        assert_eq!(order.order_type, expected_order);
    }
}

#[test]
fn patrol_coordinate_same_substate_still_calls_friendly_state_without_stop_prefix() {
    let mut ai = FriendlyAi::new(1);
    ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
        crate::entity_id::SoldierId(2),
    ));
    ai.base.current_state = AiState::Default;
    ai.base.current_substate = Substate::DefaultPatrolEnroute;
    ai.base.current_music_alert_status = AlertLevel::Yellow;
    ai.base.view_alert_status = AlertLevel::Yellow;
    let ctx = AiContext {
        position: Position {
            x: 100.0,
            y: 100.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        ..AiContext::test_fixture()
    };

    ai.coordinate_patrol(
        &StimulusInfo::Position(Position {
            x: 145.0,
            ..ctx.position
        }),
        &ctx,
        Position {
            x: 200.0,
            ..ctx.position
        },
    );

    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Green);
    assert_eq!(ai.base.view_alert_status, AlertLevel::Green);
    let [AiOwnerWork::StateChange(notification)] = ai.base.outbox.reentrant.owner_work.as_slice()
    else {
        panic!("friendly state changes must notify even when the substate is unchanged");
    };
    assert!(notification.actor_effects_before_callback.is_none());
    let [order] = ai.base.outbox.actor.orders.as_slice() else {
        panic!("same-substate patrol update must queue its movement");
    };
    assert_eq!(order.order_type, crate::order::OrderType::WalkingUpright);
}

#[test]
#[should_panic(expected = "civilian 1 running to required antagonist 42 has no entity view")]
fn review_running_to_soldier_requires_the_live_antagonist_view() {
    let sim = crate::sim_rng::test_context();
    let mut ai = FriendlyAi::new(1);
    ai.base.antagonist = Some(AiEntityHandle::new(42));
    ai.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);
    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventReachPoint),
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );
}

#[test]
fn civilian_return_to_duty() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    ai.fleeing_seen_enemy_counter = 5;
    ai.set_state(AiState::Fleeing, Substate::FleeingPanic);
    ai.return_to_duty(sim, DutyFlags::empty(), &AiContext::test_fixture());
    assert_eq!(ai.base.current_state, AiState::Default);
    // NPC walks back to initial position first, then transitions
    // to DefaultOnPost via EventReachPoint → DefaultGotoPostTurn → EventDone.
    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
    assert_eq!(ai.fleeing_seen_enemy_counter, 0);
}

#[test]
fn civilian_panic_from_point() {
    let mut ai = FriendlyAi::new(1);
    ai.panic_from_point(100.0, 200.0, 8);
    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert_eq!(ai.base.panic_center_x, 100.0);
    assert_eq!(ai.base.panic_center_y, 200.0);
    assert_eq!(ai.base.lasting_panic_runs, 8);
    assert!(ai.base.directed_panic);
    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Yellow);
}

#[test]
fn civilian_panic_undirected() {
    let mut ai = FriendlyAi::new(1);
    ai.panic_undirected(4);
    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert_eq!(ai.base.lasting_panic_runs, 4);
    assert!(!ai.base.directed_panic);
}

#[test]
#[should_panic(expected = "CALL_YOU_JUST_WAIT civilian 1 requires chaser 42 entity view")]
fn apple_chase_does_not_replace_a_missing_chaser_with_undirected_panic() {
    let sim = crate::sim_rng::test_context();
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    ai.think_unexpected_event(
        &sim,
        &Stimulus::with_human(StimulusType::CallYouJustWait, 42),
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );
}

#[test]
fn think_expected_admiring_hero_returns_to_duty() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    ai.set_state(AiState::Wondering, Substate::WonderingCivilianAdmiringHero);

    let stimulus = Stimulus::new(StimulusType::EventTimer);
    ai.think_expected_event(
        sim,
        &stimulus,
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Default);
    // Walks back to post first (DefaultGotoPost → EventReachPoint → OnPost).
    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
}

#[test]
fn think_alerting_event_panic() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let pos = Position {
        x: 50.0,
        y: 75.0,
        sector: None,
        level: 0,
    };
    let stimulus = Stimulus::with_position(StimulusType::EventPanic, pos);

    ai.think_alerting_event(sim, &stimulus, &AiContext::test_fixture(), None, None);

    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert_eq!(ai.base.panic_center_x, 50.0);
    assert_eq!(ai.base.panic_center_y, 75.0);
}

#[test]
fn repeated_event_panic_preserves_red_alert_until_the_panic_drain() {
    let sim = crate::sim_rng::test_context();
    let mut ai = FriendlyAi::new(1);
    ai.set_state(AiState::Fleeing, Substate::FleeingPanic);
    ai.base.set_alert_status(AlertLevel::Red);
    ai.base.outbox.reentrant.owner_work.clear();

    let panic_center = Position {
        x: 50.0,
        y: 75.0,
        sector: None,
        level: 0,
    };
    ai.think_alerting_event(
        &sim,
        &Stimulus::with_position(StimulusType::EventPanic, panic_center),
        &AiContext::test_fixture(),
        None,
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert_eq!(ai.base.current_music_alert_status, AlertLevel::Red);
    assert_eq!(ai.base.view_alert_status, AlertLevel::Red);
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
    let request = ai
        .base
        .outbox
        .actor
        .begin_panic
        .expect("repeated EVENT_PANIC must still reach the synchronous panic drain");
    assert_eq!(request.center, Some(panic_center));
    assert!(!request.is_new_panic);
}

#[test]
fn think_alerting_event_stop() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let stimulus = Stimulus::new(StimulusType::EventStop);

    ai.think_alerting_event(sim, &stimulus, &AiContext::test_fixture(), None, None);

    assert_eq!(ai.base.current_state, AiState::Seeking);
    assert_eq!(ai.base.current_substate, Substate::SeekingGotStopEvent);
    assert!(ai.base.timer_is_running);
}

#[test]
fn think_alerting_event_stop_while_sleeping() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    ai.set_state(AiState::Sleeping, Substate::SleepingForever);
    let stimulus = Stimulus::new(StimulusType::EventStop);

    let result = ai.think_alerting_event(sim, &stimulus, &AiContext::test_fixture(), None, None);

    // Should return false and NOT change state
    assert!(!result);
    assert_eq!(ai.base.current_state, AiState::Sleeping);
}

#[test]
fn think_unexpected_couldnt_reachpoint_returns_to_duty() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    ai.set_state(AiState::Seeking, Substate::SeekingCivilianRunningToSoldier);

    let stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
    ai.think_unexpected_event(
        sim,
        &stimulus,
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Default);
    // Walks back to post first.
    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);
}

#[test]
fn after_script_queue_rebuilds_retained_view_antagonist() {
    use crate::ai_entity_view::{AiEntityViewMap, EntityKind, shared_entity_views};
    use crate::element::Camp;

    let sim = crate::sim_rng::test_context();
    let mut global = AiGlobalState::default();
    let mut ai = FriendlyAi::new(1);
    let target = 42;
    let target_pos = Position {
        x: 150.0,
        y: 250.0,
        sector: None,
        level: 0,
    };
    let mut target_view = make_soldier_view(target_pos, Camp::Lacklandists, AiState::Attacking);
    target_view.kind = EntityKind::Pc;
    target_view.is_pc = true;
    target_view.is_swordfighting = true;
    let mut views = AiEntityViewMap::new();
    views.insert(target, target_view);
    let ctx = AiContext {
        camp: Camp::Royalists,
        entity_views: shared_entity_views(views),
        // The outer EVENT_AFTER_SCRIPT_GO_ON has no antagonist.
        antagonist: None,
        ..AiContext::test_fixture()
    };

    ai.base
        .stimulus_queue
        .push(Stimulus::with_human(StimulusType::EventView, target));
    ai.think_unexpected_event(
        &sim,
        &Stimulus::new(StimulusType::EventAfterScriptGoOn),
        &mut global,
        &ctx,
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    let request = ai
        .base
        .outbox
        .actor
        .begin_panic
        .expect("retained swordfighter view must launch panic");
    assert_eq!(request.center, Some(target_pos));
}

#[test]
fn think_unexpected_fit_again_returns_to_duty() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    ai.set_state(AiState::Sleeping, Substate::SleepingUnconscious);

    let stimulus = Stimulus::new(StimulusType::EventFitAgain);
    ai.think_unexpected_event(
        sim,
        &stimulus,
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Default);
}

#[test]
fn fit_again_queues_ordered_resurrection_eye_and_state_work() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // EVENT_FITAGAIN must fire the resurrection fan-out and
    // reset the view status to LookForward alongside the
    // return-to-duty hand-off. They share the owner FIFO with
    // the return-to-duty state callback so the engine can preserve the
    // original game's operation order.
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    ai.set_state(AiState::Sleeping, Substate::SleepingUnconscious);
    ai.base.outbox.reentrant.owner_work.clear();

    let stimulus = Stimulus::new(StimulusType::EventFitAgain);
    ai.think_unexpected_event(
        sim,
        &stimulus,
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );

    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [
            crate::ai::AiOwnerWork::InformResurrection,
            crate::ai::AiOwnerWork::SetEyeStatus(crate::element::EyeStatus::LookForward),
            crate::ai::AiOwnerWork::StateChange(_),
        ]
    ));
    assert!(!ai.base.outbox.recovery.inform_resurrection);
    assert_eq!(ai.base.outbox.recovery.set_eye_status, None);
}

#[test]
fn hiding_timer_uses_virtual_return_before_fleeing_event_view_panics() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // EVENT_VIEW while fleeing must fire a *directed* panic
    // away from the spotted human.  An earlier port used
    // `panic_undirected` which lost the center and the civilian
    // picked a random door instead of fleeing opposite the
    // threat.
    use crate::ai_entity_view::{AiEntityView, AiEntityViewMap, EntityKind};
    use crate::element::{Camp, Posture};
    use crate::order::OrderType;
    let mut ai = FriendlyAi::new(1);

    // The original game's AI hiding-timer branch uses a specialized
    // return to duty. The engine drains this owner work through
    // FriendlyAi::return_to_duty, which resets the capped
    // fleeing-view counter before running the ordinary common tail.
    ai.fleeing_seen_enemy_counter = 7;
    ai.set_state(AiState::Fleeing, Substate::FleeingHiding);
    ai.think_expected_event(
        sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut AiGlobalState::default(),
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );
    assert_eq!(ai.fleeing_seen_enemy_counter, 7);
    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingHiding);
    let work = ai
        .base
        .outbox
        .reentrant
        .owner_work
        .pop()
        .expect("hiding timer must invoke virtual ReturnToDuty");
    assert!(matches!(
        work,
        crate::ai::AiOwnerWork::VirtualReturnToDuty { .. }
    ));
    ai.return_to_duty(sim, DutyFlags::empty(), &AiContext::test_fixture());
    assert_eq!(ai.fleeing_seen_enemy_counter, 0);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultGotoPost);

    ai.set_state(AiState::Fleeing, Substate::FleeingRunToDoor);

    let human_handle: u32 = 42;
    let enemy_pos = Position {
        x: 150.0,
        y: 250.0,
        sector: None,
        level: 0,
    };
    let mut views = AiEntityViewMap::new();
    views.insert(
        human_handle,
        AiEntityView {
            original_creation_order: 41,
            position: enemy_pos,
            detection_position: MapPoint::new(enemy_pos.x, enemy_pos.y),
            detection_position_world: crate::coordinates::WorldPoint3D::new(
                enemy_pos.x,
                enemy_pos.y,
                0.0,
            ),
            direction: 0,
            posture: Posture::Upright,
            camp: Camp::Lacklandists, // different from default (Royalists)
            is_pc: false,
            is_robin: false,
            is_vip: false,
            is_beggar: false,
            is_child: false,
            kind: EntityKind::Soldier,
            is_tower_guard: false,
            is_swordfighting: false,
            is_able_to_fight: true,
            active: true,
            is_unconscious: false,
            action_state: crate::element::ActionState::Waiting,
            is_moving_map: false,
            passing_door: false,
            obstacle_idx: None,
            in_building: false,
            building_sector: None,
            ai_state: AiState::Default,
            ai_substate: Substate::DefaultOnPost,
            script_locked: false,
            forecasted_destination: crate::ai::PreparedForecastDestination::fixed(enemy_pos, 0),
            current_animation: OrderType::WalkingUpright,
            elevation: 0.0,
            object_type: crate::element_kinds::ObjectType::None,
            is_dead: false,
            is_carried: false,
            is_archer: false,
            is_rider: false,
            stuck_under_net: false,
            in_coma: false,
            guard: None,
            has_patrol_path: false,
            initial_position: enemy_pos,
            number_of_arrows: 0,
            covering_nets: Vec::new(),
            rank: crate::profiles::ProfileRank::None,
            reported_to_officer: false,
            looted_after_money_fight: false,
            current_money: 0,
            macro_in_progress: false,
            path_current_waypoint_index: 0,
            path_last_waypoint_index: 0,
            path_forward_movement: true,
            patrol_hiking_path_index: None,
            interesting_object: None,
            report_type: crate::ai::ReportType::Nothing,
            report_seek_position: enemy_pos,
            report_seen_bodies: Vec::new(),
            report_charly: None,
        },
    );
    let ctx = AiContext {
        camp: Camp::Royalists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let stimulus = Stimulus::with_human(StimulusType::EventView, human_handle);
    ai.think_alerting_event(sim, &stimulus, &ctx, None, None);

    assert!(
        ai.base.directed_panic,
        "EVENT_VIEW while fleeing must fire a *directed* panic"
    );
    let request = ai
        .base
        .outbox
        .actor
        .begin_panic
        .as_ref()
        .expect("a panic request must be queued");
    let center = request
        .center
        .expect("directed panic must carry a center point");
    assert_eq!(center.x, enemy_pos.x);
    assert_eq!(center.y, enemy_pos.y);
}

#[test]
fn think_unexpected_net_away_panics() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    let stimulus = Stimulus::new(StimulusType::EventNetAway);

    ai.think_unexpected_event(
        sim,
        &stimulus,
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
}

#[test]
fn patrol_coordinate_uses_real_chief_position_for_near_backwards_gate() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
        crate::entity_id::SoldierId(2),
    ));
    ai.set_state(AiState::Default, Substate::DefaultPatrolEnroute);

    let ctx = AiContext {
        position: Position {
            x: 0.0,
            y: 0.0,
            sector: SectorHandle::new(1),
            level: 0,
        },
        direction: 0,
        ..AiContext::test_fixture()
    };
    let tick = FriendlyPerTickData::with_patrol_chief(
        Position {
            x: 100.0,
            y: 0.0,
            ..ctx.position
        },
        AiState::Default,
    );
    let stimulus = Stimulus::with_position(
        StimulusType::CallPatrolCoordinate,
        Position {
            x: -10.0,
            y: 0.0,
            ..ctx.position
        },
    );

    ai.think_unexpected_event(sim, &stimulus, &mut global, &ctx, &tick, None, None);
    let orders = ai.base.take_pending_orders();

    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].order_type, crate::order::OrderType::Turning);
    assert!(
        !ai.base.already_on_point,
        "near-backwards patrol coordinate must turn toward the chief, not walk to the slot"
    );
}

#[test]
#[should_panic(expected = "requires a live patrol-chief snapshot")]
fn patrol_handler_cannot_silently_consume_missing_friendly_tick_data() {
    let sim = crate::sim_rng::test_context();
    let mut global = AiGlobalState::default();
    let mut ai = FriendlyAi::new(1);
    ai.base.patrol_chief = Some(crate::element::EntityId::Soldier(
        crate::entity_id::SoldierId(2),
    ));
    ai.set_state(AiState::Default, Substate::DefaultPatrolEnrouteWaiting);
    ai.think_expected_event(
        &sim,
        &Stimulus::new(StimulusType::EventTimer),
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );
}

#[test]
fn event_sees_body_sets_wondering_state() {
    let mut ai = FriendlyAi::new(1);
    ai.event_sees_body_standard_procedure(42, &AiContext::test_fixture());
    assert_eq!(ai.base.current_state, AiState::Wondering);
    assert_eq!(
        ai.base.current_substate,
        Substate::WonderingCivilianBodyReactiontime,
    );
    assert!(ai.base.outbox.reentrant.owner_work.iter().any(|work| {
        matches!(
            work,
            AiOwnerWork::Speech(AiSpeechAttempt {
                remark: Remark::CivSeesBody,
                flags: 0,
            })
        )
    }));
    assert_eq!(
        ai.base.my_reconnaissance_report.report_type,
        ReportType::Body
    );
}

#[test]
fn think_alerting_event_sees_object_is_noop() {
    let sim = crate::sim_rng::test_context();
    let mut ai = FriendlyAi::new(1);
    let mut stimulus = Stimulus::new(StimulusType::EventSeesObject);
    stimulus.info = StimulusInfo::Object(AiEntityHandle::new(42));
    // Include queued effects, not just the current state/substate.
    let before = bitcode::encode(&ai);

    let result = ai.think_alerting_event(&sim, &stimulus, &AiContext::test_fixture(), None, None);

    assert!(!result);
    assert_eq!(bitcode::encode(&ai), before);
}

#[test]
fn expected_event_body_reactiontime_alert_fails_panics() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    ai.set_state(
        AiState::Wondering,
        Substate::WonderingCivilianBodyReactiontime,
    );

    let stimulus = Stimulus::new(StimulusType::EventTimer);
    ai.think_expected_event(
        sim,
        &stimulus,
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );

    // Soldier alerting fails (stub) → should panic
    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
    assert!(ai.base.outbox.reentrant.owner_work.iter().any(|work| {
        matches!(
            work,
            AiOwnerWork::Speech(AiSpeechAttempt {
                remark: Remark::CivPanic,
                flags: 0,
            })
        )
    }));
}

#[test]
fn expected_event_whistling_child_approaches() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    ai.set_state(AiState::Wondering, Substate::WonderingWatchingWhistling);

    let stimulus = Stimulus::new(StimulusType::EventTimer);
    ai.think_expected_event(
        sim,
        &stimulus,
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Wondering);
    assert_eq!(
        ai.base.current_substate,
        Substate::WonderingChildApproachingWhistling,
    );
    assert!(ai.base.outbox.reentrant.owner_work.iter().any(|work| {
        matches!(
            work,
            AiOwnerWork::Speech(AiSpeechAttempt {
                remark: Remark::CivWhistling,
                flags: 0,
            })
        )
    }));
}

#[test]
fn fleeing_child_chased_end_returns_to_duty() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    ai.set_state(AiState::Fleeing, Substate::FleeingChildChasedEnd);

    let stimulus = Stimulus::new(StimulusType::EventTimer);
    ai.think_expected_event(
        sim,
        &stimulus,
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );

    assert_eq!(ai.base.current_state, AiState::Default);
}

#[test]
fn seeking_report_point_done_transitions() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let mut global = AiGlobalState::default();
    ai.set_state(
        AiState::Seeking,
        Substate::SeekingCivilianGiveAlertingReportToSoldierPoint,
    );

    let stimulus = Stimulus::new(StimulusType::EventDone);
    ai.think_expected_event(
        sim,
        &stimulus,
        &mut global,
        &AiContext::test_fixture(),
        &FriendlyPerTickData::without_patrol_chief(),
        None,
        None,
    );

    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingCivilianGiveAlertingReportToSoldierEnd,
    );
    assert!(ai.base.timer_is_running);
}

// ──────────────────────────────────────────────────────────
// Soldier-alert body-level regression tests
// ──────────────────────────────────────────────────────────

fn make_soldier_view(
    pos: Position,
    camp: crate::element::Camp,
    ai_state: AiState,
) -> crate::ai_entity_view::AiEntityView {
    use crate::ai_entity_view::EntityKind;
    use crate::element::Posture;
    use crate::order::OrderType;
    crate::ai_entity_view::AiEntityView {
        original_creation_order: 41,
        position: pos,
        detection_position: MapPoint::new(pos.x, pos.y),
        detection_position_world: crate::coordinates::WorldPoint3D::new(pos.x, pos.y, 0.0),
        direction: 0,
        posture: Posture::Upright,
        camp,
        is_pc: false,
        is_robin: false,
        is_vip: false,
        is_beggar: false,
        is_child: false,
        kind: EntityKind::Soldier,
        is_tower_guard: false,
        is_swordfighting: false,
        is_able_to_fight: true,
        active: true,
        is_unconscious: false,
        action_state: crate::element::ActionState::Waiting,
        is_moving_map: false,
        passing_door: false,
        obstacle_idx: None,
        in_building: false,
        building_sector: None,
        ai_state,
        ai_substate: Substate::DefaultOnPost,
        script_locked: false,
        forecasted_destination: crate::ai::PreparedForecastDestination::fixed(pos, 0),
        current_animation: OrderType::WalkingUpright,
        elevation: 0.0,
        object_type: crate::element_kinds::ObjectType::None,
        is_dead: false,
        is_carried: false,
        is_archer: false,
        is_rider: false,
        stuck_under_net: false,
        in_coma: false,
        guard: None,
        has_patrol_path: false,
        initial_position: pos,
        number_of_arrows: 0,
        covering_nets: Vec::new(),
        rank: crate::profiles::ProfileRank::None,
        reported_to_officer: false,
        looted_after_money_fight: false,
        current_money: 0,
        macro_in_progress: false,
        path_current_waypoint_index: 0,
        path_last_waypoint_index: 0,
        path_forward_movement: true,
        patrol_hiking_path_index: None,
        interesting_object: None,
        report_type: crate::ai::ReportType::Nothing,
        report_seek_position: pos,
        report_seen_bodies: Vec::new(),
        report_charly: None,
    }
}

#[test]
fn alert_soldier_short_circuits_on_nearby_alerted_friend() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    // Any same-camp soldier in ATTACKING/MENACING/FLEEING
    // within the 360° view radius short-circuits the alert —
    // no point running to a second soldier when one next door
    // is already alerted.
    use crate::ai_entity_view::AiEntityViewMap;
    use crate::element::Camp;
    let mut ai = FriendlyAi::new(1);

    let alerted_pos = Position {
        x: 10.0,
        y: 10.0,
        sector: None,
        level: 0,
    };
    let default_pos = Position {
        x: 500.0,
        y: 500.0,
        sector: None,
        level: 0,
    };

    let mut views = AiEntityViewMap::new();
    views.insert(
        10,
        make_soldier_view(alerted_pos, Camp::Royalists, AiState::Attacking),
    );
    views.insert(
        20,
        make_soldier_view(default_pos, Camp::Royalists, AiState::Default),
    );
    let ctx = AiContext {
        position: Position {
            x: 0.0,
            y: 0.0,
            sector: None,
            level: 0,
        },
        camp: Camp::Royalists,
        // Large enough so the alerted soldier is "detected 360°"
        sq_standard_view_radius: 1_000_000.0,
        sq_self_view_radius: 1_000_000.0,
        all_soldier_handles: std::sync::Arc::new(vec![10, 20]),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    let ok = ai.alert_soldier(
        sim,
        ctx.position,
        0,
        AlertSoldierFailureContinuation::Panic,
        &ctx,
        None,
        None,
    );
    assert!(
        !ok,
        "alert_soldier must return false when alerted friend nearby"
    );
    // State must not have switched to seeking.
    assert_eq!(ai.base.current_state, AiState::Default);
}

#[test]
fn alert_soldier_360_geometry_uses_raw_body_during_door_pass() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{Camp, Posture};

    let planning_position = Position {
        x: 805.0,
        y: 930.0,
        sector: None,
        level: 0,
    };
    let mut target = make_soldier_view(planning_position, Camp::Lacklandists, AiState::Attacking);
    // Door transit has committed Position(target) to the gate endpoint,
    // while world position still exposes this interpolating sprite point.
    target.detection_position_world = WorldPoint3D::new(800.752_6, 1_158.975_2, 177.907_58);
    target.elevation = 177.907_58;
    target.posture = Posture::Upright;

    let ctx = AiContext {
        position: Position {
            x: 900.0,
            y: 900.0,
            ..Position::default()
        },
        self_body_position_world: WorldPoint3D::new(859.0, 1_138.735, 241.734_92),
        ..AiContext::test_fixture()
    };

    let (viewer, detection, _) = alert_soldier_360_geometry(&ctx, &target);
    assert_eq!(viewer, WorldPoint3D::new(859.0, 1_138.735, 286.734_92));
    assert_eq!(
        detection,
        WorldPoint3D::new(800.752_6, 1_158.975_2, 222.907_58)
    );
    assert_ne!(detection.x, planning_position.x);
    assert_ne!(detection.y, planning_position.y + target.elevation + 45.0);
}

#[test]
fn alert_soldier_applies_layer_penalty() {
    // +1000 maximum-norm penalty for soldiers on a different layer.
    // A closer same-layer candidate should win over a nominally-
    // nearer cross-layer one.
    use crate::ai_entity_view::AiEntityViewMap;
    use crate::element::Camp;
    let mut ai = FriendlyAi::new(1);

    let close_cross_layer = Position {
        x: 100.0,
        y: 0.0,
        sector: None,
        level: 1, // different layer → +1000 penalty
    };
    let farther_same_layer = Position {
        x: 300.0,
        y: 0.0,
        sector: None,
        level: 0,
    };

    let mut views = AiEntityViewMap::new();
    views.insert(
        10,
        make_soldier_view(close_cross_layer, Camp::Royalists, AiState::Default),
    );
    views.insert(
        20,
        make_soldier_view(farther_same_layer, Camp::Royalists, AiState::Default),
    );
    let ctx = AiContext {
        position: Position {
            x: 0.0,
            y: 0.0,
            sector: None,
            level: 0,
        },
        camp: Camp::Royalists,
        sq_standard_view_radius: 1.0, // too small for short-circuit
        sq_self_view_radius: 1.0,
        all_soldier_handles: std::sync::Arc::new(vec![10, 20]),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    crate::sim_rng::with_seed(1, |sim| {
        let ok = ai.alert_soldier(
            sim,
            ctx.position,
            0,
            AlertSoldierFailureContinuation::Panic,
            &ctx,
            None,
            None,
        );
        assert!(ok, "alert_soldier must succeed when at least one candidate");
        // Antagonist must be the same-layer one despite being farther.
        assert_eq!(ai.base.antagonist, Some(AiEntityHandle::new(20)));
    });
}

#[test]
fn alert_soldier_ranks_with_stretched_world_max_norm() {
    use crate::ai_entity_view::AiEntityViewMap;
    use crate::element::Camp;

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let owner = Position {
        x: 1680.0,
        y: 2065.0,
        sector: crate::position_interface::SectorHandle::new(0),
        level: 0,
    };
    // Raw projected map distance makes handle 141 look nearer:
    // max(105, 379) < max(414, 213). Original stretches world Y,
    // yielding 660 for handle 141 but only 414 for handle 130.
    let mut views = AiEntityViewMap::new();
    views.insert(
        141,
        make_soldier_view(
            Position {
                x: 1785.0,
                y: 1686.0,
                sector: crate::position_interface::SectorHandle::new(0),
                level: 0,
            },
            Camp::Lacklandists,
            AiState::Default,
        ),
    );
    views.insert(
        130,
        make_soldier_view(
            Position {
                x: 1266.0,
                y: 2278.0,
                sector: crate::position_interface::SectorHandle::new(18),
                level: 0,
            },
            Camp::Lacklandists,
            AiState::Default,
        ),
    );
    let ctx = AiContext {
        position: owner,
        self_body_position_world: crate::coordinates::WorldPoint3D::new(owner.x, owner.y, 0.0),
        camp: Camp::Lacklandists,
        all_soldier_handles: std::sync::Arc::new(vec![130, 141]),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    assert!(ai.alert_soldier(
        sim,
        owner,
        0,
        AlertSoldierFailureContinuation::Panic,
        &ctx,
        None,
        None,
    ));
    assert_eq!(ai.base.antagonist, Some(AiEntityHandle::new(130)));
}

#[test]
fn alert_soldier_ranks_from_raw_body_when_planning_position_is_gate_snapped() {
    use crate::ai_entity_view::AiEntityViewMap;
    use crate::coordinates::WorldPoint3D;
    use crate::element::Camp;

    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    let mut views = AiEntityViewMap::new();

    let mut near_raw = make_soldier_view(
        Position {
            x: 300.0,
            y: 0.0,
            ..Position::default()
        },
        Camp::Lacklandists,
        AiState::Default,
    );
    near_raw.detection_position_world = WorldPoint3D::new(300.0, 0.0, 500.0);
    views.insert(10, near_raw);

    let mut near_gate = make_soldier_view(
        Position {
            x: 900.0,
            y: 0.0,
            ..Position::default()
        },
        Camp::Lacklandists,
        AiState::Default,
    );
    near_gate.detection_position_world = WorldPoint3D::new(900.0, 0.0, 100.0);
    views.insert(20, near_gate);

    let mut near_only_if_z_is_ignored = make_soldier_view(
        Position {
            x: 200.0,
            y: 0.0,
            ..Position::default()
        },
        Camp::Lacklandists,
        AiState::Default,
    );
    near_only_if_z_is_ignored.detection_position_world = WorldPoint3D::new(200.0, 0.0, 1000.0);
    views.insert(30, near_only_if_z_is_ignored);

    let ctx = AiContext {
        // AI Position() has already committed to the far gate endpoint,
        // while world position still reports the interpolating body.
        position: Position {
            x: 1000.0,
            y: 0.0,
            ..Position::default()
        },
        self_body_position_world: WorldPoint3D::new(0.0, 0.0, 100.0),
        camp: Camp::Lacklandists,
        all_soldier_handles: std::sync::Arc::new(vec![10, 20, 30]),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    assert!(ai.alert_soldier(
        sim,
        ctx.position,
        0,
        AlertSoldierFailureContinuation::Panic,
        &ctx,
        None,
        None,
    ));
    // Raw 3D maximum-norm distances are 400, 900, and 900. Reconstructing the
    // owner from the gate-snapped planning position would choose 20;
    // dropping the nonzero Z component while retaining raw X would choose
    // 30. Original's literal raw 3D operation must instead choose 10.
    assert_eq!(ai.base.antagonist, Some(AiEntityHandle::new(10)));
}

#[test]
fn alert_soldier_queues_friend_detectables_on_first_pass() {
    // Every candidate soldier gets a DETECTABLE_FRIEND add on
    // the non-door-path pass so later "is my ally still
    // nearby?" checks light up.
    use crate::ai_entity_view::AiEntityViewMap;
    use crate::element::{Camp, DetectableType};
    let mut ai = FriendlyAi::new(1);

    let mut views = AiEntityViewMap::new();
    views.insert(
        20,
        make_soldier_view(
            Position {
                x: 200.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            Camp::Royalists,
            AiState::Default,
        ),
    );
    views.insert(
        10,
        make_soldier_view(
            Position {
                x: 100.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            Camp::Royalists,
            AiState::Default,
        ),
    );
    let ctx = AiContext {
        position: Position {
            x: 0.0,
            y: 0.0,
            sector: None,
            level: 0,
        },
        camp: Camp::Royalists,
        sq_standard_view_radius: 1.0,
        sq_self_view_radius: 1.0,
        // Deliberately opposite the insertion order above: the Original
        // observes registry order, not HashMap bucket order.
        all_soldier_handles: std::sync::Arc::new(vec![20, 10]),
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    crate::sim_rng::with_seed(1, |sim| {
        ai.alert_soldier(
            sim,
            ctx.position,
            0,
            AlertSoldierFailureContinuation::Panic,
            &ctx,
            None,
            None,
        );
        let notification = ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .find_map(|work| match work {
                AiOwnerWork::StateChange(notification) => Some(notification),
                _ => None,
            })
            .expect("alerting a soldier must enter seeking through friendly state change");
        let effects = notification
            .actor_effects_before_callback
            .as_ref()
            .expect("friend detectables must precede the Friendly state callback");
        let friends: Vec<_> = effects
            .appended_detectables()
            .iter()
            .filter(|(_, t)| *t == DetectableType::Friend)
            .map(|(entity, _)| entity.index())
            .collect();
        assert_eq!(
            friends,
            vec![20, 10],
            "friend detectables must preserve the soldier registry order"
        );
    });
}

// ──────────────────────────────────────────────────────────
// Soldier-alert synchronous route-result continuation
// ──────────────────────────────────────────────────────────

#[test]
fn alert_soldier_first_route_success_delays_then_emits_success_remark() {
    let sim = crate::sim_rng::test_context();
    let mut ai = FriendlyAi::new(1);
    ai.base.outbox.reentrant.alert_soldier_completion_pending = true;
    ai.resume_alert_soldier_after_go_near(
        &sim,
        Position::default(),
        false,
        AlertSoldierFailureContinuation::PanicWithRemark,
        &AiContext::test_fixture(),
        None,
        None,
    );
    assert!(!ai.base.outbox.reentrant.alert_soldier_completion_pending);
    assert!(ai.base.outbox.actor.begin_panic.is_none());
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [AiOwnerWork::Speech(AiSpeechAttempt {
            remark: Remark::CivPanic,
            ..
        })]
    ));
}

#[test]
fn detectable_fifo_stays_inside_existing_state_change_boundaries() {
    use crate::ai::DetectableMutation::{Append, DeleteType};
    use crate::element::DetectableType::Friend;
    let target = crate::element::EntityId::Soldier(crate::entity_id::SoldierId(20));
    let mut ai = FriendlyAi::new(1);
    ai.base.outbox.actor.append_detectable((target, Friend));
    ai.delete_all_friend_detectables();
    assert!(
        ai.base.outbox.reentrant.owner_work.is_empty(),
        "mutation queueing must not introduce owner fixed points"
    );
    ai.set_state(AiState::Default, Substate::DefaultOnPost);
    ai.base.outbox.actor.append_detectable((target, Friend));
    let [AiOwnerWork::StateChange(change)] = ai.base.outbox.reentrant.owner_work.as_slice() else {
        panic!("expected only the preexisting state-change boundary");
    };
    assert_eq!(
        change
            .actor_effects_before_callback
            .as_ref()
            .unwrap()
            .detectable_mutations,
        vec![Append(target, Friend), DeleteType(Friend)]
    );
    assert_eq!(
        ai.base.outbox.actor.detectable_mutations,
        vec![Append(target, Friend)],
        "callback tail must not leak into the prefix"
    );
    let restored: FriendlyAi = serde_json::from_str(&serde_json::to_string(&ai).unwrap()).unwrap();
    assert_eq!(
        robin_util::state_hash::compute(&restored),
        robin_util::state_hash::compute(&ai)
    );
}

#[test]
fn alert_soldier_second_route_failure_runs_each_caller_tail_once() {
    let sim = crate::sim_rng::test_context();
    for failure in [
        AlertSoldierFailureContinuation::PanicWithRemark,
        AlertSoldierFailureContinuation::Panic,
        AlertSoldierFailureContinuation::ReturnToDuty,
    ] {
        let mut ai = FriendlyAi::new(1);
        ai.base.couldnt_reachpoint = true;
        ai.base.outbox.reentrant.alert_soldier_completion_pending = true;
        ai.resume_alert_soldier_after_go_near(
            &sim,
            Position::default(),
            true,
            failure,
            &AiContext::test_fixture(),
            None,
            None,
        );
        assert!(!ai.base.outbox.reentrant.alert_soldier_completion_pending);
        let live_deletes = ai
            .base
            .outbox
            .actor
            .deleted_detectable_types()
            .iter()
            .filter(|&&kind| kind == crate::element::DetectableType::Friend)
            .count();
        let callback_prefix_deletes = ai
            .base
            .outbox
            .reentrant
            .owner_work
            .iter()
            .filter_map(|work| match work {
                AiOwnerWork::StateChange(change) => change.actor_effects_before_callback.as_ref(),
                _ => None,
            })
            .flat_map(|effects| effects.deleted_detectable_types().into_iter())
            .filter(|&kind| kind == crate::element::DetectableType::Friend)
            .count();
        assert_eq!(live_deletes + callback_prefix_deletes, 1);
        match failure {
            AlertSoldierFailureContinuation::PanicWithRemark => {
                assert!(ai.base.outbox.actor.begin_panic.is_some());
                assert!(matches!(
                    ai.base.outbox.reentrant.owner_work.first(),
                    Some(AiOwnerWork::Speech(AiSpeechAttempt {
                        remark: Remark::CivPanic,
                        ..
                    }))
                ));
            }
            AlertSoldierFailureContinuation::Panic => {
                assert!(ai.base.outbox.actor.begin_panic.is_some());
                assert!(
                    !ai.base
                        .outbox
                        .reentrant
                        .owner_work
                        .iter()
                        .any(|work| matches!(work, AiOwnerWork::Speech(_)))
                );
            }
            AlertSoldierFailureContinuation::ReturnToDuty => {
                assert!(ai.base.outbox.actor.begin_panic.is_none());
                assert_eq!(ai.base.current_state, AiState::Default);
                assert_ne!(
                    ai.base.current_substate,
                    Substate::SeekingCivilianRunningToSoldier
                );
            }
        }
    }
}

#[test]
fn alert_soldier_first_failure_then_retry_success_avoids_failure_tail() {
    use crate::ai_entity_view::{AiEntityViewMap, shared_entity_views};
    use crate::element::Camp;
    let sim = crate::sim_rng::test_context();
    let mut views = AiEntityViewMap::new();
    views.insert(
        20,
        make_soldier_view(
            Position {
                x: 100.0,
                y: 0.0,
                ..Position::default()
            },
            Camp::Royalists,
            AiState::Default,
        ),
    );
    let ctx = AiContext {
        camp: Camp::Royalists,
        all_soldier_handles: std::sync::Arc::new(vec![20]),
        entity_views: shared_entity_views(views),
        ..AiContext::test_fixture()
    };
    let mut ai = FriendlyAi::new(1);
    ai.base.couldnt_reachpoint = true;
    ai.base.outbox.reentrant.alert_soldier_completion_pending = true;
    ai.resume_alert_soldier_after_go_near(
        &sim,
        Position::default(),
        false,
        AlertSoldierFailureContinuation::Panic,
        &ctx,
        None,
        None,
    );
    assert!(ai.base.outbox.reentrant.alert_soldier_completion_pending);
    assert!(ai.base.outbox.actor.begin_panic.is_none());
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.last(),
        Some(AiOwnerWork::ResumeFriendlyAlertSoldierAfterGoNear {
            check_door_path: true,
            ..
        })
    ));
    ai.base.outbox.reentrant.owner_work.clear();
    ai.resume_alert_soldier_after_go_near(
        &sim,
        Position::default(),
        true,
        AlertSoldierFailureContinuation::Panic,
        &ctx,
        None,
        None,
    );
    assert!(!ai.base.outbox.reentrant.alert_soldier_completion_pending);
    assert!(ai.base.outbox.actor.begin_panic.is_none());
    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [AiOwnerWork::Speech(AiSpeechAttempt {
            remark: Remark::CivPanic,
            ..
        })]
    ));
}

// ──────────────────────────────────────────────────────────
// Apple-chase flee: full scan, not a single-guess stub
// ──────────────────────────────────────────────────────────

#[test]
fn propose_apple_chase_flee_returns_candidate_without_grid() {
    // Without a grid the `is_straight_movement_authorized`
    // check is skipped and the first-distance / zero-relative
    // candidate wins.  Verifies the happy path still produces a
    // flee destination.
    use crate::ai_entity_view::AiEntityViewMap;
    use crate::element::Camp;
    let mut ai = FriendlyAi::new(1);
    ai.base.antagonist = Some(AiEntityHandle::new(42));

    let mut views = AiEntityViewMap::new();
    views.insert(
        42,
        make_soldier_view(
            Position {
                x: 100.0,
                y: 0.0,
                sector: None,
                level: 0,
            },
            Camp::Royalists,
            AiState::Wondering,
        ),
    );
    let ctx = AiContext {
        position: Position {
            x: 0.0,
            y: 0.0,
            sector: None,
            level: 0,
        },
        camp: Camp::Royalists,
        entity_views: crate::ai_entity_view::shared_entity_views(views),
        ..AiContext::test_fixture()
    };

    crate::sim_rng::with_seed(1, |sim| {
        let dest = ai.propose_good_apple_chase_flee_destination(sim, &ctx, None);
        assert!(dest.is_some());
        // Flee vector should point away from antagonist at x=100
        // → destination x should be negative.
        let d = dest.unwrap();
        assert!(
            d.x < ctx.position.x,
            "flee vector must run away from antagonist"
        );
    });
}
