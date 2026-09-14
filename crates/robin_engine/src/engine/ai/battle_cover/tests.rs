use super::*;
use crate::coordinates::{MapPoint, WorldPoint3D};
use crate::element::Camp;
use crate::engine::test_support::{actors::make_test_ai_soldier, square_sector};

fn fixture(points: &[(f32, f32)]) -> (EngineInner, LevelAssets, Vec<EntityId>) {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(128, 128);
    engine.world.fast_grid_mut().allocate_layers(1);
    let index = engine.world.fast_grid_mut().add_sector(
        square_sector(1, 0, MapPoint::new(0.0, 0.0), MapPoint::new(4000.0, 4000.0)),
        0,
    );
    let sector = crate::ai::SectorHandle::new(1)
        .unwrap()
        .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
    let ids: Vec<_> = points
        .iter()
        .enumerate()
        .map(|(index, &(x, y))| {
            let mut entity = make_test_ai_soldier(if index == 0 {
                Camp::Lacklandists
            } else {
                Camp::Royalists
            });
            entity
                .element_data_mut()
                .set_position(WorldPoint3D::new(x, y, 0.0));
            entity.element_data_mut().set_sector(Some(sector));
            entity.npc_data_mut().unwrap().life_points = 100;
            engine.add_test_entity(entity)
        })
        .collect();
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    for &id in &ids {
        engine
            .get_entity_mut(id)
            .unwrap()
            .element_data_mut()
            .set_sector_topology(Some(sector), sector.arena_index());
    }
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].distance
        [crate::weapons::WeaponDistance::Maximal as usize] = 50;
    let ai = engine
        .world
        .entities
        .expect_enemy_ai_mut(ids[0], format_args!("pride fixture"));
    crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
        profile.pride = 1
    });
    ai.base.current_substate = Substate::AttackingOfficerGivingOrdersWaiting;
    ai.list_them = ids[1..].iter().map(|id| id.index()).collect();
    (engine, assets, ids)
}

#[test]
fn pride_range_uses_stretched_world_distance() {
    let (mut engine, assets, ids) = fixture(&[(621.35455, 822.2824), (615.2868, 780.4628)]);
    assert!(engine.live_ai_is_too_proud_to_attack(&assets, ids[0]));
    assert_eq!(
        engine
            .world
            .entities
            .expect_ai_controller(ids[0], format_args!("proud target"))
            .primary_target,
        Some(AiEntityHandle::new(ids[1].index()))
    );
}

#[test]
fn pride_range_uses_close_body_during_door_pass() {
    let (mut engine, assets, ids) = fixture(&[(2230.0, 405.0), (2268.0, 393.0)]);
    let target = ids[1];
    engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
        "pride_gate.scs",
    ));
    let sector = engine
        .world
        .entities
        .get(target)
        .unwrap()
        .element_data()
        .sector()
        .unwrap();
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            point_in: MapPoint::new(2301.0, 381.0),
            point_out: MapPoint::new(2301.0, 381.0),
            sector_in: crate::sector::SectorNumber::new(1),
            sector_out: crate::sector::SectorNumber::new(1),
            sector_in_index: sector.arena_index(),
            sector_out_index: sector.arena_index(),
            ..Default::default()
        });
    let mut pass = crate::sequence::SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(target),
        crate::order::OrderType::WalkingUpright,
    );
    let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass.data
    else {
        unreachable!()
    };
    *gate_id = Some(crate::gate::DoorIndex::new(0).unwrap());
    *direction = 1;
    let sequence = engine.orders.sequence_manager.launch_element(pass);
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &mut Vec::new(),
        sequence,
        0,
    );
    assert_eq!(
        engine.live_ai_position(target).map_point(),
        MapPoint::new(2301.0, 381.0)
    );
    assert!(!engine.live_ai_is_too_proud_to_attack(&assets, ids[0]));
}

#[test]
fn pride_reselection_uses_current_shared_multiplicity() {
    let (mut engine, assets, ids) = fixture(&[(100.0, 100.0), (400.0, 100.0), (450.0, 100.0)]);
    engine
        .ai
        .global
        .primary_target_multiplicity_scratch
        .insert(ids[1].index(), 1);
    engine
        .ai
        .global
        .primary_target_multiplicity_scratch
        .insert(ids[2].index(), 0);
    engine.live_ai_is_too_proud_to_attack(&assets, ids[0]);
    assert_eq!(
        engine
            .world
            .entities
            .expect_ai_controller(ids[0], format_args!("unoccupied proud target"))
            .primary_target,
        Some(AiEntityHandle::new(ids[2].index()))
    );
    engine
        .ai
        .global
        .primary_target_multiplicity_scratch
        .insert(ids[1].index(), 0);
    engine
        .ai
        .global
        .primary_target_multiplicity_scratch
        .insert(ids[2].index(), 1);
    engine.live_ai_is_too_proud_to_attack(&assets, ids[0]);
    assert_eq!(
        engine
            .world
            .entities
            .expect_ai_controller(ids[0], format_args!("updated proud target"))
            .primary_target,
        Some(AiEntityHandle::new(ids[1].index()))
    );
}

#[test]
fn rejected_cover_keeps_computed_goal_and_clears_both_protection_links() {
    let (mut engine, assets, ids) = fixture(&[
        (100.0, 100.0),
        (400.0, 400.0),
        (900.0, 900.0),
        (200.0, 200.0),
    ]);
    let (owner, bearer, target, old_bearer) = (ids[0], ids[1], ids[2], ids[3]);
    let ai = engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("covered archer"));
    ai.is_archer_unit = true;
    ai.shield_bearer_before_me = Some(AiEntityHandle::new(old_bearer.index()));
    ai.base.seek_position = Position {
        x: 123.0,
        y: 456.0,
        ..Default::default()
    };
    engine
        .world
        .entities
        .expect_enemy_ai_mut(old_bearer, format_args!("previous cover"))
        .archer_behind_me = Some(AiEntityHandle::new(owner.index()));
    engine
        .world
        .entities
        .expect_ai_controller_mut(bearer, format_args!("cover target"))
        .primary_target = Some(AiEntityHandle::new(target.index()));
    let (anchor, direction) = engine.live_shield_bearer_position(bearer);
    let [x, y] = crate::shadow_polygon::sector_to_direction(direction as i16);
    let distance = archer::DISTANCE_SHIELD_BEARER_ARCHER as f32;
    let expected = Position {
        x: anchor.x - x * distance,
        y: anchor.y - (y * crate::position_interface::ASPECT_RATIO) * distance,
        ..anchor
    };
    engine.ai.standard_view_polygon_radius = 0;
    assert_eq!(
        engine.execute_ai_battle_cover(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            bearer.index()
        ),
        ControlFlow::Continue(Decision::Shoot)
    );
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("rejected cover"));
    assert_eq!(ai.base.seek_position.x.to_bits(), expected.x.to_bits());
    assert_eq!(ai.base.seek_position.y.to_bits(), expected.y.to_bits());
    assert_eq!(
        ai.base.primary_target,
        Some(AiEntityHandle::new(target.index()))
    );
    assert_eq!(ai.shield_bearer_before_me, None);
    assert_eq!(
        engine
            .world
            .entities
            .expect_enemy_ai(old_bearer, format_args!("old cover cleared"))
            .archer_behind_me,
        None
    );
    assert_eq!(
        engine
            .world
            .entities
            .expect_enemy_ai(bearer, format_args!("new cover cleared"))
            .archer_behind_me,
        None
    );
}

#[test]
fn lost_execution_target_falls_back_to_the_source_decision() {
    let (mut engine, assets, ids) = fixture(&[(100.0, 100.0)]);
    let sim = crate::sim_rng::test_context();
    assert_eq!(
        engine.execute_ai_battle_too_proud(&sim, &assets, ids[0], Substate::AttackingReactiontime),
        ControlFlow::Continue(Decision::Reserve)
    );
    assert_eq!(
        engine.execute_ai_battle_archer_step_back(
            &sim,
            &assets,
            ids[0],
            Substate::AttackingReactiontime
        ),
        ControlFlow::Continue(Decision::Shoot)
    );
}
