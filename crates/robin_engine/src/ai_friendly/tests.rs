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
    ai.base.substate_at_last_timer_launch = ai.base.current_substate;
    ai.base.timer_is_running = false;
    ai.base.outbox = Default::default();
    let Entity::Civilian(civilian) = &mut entity else {
        unreachable!()
    };
    civilian.npc.ai_brain = AiBrain::Friendly(Box::new(ai));
    let owner = engine.add_test_entity(entity);
    let mut assets = crate::engine::LevelAssets::new();
    std::sync::Arc::make_mut(&mut assets.profile_manager)
        .civilians
        .push(Default::default());
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
    engine
        .get_entity_mut(owner)
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .eye_status = crate::element::EyeStatus::Stare;
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
    assert_eq!(
        engine
            .get_entity(owner)
            .unwrap()
            .npc_data()
            .unwrap()
            .eye_status,
        crate::element::EyeStatus::LookForward,
    );
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
