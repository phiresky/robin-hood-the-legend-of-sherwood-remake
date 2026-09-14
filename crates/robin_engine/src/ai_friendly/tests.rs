use super::*;
use crate::coordinates::MapPoint;

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
    assert_eq!(ai.base.current_state, AiState::Default);
    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
    assert_eq!(ai.base.panic_center_x, 100.0);
    assert_eq!(ai.base.panic_center_y, 200.0);
    assert_eq!(ai.base.lasting_panic_runs, 0);
    let request = ai.base.outbox.actor.begin_panic.unwrap();
    assert_eq!(request.runs, 8);
    assert!(request.is_new_panic);
    assert!(ai.base.outbox.reentrant.owner_work.is_empty());
}

#[test]
fn civilian_panic_undirected_preserves_runs_until_live_execution() {
    let mut ai = FriendlyAi::new(1);
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingPanic;
    ai.base.lasting_panic_runs = 11;
    ai.panic_undirected(4);
    assert_eq!(ai.base.lasting_panic_runs, 11);
    assert!(!ai.base.directed_panic);
    let request = ai.base.outbox.actor.begin_panic.unwrap();
    assert_eq!(request.runs, 4);
    assert!(!request.is_new_panic);
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
    ai.base.current_state = AiState::Sleeping;
    ai.base.current_substate = Substate::SleepingUnconscious;
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

// ──────────────────────────────────────────────────────────
// Apple-chase flee: full scan, not a single-guess stub
// ──────────────────────────────────────────────────────────
