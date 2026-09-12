use super::*;

fn ordinary_sector(number: i16) -> crate::fast_find_grid::GridSector {
    crate::fast_find_grid::GridSector {
        points: Vec::new(),
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::AREA | crate::sector::SectorType::MOTION,
        layer: 0,
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

#[test]
fn officer_forecast_keeps_exact_sector_for_gate18_route() {
    let arena = |value| crate::fast_find_grid::SectorIndex::new(value).unwrap();
    let exact = |public, index| {
        SectorHandle::new(public)
            .unwrap()
            .with_arena_index(arena(index))
    };
    let input = ForecastInput {
        position_map_x: 2142.0,
        position_map_y: 1001.0,
        sector: 27,
        sector_handle: Some(exact(27, 0)),
        layer: 0,
        direction: 0,
        forecasted_movement_z: 0.0,
        door_pass: None,
        passing_door_directly: false,
    };
    let sectors = vec![ordinary_sector(27)];
    let forecast = prepare_forecast_destination_for_ia(
        &input,
        &[],
        &sectors,
        &std::collections::HashMap::new(),
    )
    .resolve(&crate::sim_rng::test_context());
    assert_eq!(forecast.position.sector, Some(exact(27, 0)));

    let door = crate::gate::Door {
        active: true,
        point_out: MapPoint::new(2133.0, 1215.0),
        point_in: MapPoint::new(2136.0, 1198.0),
        sector_out: crate::sector::SectorNumber::new(24),
        sector_in: crate::sector::SectorNumber::new(27),
        sector_out_index: Some(arena(1)),
        sector_in_index: Some(arena(0)),
        ..crate::gate::Door::default()
    };
    let goal = forecast
        .position
        .sector
        .expect("forecast keeps target sector");
    let path = crate::gate::find_path_gates_with_sector_indices(
        std::slice::from_ref(&door),
        (1731.0, 1556.0),
        24,
        Some(arena(1)),
        (forecast.position.x, forecast.position.y),
        u16::from(goal),
        goal.arena_index(),
        None,
        false,
        &|_| true,
        &|_| None,
    )
    .expect("Soldier94 must route to Officer71 through gate18");
    assert_eq!(path.len(), 1);
    assert_eq!(
        path[0].door_index,
        crate::gate::DoorIndex::new(0).expect("valid door index")
    );
    assert!(path[0].direct);

    assert!(
        crate::gate::find_path_gates_with_sector_indices(
            std::slice::from_ref(&door),
            (1731.0, 1556.0),
            24,
            // Same public source number, distinct arena object: never
            // seed gate18 through a numeric alias.
            Some(arena(2)),
            (forecast.position.x, forecast.position.y),
            u16::from(goal),
            goal.arena_index(),
            None,
            false,
            &|_| true,
            &|_| None,
        )
        .is_none()
    );
}

#[test]
fn number_only_forecast_input_remains_number_only() {
    let input = ForecastInput {
        position_map_x: 10.0,
        position_map_y: 20.0,
        sector: 27,
        sector_handle: None,
        layer: 0,
        direction: 3,
        forecasted_movement_z: 0.0,
        door_pass: None,
        passing_door_directly: false,
    };
    let mut sector_map = std::collections::HashMap::new();
    sector_map.insert(crate::sector::SectorNumber::new(27), 0);
    let forecast =
        prepare_forecast_destination_for_ia(&input, &[], &[ordinary_sector(27)], &sector_map)
            .resolve(&crate::sim_rng::test_context());
    assert_eq!(forecast.position.sector, SectorHandle::new(27));
    assert_eq!(forecast.position.sector.unwrap().arena_index(), None);
}

#[test]
fn door_lift_and_building_forecasts_keep_canonical_endpoint_identity() {
    let arena = |value| crate::fast_find_grid::SectorIndex::new(value).unwrap();
    let exact = |public, index| {
        SectorHandle::new(public)
            .unwrap()
            .with_arena_index(arena(index))
    };
    let base_input = ForecastInput {
        position_map_x: 100.0,
        position_map_y: 200.0,
        sector: 24,
        sector_handle: Some(exact(24, 1)),
        layer: 0,
        direction: 0,
        forecasted_movement_z: 0.0,
        door_pass: None,
        passing_door_directly: false,
    };

    let crossing = crate::gate::Door {
        sector_out: crate::sector::SectorNumber::new(24),
        sector_in: crate::sector::SectorNumber::new(27),
        sector_out_index: Some(arena(1)),
        sector_in_index: Some(arena(0)),
        point_in: MapPoint::new(300.0, 400.0),
        ..crate::gate::Door::default()
    };
    let door_forecast = prepare_forecast_destination_for_ia(
        &ForecastInput {
            door_pass: Some((
                crate::gate::DoorIndex::new(0).expect("valid door index"),
                true,
            )),
            ..base_input
        },
        std::slice::from_ref(&crossing),
        &[ordinary_sector(27)],
        &std::collections::HashMap::new(),
    )
    .resolve(&crate::sim_rng::test_context());
    assert_eq!(door_forecast.position.sector, Some(exact(27, 0)));

    let mut lift_grid_sector = ordinary_sector(42);
    lift_grid_sector.sector_type = crate::sector::SectorType::LIFT;
    lift_grid_sector.lift_type = Some(crate::sector::LiftType::Wall);
    lift_grid_sector.gate_indices = vec![
        crate::gate::DoorIndex::new(0).expect("valid door index"),
        crate::gate::DoorIndex::new(1).expect("valid door index"),
        crate::gate::DoorIndex::new(2).expect("valid door index"),
    ];
    let lift_doors = [
        crate::gate::Door {
            owning_lift_sector: Some(crate::sector::SectorNumber::new(42)),
            // Wall-lift gates can expose associated motion sectors here;
            // neither endpoint is the owning lift sector.
            sector_in: crate::sector::SectorNumber::new(70),
            sector_out: crate::sector::SectorNumber::new(5),
            sector_in_index: Some(arena(3)),
            sector_out_index: Some(arena(1)),
            point_out: MapPoint::new(10.0, 100.0),
            ..crate::gate::Door::default()
        },
        crate::gate::Door {
            owning_lift_sector: Some(crate::sector::SectorNumber::new(42)),
            sector_in: crate::sector::SectorNumber::new(71),
            sector_out: crate::sector::SectorNumber::new(6),
            sector_in_index: Some(arena(4)),
            sector_out_index: Some(arena(2)),
            point_out: MapPoint::new(20.0, 10.0),
            ..crate::gate::Door::default()
        },
        // Grid gate lists include linked foreign doors as well as the
        // doors embedded in the lift proto. Its more-extreme Y must not
        // replace the lift-owned high endpoint selected by Original.
        crate::gate::Door {
            owning_lift_sector: None,
            sector_in: crate::sector::SectorNumber::new(42),
            sector_out: crate::sector::SectorNumber::new(99),
            sector_in_index: Some(arena(0)),
            sector_out_index: Some(arena(5)),
            point_out: MapPoint::new(30.0, -100.0),
            ..crate::gate::Door::default()
        },
    ];
    let lift_forecast = prepare_forecast_destination_for_ia(
        &ForecastInput {
            sector: 42,
            sector_handle: Some(exact(42, 0)),
            forecasted_movement_z: 1.0,
            ..base_input
        },
        &lift_doors,
        &[lift_grid_sector],
        &std::collections::HashMap::new(),
    )
    .resolve(&crate::sim_rng::test_context());
    assert_eq!(lift_forecast.position.sector, Some(exact(6, 2)));

    let mut building_grid_sector = ordinary_sector(50);
    building_grid_sector.sector_type = crate::sector::SectorType::BUILDING;
    let building_doors = [
        crate::gate::Door {
            sector_in: crate::sector::SectorNumber::new(50),
            sector_out: crate::sector::SectorNumber::new(24),
            sector_in_index: Some(arena(0)),
            sector_out_index: Some(arena(1)),
            ..crate::gate::Door::default()
        },
        crate::gate::Door {
            sector_in: crate::sector::SectorNumber::new(50),
            sector_out: crate::sector::SectorNumber::new(27),
            sector_in_index: Some(arena(0)),
            sector_out_index: Some(arena(2)),
            ..crate::gate::Door::default()
        },
    ];
    let building_forecast = prepare_forecast_destination_for_ia(
        &ForecastInput {
            sector: 50,
            sector_handle: Some(exact(50, 0)),
            door_pass: Some((
                crate::gate::DoorIndex::new(0).expect("valid door index"),
                true,
            )),
            passing_door_directly: true,
            ..base_input
        },
        &building_doors,
        &[building_grid_sector],
        &std::collections::HashMap::new(),
    )
    .resolve(&crate::sim_rng::test_context());
    assert_eq!(building_forecast.position.sector, Some(exact(27, 2)));
}
