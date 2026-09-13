use super::*;

fn replay_goal_sector(number: i16, layer: u16) -> crate::fast_find_grid::GridSector {
    crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: MapBBox::new(),
        sector_type: SectorType::MOTION | SectorType::AREA,
        layer,
        sector_number: crate::sector::SectorNumber::new(number),
        door_index: None,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    }
}

fn square_group_sector(
    number: i16,
    layer: u16,
    min: MapPoint,
    max: MapPoint,
) -> crate::fast_find_grid::GridSector {
    crate::fast_find_grid::GridSector {
        points: vec![
            min,
            MapPoint::new(max.x, min.y),
            max,
            MapPoint::new(min.x, max.y),
        ],
        bounding_box: MapBBox::from_corners(min, max),
        ..replay_goal_sector(number, layer)
    }
}

fn group_move_element(
    point: MapPoint,
    sector: crate::position_interface::SectorHandle,
    layer: u16,
) -> crate::element::ElementData {
    let mut element = {
        let mut initial_element = crate::element::ElementData::default();
        initial_element.kind = crate::element::ElementKind::ActorPc;
        initial_element
    };
    element.set_position_map(point);
    element.set_sector(Some(sector));
    element.set_layer(layer);
    element
}

#[test]
fn group_move_live_snapshot_recovers_duplicate_public_source_for_three_gate_route() {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(32, 32);
    engine.world.fast_grid_mut().allocate_layers(3);
    let wrong_88_raw = engine.world.fast_grid_mut().add_sector(
        square_group_sector(
            88,
            2,
            MapPoint::new(100.0, 100.0),
            MapPoint::new(200.0, 200.0),
        ),
        2,
    );
    let source_88_raw = engine.world.fast_grid_mut().add_sector(
        square_group_sector(
            88,
            2,
            MapPoint::new(650.0, 1550.0),
            MapPoint::new(750.0, 1700.0),
        ),
        2,
    );
    let transit_70_raw = engine.world.fast_grid_mut().add_sector(
        square_group_sector(
            70,
            1,
            MapPoint::new(500.0, 1400.0),
            MapPoint::new(600.0, 1500.0),
        ),
        1,
    );
    let outside_0_raw = engine.world.fast_grid_mut().add_sector(
        square_group_sector(
            0,
            0,
            MapPoint::new(700.0, 1300.0),
            MapPoint::new(800.0, 1400.0),
        ),
        0,
    );
    let goal_77_raw = engine.world.fast_grid_mut().add_sector(
        square_group_sector(
            77,
            1,
            MapPoint::new(950.0, 1550.0),
            MapPoint::new(1050.0, 1700.0),
        ),
        1,
    );
    assert_ne!(wrong_88_raw, source_88_raw);
    let source_88 = crate::fast_find_grid::SectorIndex::new(source_88_raw).unwrap();
    let transit_70 = crate::fast_find_grid::SectorIndex::new(transit_70_raw).unwrap();
    let outside_0 = crate::fast_find_grid::SectorIndex::new(outside_0_raw).unwrap();
    let goal_77 = crate::fast_find_grid::SectorIndex::new(goal_77_raw).unwrap();

    let actor = EntityId::Pc(crate::entity_id::PcId(137));
    let source_point = MapPoint::new(691.83026, 1641.3748);
    let source = group_move_source_sector(
        &engine,
        actor,
        &group_move_element(
            source_point,
            crate::position_interface::SectorHandle::new(88).unwrap(),
            2,
        ),
    );
    assert_eq!(source.arena_index(), Some(source_88));

    let mut doors = vec![crate::gate::Door::default(); 115];
    for door in &mut doors {
        door.active = false;
    }
    doors[114] = crate::gate::Door {
        active: true,
        sector_out: crate::sector::SectorNumber::new(88),
        sector_in: crate::sector::SectorNumber::new(70),
        sector_out_index: Some(source_88),
        sector_in_index: Some(transit_70),
        point_out: source_point,
        point_in: MapPoint::new(575.0, 1450.0),
        ..Default::default()
    };
    doors[111] = crate::gate::Door {
        active: true,
        sector_out: crate::sector::SectorNumber::new(0),
        sector_in: crate::sector::SectorNumber::new(70),
        sector_out_index: Some(outside_0),
        sector_in_index: Some(transit_70),
        point_out: MapPoint::new(750.0, 1350.0),
        point_in: MapPoint::new(550.0, 1450.0),
        ..Default::default()
    };
    doors[60] = crate::gate::Door {
        active: true,
        sector_out: crate::sector::SectorNumber::new(0),
        sector_in: crate::sector::SectorNumber::new(77),
        sector_out_index: Some(outside_0),
        sector_in_index: Some(goal_77),
        point_out: MapPoint::new(775.0, 1350.0),
        point_in: MapPoint::new(1000.0, 1600.0),
        ..Default::default()
    };
    crate::gate::build_gate_links(&mut doors);
    let path = find_group_move_gate_path(
        &doors,
        actor,
        source_point,
        source,
        MapPoint::new(1004.536, 1614.76),
        crate::sector::SectorNumber::new(77),
        Some(goal_77),
        1,
        None,
        &|_| true,
        &|_| None,
    )
    .expect("Pc137-style exact GroupMove must traverse the indexed gate graph");
    assert_eq!(
        path.iter()
            .map(|step| (step.door_index.get(), step.direct))
            .collect::<Vec<_>>(),
        vec![(114, true), (111, false), (60, true)]
    );
}

#[test]
fn group_move_live_snapshot_keeps_empty_grid_numeric_compatibility() {
    let engine = EngineInner::new();
    let actor = EntityId::Pc(crate::entity_id::PcId(137));
    let source = group_move_source_sector(
        &engine,
        actor,
        &group_move_element(
            MapPoint::new(10.0, 20.0),
            crate::position_interface::SectorHandle::new(88).unwrap(),
            2,
        ),
    );
    assert_eq!(source.get(), 88);
    assert_eq!(source.arena_index(), None);
}

#[test]
#[should_panic(expected = "ambiguous in the exact arena")]
fn group_move_live_snapshot_rejects_ambiguous_duplicate_public_source() {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(8, 8);
    engine.world.fast_grid_mut().allocate_layers(3);
    for _ in 0..2 {
        engine.world.fast_grid_mut().add_sector(
            square_group_sector(
                88,
                2,
                MapPoint::new(100.0, 100.0),
                MapPoint::new(200.0, 200.0),
            ),
            2,
        );
    }
    let _ = group_move_source_sector(
        &engine,
        EntityId::Pc(crate::entity_id::PcId(137)),
        &group_move_element(
            MapPoint::new(150.0, 150.0),
            crate::position_interface::SectorHandle::new(88).unwrap(),
            2,
        ),
    );
}

#[test]
fn group_move_current_door_endpoint_precedes_ambiguous_raw_position_recovery() {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(8, 8);
    engine.world.fast_grid_mut().allocate_layers(3);
    for _ in 0..2 {
        engine.world.fast_grid_mut().add_sector(
            square_group_sector(
                88,
                2,
                MapPoint::new(100.0, 100.0),
                MapPoint::new(200.0, 200.0),
            ),
            2,
        );
    }
    let far_index = crate::fast_find_grid::SectorIndex::new(37).unwrap();
    let far_point = MapPoint::new(400.0, 500.0);
    let door = crate::gate::Door {
        sector_out: crate::sector::SectorNumber::new(88),
        sector_in: crate::sector::SectorNumber::new(77),
        sector_in_index: Some(far_index),
        layer_in: 1,
        point_in: far_point,
        ..Default::default()
    };
    let mut entity = crate::element::Entity::Pc(crate::element::ActorPc {
        element: group_move_element(
            MapPoint::new(150.0, 150.0),
            crate::position_interface::SectorHandle::new(88).unwrap(),
            2,
        ),
        actor: Default::default(),
        human: Default::default(),
        pc: Default::default(),
    });
    entity.position_iface_mut().set_door(
        crate::position_interface::DoorHandle::new(0).expect("valid door index"),
        true,
    );

    let (point, sector, layer) = group_move_route_source(
        &engine,
        EntityId::Pc(crate::entity_id::PcId(137)),
        &entity,
        &[door],
    );
    assert_eq!(point, far_point);
    assert_eq!(sector.get(), 77);
    assert_eq!(sector.arena_index(), Some(far_index));
    assert_eq!(layer, 1);
}

#[test]
fn replay_exact_group_move_goal_survives_spatial_miss_and_duplicate_public_sectors() {
    let mut level = crate::fast_find_grid::LevelGrid::default();
    level.sectors.push(replay_goal_sector(421, 6));
    level.sectors.push(replay_goal_sector(421, 6));
    // The recorded movement level is a position property, independent
    // from the retained sector reference's own topology layer.
    level.sectors.push(replay_goal_sector(422, 8));
    let exact_421 = crate::fast_find_grid::SectorIndex::new(1).unwrap();
    let exact_422 = crate::fast_find_grid::SectorIndex::new(2).unwrap();

    for (recorded, exact) in [
        ((crate::sector::SectorNumber::new(421), 6), exact_421),
        ((crate::sector::SectorNumber::new(422), 2), exact_422),
    ] {
        assert_eq!(
            resolve_group_move_route_goal_index(
                Some(recorded),
                Some(exact),
                Some(crate::sector::SectorNumber::new(116)),
                None,
                8,
                None,
                &level,
            ),
            Some(exact),
            "the retained sparse slot is authoritative when the click misses the recorded sector"
        );
    }

    assert_eq!(
        resolve_group_move_route_goal_index(
            Some((crate::sector::SectorNumber::new(421), 6)),
            None,
            Some(crate::sector::SectorNumber::new(116)),
            None,
            8,
            None,
            &level,
        ),
        None,
        "live and legacy commands retain spatial resolution instead of guessing among duplicate public sectors"
    );
}

#[test]
#[should_panic(expected = "disagrees with its recorded public sector")]
fn replay_exact_group_move_goal_rejects_inconsistent_public_identity() {
    let mut level = crate::fast_find_grid::LevelGrid::default();
    level.sectors.push(replay_goal_sector(421, 6));
    resolve_group_move_route_goal_index(
        Some((crate::sector::SectorNumber::new(422), 2)),
        crate::fast_find_grid::SectorIndex::new(0),
        None,
        None,
        0,
        None,
        &level,
    );
}
use crate::coordinates::MoveBox;
use crate::sector::SectorType;

#[test]
fn ordinary_formation_uses_live_actor_box_not_generic_upright_box() {
    let bbox = group_move_candidate_box(
        MapBBox::from_coords(90.0, 90.0, 110.0, 110.0),
        MoveBox::from_coords(-2.0, -2.0, 2.0, 2.0),
        MapPoint::new(100.0, 100.0),
        MapPoint::new(200.0, 220.0),
        false,
    );
    assert_eq!((bbox.x_min(), bbox.y_min()), (190.0, 210.0));
    assert_eq!((bbox.x_max(), bbox.y_max()), (210.0, 230.0));
}

#[test]
fn ordinary_formation_preserves_live_box_offset_from_actor_position() {
    let bbox = group_move_candidate_box(
        MapBBox::from_coords(94.0, 97.0, 109.0, 112.0),
        MoveBox::from_coords(-20.0, -20.0, 20.0, 20.0),
        MapPoint::new(100.0, 100.0),
        MapPoint::new(300.0, 400.0),
        false,
    );
    assert_eq!((bbox.x_min(), bbox.y_min()), (294.0, 397.0));
    assert_eq!((bbox.x_max(), bbox.y_max()), (309.0, 412.0));
}

#[test]
fn mercenary_box_preserves_original_float_operation_order() {
    // Savegame_032/replay-006 exposed this exact boundary: collapsing
    // `(box - center) + click` into `box + (click - center)` rounds the
    // final X coordinate down by one ULP.
    let actor_x = f32::from_bits(1_151_945_109);
    let click_x = f32::from_bits(1_124_501_081);
    let actor = MapPoint::new(actor_x, 688.9211);
    let click = MapPoint::new(click_x, 489.68);
    let live_box = MapBBox::from_coords(actor.x - 6.0, actor.y - 4.0, actor.x + 6.0, actor.y + 4.0);

    let source_order = group_move_mercenary_box(
        live_box,
        MoveBox::from_coords(-6.0, -4.0, 6.0, 4.0),
        actor,
        actor,
        click,
        false,
    );
    let collapsed = live_box.translated(click - actor);

    assert_eq!(source_order.center().x.to_bits(), 1_124_501_081);
    assert_eq!(collapsed.center().x.to_bits(), 1_124_501_080);
}

#[test]
fn lift_formation_uses_upright_zero_centered_box() {
    let bbox = group_move_candidate_box(
        MapBBox::from_coords(90.0, 90.0, 110.0, 110.0),
        MoveBox::from_coords(-3.0, -4.0, 5.0, 6.0),
        MapPoint::new(100.0, 100.0),
        MapPoint::new(300.0, 400.0),
        true,
    );
    assert_eq!((bbox.x_min(), bbox.y_min()), (297.0, 396.0));
    assert_eq!((bbox.x_max(), bbox.y_max()), (305.0, 406.0));
}

#[test]
fn replay_goal_sector_kind_retains_lift_door_and_jump_flags() {
    assert_eq!(
        group_move_sector_kinds(SectorType::LIFT),
        (true, false, false)
    );
    assert_eq!(
        group_move_sector_kinds(SectorType::DOOR),
        (false, true, false)
    );
    assert_eq!(
        group_move_sector_kinds(SectorType::JUMP),
        (false, false, true)
    );
}

#[test]
fn recorded_route_goal_remains_independent_of_coincident_selected_overlay() {
    // Savegame_linux3/Profile003/Savegame008/replay018 frame 16221:
    // the selected overlay resolves at the click independently, while
    // The recorded group's patch-aware goal remains sector 288/L4.
    // Losing the recorded identity turns the command into a same-sector
    // move; preserving it reaches gate A*, whose failure leaves the old
    // Wait sequence installed just as movement-sequence construction does.
    let selected_overlay = Some(crate::sector::SectorNumber::new(33));
    let recorded = Some((crate::sector::SectorNumber::new(288), 4));

    assert_eq!(
        group_move_route_goal(recorded, selected_overlay, 0),
        (Some(crate::sector::SectorNumber::new(288)), 4)
    );
    assert_eq!(
        group_move_sector_kinds(SectorType::MOTION),
        (false, false, false),
        "selected-sector semantics are still derived from the overlay"
    );
}

#[test]
fn recorded_ordinary_route_keeps_door_placement_but_uses_ordinary_path() {
    assert_eq!(
        group_move_door_selection(Some(86), true, Some(false)),
        (None, false, false)
    );
    assert_eq!(
        group_move_door_selection(Some(86), true, None),
        (Some(86), true, true)
    );
    assert_eq!(
        group_move_door_selection(Some(86), true, Some(true)),
        (Some(86), true, true)
    );
    assert_eq!(
        group_move_door_selection(None, false, Some(false)),
        (None, false, false)
    );
}

#[test]
fn explicit_door_route_preserves_selected_door_over_translated_terminal_arena() {
    assert!(!group_move_masks_spatial_door_for_recorded_goal(
        true,
        Some(true),
    ));
    assert!(group_move_masks_spatial_door_for_recorded_goal(
        true,
        Some(false),
    ));
    assert!(group_move_masks_spatial_door_for_recorded_goal(true, None));
    assert!(!group_move_masks_spatial_door_for_recorded_goal(
        false,
        Some(false),
    ));
}

#[test]
fn same_topology_door_overlay_still_uses_simple_move() {
    let sector = Some(crate::sector::SectorNumber::new(50));
    let exact = crate::fast_find_grid::SectorIndex::new(12);

    assert!(group_move_uses_simple_route(
        false, true, true, sector, exact, 0, 50, exact, 0,
    ));
    assert_eq!(
        group_move_door_selection(Some(86), true, None),
        (Some(86), true, true),
        "the selected door overlay must still bypass destination authorization"
    );
}

#[test]
fn distinct_goal_door_overlay_keeps_gate_route() {
    assert!(!group_move_uses_simple_route(
        false,
        true,
        true,
        Some(crate::sector::SectorNumber::new(51)),
        None,
        0,
        50,
        None,
        0,
    ));
}

#[test]
fn duplicate_public_sector_with_distinct_exact_identity_keeps_gate_route() {
    assert!(!group_move_uses_simple_route(
        false,
        true,
        true,
        Some(crate::sector::SectorNumber::new(50)),
        crate::fast_find_grid::SectorIndex::new(13),
        0,
        50,
        crate::fast_find_grid::SectorIndex::new(12),
        0,
    ));
}

#[test]
fn recorded_gate_route_overrides_reconstructed_same_topology() {
    let sector = Some(crate::sector::SectorNumber::new(319));
    let exact = crate::fast_find_grid::SectorIndex::new(153);

    assert!(!group_move_uses_simple_route(
        true, false, true, sector, exact, 0, 319, exact, 0,
    ));
}

#[test]
fn recorded_failed_group_move_route_suppresses_live_a_star() {
    let actor = EntityId::Pc(crate::entity_id::PcId(136));
    assert_eq!(
        recorded_group_move_route_result::<Vec<crate::gate::GatePathStep>>(actor, None, 1),
        Some(None),
        "an observed Original failure is an authoritative route result"
    );
    assert_eq!(
        recorded_group_move_route_result(actor, Some(vec![7_u32]), 0),
        Some(Some(vec![7_u32]))
    );
    assert_eq!(
        recorded_group_move_route_result::<Vec<u32>>(actor, None, 0),
        None,
        "live commands with no recorded outcome still run route resolution"
    );
}

#[test]
fn player_group_move_uses_resolved_upright_click_action() {
    assert_eq!(player_group_move_action(false), OrderType::WalkingUpright);
    assert_eq!(player_group_move_action(true), OrderType::RunningUpright);
}

#[test]
fn pc_group_move_routes_through_exact_gate_graph_and_retains_numeric_control() {
    let owner = EntityId::Pc(crate::entity_id::PcId(342));
    let source_index = crate::fast_find_grid::SectorIndex::new(10).unwrap();
    let goal_index = crate::fast_find_grid::SectorIndex::new(77).unwrap();
    let source_exact = crate::position_interface::SectorHandle::new(1)
        .unwrap()
        .with_arena_index(source_index);
    let exact_door = crate::gate::Door {
        sector_out: crate::sector::SectorNumber::new(1),
        sector_in: crate::sector::SectorNumber::new(77),
        sector_out_index: Some(source_index),
        sector_in_index: Some(goal_index),
        point_out: MapPoint::new(0.0, 0.0),
        point_in: MapPoint::new(10.0, 0.0),
        ..crate::gate::Door::default()
    };
    let adapted = adapt_source_to_current_door_with_identity(
        std::slice::from_ref(&exact_door),
        crate::position_interface::DoorHandle::new(0).expect("valid door index"),
        true,
    )
    .expect("current-door route source must resolve its canonical inside endpoint");
    assert_eq!(adapted.1.get(), 77);
    assert_eq!(adapted.1.arena_index(), Some(goal_index));
    let exact_path = find_group_move_gate_path(
        std::slice::from_ref(&exact_door),
        owner,
        MapPoint::new(0.0, 0.0),
        source_exact,
        MapPoint::new(10.0, 0.0),
        crate::sector::SectorNumber::new(77),
        Some(goal_index),
        1,
        None,
        &|_| true,
        &|_| None,
    )
    .expect("PC342-style exact group move must seed the exact door endpoint");
    assert_eq!(exact_path.len(), 1);
    assert!(exact_path[0].direct);

    let numeric_door = crate::gate::Door {
        sector_out: crate::sector::SectorNumber::new(1),
        sector_in: crate::sector::SectorNumber::new(77),
        point_out: MapPoint::new(0.0, 0.0),
        point_in: MapPoint::new(10.0, 0.0),
        ..crate::gate::Door::default()
    };
    let numeric_path = find_group_move_gate_path(
        &[numeric_door],
        owner,
        MapPoint::new(0.0, 0.0),
        crate::position_interface::SectorHandle::new(1).unwrap(),
        MapPoint::new(10.0, 0.0),
        crate::sector::SectorNumber::new(77),
        None,
        1,
        None,
        &|_| true,
        &|_| None,
    )
    .expect("legacy all-numeric group-move graph remains supported");
    assert_eq!(numeric_path.len(), 1);
    assert!(numeric_path[0].direct);

    assert_eq!(
        find_group_move_gate_path(
            std::slice::from_ref(&exact_door),
            owner,
            MapPoint::new(0.0, 0.0),
            source_exact,
            MapPoint::new(20.0, 0.0),
            crate::sector::SectorNumber::new(411),
            None,
            3,
            None,
            &|_| true,
            &|_| None,
        ),
        None,
        "an unmapped mission-patch goal cannot match an exact door endpoint"
    );

    assert_eq!(
        group_move_route_goal_index(
            Some((crate::sector::SectorNumber::new(77), 1)),
            Some(crate::sector::SectorNumber::new(77)),
            Some(goal_index),
            1,
            None,
            &crate::fast_find_grid::LevelGrid::default(),
        ),
        Some(goal_index),
        "a recorded goal matching the spatial hit retains that hit's exact arena provenance"
    );
}

#[test]
#[should_panic(expected = "goal sector 77 on layer 1 lacks exact arena provenance")]
fn represented_group_move_goal_rejects_lost_exact_identity() {
    let source_index = crate::fast_find_grid::SectorIndex::new(10).unwrap();
    let goal_index = crate::fast_find_grid::SectorIndex::new(77).unwrap();
    let source = crate::position_interface::SectorHandle::new(1)
        .unwrap()
        .with_arena_index(source_index);
    let door = crate::gate::Door {
        sector_out: crate::sector::SectorNumber::new(1),
        sector_in: crate::sector::SectorNumber::new(77),
        sector_out_index: Some(source_index),
        sector_in_index: Some(goal_index),
        point_out: MapPoint::new(0.0, 0.0),
        point_in: MapPoint::new(10.0, 0.0),
        ..crate::gate::Door::default()
    };

    let _ = find_group_move_gate_path(
        &[door],
        EntityId::Pc(crate::entity_id::PcId(342)),
        MapPoint::new(0.0, 0.0),
        source,
        MapPoint::new(10.0, 0.0),
        crate::sector::SectorNumber::new(77),
        None,
        1,
        None,
        &|_| true,
        &|_| None,
    );
}

#[test]
fn legacy_unmapped_jump_goal_collapses_only_onto_its_underlying_source() {
    let source = crate::fast_find_grid::SectorIndex::new(0).unwrap();
    let mut topology = crate::engine::LegacyGridTopologyAssets::default();
    topology
        .sectors
        .resize(807, crate::engine::LegacyGridSectorAsset::NullOrOrdinary);
    topology.position_sector_numbers.resize(807, None);
    topology.position_sector_indices.resize(807, None);
    let mut level = crate::fast_find_grid::LevelGrid::default();
    level.sectors.push(replay_goal_sector(0, 0));
    let mut jump = replay_goal_sector(806, 0);
    jump.sector_type = crate::sector::SectorType::MOUSE | crate::sector::SectorType::JUMP;
    jump.underlying_sector = Some(source);
    level.sectors.push(jump.clone());

    let recognized = |exact_goal, route_outcome, door_route, door, jump, lift, all_match| {
        let retained_jump_falls_back_to_spatial = retained_jump_goal_uses_underlying_sector(
            &level,
            crate::sector::SectorNumber::new(806),
            0,
            Some(source),
        );
        legacy_unmapped_jump_goal_matches_spatial_source(
            Some(&topology),
            LegacyUnmappedJumpGoal {
                recorded_goal: Some((crate::sector::SectorNumber::new(806), 0)),
                exact_goal_index: exact_goal,
                has_recorded_route_outcome: route_outcome,
                recorded_door_route: door_route,
                is_door_click: door,
                is_jump_click: jump,
                is_lift_click: lift,
                is_valid: true,
                selected_sector_index: Some(source),
                selected_layer: 0,
                retained_jump_falls_back_to_spatial: retained_jump_falls_back_to_spatial,
                all_source_arenas_match_spatial: all_match,
            },
        )
    };

    assert!(recognized(None, false, None, false, false, false, true));
    assert!(recognized(
        None,
        false,
        Some(false),
        false,
        false,
        false,
        true
    ));
    assert!(!recognized(
        Some(source),
        false,
        None,
        false,
        false,
        false,
        true
    ));
    assert!(!recognized(None, true, None, false, false, false, true));
    assert!(!recognized(
        None,
        false,
        Some(true),
        false,
        false,
        false,
        true
    ));
    assert!(!recognized(None, false, None, true, false, false, true));
    assert!(!recognized(None, false, None, false, true, false, true));
    assert!(!recognized(None, false, None, false, false, true, true));
    assert!(!recognized(None, false, None, false, false, false, false));
    assert!(!legacy_unmapped_jump_goal_matches_spatial_source(
        Some(&topology),
        LegacyUnmappedJumpGoal {
            recorded_goal: Some((crate::sector::SectorNumber::new(806), 0)),
            exact_goal_index: None,
            has_recorded_route_outcome: false,
            recorded_door_route: Some(false),
            is_door_click: false,
            is_jump_click: false,
            is_lift_click: false,
            is_valid: true,
            selected_sector_index: Some(source),
            selected_layer: 0,
            retained_jump_falls_back_to_spatial: false,
            all_source_arenas_match_spatial: true,
        }
    ));

    level.sectors.push(jump);
    assert!(!legacy_unmapped_jump_goal_matches_spatial_source(
        Some(&topology),
        LegacyUnmappedJumpGoal {
            recorded_goal: Some((crate::sector::SectorNumber::new(806), 0)),
            exact_goal_index: None,
            has_recorded_route_outcome: false,
            recorded_door_route: None,
            is_door_click: false,
            is_jump_click: false,
            is_lift_click: false,
            is_valid: true,
            selected_sector_index: Some(source),
            selected_layer: 0,
            retained_jump_falls_back_to_spatial: retained_jump_goal_uses_underlying_sector(
                &level,
                crate::sector::SectorNumber::new(806),
                0,
                Some(source),
            ),
            all_source_arenas_match_spatial: true,
        }
    ));
}

fn cyrdach_path_waiter_doors(
    exact: bool,
) -> (
    Vec<crate::gate::Door>,
    Option<crate::fast_find_grid::SectorIndex>,
    Option<crate::fast_find_grid::SectorIndex>,
) {
    let source_index = crate::fast_find_grid::SectorIndex::new(62).unwrap();
    let shared_index = crate::fast_find_grid::SectorIndex::new(24).unwrap();
    let goal_index = crate::fast_find_grid::SectorIndex::new(27).unwrap();
    let mut doors = vec![crate::gate::Door::default(); 74];
    for door in &mut doors {
        door.active = false;
    }
    doors[73] = crate::gate::Door {
        active: true,
        sector_out: crate::sector::SectorNumber::new(24),
        sector_in: crate::sector::SectorNumber::new(62),
        sector_out_index: exact.then_some(shared_index),
        sector_in_index: exact.then_some(source_index),
        point_out: MapPoint::new(10.0, 0.0),
        point_in: MapPoint::new(0.0, 0.0),
        ..crate::gate::Door::default()
    };
    doors[18] = crate::gate::Door {
        active: true,
        sector_out: crate::sector::SectorNumber::new(24),
        sector_in: crate::sector::SectorNumber::new(27),
        sector_out_index: exact.then_some(shared_index),
        sector_in_index: exact.then_some(goal_index),
        point_out: MapPoint::new(20.0, 0.0),
        point_in: MapPoint::new(30.0, 0.0),
        ..crate::gate::Door::default()
    };
    crate::gate::build_gate_links(&mut doors);
    (
        doors,
        exact.then_some(source_index),
        exact.then_some(goal_index),
    )
}

#[test]
fn path_waiter_preflight_accepts_exact_gate_73_then_18_and_numeric_legacy() {
    for exact in [true, false] {
        let (doors, source_index, goal_index) = cyrdach_path_waiter_doors(exact);
        let path = find_ai_move_gate_path(
            &doors,
            MapPoint::new(0.0, 0.0),
            crate::position_interface::SectorHandle::new(62).unwrap(),
            source_index,
            MapPoint::new(30.0, 0.0),
            crate::position_interface::SectorHandle::new(27).unwrap(),
            goal_index,
            None,
            None,
            false,
            &|_| true,
            &|_| None,
        )
        .expect("path-waiter preflight must accept the authored gate chain");
        assert_eq!(path.len(), 2);
        assert_eq!(
            path[0].door_index,
            crate::gate::DoorIndex::new(73).expect("valid door index")
        );
        assert!(!path[0].direct);
        assert_eq!(
            path[1].door_index,
            crate::gate::DoorIndex::new(18).expect("valid door index")
        );
        assert!(path[1].direct);
    }
}

#[test]
fn path_waiter_preflight_rejects_duplicate_public_source_with_wrong_identity() {
    let (doors, _, goal_index) = cyrdach_path_waiter_doors(true);
    let duplicate_source_index = crate::fast_find_grid::SectorIndex::new(61).unwrap();
    assert!(
        find_ai_move_gate_path(
            &doors,
            MapPoint::new(0.0, 0.0),
            crate::position_interface::SectorHandle::new(62).unwrap(),
            Some(duplicate_source_index),
            MapPoint::new(30.0, 0.0),
            crate::position_interface::SectorHandle::new(27).unwrap(),
            goal_index,
            None,
            None,
            false,
            &|_| true,
            &|_| None,
        )
        .is_none()
    );
}

#[test]
fn ai_move_goal_kind_uses_exact_duplicate_sector_identity() {
    use crate::fast_find_grid::{GridSector, SectorIndex};

    let sector = |sector_type, door_index| GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type,
        layer: 2,
        sector_number: crate::sector::SectorNumber::new(59),
        door_index,
        lift_type: None,
        lift_direction: 0,
        force_crouched: false,
        building_index: None,
        low_exit_point: None,
        high_exit_point: None,
        lowest_door_index: None,
        jump_line_indices: Vec::new(),
        gate_indices: Vec::new(),
        underlying_sector: None,
    };
    let mut engine = EngineInner::new();
    let level = std::sync::Arc::make_mut(&mut engine.world.fast_grid_mut().level);
    level.sectors.push(sector(SectorType::DOOR, Some(68)));
    level
        .sectors
        .push(sector(SectorType::MOTION | SectorType::AREA, None));
    level
        .sector_number_map
        .insert(crate::sector::SectorNumber::new(59), 0);

    let exact_motion = crate::position_interface::SectorHandle::new(59)
        .unwrap()
        .with_arena_index(SectorIndex::new(1).unwrap());
    assert_eq!(
        ai_move_goal_door(&engine, exact_motion, exact_motion.arena_index()),
        None,
        "AI launch must classify the exact ordinary sector, not the conflicting public-number door overlay"
    );
    assert!(
        engine
            .grid_sector_by_number(crate::sector::SectorNumber::new(59))
            .expect("numeric compatibility sector must resolve")
            .sector_type
            .is_door(),
        "the regression requires the public-number map to select the conflicting door overlay"
    );
}

#[test]
fn non_sprite_movement_actions_return_authoritative_motion_states() {
    assert_eq!(
        non_sprite_movement_motion(OrderType::Freezing),
        Some(MotionState::InProgress)
    );
    assert_eq!(
        non_sprite_movement_motion(OrderType::PassingDoor),
        Some(MotionState::Terminated)
    );
    assert_eq!(non_sprite_movement_motion(OrderType::WalkingUpright), None);
}

#[test]
fn authored_running_action_stays_fast_on_climb_without_fast_flag() {
    assert_eq!(
        climb_lift_translation_input(OrderType::RunningUpright, false),
        OrderType::RunningUpright
    );
    assert_eq!(
        crate::sector::LiftType::Ladder.translate_climb_action(
            climb_lift_translation_input(OrderType::RunningUpright, false),
            false,
        ),
        OrderType::ClimbingLadderUpFast
    );
}

#[test]
fn concrete_door_walk_replaces_a_retired_transition_mirror() {
    let mut mirrored = OrderType::TransitionWalkingUprightRunningUpright;
    synchronize_selected_door_pass_walk_action(&mut mirrored, OrderType::RunningUpright);
    assert_eq!(mirrored, OrderType::RunningUpright);

    synchronize_selected_door_pass_walk_action(&mut mirrored, OrderType::PassingDoor);
    assert_eq!(
        mirrored,
        OrderType::RunningUpright,
        "a non-animation action point must leave the last sprite action intact"
    );
}

#[test]
fn exhausted_transition_discards_zero_destination_door_tail() {
    let mut pass = ActiveDoorPass {
        door_index: crate::gate::DoorIndex::new(67).expect("valid door index"),
        direct: false,
        position_direct: false,
        steps: [crate::element::DoorPassStep::PassingDoor].into(),
        preallocated_order_ids: [std::num::NonZeroU32::new(41)].into(),
        triggers_fired: 1,
        current_action: OrderType::TransitionWalkingUprightRunningUpright,
        current_reverse: false,
        saved_action_state: None,
    };

    discard_lazy_door_pass_following_orders(Some(&mut pass));

    assert!(
        pass.steps.is_empty(),
        "Original deletes the trailing zero-destination PassingDoor order"
    );
    assert!(pass.preallocated_order_ids.is_empty());
    assert_eq!(
        completed_door_pass_to_commit(
            true,
            Some((
                crate::gate::DoorIndex::new(67).expect("valid door index"),
                false
            ))
        ),
        None,
        "a deleted final PassingDoor cannot snap the actor to the authored door endpoint"
    );
    assert_eq!(
        completed_door_pass_to_commit(
            false,
            Some((
                crate::gate::DoorIndex::new(67).expect("valid door index"),
                false
            ))
        ),
        Some((
            crate::gate::DoorIndex::new(67).expect("valid door index"),
            false
        )),
        "an ordinarily completed door pass still performs its final position commit"
    );
}

#[test]
fn explicit_door_speed_transition_ignores_stale_walk_mirror() {
    assert_eq!(
        door_pass_sprite_animation_override(
            OrderType::TransitionWaitingUprightRunningUpright,
            Some(OrderType::WalkingUpright),
        ),
        None,
        "Original executes the concrete transition inserted by MakeFast"
    );
    assert_eq!(
        door_pass_sprite_animation_override(
            OrderType::RunningUpright,
            Some(OrderType::WalkingUpright),
        ),
        Some(OrderType::WalkingUpright),
        "concrete distance motion still accepts the active door-route animation"
    );
    assert_eq!(
        door_pass_sprite_animation_override(
            OrderType::TransitionWaitingUprightClimbingWallUp,
            Some(OrderType::TransitionWaitingUprightClimbingWallUp),
        ),
        Some(OrderType::TransitionWaitingUprightClimbingWallUp),
        "an agreeing door-authored transition mirror remains valid"
    );
}

#[test]
fn recursively_reached_climb_keeps_transition_facing() {
    assert!(!initialising_climb_uses_lift_direction(
        OrderType::ClimbingLadderUp,
        crate::sector::LiftType::Ladder,
        false,
    ));
    assert!(initialising_climb_uses_lift_direction(
        OrderType::ClimbingLadderUp,
        crate::sector::LiftType::Ladder,
        true,
    ));
}
