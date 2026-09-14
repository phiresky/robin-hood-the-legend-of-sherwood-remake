//! Live battle pursuit, panic, and help fallbacks.
use super::battle_decision_observation_tests::fixture;
use super::*;
use crate::ai::{AiEntityHandle, AiState, Decision, LogLineType, Substate};
use crate::coordinates::WorldPoint3D;

#[test]
fn missed_pc_search_reads_current_forecast_instead_of_old_seek_position() {
    let (mut engine, assets, owner, target) = fixture(false);
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(750.0, 200.0, 0.0));
    let forecast = engine.live_ai_position(target);
    let ai = engine
        .get_entity_mut(owner)
        .unwrap()
        .enemy_ai_mut()
        .unwrap();
    ai.list_them.clear();
    ai.pc_missed = true;
    ai.missed_pc = Some(AiEntityHandle::new(target.index()));
    ai.base.seek_position.x = 111.0;
    engine.execute_live_battle_without_visible_enemies(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        Vec::new(),
    );
    let ai = engine.get_entity(owner).unwrap().enemy_ai().unwrap();
    assert_eq!(ai.seek_center, forecast);
    assert!(ai.seek_flags.contains(crate::ai_enemy::SeekFlags::HOUSE));
}

#[test]
#[should_panic]
fn missed_pc_search_requires_the_current_target_entity() {
    let (mut engine, assets, owner, _) = fixture(false);
    let ai = engine
        .get_entity_mut(owner)
        .unwrap()
        .enemy_ai_mut()
        .unwrap();
    ai.list_them.clear();
    ai.pc_missed = true;
    ai.missed_pc = Some(AiEntityHandle::new(u32::MAX));
    engine.execute_live_battle_without_visible_enemies(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        Vec::new(),
    );
}

#[test]
fn directed_cassos_reads_the_live_target_and_completes_panic() {
    let (mut engine, assets, owner, target) = fixture(false);
    engine
        .get_entity_mut(target)
        .unwrap()
        .element_data_mut()
        .set_position(WorldPoint3D::new(725.0, 175.0, 0.0));
    let position = engine.live_ai_position(target);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .base
        .seek_position
        .x = 321.0;
    let result = engine.execute_live_battle_decision(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        Decision::Cassos,
        Substate::AttackingReactiontimeRunning,
        0,
        false,
    );
    assert_eq!(result, Some(Decision::Cassos));
    let ai = &engine.get_entity(owner).unwrap().enemy_ai().unwrap().base;
    assert_eq!(
        (ai.panic_center_x, ai.panic_center_y),
        (position.x, position.y)
    );
    assert!(ai.directed_panic);
    assert_eq!(ai.current_state, AiState::Fleeing);
    assert!(ai.outbox.actor.begin_panic.is_none());
}

#[test]
fn repeated_cassos_without_a_target_remains_undirected() {
    let (mut engine, assets, owner, _) = fixture(false);
    let ai = engine
        .get_entity_mut(owner)
        .unwrap()
        .enemy_ai_mut()
        .unwrap();
    ai.list_them.clear();
    ai.base.primary_target = None;
    ai.base.current_state = AiState::Fleeing;
    ai.base.current_substate = Substate::FleeingPanic;
    ai.base.lasting_panic_runs = 11;
    let result = engine.execute_live_battle_decision(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        Decision::Cassos,
        Substate::FleeingPanic,
        0,
        false,
    );
    assert_eq!(result, Some(Decision::Cassos));
    let ai = &engine.get_entity(owner).unwrap().enemy_ai().unwrap().base;
    assert!(!ai.directed_panic);
    assert_eq!(ai.lasting_panic_runs, 11);
    assert!(ai.outbox.actor.begin_panic.is_none());
}

#[test]
#[should_panic]
fn cassos_rejects_a_stale_persistent_target_instead_of_using_seek_position() {
    let (mut engine, assets, owner, _) = fixture(false);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .list_them = vec![u32::MAX];
    engine.execute_live_battle_decision(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        Decision::Cassos,
        Substate::AttackingReactiontimeRunning,
        0,
        false,
    );
}

#[test]
fn help_decision_finishes_real_officer_route_before_fallback_and_logs_once() {
    for disconnected in [false, true] {
        let (mut engine, mut assets, owner, target) = fixture(disconnected);
        let officer = engine.add_test_entity(
            crate::engine::test_support::actors::make_test_ai_soldier(Camp::Lacklandists),
        );
        let sector = engine.get_entity(target).unwrap().element_data().sector();
        let entity = engine.get_entity_mut(officer).unwrap();
        entity.element_data_mut().set_sector(sector);
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(400.0, 100.0, 0.0));
        entity.actor_data_mut().unwrap().action_state = crate::element::ActionState::Waiting;
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        let officer_ai = engine
            .get_entity_mut(officer)
            .unwrap()
            .enemy_ai_mut()
            .unwrap();
        officer_ai.soldier_profile_rank = crate::profiles::ProfileRank::Officer;
        officer_ai.base.current_state = AiState::Default;
        officer_ai.base.current_substate = Substate::DefaultOnPost;
        let ai = engine
            .get_entity_mut(owner)
            .unwrap()
            .enemy_ai_mut()
            .unwrap();
        ai.soldier_profile_rank = crate::profiles::ProfileRank::Soldier;
        ai.forced_next_battle_decision = Decision::LookForHelp;
        let (_, draws) = crate::sim_rng::with_draw_trace(|| {
            engine.execute_battle_decisions(&crate::sim_rng::test_context(), &assets, owner);
        });
        assert_eq!(
            draws
                .iter()
                .filter(|&&site| site == crate::sim_rng::RngSite::BattlePanicRemark)
                .count(),
            1
        );
        let ai = &engine.get_entity(owner).unwrap().enemy_ai().unwrap().base;
        assert!(!ai.couldnt_reachpoint);
        assert_eq!(
            ai.current_state,
            if disconnected {
                AiState::Fleeing
            } else {
                AiState::Seeking
            }
        );
        let logs: Vec<_> = ai
            .ai_log
            .iter()
            .filter(|line| line.line_type == LogLineType::BattleDecision)
            .collect();
        assert_eq!(logs.len(), 1);
        assert_eq!(
            logs[0].info,
            if disconnected {
                Decision::Cassos
            } else {
                Decision::LookForHelp
            } as u16
        );
    }
}

#[test]
fn archer_step_back_without_target_completes_shoot_to_observation_fallback() {
    let (mut engine, assets, owner, _) = fixture(false);
    engine
        .get_entity_mut(owner)
        .unwrap()
        .enemy_ai_mut()
        .unwrap()
        .list_them
        .clear();
    engine
        .get_entity_mut(owner)
        .unwrap()
        .ai_actor_data_mut()
        .unwrap()
        .number_of_arrows = 1;
    let result = engine.execute_live_battle_decision(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        Decision::ArcherStepBack,
        Substate::AttackingReactiontimeRunning,
        0,
        false,
    );
    assert_eq!(result, Some(Decision::ArcherObserve));
    let ai = &engine.get_entity(owner).unwrap().enemy_ai().unwrap().base;
    assert_eq!(ai.primary_target, None);
    assert_eq!(ai.current_substate, Substate::AttackingBowObservingLoading);
}
