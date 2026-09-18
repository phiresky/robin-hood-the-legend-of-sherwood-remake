use super::*;
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::engine::test_support::{actors::make_test_ai_soldier, square_sector};

fn fixture(positions: &[(f32, f32)]) -> (EngineInner, LevelAssets, Vec<EntityId>) {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(256, 256);
    engine.world.fast_grid_mut().allocate_layers(1);
    let index = engine.world.fast_grid_mut().add_sector(
        square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(4000.0, 4000.0)),
        0,
    );
    let sector = crate::ai::SectorHandle::new(1)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
    let mut ids = Vec::new();
    for &(x, y) in positions {
        let mut entity = make_test_ai_soldier(crate::element::Camp::Lacklandists);
        entity
            .element_data_mut()
            .set_position(WorldPoint3D::new(x, y, 0.0));
        entity.element_data_mut().set_sector(Some(sector));
        ids.push(engine.add_test_entity(entity));
    }
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    (engine, assets, ids)
}

#[test]
fn drunk_body_observer_records_the_body_before_rejecting_its_priority() {
    let (mut engine, assets, ids) = fixture(&[(500.0, 500.0), (600.0, 500.0)]);
    let (owner, body) = (ids[0], ids[1]);
    let ai = engine.seek_enemy_mut(owner);
    ai.base.current_substate = Substate::DefaultOnPost;
    ai.base.blood_alcohol = (crate::parameters_ai::AI_DEBILITY_ALCOHOL_LIMIT + 1) as u8;
    engine
        .world
        .entities
        .expect_entity_mut(body, format_args!("dead body"))
        .npc_data_mut()
        .expect("body is an NPC")
        .life_points = 0;
    engine.execute_ai_seen_body(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        body.index(),
    );
    let ai = engine.seek_enemy(owner);
    assert_eq!(ai.base.current_substate, Substate::DefaultOnPost);
    assert!(
        ai.base
            .my_reconnaissance_report
            .seen_bodies
            .contains(&body.index())
    );
    assert_eq!(
        ai.base.my_reconnaissance_report.report_type,
        ReportType::Body
    );
    assert!(ai.base.missed_in_action.contains(&body.index()));
    assert!(ai.base.detected_body.is_none());
}

#[test]
fn informed_officer_scan_checks_visibility_before_lock_and_body_list() {
    let (mut engine, mut assets, ids) = fixture(&[
        (500.0, 500.0),
        (510.0, 500.0),
        (520.0, 500.0),
        (530.0, 500.0),
        (540.0, 500.0),
    ]);
    let (owner, body) = (ids[0], ids[1]);
    engine.ai_actor_mut(owner, "advice viewer").view_radius = 400;
    for &officer in &ids[2..] {
        crate::engine::test_support::actors::edit_enemy_profile(
            &mut assets,
            engine.seek_enemy_mut(officer),
            |profile| profile.rank = ProfileRank::Officer,
        );
    }
    engine.seek_enemy_mut(ids[2]).base.script_locked = true;
    engine
        .ai_actor_mut(ids[3], "uninformed officer")
        .detectable_lists[crate::element::DetectableType::Body as usize]
        .push(crate::element::Detectable {
            element: Some(body),
            detectable_type: crate::element::DetectableType::Body,
            ..Default::default()
        });
    crate::sight_obstacle::begin_parity_visibility_capture();
    let officer = engine.near_officer_informed_about_body(&assets, owner, body);
    let rays = crate::sight_obstacle::take_parity_visibility_capture();
    assert_eq!(officer, Some(ids[4]));
    assert_eq!(
        rays.len(),
        3,
        "locked and uninformed officers still receive their ordered LOS check"
    );
    assert_eq!(
        rays.iter()
            .map(|ray| ray.destination[0])
            .collect::<Vec<_>>(),
        vec![520.0, 530.0, 540.0]
    );
}

fn react_as_officer(body_y: f32) -> (EngineInner, EntityId, EntityId) {
    let (mut engine, mut assets, ids) = fixture(&[(500.0, 500.0), (550.0, body_y), (520.0, 500.0)]);
    let (owner, body) = (ids[0], ids[1]);
    engine.control.frame_counter = 1200;
    let position = engine.live_ai_position(body);
    let ai = engine.seek_enemy_mut(owner);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBodyReactiontime;
    ai.base.detected_body = Some(AiEntityHandle::new(body.index()));
    ai.base
        .my_reconnaissance_report
        .update(ReportType::Body, position);
    crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
        profile.rank = ProfileRank::Officer
    });
    crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
        profile.initiative = 0
    });
    let Entity::Soldier(target) = engine
        .world
        .entities
        .expect_entity_mut(body, format_args!("sleeping body"))
    else {
        unreachable!()
    };
    target.human.unconscious = true;
    engine.seek_enemy_mut(ids[2]).base.current_substate = Substate::DefaultOnPost;
    engine.execute_ai_body_reaction_timer(&crate::sim_rng::test_context(), &assets, owner);
    (engine, owner, body)
}

#[test]
fn officer_body_reaction_uses_stretched_max_norm_to_delegate() {
    assert!(149.5 < 150.0 && 149.5 * crate::position_interface::INVERSE_ASPECT_RATIO > 150.0);
    let (engine, owner, _) = react_as_officer(649.5);
    let ai = engine.seek_enemy(owner);
    assert_eq!(
        ai.base.current_substate,
        Substate::SeekingOfficerLookingForSoldiers1
    );
    assert_eq!(ai.base.when_does_timer_ring, 1220);
    assert!(
        !engine
            .expect_entity(owner, "delegating officer")
            .ai_actor_data()
            .unwrap()
            .detectable_lists[crate::element::DetectableType::Friend as usize]
            .is_empty()
    );
}

#[test]
fn officer_body_reaction_examines_body_within_stretched_threshold() {
    assert!(80.0 * crate::position_interface::INVERSE_ASPECT_RATIO <= 150.0);
    let (engine, owner, body) = react_as_officer(580.0);
    let ai = engine.seek_enemy(owner);
    assert_eq!(ai.base.current_substate, Substate::SeekingBody);
    assert_eq!(
        ai.base.detected_body,
        Some(AiEntityHandle::new(body.index()))
    );
    assert_eq!(
        (ai.base.seek_position.x, ai.base.seek_position.y),
        (550.0, 580.0)
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, crate::element::Command::Move)
    );
}

#[test]
fn self_body_sighting_updates_report_and_queues_another_examination() {
    let (mut engine, assets, ids) = fixture(&[(500.0, 500.0), (600.0, 500.0)]);
    let (owner, previous_body) = (ids[0], ids[1]);
    engine.add_detectable_for_all_npc(owner, crate::element::DetectableType::Body);
    let ai = engine.seek_enemy_mut(owner);
    ai.base.current_state = AiState::Seeking;
    ai.base.current_substate = Substate::SeekingBodyReactiontime;
    ai.base.detected_body = Some(AiEntityHandle::new(previous_body.index()));
    ai.base.launch_timer(123, 0);
    ai.current_task_priority = crate::ai_enemy::task_priority::BODY;
    ai.new_task_priority = crate::ai_enemy::task_priority::BODY;
    engine.execute_ai_seen_body(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        owner.index(),
    );
    let ai = engine.seek_enemy(owner);
    assert!(
        ai.base
            .my_reconnaissance_report
            .seen_bodies
            .contains(&owner.index())
    );
    assert_eq!(
        ai.base.my_reconnaissance_report.report_type,
        ReportType::Body
    );
    assert_eq!(
        ai.base.detected_body,
        Some(AiEntityHandle::new(previous_body.index()))
    );
    assert_eq!(ai.other_bodies_to_examine, vec![owner.index()]);
    assert_eq!(ai.base.current_substate, Substate::SeekingBodyReactiontime);
    assert!(ai.base.timer_is_running);
    assert_eq!(ai.base.when_does_timer_ring, 123);
}
