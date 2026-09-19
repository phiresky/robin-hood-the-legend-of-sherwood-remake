use super::MissionLevelBuilder;
use crate::coordinates::MapPoint;
use crate::element::{
    ActorCivilian, ActorData, ActorPc, CivilianData, ElementData, ElementKind, Entity, HumanData,
    NpcData, PcData,
};
use crate::engine::{
    EngineInner, JumpGateAttachment, LegacyGridSectorAsset, LegacyGridTopologyAssets, LevelAssets,
    LevelLoadStaging, MissionLevelBuildError,
};
use crate::level_data::{
    RawBuildingEntry, RawBuildingTenants, RawDoor, RawLift, RawReinforcementPoint, RawTacticData,
    SectorPolygon,
};

fn door(door_type: u8) -> RawDoor {
    RawDoor {
        door_type,
        active: true,
        locked_pc: false,
        unlockable: false,
        locked_npc_villain: false,
        locked_npc_civilian: false,
        locked_pc_after_patch: false,
        unlockable_after_patch: false,
        locked_npc_villain_after_patch: false,
        locked_npc_civilian_after_patch: false,
        door_sector: SectorPolygon { points: Vec::new() },
        point_out: (0, 0),
        sector_out: 0,
        layer_out: 0,
        point_mid: (10, 20),
        point_in: (30, 40),
        sector_in: 1,
        layer_in: 0,
    }
}

fn civilian() -> Entity {
    Entity::Civilian(ActorCivilian {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorCivilian;
            initial_element.active = true;
            initial_element
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        npc: NpcData::default(),
        civilian: CivilianData::default(),
    })
}

/// Minimal retained sparse-sector table for synthetic door fixtures.
/// Door endpoints in the shipped format are indices into Original's
/// the sector collection, so tests that exercise the production builder must
/// retain the same identity table instead of relying on public-number
/// guessing.
fn door_assets(slot_count: usize, building_slot: usize) -> LevelAssets {
    assert!(building_slot < slot_count);
    let mut sectors = vec![LegacyGridSectorAsset::NullOrOrdinary; slot_count];
    sectors[building_slot] = LegacyGridSectorAsset::Building;
    let mut assets = LevelAssets::new();
    assets.navigation.legacy_grid_topology = Some(LegacyGridTopologyAssets {
        sectors,
        position_sector_numbers: (0..slot_count)
            .map(|slot| Some(i16::try_from(slot).expect("test sector slot fits i16")))
            .collect(),
        position_sector_indices: (0..slot_count)
            .map(|slot| {
                crate::fast_find_grid::SectorIndex::new(
                    u32::try_from(slot).expect("test sector slot fits u32"),
                )
            })
            .collect(),
        ..LegacyGridTopologyAssets::default()
    });
    assets
}

#[test]
fn door_stage_keeps_building_and_standalone_authored_order() {
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.proto.buildings = vec![
        RawBuildingEntry::Building {
            doors: vec![door(1), door(2)],
        },
        RawBuildingEntry::StandaloneDoors {
            doors: vec![door(3)],
        },
        RawBuildingEntry::Building {
            doors: vec![door(1)],
        },
    ];
    let builder = MissionLevelBuilder::new("ordering", true, &loaded);

    let stage = builder.door_stage(&loaded).expect("valid authored doors");

    assert_eq!(stage.authored_door_count, 4);
    assert_eq!(
        stage.building_gates,
        vec![
            vec![
                crate::natives::ScriptHandleCodec::door_handle_from_index(0),
                crate::natives::ScriptHandleCodec::door_handle_from_index(1),
            ],
            vec![crate::natives::ScriptHandleCodec::door_handle_from_index(3,)],
        ]
    );
}

#[test]
fn door_stage_rejects_illegal_standalone_type_with_context() {
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.proto.buildings = vec![RawBuildingEntry::StandaloneDoors {
        doors: vec![door(4)],
    }];
    let builder = MissionLevelBuilder::new("bad-door", true, &loaded);

    assert_eq!(
        builder.door_stage(&loaded),
        Err(MissionLevelBuildError::InvalidStandaloneDoorType {
            entry_index: 0,
            door_index: 0,
            door_type: 4,
            x: 10,
            y: 20,
        })
    );
}

#[test]
fn script_preflight_rejects_authored_level_without_startup_when_enabled() {
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.proto.buildings = vec![RawBuildingEntry::StandaloneDoors { doors: Vec::new() }];
    let builder = MissionLevelBuilder::new("missing-script", true, &loaded);

    assert_eq!(
        builder.preflight_script_binding(&EngineInner::new()),
        Err(MissionLevelBuildError::MissingMissionScript {
            mission: "missing-script".to_owned(),
        })
    );
}

#[test]
fn no_script_mode_still_constructs_doors_lifts_and_sector_links() {
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.proto.buildings = vec![
        RawBuildingEntry::StandaloneDoors {
            doors: vec![door(3)],
        },
        RawBuildingEntry::Building {
            doors: vec![door(8)],
        },
    ];
    loaded.mission.building_tenants = vec![RawBuildingTenants {
        tenant_element_indices: Vec::new(),
        arrow_reserve: false,
    }];
    let mut lift_door = door(4);
    lift_door.sector_out = 20;
    lift_door.sector_in = 21;
    loaded.proto.lifts = vec![RawLift {
        motion_area_index: 0,
        lift_type: 0,
        doors: vec![lift_door],
        direction: 0,
    }];
    let builder = MissionLevelBuilder::new("no-script", false, &loaded);
    let assets = door_assets(22, 1);
    let mut engine = EngineInner::new();

    let plan = builder
        .preflight(&engine, &assets, &loaded)
        .expect("disabled scripting must not require a mission VM");
    engine
        .build_mission_level_stages(&assets, &loaded, &plan)
        .expect("non-script domains still construct");

    assert!(engine.scripts.mission.is_none());
    assert_eq!(engine.script_domains.interactables.doors.len(), 3);
    assert_eq!(
        engine.script_domains.interactables.doors[0].door_type,
        crate::gate::DoorType::Gate
    );
    assert_eq!(
        engine.script_domains.interactables.doors[1].door_type,
        crate::gate::DoorType::Reinforcement
    );
    assert_eq!(
        engine.script_domains.interactables.doors[2].door_type,
        crate::gate::DoorType::LiftHigh
    );

    engine.cache_door_ai_metadata();
    assert_eq!(engine.ai.global.door_seek_infos.len(), 3);
    assert_eq!(engine.ai.global.reinforcement_doors.len(), 1);
    assert_eq!(
        engine.ai.global.reinforcement_doors[0].door_index,
        crate::gate::DoorIndex::new(1).expect("valid door index")
    );

    for sector_number in [0_i16, 1_i16] {
        let level = engine.world.fast_grid_mut().level_mut();
        let grid_index = level.sectors.len();
        level
            .sector_number_map
            .insert(crate::sector::SectorNumber::new(sector_number), grid_index);
        level.sectors.push(crate::fast_find_grid::GridSector {
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
            sector_number: crate::sector::SectorNumber::new(sector_number),
            ..Default::default()
        });
    }
    let grid_allocation = std::sync::Arc::as_ptr(&engine.world.fast_grid.level);
    engine.populate_sector_gates_from_doors();
    assert_eq!(
        std::sync::Arc::as_ptr(&engine.world.fast_grid.level),
        grid_allocation,
        "resolving door endpoints must not clone the uniquely owned static grid"
    );
    assert_eq!(
        engine.world.fast_grid.level.sectors[0].gate_indices,
        vec![
            crate::gate::DoorIndex::new(0).expect("valid door index"),
            crate::gate::DoorIndex::new(1).expect("valid door index")
        ]
    );
    assert_eq!(
        engine.world.fast_grid.level.sectors[1].gate_indices,
        vec![
            crate::gate::DoorIndex::new(0).expect("valid door index"),
            crate::gate::DoorIndex::new(1).expect("valid door index")
        ]
    );
}

#[test]
fn reinforcement_door_resolves_exact_out_of_map_endpoint_identity() {
    let mut engine = EngineInner::new();
    for sector_number in [7_i16, -1_i16] {
        engine
            .world
            .fast_grid_mut()
            .level_mut()
            .sectors
            .push(crate::fast_find_grid::GridSector {
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                sector_number: crate::sector::SectorNumber::new(sector_number),
                ..Default::default()
            });
    }
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            door_type: crate::gate::DoorType::Reinforcement,
            sector_in: crate::sector::SectorNumber::new(7),
            sector_out: crate::sector::SectorNumber::new(-1),
            ..Default::default()
        });

    engine.populate_sector_gates_from_doors();

    let door = &engine.script_domains.interactables.doors[0];
    assert_eq!(
        door.sector_in_index,
        crate::fast_find_grid::SectorIndex::new(0)
    );
    assert_eq!(
        door.sector_out_index,
        crate::fast_find_grid::SectorIndex::new(1)
    );
    assert_eq!(
        engine.world.fast_grid.level.sectors[0].gate_indices,
        vec![crate::gate::DoorIndex::new(0).expect("valid door index")]
    );
    assert_eq!(
        engine.world.fast_grid.level.sectors[1].gate_indices,
        vec![crate::gate::DoorIndex::new(0).expect("valid door index")]
    );
}

#[test]
fn reinforcement_install_resolves_sparse_slot_across_public_sector_collision() {
    let motion_area = |public| crate::fast_find_grid::GridSector {
        bounding_box: crate::coordinates::MapBBox::new(),
        sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
        sector_number: crate::sector::SectorNumber::new(public),
        ..Default::default()
    };
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().level_mut().sectors =
        vec![motion_area(18), motion_area(18), motion_area(-1)];
    engine
        .world
        .fast_grid_mut()
        .level_mut()
        .map_bbox
        .expand_point(MapPoint::new(0.0, 0.0));
    engine
        .world
        .fast_grid_mut()
        .level_mut()
        .map_bbox
        .expand_point(MapPoint::new(100.0, 100.0));
    let mut assets = LevelAssets::new();
    assets.navigation.legacy_grid_topology = Some(LegacyGridTopologyAssets {
        sectors: vec![
            LegacyGridSectorAsset::NullOrOrdinary,
            LegacyGridSectorAsset::NullOrOrdinary,
        ],
        position_sector_numbers: vec![Some(18), Some(18)],
        position_sector_indices: vec![
            crate::fast_find_grid::SectorIndex::new(0),
            crate::fast_find_grid::SectorIndex::new(1),
        ],
        ..LegacyGridTopologyAssets::default()
    });
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.mission.tactic_data = Some(RawTacticData {
        reinforcement_points: vec![RawReinforcementPoint {
            x: 10,
            y: 20,
            direction: 14,
            action: 0,
            obstacle_index: 0,
            sector: 1,
            layer: 0,
        }],
        ambush_points: Vec::new(),
        seek_points: Vec::new(),
        archery_sectors: Vec::new(),
    });

    engine.install_reinforcement_doors_stage(&assets, &loaded);

    let door = &engine.script_domains.interactables.doors[0];
    let (computed_border, computed_outside) = crate::natives::compute_border_point_bbox(
        engine.world.fast_grid.level.map_bbox,
        (10.0, 20.0),
        14,
    );
    assert_ne!(
        computed_outside.0,
        computed_outside.0.trunc(),
        "fixture must exercise Original's float-to-SWORD narrowing"
    );
    assert_eq!(
        door.point_mid,
        MapPoint::new(
            computed_border.0 as i16 as f32,
            computed_border.1 as i16 as f32,
        )
    );
    assert_eq!(
        door.point_out,
        MapPoint::new(
            computed_outside.0 as i16 as f32,
            computed_outside.1 as i16 as f32,
        )
    );
    assert_eq!(door.sector_in, crate::sector::SectorNumber::new(18));
    assert_eq!(
        door.sector_in_index,
        crate::fast_find_grid::SectorIndex::new(1),
        "the authored sparse slot must select the second arena object despite equal public numbers"
    );
    assert_eq!(door.sector_out, crate::sector::SectorNumber::new(-1));
    assert_eq!(
        door.sector_out_index,
        crate::fast_find_grid::SectorIndex::new(2)
    );

    engine.cache_door_ai_metadata();
    let cached_inside = engine.ai.global.door_seek_infos[0]
        .position_in
        .sector
        .expect("reinforcement initialization must cache its inside position");
    assert_eq!(cached_inside.get(), 18);
    assert_eq!(
        cached_inside.arena_index(),
        crate::fast_find_grid::SectorIndex::new(1),
        "the init cache must receive the exact sparse-slot identity"
    );
    assert_eq!(
        engine.ai.global.reinforcement_doors[0]
            .position_in
            .sector
            .and_then(|sector| sector.arena_index()),
        crate::fast_find_grid::SectorIndex::new(1)
    );
}

#[test]
fn no_script_mode_still_attaches_jump_gates() {
    let mut engine = EngineInner::new();
    let mut staging = LevelLoadStaging::default();
    staging.attachments.jump_gates.push(JumpGateAttachment {
        point_out: MapPoint::new(1.0, 2.0),
        point_in: MapPoint::new(3.0, 4.0),
        layer_out: 0,
        layer_in: 1,
        sector_out: crate::sector::SectorNumber::new(10),
        sector_in: crate::sector::SectorNumber::new(11),
        sector_out_index: crate::fast_find_grid::SectorIndex::new(10).unwrap(),
        sector_in_index: crate::fast_find_grid::SectorIndex::new(11).unwrap(),
        jump_line_out: 7,
        jump_line_in: 8,
        jump_line_in_helper_needed: false,
        jump_line_out_helper_needed: true,
        penalty: 9.0,
    });

    engine
        .attach_jump_gates(&mut staging)
        .expect("jump gates do not require a mission VM");

    assert!(engine.scripts.mission.is_none());
    assert!(staging.attachments.jump_gates.is_empty());
    assert_eq!(engine.script_domains.interactables.doors.len(), 1);
    assert_eq!(
        engine.script_domains.interactables.doors[0].gate_type,
        crate::gate::GateType::Jump
    );
}

#[test]
fn building_stage_requires_one_tenant_record_per_building() {
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.proto.buildings = vec![RawBuildingEntry::Building {
        doors: vec![door(1)],
    }];
    loaded.mission.building_tenants = vec![
        RawBuildingTenants {
            tenant_element_indices: Vec::new(),
            arrow_reserve: false,
        },
        RawBuildingTenants {
            tenant_element_indices: Vec::new(),
            arrow_reserve: true,
        },
    ];
    let builder = MissionLevelBuilder::new("bad-tenants", true, &loaded);

    let error = builder
        .building_stage(&EngineInner::new(), &loaded)
        .expect_err("mismatched tenant table must fail");
    assert_eq!(
        error,
        MissionLevelBuildError::BuildingTenantCountMismatch {
            tenant_count: 2,
            building_count: 1,
        }
    );
}

#[test]
fn building_without_doors_is_valid_when_it_has_no_tenants() {
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.proto.buildings = vec![RawBuildingEntry::Building { doors: Vec::new() }];
    loaded.mission.building_tenants = vec![RawBuildingTenants {
        tenant_element_indices: Vec::new(),
        arrow_reserve: true,
    }];
    let builder = MissionLevelBuilder::new("empty-building", false, &loaded);

    let stage = builder
        .building_stage(&EngineInner::new(), &loaded)
        .expect("an empty building does not need an attachment door");

    assert_eq!(stage.attachments.len(), 1);
    assert_eq!(stage.attachments[0].first_door_index, None);
    assert!(stage.attachments[0].arrow_reserve);
}

#[test]
fn building_trap_tenant_uses_canonical_adapted_first_door() {
    let mut loaded = crate::level_data::LoadedLevel::empty();
    loaded.proto.buildings = vec![
        RawBuildingEntry::StandaloneDoors {
            doors: vec![door(3)],
        },
        RawBuildingEntry::Building {
            doors: vec![door(2)],
        },
    ];
    loaded.mission.building_tenants = vec![RawBuildingTenants {
        tenant_element_indices: vec![1],
        arrow_reserve: false,
    }];
    let builder = MissionLevelBuilder::new("trap-tenant", false, &loaded);
    let assets = door_assets(2, 1);
    let mut engine = EngineInner::new();
    let carried_id = engine.add_test_entity(civilian());
    {
        let carried = engine.elem_mut(carried_id);
        carried.set_layer(2);
        carried.set_sector(crate::position_interface::SectorHandle::new(12));
        carried.set_position_map(MapPoint::new(80.0, 90.0));
    }
    engine.add_test_entity(Entity::Pc(ActorPc {
        element: {
            let mut initial_element = ElementData::default();
            initial_element.kind = ElementKind::ActorPc;
            initial_element.active = true;
            initial_element
        },
        actor: ActorData::default(),
        human: HumanData::default(),
        pc: PcData {
            carried: Some(carried_id),
            ..Default::default()
        },
    }));

    let plan = builder
        .preflight(&engine, &assets, &loaded)
        .expect("valid trap tenant plan");
    assert_eq!(
        plan.buildings.attachments[0].first_door_index,
        Some(crate::gate::DoorIndex::new(1).expect("valid door index"))
    );
    engine
        .build_mission_level_stages(&assets, &loaded, &plan)
        .expect("construct canonical trap door");
    let adapted_point = engine.script_domains.interactables.doors[1].point_in;
    let adapted_sector = engine.script_domains.interactables.doors[1].sector_in;
    let adapted_sector_index = engine.script_domains.interactables.doors[1]
        .sector_in_index
        .expect("canonical building door retains its interior arena identity");
    assert_ne!(adapted_point, MapPoint::new(30.0, 40.0));

    engine
        .attach_mission_level_stage(&plan)
        .expect("attach tenant through canonical door");

    let (_, tenant) = engine
        .world
        .entities
        .get_legacy_slot(1)
        .expect("tenant remains in authored slot");
    assert_eq!(
        tenant.element_data().sprite.position_iface.map_position(),
        adapted_point
    );
    let tenant_sector = tenant
        .element_data()
        .sector()
        .expect("attached building tenant has a sector");
    assert_eq!(tenant_sector.get(), u16::from(adapted_sector));
    assert_eq!(tenant_sector.arena_index(), Some(adapted_sector_index));
    assert!(!tenant.element_data().active);

    let carried = engine.ent(carried_id);
    assert!(
        carried.element_data().active,
        "occupant initialization changes the tenant's active state, not its carried actor's"
    );
    let carried_sector = carried
        .element_data()
        .sector()
        .expect("topology changes propagate the building sector to carried actors");
    assert_eq!(carried_sector.get(), u16::from(adapted_sector));
    assert_eq!(carried_sector.arena_index(), Some(adapted_sector_index));
    assert_eq!(
        carried.element_data().layer(),
        tenant.element_data().layer()
    );
    assert_eq!(
        carried.element_data().position_map(),
        MapPoint::new(80.0, 90.0),
        "occupant initialization changes only the tenant's position after propagated topology"
    );
}
