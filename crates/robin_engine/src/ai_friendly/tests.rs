use super::*;

fn duty_fixture(
    mut ai: FriendlyAi,
) -> (
    crate::engine::EngineInner,
    crate::engine::LevelAssets,
    crate::element::EntityId,
) {
    use crate::element::{AiBrain, Entity, Posture};
    let mut engine = crate::engine::EngineInner::new();
    let mut entity = crate::engine::test_support::actors::make_test_civilian(Posture::Leisure);
    // A civilian already resting at its post needs no navigation fixture.
    ai.base.special_action = true;
    ai.base.outbox = Default::default();
    let Entity::Civilian(civilian) = &mut entity else {
        unreachable!()
    };
    civilian.npc.ai_brain = AiBrain::Friendly(Box::new(ai));
    let owner = engine.add_test_entity(entity);
    let mut assets = crate::engine::LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    (engine, assets, owner)
}

fn friendly(engine: &crate::engine::EngineInner, owner: crate::element::EntityId) -> &FriendlyAi {
    engine.get_entity(owner).unwrap().friendly_ai().unwrap()
}

impl FriendlyAi {
    /// Raw-coordinate panic entry point (tests only).  Production
    /// code uses [`Self::panic_from_point_at`] so the panic
    /// center carries a valid sector/level for the multi-level
    /// door lookup.
    fn panic_from_point(&mut self, center_x: f32, center_y: f32, runs: u8) {
        self.panic_from_point_at(
            Position {
                x: center_x,
                y: center_y,
                sector: None,
                level: 0,
            },
            runs,
        );
    }
}

#[test]
fn friendly_ai_defaults() {
    let ai = FriendlyAi::new(99);
    assert_eq!(ai.base.me, 99);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.beggar_dont_talk_counter, 0);
}

#[test]
fn civilian_start_think_distinguishes_static_and_ailock_freeze() {
    let ctx = AiAdmission {
        frame: 0,
        original_creation_order: 0,
        think_depth: 0,
        in_building: false,
        self_is_rider: false,
        self_is_dead: false,
        self_is_unconscious: false,
        posture: crate::element::Posture::Upright,
        position: Position::default(),
    };
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
fn civilian_return_to_duty() {
    let sim = crate::sim_rng::test_context();
    let mut ai = FriendlyAi::new(1);
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingPanic;
    ai.fleeing_seen_enemy_counter = 5;
    let (mut engine, assets, owner) = duty_fixture(ai);
    engine.execute_ai_return_to_duty(&sim, &assets, owner, DutyFlags::empty());
    let ai = friendly(&engine, owner);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
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
        None,
        None,
    )
    .unwrap();
}

#[test]
fn think_expected_admiring_hero_returns_to_duty() {
    let sim = crate::sim_rng::test_context();
    let mut ai = FriendlyAi::new(1);
    ai.base.current_state = AiState::Wondering;
    ai.base.current_substate = Substate::WonderingCivilianAdmiringHero;
    ai.fleeing_seen_enemy_counter = 5;
    let (mut engine, assets, owner) = duty_fixture(ai);
    engine.execute_ai_callback(
        &sim,
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventTimer),
    );
    let ai = friendly(&engine, owner);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
    assert_eq!(ai.fleeing_seen_enemy_counter, 0);
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
    let sim = crate::sim_rng::test_context();
    let mut ai = FriendlyAi::new(1);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingCivilianRunningToSoldier;
    ai.fleeing_seen_enemy_counter = 5;
    let (mut engine, assets, owner) = duty_fixture(ai);
    engine.execute_ai_callback(
        &sim,
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventCouldntReachPoint),
    );
    let ai = friendly(&engine, owner);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
    assert_eq!(ai.fleeing_seen_enemy_counter, 0);
}

#[test]
fn after_script_queue_rebuilds_retained_view_antagonist() {
    use crate::element::Camp;
    let sim = crate::sim_rng::test_context();
    let (mut engine, mut assets, owner) = duty_fixture(FriendlyAi::new(1));
    let mut target = crate::engine::test_support::actors::make_test_ai_soldier(Camp::Lacklandists);
    target
        .element_data_mut()
        .set_position_map(MapPoint::new(150.0, 250.0));
    target.human_data_mut().unwrap().opponents.push(owner);
    let target = engine.add_test_entity(target);
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_controller_mut()
        .unwrap()
        .stimulus_queue
        .push(Stimulus::with_human(
            StimulusType::EventView,
            target.index(),
        ));
    engine.execute_ai_callback(
        &sim,
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventAfterScriptGoOn),
    );
    let ai = friendly(&engine, owner);
    assert!(ai.base.stimulus_queue.is_empty());
    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert!(
        !ai.base.directed_panic,
        "without an away-door, panic retries without a directional restriction"
    );
    assert_eq!(ai.base.panic_center_x, 150.0);
    assert_eq!(ai.base.panic_center_y, 250.0);
}

#[test]
fn think_unexpected_fit_again_returns_to_duty() {
    let sim = crate::sim_rng::test_context();
    let mut ai = FriendlyAi::new(1);
    ai.base.current_state = AiState::Sleeping;
    ai.base.current_substate = Substate::SleepingUnconscious;
    ai.fleeing_seen_enemy_counter = 5;
    let (mut engine, assets, owner) = duty_fixture(ai);
    engine.execute_ai_callback(
        &sim,
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventFitAgain),
    );
    let ai = friendly(&engine, owner);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
    assert_eq!(ai.fleeing_seen_enemy_counter, 0);
}

#[test]
fn fit_again_returns_duty_after_ordered_resurrection_and_eye_prefix() {
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
    let duty = ai
        .think_unexpected_event(
            sim,
            &stimulus,
            &mut global,
            &AiContext::test_fixture(),
            None,
            None,
        )
        .expect_err("recovery hands duty to the engine after its actor prefix");
    assert!(!duty.think_result);
    assert!(duty.flags.is_empty());

    assert!(matches!(
        ai.base.outbox.reentrant.owner_work.as_slice(),
        [
            crate::ai::AiOwnerWork::InformResurrection,
            crate::ai::AiOwnerWork::SetEyeStatus(crate::element::EyeStatus::LookForward),
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

    ai.fleeing_seen_enemy_counter = 7;
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingHiding;
    let (mut engine, assets, owner) = duty_fixture(ai);
    engine.execute_ai_callback(
        sim,
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventTimer),
    );
    let mut ai = friendly(&engine, owner).clone();
    assert_eq!(ai.fleeing_seen_enemy_counter, 0);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);

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
        None,
        None,
    )
    .unwrap();

    assert_eq!(ai.base.current_state, AiState::Fleeing);
    assert_eq!(ai.base.current_substate, Substate::FleeingPanic);
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
fn expected_event_whistling_child_approaches() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut ai = FriendlyAi::new(1);
    ai.set_state(AiState::Wondering, Substate::WonderingWatchingWhistling);

    let stimulus = Stimulus::new(StimulusType::EventTimer);
    ai.think_expected_event(sim, &stimulus, &AiContext::test_fixture(), None, None)
        .unwrap();

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
    let sim = crate::sim_rng::test_context();
    let mut ai = FriendlyAi::new(1);
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingChildChasedEnd;
    ai.fleeing_seen_enemy_counter = 5;
    let (mut engine, assets, owner) = duty_fixture(ai);
    engine.execute_ai_callback(
        &sim,
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventTimer),
    );
    let ai = friendly(&engine, owner);
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
    assert_eq!(ai.fleeing_seen_enemy_counter, 0);
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
    }
}

// ──────────────────────────────────────────────────────────
// Soldier-alert synchronous route-result continuation
// ──────────────────────────────────────────────────────────

#[test]
fn detectable_fifo_stays_inside_existing_state_change_boundaries() {
    use crate::ai::DetectableMutation::{Append, DeleteType};
    use crate::element::DetectableType::Friend;
    let target = crate::element::EntityId::Soldier(crate::entity_id::SoldierId(20));
    let mut ai = FriendlyAi::new(1);
    ai.base.outbox.actor.append_detectable((target, Friend));
    ai.base.outbox.actor.delete_detectable_type(Friend);
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
