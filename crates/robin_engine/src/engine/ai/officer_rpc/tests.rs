use super::*;

#[test]
fn rebuilt_live_enemies_preserve_detectable_order_duplicates_and_liveness() {
    use crate::element::{Detectable, DetectableType};
    let (mut engine, mut assets, owner, target) = fixture();
    let dead = engine.add_test_entity(make_test_ai_soldier(Camp::Lacklandists));
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine
        .world
        .entities
        .get_mut(dead)
        .unwrap()
        .npc_data_mut()
        .unwrap()
        .life_points = 0;
    let retained = vec![
        Detectable {
            element: Some(target),
            seen_now: true,
            ..Detectable::default()
        },
        Detectable {
            element: Some(owner),
            seen_now: false,
            ..Detectable::default()
        },
        Detectable {
            element: Some(dead),
            seen_now: true,
            ..Detectable::default()
        },
        Detectable {
            element: None,
            seen_now: true,
            ..Detectable::default()
        },
        Detectable {
            element: Some(owner),
            seen_now: true,
            ..Detectable::default()
        },
        Detectable {
            element: Some(target),
            seen_now: true,
            ..Detectable::default()
        },
    ];
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .ai_actor_data_mut()
        .unwrap()
        .detectable_lists[DetectableType::Enemy as usize] = retained.clone();
    let ai = engine.observation_ai_mut(owner);
    ai.list_them = vec![dead.index()];
    ai.base.primary_target = Some(AiEntityHandle::new(dead.index()));
    engine.reinitialize_live_ai_enemies(owner);
    let ai = engine.observation_ai(owner);
    assert_eq!(
        ai.list_them,
        [target.index(), owner.index(), target.index()]
    );
    assert_eq!(
        ai.base.primary_target,
        Some(AiEntityHandle::new(dead.index()))
    );
    let actual = &engine
        .world
        .entities
        .get(owner)
        .unwrap()
        .ai_actor_data()
        .unwrap()
        .detectable_lists[DetectableType::Enemy as usize];
    assert_eq!(
        actual
            .iter()
            .map(|d| (d.element, d.seen_now))
            .collect::<Vec<_>>(),
        retained
            .iter()
            .map(|d| (d.element, d.seen_now))
            .collect::<Vec<_>>()
    );
}

#[test]
fn rebuilt_empty_live_enemies_do_not_preserve_unseen_primary_target_in_list() {
    let (mut engine, _, owner, target) = fixture();
    let ai = engine.observation_ai_mut(owner);
    ai.base.primary_target = Some(AiEntityHandle::new(target.index()));
    ai.list_them = vec![target.index(), owner.index()];
    engine.reinitialize_live_ai_enemies(owner);
    let ai = engine.observation_ai(owner);
    assert!(ai.list_them.is_empty());
    assert_eq!(
        ai.base.primary_target,
        Some(AiEntityHandle::new(target.index()))
    );
}

#[test]
fn tower_alert_finishes_recipient_callback_and_battle_decision_inline() {
    let (mut engine, assets, owner, recipient) = fixture();
    let center = Position {
        x: 120.0,
        y: 80.0,
        ..engine.live_ai_position(owner)
    };
    let ai = engine.observation_ai_mut(owner);
    ai.tower_guard = true;
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingTowerGuardAlert;
    ai.base.seek_position = center;
    ai.list_them.clear();
    engine.observation_ai_mut(recipient).soldier_profile_rank = ProfileRank::Knight;
    engine.execute_ai_callback(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventDone),
    );
    let recipient = engine.observation_ai(recipient);
    assert_eq!(
        recipient.base.current_substate,
        Substate::SeekingKnightWatchingTowerGuard
    );
    assert_eq!(recipient.base.seek_position, center);
    assert_eq!(recipient.base.when_does_timer_ring, 200);
    assert_eq!(
        engine.observation_ai(owner).base.current_substate,
        Substate::DefaultGotoPost
    );
}

#[test]
fn officer_half_plane_detection_reads_officers_live_facing() {
    let (mut engine, _, officer, target) = fixture();
    engine.observation_ai_mut(officer).soldier_profile_rank = ProfileRank::Officer;
    let origin = engine.live_ai_position(officer);
    let target = engine.live_ai_position(target);
    for (direction, seen) in [(4, true), (12, false)] {
        engine
            .world
            .entities
            .get_mut(officer)
            .unwrap()
            .element_data_mut()
            .set_direction_instantly(direction);
        let direction = engine
            .world
            .entities
            .get(officer)
            .unwrap()
            .element_data()
            .direction();
        assert_eq!(
            crate::ai_enemy::detects_position_180_raw(
                origin,
                direction as u16,
                target,
                350.0 * 350.0
            ),
            seen
        );
    }
}

#[test]
fn checkpoint_search_preserves_exact_sector_and_cursor_before_pivot_skip() {
    for attached in [false, true] {
        let (mut engine, mut assets, owner, charly) = fixture();
        let sector = engine.live_ai_position(owner).sector.unwrap();
        assets.navigation.hiking_paths =
            std::sync::Arc::new(vec![crate::level_data::RawHikingPath {
                waypoints: [(490, 500), (600, 500), (900, 500)]
                    .into_iter()
                    .map(|(x, y)| crate::level_data::RawWaypoint {
                        x,
                        y,
                        sector: 1,
                        level: 0,
                        command: crate::level_data::WaypointCommand::None,
                    })
                    .collect(),
            }]);
        assets.navigation.hiking_waypoint_sectors =
            Some(std::sync::Arc::new(vec![vec![sector; 3]]));
        let ai = engine.observation_ai_mut(charly);
        ai.base.has_patrol_path = true;
        if attached {
            ai.base.patrol_path = crate::ai::PatrolPath::new(
                crate::ai::PathId::new(0).unwrap(),
                &assets.navigation.hiking_paths,
            );
            ai.base.patrol_path.as_mut().unwrap().current_waypoint_index = 2;
        } else {
            ai.base.detached_patrol_path_status.hiking_path_index = crate::ai::PathId::new(0);
            ai.base.detached_patrol_path_status.current_waypoint_index = 2;
        }
        let ai = engine.observation_ai_mut(owner);
        ai.soldier_profile_rank = ProfileRank::Soldier;
        ai.base.checkpoint_charly = Some(AiEntityHandle::new(charly.index()));
        ai.base.macro_in_progress = true;
        engine.execute_ai_search_charly(&crate::sim_rng::test_context(), &assets, owner);
        let ai = engine.observation_ai(owner);
        assert!(!ai.base.macro_in_progress);
        assert_eq!(
            ai.search_charly_way.iter().map(|p| p.x).collect::<Vec<_>>(),
            [600.0, 900.0, 490.0]
        );
        assert!(
            ai.search_charly_way
                .iter()
                .all(|p| p.sector == Some(sector))
        );
        assert_eq!(ai.base.last_goto_destination, ai.search_charly_way[0]);
        let checkpoint = &engine.observation_ai(charly).base;
        let (current, last) = checkpoint.patrol_path.as_ref().map_or(
            (
                checkpoint
                    .detached_patrol_path_status
                    .current_waypoint_index,
                checkpoint.detached_patrol_path_status.last_waypoint_index,
            ),
            |path| (path.current_waypoint_index, path.last_waypoint_index),
        );
        assert_eq!((current, last), (0, 2));
    }
}

#[test]
fn officer_missing_checkpoint_reports_and_alerts_without_building_search_route() {
    let (mut engine, assets, owner, charly) = fixture();
    let ai = engine.observation_ai_mut(owner);
    ai.soldier_profile_rank = ProfileRank::Officer;
    ai.base.checkpoint_charly = Some(AiEntityHandle::new(charly.index()));
    ai.base.macro_in_progress = true;
    ai.base.macro_command_offset = 23;
    engine.observation_ai_mut(charly).reported_to_officer = true;
    engine.execute_ai_search_charly(&crate::sim_rng::test_context(), &assets, owner);
    let ai = engine.observation_ai(owner);
    assert!(ai.search_charly_way.is_empty());
    assert_eq!(
        ai.base.my_reconnaissance_report.report_type,
        crate::ai::ReportType::MissedCharly
    );
    assert_eq!(
        ai.base.my_reconnaissance_report.charly,
        Some(AiEntityHandle::new(charly.index()))
    );
    assert_eq!(ai.base.frame_when_enemy_detected, 100);
    assert!(!engine.observation_ai(charly).reported_to_officer);
}

use crate::element::{ActionState, Camp};
use crate::engine::test_support::{actors::make_test_ai_soldier, square_sector};

fn fixture() -> (EngineInner, LevelAssets, EntityId, EntityId) {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(128, 128);
    engine.world.fast_grid_mut().allocate_layers(1);
    let index = engine.world.fast_grid_mut().add_sector(
        square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(2000.0, 2000.0)),
        0,
    );
    let sector = crate::position_interface::SectorHandle::new(1)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
    let ids: Vec<_> = [500.0, 700.0]
        .into_iter()
        .map(|x| {
            let mut entity = make_test_ai_soldier(Camp::Lacklandists);
            entity
                .element_data_mut()
                .set_position(crate::coordinates::WorldPoint3D::new(x, 500.0, 0.0));
            entity
                .element_data_mut()
                .set_sector_topology(Some(sector), crate::fast_find_grid::SectorIndex::new(index));
            entity.element_data_mut().set_direction_instantly(4);
            entity.actor_data_mut().unwrap().action_state = ActionState::Waiting;
            let npc = entity.ai_actor_data_mut().unwrap();
            npc.view_radius = 500;
            npc.view_direction = [1.0, 0.0];
            npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
            entity.npc_data_mut().unwrap().life_points = 100;
            engine.add_test_entity(entity)
        })
        .collect();
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    engine.control.frame_counter = 100;
    for &id in &ids {
        let ai = engine.observation_ai_mut(id);
        ai.base.initial_position = Position {
            x: 900.0,
            y: 500.0,
            sector: Some(sector),
            level: 0,
        };
        ai.base.current_state = AiState::Default;
        ai.base.current_substate = Substate::DefaultOnPost;
    }
    engine.enter_ai_think_frame(ids[0]);
    (engine, assets, ids[0], ids[1])
}

#[test]
fn officer_report_uses_live_cone_and_synchronous_acceptance_or_refusal() {
    for (visible, accept) in [(true, true), (true, false), (false, true)] {
        let (mut engine, assets, owner, officer) = fixture();
        if !visible {
            engine
                .world
                .entities
                .get_mut(officer)
                .unwrap()
                .element_data_mut()
                .set_position(crate::coordinates::WorldPoint3D::new(300.0, 500.0, 0.0));
        }
        let ai = engine.observation_ai_mut(officer);
        ai.soldier_profile_rank = ProfileRank::Officer;
        if !accept {
            ai.base.current_state = AiState::Attacking;
            ai.base.current_substate = Substate::AttackingSwordfight;
        }
        let ai = engine.observation_ai_mut(owner);
        ai.base.current_state = AiState::Seeking;
        ai.base.current_substate = Substate::SeekingCharlyGoToOfficer;
        ai.base.antagonist = Some(AiEntityHandle::new(officer.index()));
        assert_eq!(
            engine.npc_is_detecting_human(&assets, owner, officer, 100),
            visible
        );
        engine.execute_ai_officer_rpc(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            &Stimulus::new(StimulusType::EventTimer),
        );
        let ai = &engine.observation_ai(owner).base;
        if !visible {
            assert_eq!(ai.current_substate, Substate::SeekingCharlyGoToOfficer);
            assert_eq!(ai.when_does_timer_ring, 110);
            assert_eq!(
                engine.observation_ai(officer).base.current_state,
                AiState::Default
            );
        } else if accept {
            assert_eq!(ai.current_substate, Substate::SeekingCharlyGoToOfficerSeen);
            assert_eq!(ai.when_does_timer_ring, 110);
            assert_eq!(
                engine.observation_ai(officer).base.current_substate,
                Substate::SeekingOfficerWaitForCharly
            );
        } else {
            assert_eq!(ai.current_state, AiState::Default);
            assert_eq!(ai.antagonist, None);
        }
    }
}

#[test]
fn report_cannot_cross_an_opaque_wall() {
    use crate::sight_obstacle::{ObstaclePoint, SightObstacle};
    let (mut engine, mut assets, owner, officer) = fixture();
    let mut wall = SightObstacle::new_default(0);
    wall.obstacle_points = [
        (595.0, 480.0),
        (605.0, 480.0),
        (605.0, 520.0),
        (595.0, 520.0),
    ]
    .into_iter()
    .map(|(x, y)| ObstaclePoint {
        x,
        y,
        z_top: 100.0,
        z_bottom: 0.0,
    })
    .collect();
    wall.top_plane_points = [
        [595.0, 480.0, 100.0],
        [605.0, 480.0, 100.0],
        [595.0, 520.0, 100.0],
    ];
    wall.bottom_plane_points = [
        [595.0, 480.0, 0.0],
        [605.0, 480.0, 0.0],
        [595.0, 520.0, 0.0],
    ];
    wall.rebuild_geometry();
    assets.environment.static_sight_obstacles = std::sync::Arc::new(vec![wall]);
    engine.world.static_sight_obstacle_active = vec![true];
    engine.observation_ai_mut(officer).soldier_profile_rank = ProfileRank::Officer;
    let ai = engine.observation_ai_mut(owner);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingCharlyGoToOfficer;
    ai.base.antagonist = Some(AiEntityHandle::new(officer.index()));
    assert!(!engine.npc_is_detecting_human(&assets, owner, officer, 100));
    engine.execute_ai_officer_rpc(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventTimer),
    );
    assert_eq!(engine.observation_ai(owner).base.when_does_timer_ring, 110);
    assert_eq!(
        engine.observation_ai(owner).base.current_substate,
        Substate::SeekingCharlyGoToOfficer
    );
    assert_eq!(
        engine.observation_ai(officer).base.current_state,
        AiState::Default
    );
}

#[test]
fn referral_completion_faces_live_friend_and_arms_wait_timer() {
    let (mut engine, assets, owner, friend) = fixture();
    let ai = engine.observation_ai_mut(owner);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSendCharlyToOfficer;
    ai.base.friend_in_trouble = Some(AiEntityHandle::new(friend.index()));
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(12);
    engine.execute_ai_officer_rpc(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventMyTalk2),
    );
    let ai = &engine.observation_ai(owner).base;
    assert_eq!(
        ai.current_substate,
        Substate::SeekingLookingResurrectedCharly
    );
    assert_eq!(ai.when_does_timer_ring, 200);
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|s| s.elements.iter())
            .any(|e| e.owner == Some(owner) && e.command == crate::element::Command::Turn)
    );
}

#[test]
#[should_panic(expected = "checkpoint referral requires friend")]
fn referral_completion_requires_its_friend() {
    let (mut engine, assets, owner, _) = fixture();
    let ai = engine.observation_ai_mut(owner);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingSendCharlyToOfficer;
    engine.execute_ai_officer_rpc(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventMyTalk2),
    );
}

#[test]
fn lecture_defence_relays_to_live_officer_and_ignores_unrelated_timer() {
    let (mut engine, assets, owner, officer) = fixture();
    let sim = crate::sim_rng::test_context();
    let ai = engine.observation_ai_mut(owner);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingCharlyGetLectureByOfficer;
    ai.base.antagonist = Some(AiEntityHandle::new(officer.index()));
    let ai = engine.observation_ai_mut(officer);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingOfficerLectureCharly;
    ai.base.antagonist = Some(AiEntityHandle::new(owner.index()));
    engine.execute_ai_officer_rpc(
        &sim,
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventTimer),
    );
    assert_eq!(
        engine.observation_ai(owner).base.current_substate,
        Substate::SeekingCharlyGetLectureByOfficer
    );
    engine.execute_ai_officer_rpc(
        &sim,
        &assets,
        owner,
        &Stimulus::new(StimulusType::CallYourTalk1),
    );
    assert_eq!(
        engine.observation_ai(owner).base.current_substate,
        Substate::SeekingCharlyGetLectureByOfficer2
    );
    engine.execute_ai_officer_rpc(
        &sim,
        &assets,
        owner,
        &Stimulus::new(StimulusType::EventMyTalk1),
    );
    // The recipient's speech executes before the caller returns, including a rejected line.
    assert!(
        engine
            .observation_ai(officer)
            .base
            .outbox
            .reentrant
            .owner_work
            .is_empty()
    );
}
