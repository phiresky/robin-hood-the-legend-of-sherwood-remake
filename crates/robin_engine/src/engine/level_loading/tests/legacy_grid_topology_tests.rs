use super::*;
use crate::level_data::{
    ProtoGridChunk, RawAmbushPoint, RawArcheryPoint, RawArcherySector, RawBuildingEntry, RawDoor,
    RawJumpLine, RawJumpLinePair, RawJumpZone, RawLift, RawSeekPoint, RawTacticData, SectorPolygon,
};

fn door(has_click_sector: bool) -> RawDoor {
    RawDoor {
        door_type: 0,
        active: true,
        locked_pc: false,
        unlockable: false,
        locked_npc_villain: false,
        locked_npc_civilian: false,
        locked_pc_after_patch: false,
        unlockable_after_patch: false,
        locked_npc_villain_after_patch: false,
        locked_npc_civilian_after_patch: false,
        door_sector: SectorPolygon {
            points: has_click_sector
                .then_some(vec![(0, 0), (1, 0), (0, 1)])
                .unwrap_or_default(),
        },
        point_out: (0, 0),
        sector_out: 0,
        layer_out: 0,
        point_mid: (0, 0),
        point_in: (0, 0),
        sector_in: 0,
        layer_in: 0,
    }
}

fn jump_line() -> RawJumpLine {
    RawJumpLine {
        point_a: (0, 0, 0),
        point_b: (1, 0, 0),
        jump_zone_index: 0,
    }
}

fn runtime_position_sector(
    number: i16,
    sector_type: crate::sector::SectorType,
) -> crate::fast_find_grid::GridSector {
    crate::fast_find_grid::GridSector {
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type,
        sector_number: crate::sector::SectorNumber::new(number),
        ..Default::default()
    }
}

#[test]
fn constructor_holes_only_enter_sparse_array_when_later_add_exposes_them() {
    let mut builder = LegacySectorTopologyBuilder::default();
    builder.construct();
    let door = builder.construct();
    builder.add(door, LegacyGridSectorAsset::Door { gate_index: 0 });
    builder.construct();

    assert_eq!(
        builder.slots,
        vec![
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::Door { gate_index: 0 },
        ]
    );
    assert_eq!(builder.position_sector_numbers, vec![None, None]);
    assert_eq!(builder.position_sector_indices, vec![None, None]);
}

#[test]
fn retains_mixed_door_jump_order_and_sparse_special_sector_slots() {
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.proto.grid_chunk_order = vec![
        ProtoGridChunk::Lift,
        ProtoGridChunk::Building,
        ProtoGridChunk::Jump,
    ];
    loaded.proto.lifts.push(RawLift {
        motion_area_index: 0,
        lift_type: 1,
        doors: vec![door(false), door(true)],
        direction: 0,
    });
    loaded.proto.buildings = vec![
        RawBuildingEntry::Building {
            doors: vec![door(true)],
        },
        RawBuildingEntry::StandaloneDoors {
            doors: vec![door(false)],
        },
    ];
    loaded.proto.jump_zones.push(RawJumpZone {
        polygon: SectorPolygon {
            points: vec![(0, 0), (1, 0), (0, 1)],
        },
        sector: 0,
        layer: 0,
        helper_needed: false,
    });
    loaded.proto.jump_line_pairs.push(RawJumpLinePair {
        line1: jump_line(),
        line2: jump_line(),
        jump_long: false,
    });

    let mut assets = LevelAssets::new();
    retain_legacy_grid_topology(&mut assets, &loaded, false).unwrap();
    let topology = assets.navigation.legacy_grid_topology.unwrap();

    assert_eq!(
        topology.gates,
        vec![
            LegacyGridGateAsset::Door,
            LegacyGridGateAsset::Door,
            LegacyGridGateAsset::Door,
            LegacyGridGateAsset::Door,
            LegacyGridGateAsset::Stateless,
        ]
    );
    assert_eq!(
        topology.sectors,
        vec![
            // Lift associated-sector and empty lift-door holes.
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::Door { gate_index: 1 },
            // Building is constructed before its door, then added after it.
            LegacyGridSectorAsset::Building,
            LegacyGridSectorAsset::Door { gate_index: 2 },
            // Empty standalone door exposed by later jump-zone sector registration.
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::NullOrOrdinary,
        ]
    );
    assert_eq!(
        topology.position_sector_numbers,
        vec![None, None, None, Some(3), None, None, None],
        "the building keeps its sparse Original sector number independently of its compact Rust arena index"
    );
    assert_eq!(
        topology.position_sector_indices,
        vec![
            None,
            None,
            None,
            crate::fast_find_grid::SectorIndex::new(0),
            None,
            None,
            None,
        ],
        "the sparse Original building slot retains its exact runtime arena identity"
    );
}

#[test]
fn building_doors_share_sparse_public_and_exact_arena_identity() {
    let mut assets = LevelAssets::new();
    let mut sectors = vec![LegacyGridSectorAsset::NullOrOrdinary; 250];
    sectors[249] = LegacyGridSectorAsset::Building;
    let mut public_numbers = vec![None; 250];
    public_numbers[0] = Some(95);
    public_numbers[249] = Some(249);
    let mut arena_indices = vec![None; 250];
    arena_indices[0] = crate::fast_find_grid::SectorIndex::new(0);
    arena_indices[249] = crate::fast_find_grid::SectorIndex::new(1);
    assets.navigation.legacy_grid_topology = Some(LegacyGridTopologyAssets {
        sectors,
        position_sector_numbers: public_numbers,
        position_sector_indices: arena_indices,
        ..LegacyGridTopologyAssets::default()
    });

    let mut loaded = crate::level_data::LoadedLevel::empty();
    let mut first = door(false);
    first.door_type = 1;
    first.sector_out = 0;
    let mut second = first.clone();
    second.point_out = (20, 0);
    second.point_in = (10, 0);
    loaded.proto.buildings = vec![RawBuildingEntry::Building {
        doors: vec![first, second],
    }];

    let mut engine = EngineInner::new();
    let outside = runtime_position_sector(
        95,
        crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
    );
    let mut building = runtime_position_sector(
        95,
        crate::sector::SectorType::MOTION
            | crate::sector::SectorType::AREA
            | crate::sector::SectorType::BUILDING,
    );
    building.layer = 13;
    engine.world.fast_grid_mut().level_mut().sectors = vec![outside, building];
    engine
        .world
        .fast_grid_mut()
        .level_mut()
        .sector_number_map
        .insert(crate::sector::SectorNumber::new(95), 1);

    canonicalize_building_position_sectors(
        &assets,
        &mut loaded,
        engine.world.fast_grid_mut().level_mut(),
    );
    validate_legacy_position_sector_bijection(
        assets.navigation.legacy_grid_topology.as_ref().unwrap(),
        &engine.world.fast_grid.level.sectors,
    )
    .expect("sparse public numbers and dense arena indices stay a bijection");
    engine.build_door_stage(&assets, &loaded, &MissionLevelBuildPlan::default());

    for built in &engine.script_domains.interactables.doors {
        assert_eq!(built.sector_in, crate::sector::SectorNumber::new(249));
        assert_eq!(
            built.sector_in_index,
            crate::fast_find_grid::SectorIndex::new(1),
            "every adjacent door side must retain the one constructed building-sector identity"
        );
    }
    assert_eq!(
        engine.world.fast_grid.level.sector_number_map[&crate::sector::SectorNumber::new(95)],
        0,
        "removing the obsolete dense building alias must expose the real motion sector"
    );
}

#[test]
fn door_sparse_endpoints_resolve_distinct_arena_objects_with_same_public_number() {
    let mut assets = LevelAssets::new();
    assets.navigation.legacy_grid_topology = Some(LegacyGridTopologyAssets {
        sectors: vec![
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::NullOrOrdinary,
        ],
        position_sector_numbers: vec![Some(18), Some(18)],
        position_sector_indices: vec![
            crate::fast_find_grid::SectorIndex::new(40),
            crate::fast_find_grid::SectorIndex::new(41),
        ],
        ..LegacyGridTopologyAssets::default()
    });

    let outside = EngineInner::resolve_sparse_position_sector(&assets, 0);
    let inside = EngineInner::resolve_sparse_position_sector(&assets, 1);

    assert_eq!(outside.0, crate::sector::SectorNumber::new(18));
    assert_eq!(inside.0, crate::sector::SectorNumber::new(18));
    assert_eq!(
        outside.1,
        crate::fast_find_grid::SectorIndex::new(40).unwrap()
    );
    assert_eq!(
        inside.1,
        crate::fast_find_grid::SectorIndex::new(41).unwrap()
    );
    assert_ne!(outside.1, inside.1);
}

#[test]
fn mission_element_placement_retains_sparse_sector_object_identity() {
    let mut assets = LevelAssets::new();
    assets.navigation.legacy_grid_topology = Some(LegacyGridTopologyAssets {
        sectors: vec![
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::NullOrOrdinary,
        ],
        position_sector_numbers: vec![Some(18), Some(18)],
        position_sector_indices: vec![
            crate::fast_find_grid::SectorIndex::new(40),
            crate::fast_find_grid::SectorIndex::new(41),
        ],
        ..LegacyGridTopologyAssets::default()
    });

    let exact = EngineInner::resolve_sparse_position_handle(&assets, 1);
    let mut sprite = crate::sprite::Sprite::default();
    sprite.apply_placement(
        MapPoint::new(222.0, 2401.0),
        0,
        Some(exact),
        0,
        crate::element::GameMaterial::default(),
        None,
        None,
    );

    assert_eq!(exact.get(), 18);
    assert_eq!(
        sprite.position_iface.get_sector_topology(),
        (Some(exact), crate::fast_find_grid::SectorIndex::new(41)),
        "the mission's 16-bit sector value is a sparse array slot, not public sector number 1"
    );
}

#[test]
fn tactic_seek_position_resolves_sparse_slot_to_exact_sector_object() {
    let mut assets = LevelAssets::new();
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.mission.tactic_data = Some(RawTacticData {
        reinforcement_points: Vec::new(),
        ambush_points: Vec::new(),
        seek_points: vec![RawSeekPoint {
            x: 658,
            y: 2905,
            sector: 1,
            level: 0,
            direction: 7,
        }],
        archery_sectors: Vec::new(),
    });
    let mut engine = EngineInner::new();

    // The environment stage runs before the current mission retains its
    // topology. It must not install seek positions through stale assets.
    assets.navigation.legacy_grid_topology = Some(LegacyGridTopologyAssets {
        sectors: vec![
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::NullOrOrdinary,
        ],
        position_sector_numbers: vec![Some(9), Some(9)],
        position_sector_indices: vec![
            crate::fast_find_grid::SectorIndex::new(38),
            crate::fast_find_grid::SectorIndex::new(39),
        ],
        ..LegacyGridTopologyAssets::default()
    });
    engine.load_environment_stage(&mut assets, &mut loaded, false);
    assert!(engine.ai.global.seek_points.is_empty());

    // The deferred stage observes only the newly retained topology and
    // preserves duplicate-public-sector object identity.
    assets.navigation.legacy_grid_topology = Some(LegacyGridTopologyAssets {
        sectors: vec![
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::NullOrOrdinary,
        ],
        position_sector_numbers: vec![Some(18), Some(18)],
        position_sector_indices: vec![
            crate::fast_find_grid::SectorIndex::new(40),
            crate::fast_find_grid::SectorIndex::new(41),
        ],
        ..LegacyGridTopologyAssets::default()
    });
    let tactic_position = EngineInner::resolve_sparse_position_handle(&assets, 1);
    assert_eq!(tactic_position.get(), 18);
    assert_eq!(
        tactic_position.arena_index(),
        crate::fast_find_grid::SectorIndex::new(41),
        "a tactic position must retain its sparse-slot sector identity rather than interpreting the slot as public sector 1"
    );
    engine.install_tactic_seek_points_stage(&assets, &loaded);
    let installed = &engine.ai.global.seek_points[0].position;
    assert_eq!((installed.x, installed.y), (658.0, 2905.0));
    assert_eq!(installed.sector, Some(tactic_position));
}

#[test]
fn tactic_ambush_position_resolves_sparse_slot_to_exact_sector_object() {
    let mut assets = LevelAssets::new();
    assets.navigation.legacy_grid_topology = Some(LegacyGridTopologyAssets {
        sectors: vec![
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::NullOrOrdinary,
        ],
        position_sector_numbers: vec![Some(27), Some(27)],
        position_sector_indices: vec![
            crate::fast_find_grid::SectorIndex::new(42),
            crate::fast_find_grid::SectorIndex::new(43),
        ],
        ..LegacyGridTopologyAssets::default()
    });
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.mission.tactic_data = Some(RawTacticData {
        reinforcement_points: Vec::new(),
        ambush_points: vec![RawAmbushPoint {
            x: 1120,
            y: 840,
            sector: 1,
            level: 3,
        }],
        seek_points: Vec::new(),
        archery_sectors: Vec::new(),
    });
    let mut engine = EngineInner::new();

    engine.install_tactic_ambush_points_stage(&assets, &loaded);

    let installed = &engine.ai.global.ambush_points[0].position;
    assert_eq!(
        (installed.x, installed.y, installed.level),
        (1120.0, 840.0, 3)
    );
    let sector = installed.sector.expect("ambush point sector");
    assert_eq!(sector.get(), 27);
    assert_eq!(
        sector.arena_index(),
        crate::fast_find_grid::SectorIndex::new(43),
        "the AMBU sector field is an Original sparse slot, not public sector 1"
    );
}

#[test]
fn archery_waypoint_resolves_to_exact_motion_sector_after_loading() {
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.mission.tactic_data = Some(RawTacticData {
        reinforcement_points: Vec::new(),
        ambush_points: Vec::new(),
        seek_points: Vec::new(),
        archery_sectors: vec![RawArcherySector {
            sector_ref: 25,
            polygon: SectorPolygon::default(),
            points: vec![RawArcheryPoint {
                x: 983,
                y: 1518,
                sector: 25,
                is_shooting_point: false,
                direction: 0,
            }],
        }],
    });
    let mut engine = EngineInner::new();
    engine.ai.global.archery_sectors = vec![crate::ai::SectorArchery {
        points: vec![crate::ai::PointArchery {
            position: crate::ai::Position {
                x: 983.0,
                y: 1518.0,
                sector: crate::position_interface::SectorHandle::new(25),
                level: 0,
            },
            direction: 0,
            is_shooting_point: false,
            sector_index: crate::sector::SectorNumber::new(25),
            owner: None,
        }],
        polygon: Vec::new(),
        layer: 0,
        index_first_shooting_point: None,
        index_last_shooting_point: None,
        num_shooting_points: 0,
        num_owners: 0,
    }];
    let mut motion_sector = runtime_position_sector(
        25,
        crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
    );
    motion_sector.layer = 2;
    let arena = engine.world.fast_grid_mut().add_sector(motion_sector, 2);

    engine
        .resolve_archery_topology_after_motion(&loaded)
        .expect("authored archery topology resolves after motion loading");

    let point = &engine.ai.global.archery_sectors[0].points[0].position;
    assert_eq!(point.level, 2);
    assert_eq!(point.sector.expect("archery point sector").get(), 25);
    assert_eq!(
        point.sector.and_then(|sector| sector.arena_index()),
        crate::fast_find_grid::SectorIndex::new(arena),
        "AI movement must not mix an exact actor source with a number-only archery destination"
    );
}

#[test]
fn door_ai_cache_retains_exact_endpoint_when_public_numbers_overlap() {
    let outside = crate::fast_find_grid::SectorIndex::new(40).unwrap();
    let inside = crate::fast_find_grid::SectorIndex::new(41).unwrap();
    let mut engine = EngineInner::new();
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            door_type: crate::gate::DoorType::Building,
            sector_out: crate::sector::SectorNumber::new(18),
            sector_in: crate::sector::SectorNumber::new(18),
            sector_out_index: Some(outside),
            sector_in_index: Some(inside),
            ..crate::gate::Door::default()
        });
    let reinforcement_outside = crate::fast_find_grid::SectorIndex::new(50).unwrap();
    let reinforcement_inside = crate::fast_find_grid::SectorIndex::new(51).unwrap();
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            door_type: crate::gate::DoorType::Reinforcement,
            sector_out: crate::sector::SectorNumber::new(19),
            sector_in: crate::sector::SectorNumber::new(19),
            sector_out_index: Some(reinforcement_outside),
            sector_in_index: Some(reinforcement_inside),
            ..crate::gate::Door::default()
        });

    engine.cache_door_ai_metadata();

    let cached = engine.ai.global.door_seek_infos[0]
        .position_in
        .sector
        .expect("building door cache must retain its interior sector");
    assert_eq!(cached.get(), 18);
    assert_eq!(cached.arena_index(), Some(inside));
    assert_ne!(cached.arena_index(), Some(outside));

    let reinforcement = &engine.ai.global.reinforcement_doors[0];
    let reinforcement_in = reinforcement
        .position_in
        .sector
        .expect("reinforcement cache must retain its interior sector");
    let reinforcement_out = reinforcement
        .sector_out
        .expect("reinforcement cache must retain its exterior sector");
    assert_eq!(reinforcement_in.get(), 19);
    assert_eq!(reinforcement_out.get(), 19);
    assert_eq!(reinforcement_in.arena_index(), Some(reinforcement_inside));
    assert_eq!(reinforcement_out.arena_index(), Some(reinforcement_outside));
    assert_ne!(
        reinforcement_in.arena_index(),
        reinforcement_out.arena_index(),
        "equal public reinforcement endpoints must not collapse their arena identity"
    );
}

#[test]
fn retained_position_bijection_excludes_appended_out_of_map_sector() {
    let topology = LegacyGridTopologyAssets {
        sectors: vec![LegacyGridSectorAsset::NullOrOrdinary],
        position_sector_numbers: vec![Some(7)],
        position_sector_indices: vec![crate::fast_find_grid::SectorIndex::new(0)],
        ..LegacyGridTopologyAssets::default()
    };
    let runtime = vec![
        runtime_position_sector(
            7,
            crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        ),
        // The original game appends this real identity after authored position
        // sectors. It is required for reinforcement gates, but is not
        // part of the retained authored-position prefix.
        runtime_position_sector(
            -1,
            crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        ),
    ];

    validate_legacy_position_sector_bijection(&topology, &runtime)
        .expect("the appended out-of-map sector must not extend the authored prefix");
}

#[test]
fn waypoint_sparse_slot_resolves_exact_overlapping_sector_identity() {
    use crate::level_data::{RawHikingPath, RawWaypoint, WaypointCommand};

    let mut assets = LevelAssets::new();
    assets.navigation.hiking_paths = std::sync::Arc::new(vec![RawHikingPath {
        waypoints: vec![RawWaypoint {
            x: 1432,
            y: 930,
            // the original game's serialized sparse sector slot, not the
            // public sector number shared by both runtime objects.
            sector: 2,
            level: 6,
            command: WaypointCommand::None,
        }],
    }]);
    assets.navigation.legacy_grid_topology = Some(LegacyGridTopologyAssets {
        sectors: vec![
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::NullOrOrdinary,
        ],
        position_sector_numbers: vec![Some(82), None, Some(82)],
        position_sector_indices: vec![
            crate::fast_find_grid::SectorIndex::new(0),
            None,
            crate::fast_find_grid::SectorIndex::new(1),
        ],
        ..LegacyGridTopologyAssets::default()
    });
    let runtime = vec![
        runtime_position_sector(
            82,
            crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        ),
        runtime_position_sector(
            82,
            crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        ),
    ];

    resolve_hiking_waypoint_sector_identities(&mut assets, &runtime);

    assert_eq!(assets.navigation.hiking_paths[0].waypoints[0].sector, 82);
    let exact = assets.navigation.hiking_waypoint_sectors.as_ref().unwrap()[0][0];
    assert_eq!(exact.get(), 82);
    assert_eq!(
        exact.arena_index(),
        crate::fast_find_grid::SectorIndex::new(1)
    );
}
