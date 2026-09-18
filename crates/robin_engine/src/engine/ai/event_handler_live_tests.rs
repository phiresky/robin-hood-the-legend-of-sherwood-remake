use super::battle_decision_observation_tests::fixture;
use crate::ai::{AiEntityHandle, AiState, Position, SeekPoint, Stimulus, StimulusType, Substate};
use crate::engine::TickCtx;

#[test]
fn failed_combat_routes_execute_overview_without_rebuilding_friends() {
    for substate in [
        Substate::AttackingRunningToEnemy,
        Substate::AttackingRunningToLadder,
        Substate::AttackingRunToAvengerOnRoof,
    ] {
        for decision_frame in [99, 100] {
            let (mut engine, assets, owner, target) = fixture(false);
            let friends = vec![owner.index(), target.index()];
            let ai = engine.enemy_mut(owner);
            ai.base.current_substate = substate;
            ai.base.list_us = friends.clone();
            ai.base.ai_log.push(crate::ai::LogLine {
                line_type: crate::ai::LogLineType::BattleDecision,
                info: crate::ai::Decision::Fight as u16,
                frame: decision_frame,
            });
            let stimulus = Stimulus::new(StimulusType::EventCouldntReachPoint);
            engine.execute_ai_handler_body(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                owner,
                &stimulus,
            );
            let ai = engine.enemy(owner);
            assert_eq!(
                ai.base.current_substate,
                Substate::AttackingOverviewLookLeft
            );
            assert_eq!(ai.base.list_us, friends);
        }
    }
}

#[test]
fn failed_body_route_examines_queued_body_before_searching() {
    let (mut engine, assets, owner, body) = fixture(false);
    engine.human_mut(body).unconscious = true;
    let body_position = engine.live_ai_position(body);
    let ai = engine.enemy_mut(owner);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBody;
    ai.other_bodies_to_examine.push(body.index());
    engine.execute_ai_handler_body(
        TickCtx::new(&crate::sim_rng::test_context(), &assets),
        owner,
        &Stimulus::new(StimulusType::EventCouldntReachPoint),
    );
    let ai = engine.enemy(owner);
    assert_eq!(
        ai.base.detected_body,
        Some(AiEntityHandle::new(body.index()))
    );
    assert_eq!(ai.base.seek_position, body_position);
    assert_eq!(ai.base.current_substate, Substate::SeekingBody);
    assert!(ai.my_seek_points.is_empty());
}

#[test]
fn body_route_failure_and_lost_roof_target_search_from_live_owner() {
    for substate in [
        Substate::SeekingBody,
        Substate::AttackingWaitForAvengerOnRoof,
    ] {
        let (mut engine, assets, owner, target) = fixture(false);
        let live_position = engine.live_ai_position(owner);
        let near_point = Position {
            x: 250.0,
            ..live_position
        };
        let stale_position = Position {
            x: 8000.0,
            ..live_position
        };
        engine.ai.global.seek_points = [near_point, stale_position]
            .into_iter()
            .enumerate()
            .map(|(id, position)| SeekPoint {
                position,
                frame_when_full_interest: 0,
                directions: vec![0],
                last_calculated_interest: 100,
                locked: false,
                id: id as u16,
            })
            .collect();
        let ai = engine.enemy_mut(owner);
        ai.base.current_state = if substate == Substate::SeekingBody {
            AiState::Seeking
        } else {
            AiState::Attacking
        };
        ai.base.current_substate = substate;
        ai.base.primary_target = None;
        ai.list_them.clear();
        ai.base.seek_position = stale_position;
        let stimulus = if substate == Substate::SeekingBody {
            Stimulus::new(StimulusType::EventCouldntReachPoint)
        } else {
            Stimulus::with_human(StimulusType::EventOutOfView, target.index())
        };
        engine.execute_ai_handler_body(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            owner,
            &stimulus,
        );
        let ai = engine.enemy(owner);
        assert_eq!(ai.seek_center, live_position);
        assert_eq!(ai.actual_seek_point, Some(0));
        assert_eq!(ai.base.last_goto_destination, near_point);
        assert_ne!(ai.base.last_goto_destination, stale_position);
    }
}
