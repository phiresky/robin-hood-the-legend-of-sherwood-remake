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
        let mut conversion = vec![u16::MAX; crate::order::OrderType::WaitingShield as usize + 1];
        conversion[crate::order::OrderType::WaitingShield as usize] = 0;
        entity.element_data_mut().sprite.conversion = std::sync::Arc::new(conversion);
        ids.push(engine.add_test_entity(entity));
    }
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    std::sync::Arc::make_mut(&mut assets.profile_manager).hth_weapons[0].shield = true;
    (engine, assets, ids)
}

fn make_archers(assets: &mut LevelAssets) {
    let profiles = std::sync::Arc::make_mut(&mut assets.profile_manager);
    profiles.bows.push(crate::profiles::BowProfile::default());
    for profile in &mut profiles.soldiers {
        profile.shooting_weapon_id = 1;
    }
}

#[test]
fn protection_counts_inactive_archer_but_keeps_state_and_strict_radius_gates() {
    let (mut engine, mut assets, ids) = fixture(&[(1520.0, 900.0), (1328.0, 1033.0)]);
    let [owner, orphan] = ids.as_slice() else {
        unreachable!()
    };
    let (owner, orphan) = (*owner, *orphan);
    make_archers(&mut assets);
    engine
        .world
        .entities
        .expect_entity_mut(orphan, format_args!("inactive orphan"))
        .element_data_mut()
        .active = false;
    engine
        .world
        .entities
        .expect_enemy_ai_mut(orphan, format_args!("seeking orphan"))
        .base
        .current_state = AiState::Seeking;
    assert_eq!(engine.live_archers_needing_protection(&assets, owner), 1);
    engine
        .world
        .entities
        .expect_enemy_ai_mut(orphan, format_args!("default orphan"))
        .base
        .current_state = AiState::Default;
    assert_eq!(engine.live_archers_needing_protection(&assets, owner), 0);
    engine
        .world
        .entities
        .expect_enemy_ai_mut(orphan, format_args!("seeking orphan"))
        .base
        .current_state = AiState::Seeking;
    engine
        .world
        .entities
        .expect_entity_mut(orphan, format_args!("radius boundary"))
        .element_data_mut()
        .set_position(WorldPoint3D::new(2020.0, 900.0, 0.0));
    assert_eq!(engine.live_archers_needing_protection(&assets, owner), 0);
}

#[test]
fn protection_reads_reciprocal_unlink_after_state_change() {
    let (mut engine, mut assets, ids) = fixture(&[(900.0, 2500.0), (920.0, 2510.0)]);
    let [owner, orphan] = ids.as_slice() else {
        unreachable!()
    };
    let (owner, orphan) = (*owner, *orphan);
    make_archers(&mut assets);
    let ai = engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("linked shield"));
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingPhalanx;
    ai.archer_behind_me = Some(AiEntityHandle::new(orphan.index()));
    ai.tower_guard = true;
    let ai = engine
        .world
        .entities
        .expect_enemy_ai_mut(orphan, format_args!("protected archer"));
    ai.base.current_state = AiState::Attacking;
    ai.shield_bearer_before_me = Some(AiEntityHandle::new(owner.index()));
    assert_eq!(engine.live_archers_needing_protection(&assets, owner), 0);
    engine.duty_set_state(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        AiState::Attacking,
        Substate::AttackingApproachToObserve,
    );
    assert_eq!(
        engine
            .world
            .entities
            .expect_enemy_ai(orphan, format_args!("orphan link"))
            .shield_bearer_before_me,
        None
    );
    assert_eq!(engine.live_archers_needing_protection(&assets, owner), 1);
}

#[test]
fn phalanx_uses_raw_distance_and_running_members_future_anchor() {
    let (mut engine, assets, ids) = fixture(&[
        (1263.1832, 2281.7712),
        (1322.0, 2276.0),
        (1306.8123, 2262.1873),
    ]);
    let [owner, farther, nearest] = ids.as_slice() else {
        unreachable!()
    };
    let (owner, farther, nearest) = (*owner, *farther, *nearest);
    engine
        .world
        .entities
        .expect_enemy_ai_mut(farther, format_args!("farther shield"))
        .base
        .current_substate = Substate::AttackingProtectingWithShield;
    let sector = crate::ai::SectorHandle::new(0).unwrap();
    let ai = engine
        .world
        .entities
        .expect_enemy_ai_mut(nearest, format_args!("running shield"));
    ai.base.current_substate = Substate::AttackingRunningToPhalanx;
    ai.base.seek_position = Position {
        x: 1310.3472,
        y: 2209.295,
        sector: Some(sector),
        level: 0,
    };
    ai.shield_bearer_direction = 8;
    engine
        .world
        .entities
        .expect_entity_mut(nearest, format_args!("inactive shield"))
        .element_data_mut()
        .active = false;
    assert_eq!(
        engine.nearest_live_free_shield_bearer(&assets, owner),
        Some(nearest)
    );
    let (slot, _, left, right) = engine
        .live_phalanx_place(&assets, owner)
        .expect("reachable future slot");
    assert_eq!((left, right), (Some(nearest), None));
    assert_eq!(slot.sector, Some(sector));
    assert_eq!(slot.x.to_bits(), 1285.3472_f32.to_bits());
    assert_eq!(slot.y.to_bits(), 2209.295_f32.to_bits());
}

#[test]
fn close_phalanx_slot_with_different_sector_needs_no_movement_order() {
    let (mut engine, assets, ids) = fixture(&[(100.0, 100.0)]);
    let owner = ids[0];
    let index = engine.world.fast_grid_mut().add_sector(
        square_sector(2, 0, MapPoint::new(90.0, 90.0), MapPoint::new(200.0, 200.0)),
        0,
    );
    let destination = Position {
        x: 102.0,
        y: 100.0,
        sector: crate::ai::SectorHandle::new(2)
            .map(|s| s.with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap())),
        level: 0,
    };
    engine.enter_ai_think_frame(owner);
    engine.duty_go_to(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        destination,
        GotoFlags::RUN,
    );
    let ai = engine
        .world
        .entities
        .expect_ai_controller(owner, format_args!("close formation destination"));
    assert!(ai.already_on_point);
    assert!(ai.outbox.actor.orders.is_empty());
    assert!(
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .all(|s| s.elements.iter().all(|e| !matches!(
                e.data,
                crate::sequence::SequenceElementData::Movement { .. }
            )))
    );
}

#[test]
fn phalanx_walks_the_complete_live_chain_and_observes_relinks() {
    let positions: Vec<_> = (0..20).map(|i| (100.0 + i as f32 * 25.0, 100.0)).collect();
    let (mut engine, _, ids) = fixture(&positions);
    for pair in ids.windows(2) {
        engine
            .world
            .entities
            .expect_enemy_ai_mut(pair[0], format_args!("right chain"))
            .right_combat_neighbour = Some(AiEntityHandle::new(pair[1].index()));
        engine
            .world
            .entities
            .expect_enemy_ai_mut(pair[1], format_args!("left chain"))
            .left_combat_neighbour = Some(AiEntityHandle::new(pair[0].index()));
    }
    assert_eq!(engine.live_phalanx_end(ids[0], false), ids[19]);
    assert_eq!(engine.live_phalanx_end(ids[19], true), ids[0]);
    engine.apply_update_right_combat_neighbour(
        ids[8].index(),
        Some(AiEntityHandle::new(ids[9].index())),
        None,
    );
    assert_eq!(engine.live_phalanx_end(ids[0], false), ids[8]);
    assert_eq!(engine.live_phalanx_end(ids[19], true), ids[9]);
}

#[test]
fn shield_sequence_keeps_stored_world_y_without_map_roundtrip() {
    let (mut engine, _, ids) = fixture(&[(100.0, 100.0)]);
    let owner = ids[0];
    let point = WorldPoint3D::new(100.0, 503.01535, 4.06752014);
    let element = engine
        .world
        .entities
        .expect_entity_mut(owner, format_args!("shield threat"))
        .element_data_mut();
    element.set_position(point);
    assert_ne!(
        (element.position_map().y + point.z).to_bits(),
        point.y.to_bits()
    );
    let stored = element.position();
    let ai = engine
        .world
        .entities
        .expect_ai_controller_mut(owner, format_args!("shield sequence"));
    ai.raise_shield_world(stored);
    let sequence = ai.outbox.actor.launch_sequences.last().unwrap();
    let element = &sequence.elements[0];
    assert!(
        matches!(element.get_property(crate::sequence::Field::ShieldDangerPoint),
        Some(crate::sequence::FieldValue::Point3D { x, y, z })
            if x.to_bits() == point.x.to_bits() && y.to_bits() == point.y.to_bits() && z.to_bits() == point.z.to_bits())
    );
}

fn periodic_phalanx_fixture(
    owner_x: f32,
    command: crate::element::Command,
) -> (EngineInner, LevelAssets, EntityId) {
    let (mut engine, assets, ids) = fixture(&[(owner_x, 500.0), (1500.0, 500.0), (600.0, 500.0)]);
    let (owner, enemy, neighbour) = (ids[0], ids[1], ids[2]);
    let Entity::Soldier(target) = engine
        .world
        .entities
        .expect_entity_mut(enemy, format_args!("periodic bow threat"))
    else {
        unreachable!()
    };
    target.soldier.cached_camp = crate::element::Camp::Royalists;
    target.actor.action_state = crate::element::ActionState::AimingWithBow;
    let ai = engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("periodic shield owner"));
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.stuck_counter = 2;
    ai.list_them = vec![enemy.index()];
    let npc = engine
        .world
        .entities
        .expect_ai_actor_data_mut(owner, format_args!("periodic visible enemy"));
    npc.detectable_lists[crate::element::DetectableType::Enemy as usize].push(
        crate::element::Detectable {
            element: Some(enemy),
            detectable_type: crate::element::DetectableType::Enemy,
            seen_last_frame: true,
            ..Default::default()
        },
    );
    engine.control.frame_counter = u32::from(npc.register_number) + 100;
    engine
        .world
        .entities
        .expect_enemy_ai_mut(neighbour, format_args!("periodic phalanx anchor"))
        .base
        .current_substate = Substate::AttackingPhalanx;
    let selected = engine.orders.sequence_manager.launch_element(
        crate::sequence::SequenceElement::new_generic(1, command, Some(owner)),
    );
    engine
        .orders
        .sequence_manager
        .element_in_progress(selected, 0);
    engine
        .world
        .entities
        .expect_entity_mut(owner, format_args!("periodic selected animation"))
        .actor_data_mut()
        .unwrap()
        .installed_order = Some(crate::element::InstalledActorOrder {
        order_id: std::num::NonZeroU32::new(1).unwrap(),
        order_type: crate::order::OrderType::WaitingUpright,
    });
    (engine, assets, owner)
}

#[test]
fn periodic_phalanx_move_is_registered_before_idle_stuck_check() {
    let (mut engine, assets, owner) =
        periodic_phalanx_fixture(500.0, crate::element::Command::Wait);
    engine.tick_periodic_ai_for_npc(&crate::sim_rng::test_context(), owner, &assets);
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("periodic movement result"));
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunningToPhalanx
    );
    assert_eq!(
        (
            ai.base.last_goto_destination.x,
            ai.base.last_goto_destination.y
        ),
        (575.0, 500.0)
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, crate::element::Command::Move)
    );
    assert_eq!(
        ai.base.stuck_counter, 0,
        "registered movement suppresses an idle stuck increment"
    );
}

#[test]
fn periodic_phalanx_move_keeps_attentive_command_classification() {
    let (mut engine, assets, owner) =
        periodic_phalanx_fixture(500.0, crate::element::Command::EnterAttentiveMode);
    engine
        .world
        .entities
        .expect_enemy_ai_mut(owner, format_args!("attentive counter"))
        .base
        .stuck_counter = 0;
    engine.tick_periodic_ai_for_npc(&crate::sim_rng::test_context(), owner, &assets);
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("attentive periodic result"));
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunningToPhalanx
    );
    assert_eq!(
        (
            ai.base.last_goto_destination.x,
            ai.base.last_goto_destination.y
        ),
        (575.0, 500.0)
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, crate::element::Command::Move)
    );
    assert_eq!(
        engine.actor_command(owner),
        crate::element::Command::EnterAttentiveMode
    );
    assert_eq!(ai.base.stuck_counter, 0);
}

#[test]
fn periodic_phalanx_already_on_point_does_not_register_a_move() {
    let (mut engine, assets, owner) =
        periodic_phalanx_fixture(575.0, crate::element::Command::Wait);
    // Hold the arrival decision so the watchdog observes the already-on-point
    // movement result independently of the subsequent face-and-raise sequence.
    engine
        .world
        .entities
        .expect_ai_controller_mut(owner, format_args!("locked arrival"))
        .script_locked = true;
    engine.tick_periodic_ai_for_npc(&crate::sim_rng::test_context(), owner, &assets);
    let ai = engine
        .world
        .entities
        .expect_enemy_ai(owner, format_args!("already-on-point result"));
    assert_eq!(
        ai.base.current_substate,
        Substate::AttackingRunningToPhalanx
    );
    assert_eq!(
        (ai.base.seek_position.x, ai.base.seek_position.y),
        (575.0, 500.0)
    );
    assert!(
        !engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, crate::element::Command::Move)
    );
    assert!(
        ai.base
            .stimulus_queue
            .iter()
            .any(|stimulus| stimulus.stimulus_type == crate::ai::StimulusType::EventReachPoint)
    );
    assert_eq!(ai.base.stuck_counter, 3);
}
