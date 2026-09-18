use super::*;

#[test]
fn phalanx_arrival_reads_target_position_after_state_callback() {
    use crate::engine::test_support::asm::*;
    use crate::engine::types::MissionScript;
    use crate::natives::{NativeFn, ScriptHandleCodec};
    use crate::scb::{ClassEntry, Function, ScbFile};
    let (mut engine, mut assets, ids) = fixture(&[(100.0, 100.0), (120.0, 100.0), (300.0, 300.0)]);
    let (owner, neighbour, target) = (ids[0], ids[1], ids[2]);
    let target_handle = ScriptHandleCodec::actor_handle(target);
    assets.scripts.location_count = 1;
    assets.scripts.point_count = 1;
    assets.scripts.location_positions = std::sync::Arc::new(vec![(600.0, 700.0)]);
    assets.scripts.location_layers = std::sync::Arc::new(vec![0]);
    assets.scripts.location_sectors = std::sync::Arc::new(vec![1]);
    assets.scripts.location_sector_handles = std::sync::Arc::new(vec![
        engine.get_entity(target).unwrap().element_data().sector(),
    ]);
    engine
        .world
        .entities
        .expect_entity_mut(owner, format_args!("scripted arrival"))
        .actor_data_mut()
        .unwrap()
        .script_class = "MoveShieldTarget".into();
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: crate::scb::SCB_VERSION,
            classes: vec![
                empty_startup_class("shield.scs".into()),
                ClassEntry {
                    source_file: "shield.scs".into(),
                    class_name: "MoveShieldTarget".into(),
                    size_of_member_variables: 0,
                    member_variables: vec![],
                    functions: vec![Function {
                        name: "FilterAIEvent".into(),
                        address: 0,
                        num_parameters: 3,
                        size_of_return_value: 4,
                        size_of_parameters: 12,
                        size_of_volatile: 0,
                        size_of_temporary: 8,
                    }],
                    quads: vec![
                        q_begin_function(0, 2),
                        q_aff1_get_param(0xC000, 4),
                        q_aff0_iconstant(0xC004, AiState::Attacking.state_change_event_code()),
                        q_ieq(0xC000, 0xC000, 0xC004),
                        q_if_not_zero_goto(0xC000, 7),
                        q_aff0_iconstant(0xC000, 1),
                        q_return_val(0xC000),
                        q_aff0_iconstant(0xC000, target_handle),
                        q_aff0_iconstant(0xC004, ScriptHandleCodec::location_handle_from_index(0)),
                        q_native_param(0xC000),
                        q_native_param(0xC004),
                        q_native_call(NativeFn::SetActorLocation as u32),
                        q_aff0_iconstant(0xC000, 1),
                        q_return_val(0xC000),
                        q_end_function(),
                    ],
                },
            ],
        })
        .unwrap(),
    );
    engine.attach_script_bindings(&assets);
    engine
        .scripts
        .mission
        .as_mut()
        .unwrap()
        .bind_actor(ScriptHandleCodec::actor_handle(owner), "MoveShieldTarget");
    let ai = engine.enemy_ai_mut(owner, "arrival owner");
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingRunningToPhalanx;
    ai.left_combat_neighbour = Some(AiEntityHandle::new(neighbour.index()));
    engine.ai_mut(neighbour, "arrival neighbour").primary_target =
        Some(AiEntityHandle::new(target.index()));
    assert!(engine.execute_ai_shield_expected_event(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        crate::ai::StimulusType::EventDone
    ));
    let raw = engine
        .expect_entity(target, "moved target")
        .element_data()
        .position();
    assert_eq!((raw.x, raw.y), (600.0, 700.0));
    let raise = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| sequence.elements.iter())
        .find(|element| {
            element.owner == Some(owner) && element.command == crate::element::Command::RaiseShield
        })
        .expect("arrival raises shield");
    assert!(
        matches!(raise.get_property(crate::sequence::Field::ShieldDangerPoint),
        Some(crate::sequence::FieldValue::Point3D { x, y, z })
            if x.to_bits() == raw.x.to_bits() && y.to_bits() == raw.y.to_bits() && z.to_bits() == raw.z.to_bits())
    );
}
#[test]
fn advancing_shield_uses_live_target_sector_for_indexed_route() {
    use crate::fast_find_grid::SectorIndex;
    use crate::gate::{Door, DoorIndex, build_gate_links, find_path_gates_with_sector_indices};
    use crate::sector::SectorNumber;

    let sim = crate::sim_rng::test_context();
    let arena = |index| SectorIndex::new(index).unwrap();
    let sector = |public, index| {
        crate::ai::SectorHandle::new(public)
            .unwrap()
            .with_arena_index(arena(index))
    };
    let source = Position {
        x: 100.0,
        y: 100.0,
        sector: Some(sector(0, 10)),
        level: 0,
    };
    let target = Position {
        x: 735.0,
        y: 1_659.0,
        sector: Some(sector(88, 12)),
        level: 2,
    };

    let mut doors = (0..114)
        .map(|_| Door {
            active: false,
            ..Door::default()
        })
        .collect::<Vec<_>>();
    doors[111] = Door {
        active: true,
        point_out: crate::coordinates::MapPoint::new(100.0, 100.0),
        point_in: crate::coordinates::MapPoint::new(400.0, 800.0),
        sector_out: SectorNumber::new(0),
        sector_in: SectorNumber::new(70),
        sector_out_index: Some(arena(10)),
        sector_in_index: Some(arena(11)),
        ..Door::default()
    };
    doors[113] = Door {
        active: true,
        point_out: crate::coordinates::MapPoint::new(735.0, 1_659.0),
        point_in: crate::coordinates::MapPoint::new(500.0, 1_000.0),
        sector_out: SectorNumber::new(88),
        sector_in: SectorNumber::new(70),
        sector_out_index: Some(arena(12)),
        sector_in_index: Some(arena(11)),
        ..Door::default()
    };
    build_gate_links(&mut doors);

    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(256, 256);
    engine.world.fast_grid_mut().allocate_layers(3);
    for index in 0..14 {
        let (public, layer, x, y) = match index {
            10 => (0, 0, 50.0, 50.0),
            11 => (70, 1, 350.0, 750.0),
            12 => (88, 2, 700.0, 1600.0),
            13 => (88, 2, 2000.0, 2000.0),
            _ => (100 + index as u16, 0, 2500.0 + index as f32 * 20.0, 2500.0),
        };
        let added = engine.world.fast_grid_mut().add_sector(
            square_sector(
                public as i16,
                layer,
                MapPoint::new(x, y),
                MapPoint::new(x + 200.0, y + 400.0),
            ),
            layer,
        );
        assert_eq!(added, index);
    }
    let mut ids = Vec::new();
    for position in [source, target] {
        let mut entity = make_test_ai_soldier(crate::element::Camp::Lacklandists);
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(position.x, position.y));
        entity.element_data_mut().set_sector(position.sector);
        entity.element_data_mut().set_layer(position.level);
        entity
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_coords(
                -4.0, -4.0, 4.0, 4.0,
            ));
        ids.push(engine.add_test_entity(entity));
    }
    let (owner, target_id) = (ids[0], ids[1]);
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    doors[111].layer_in = 1;
    doors[113].layer_out = 2;
    doors[113].layer_in = 1;
    engine.script_domains.interactables.doors = doors.clone();
    let ai = engine.enemy_ai_mut(owner, "advancing shield owner");
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingAdvancingWithShield;
    ai.base.primary_target = Some(AiEntityHandle::new(target_id.index()));
    assert!(engine.execute_ai_shield_expected_event(
        &sim,
        &assets,
        owner,
        crate::ai::StimulusType::EventDone
    ));
    let ai = engine.ai(owner, "shield destination");
    let destination = ai.last_goto_destination;
    assert_eq!(destination, target);
    assert!(
        ai.last_goto_flags
            .contains(GotoFlags::RUN | GotoFlags::NEAR)
    );
    assert_eq!(destination.sector.unwrap().arena_index(), Some(arena(12)));
    assert_eq!(destination.level, 2);

    let route = find_path_gates_with_sector_indices(
        &doors,
        (source.x, source.y),
        source.sector.unwrap().get(),
        source.sector.unwrap().arena_index(),
        (destination.x, destination.y),
        destination.sector.unwrap().get(),
        destination.sector.unwrap().arena_index(),
        None,
        false,
        &|_| true,
        &|_| None,
    )
    .expect("exact live target identity must launch the indexed route");
    assert_eq!(
        route
            .iter()
            .map(|step| (step.door_index, step.direct))
            .collect::<Vec<_>>(),
        vec![
            (DoorIndex::new(111).expect("valid door index"), true),
            (DoorIndex::new(113).expect("valid door index"), false)
        ]
    );

    // Public sector 88 has a distinct duplicate in the arena. Losing the
    // live target's identity would make this route unresolvable.
    assert!(
        find_path_gates_with_sector_indices(
            &doors,
            (source.x, source.y),
            source.sector.unwrap().get(),
            source.sector.unwrap().arena_index(),
            (destination.x, destination.y),
            88,
            Some(arena(13)),
            None,
            false,
            &|_| true,
            &|_| None,
        )
        .is_none()
    );
}
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
        entity
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-10.0, -5.0),
                crate::coordinates::MapVec::new(10.0, 5.0),
            ));
        let mut conversion = vec![u16::MAX; crate::order::OrderType::WaitingShield as usize + 1];
        conversion[crate::order::OrderType::WaitingShield as usize] = 0;
        entity.element_data_mut().sprite.conversion = std::sync::Arc::new(conversion);
        ids.push(engine.add_test_entity(entity));
    }
    let mut assets = LevelAssets::new();
    crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
    for &id in &ids {
        let entity = engine.get_entity_mut(id).unwrap();
        entity
            .element_data_mut()
            .set_sector_topology(Some(sector), sector.arena_index());
        entity.enemy_ai_mut().unwrap().base.owner_entity_id = Some(id);
    }
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
fn live_shield_replacement_is_assigned_before_bow_rng() {
    use crate::ai::StimulusType;
    let seed = (0..100)
        .find(|seed| {
            crate::sim_rng::u32(
                &crate::sim_rng::SimulationContext::with_seed(*seed),
                crate::sim_rng::RngSite::ShieldAdvance,
                0..4,
            ) != 0
        })
        .unwrap();
    for protected_archer in [false, true] {
        let (mut engine, assets, ids) = fixture(&[(100.0, 100.0), (200.0, 100.0)]);
        let (owner, target) = (ids[0], ids[1]);
        engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
            "shield.scs",
        ));
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = crate::element::ActionState::HoldingShield;
        engine
            .world
            .entities
            .get_mut(target)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .action_state = crate::element::ActionState::AimingWithBow;
        let ai = engine.enemy_ai_mut(owner, "shield replacement fixture");
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingProtectingWithShield;
        ai.base.primary_target = None;
        ai.list_them = vec![target.index()];
        ai.archer_behind_me = protected_archer.then_some(AiEntityHandle::new(target.index()));
        engine.enter_ai_think_frame(owner);
        let sim = crate::sim_rng::SimulationContext::with_seed(seed);
        let expected = crate::sim_rng::SimulationContext::with_seed(seed);
        if !protected_archer {
            let _ = crate::sim_rng::u32(&expected, crate::sim_rng::RngSite::ShieldAdvance, 0..4);
        }
        assert!(engine.execute_ai_shield_expected_event(
            &sim,
            &assets,
            owner,
            StimulusType::EventTimer
        ));
        let ai = engine.enemy_ai(owner, "assigned shield target");
        assert_eq!(
            ai.base.primary_target,
            Some(AiEntityHandle::new(target.index()))
        );
        assert_eq!(
            ai.base.when_does_timer_ring,
            engine.control.frame_counter + if protected_archer { 30 } else { 10 }
        );
        assert_eq!(sim.seed(), expected.seed());
    }
}

#[test]
fn live_phalanx_neighbour_selection_preserves_null_and_skips_pc() {
    let (mut engine, _, ids) = fixture(&[(100.0, 100.0), (120.0, 100.0), (140.0, 100.0)]);
    let (owner, left, right) = (ids[0], ids[1], ids[2]);
    let pc = engine.add_test_entity(crate::engine::test_support::actors::make_test_pc(
        crate::element::Posture::Upright,
    ));
    let ai = engine.enemy_ai_mut(owner, "neighbour fixture");
    ai.left_combat_neighbour = Some(AiEntityHandle::new(left.index()));
    ai.right_combat_neighbour = Some(AiEntityHandle::new(right.index()));
    ai.list_them = vec![pc.index()];
    engine.ai_mut(right, "right target").primary_target = Some(AiEntityHandle::new(pc.index()));
    assert_eq!(engine.live_phalanx_neighbour_target(owner), Some(None));
    engine
        .enemy_ai_mut(owner, "non-soldier neighbour")
        .left_combat_neighbour = Some(AiEntityHandle::new(pc.index()));
    assert_eq!(
        engine.live_phalanx_neighbour_target(owner),
        Some(Some(AiEntityHandle::new(pc.index())))
    );
    engine.ai_mut(right, "right target cleared").primary_target = None;
    assert_eq!(engine.live_phalanx_neighbour_target(owner), Some(None));
}

#[test]
fn live_cover_position_preserves_aspect_then_distance_rounding() {
    let (mut engine, assets, ids) =
        fixture(&[(1000.0, 100.0), (1072.6248, 70.348755), (1200.0, 100.0)]);
    let (owner, bearer, target) = (ids[0], ids[1], ids[2]);
    engine
        .world
        .entities
        .get_mut(bearer)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(10);
    engine.ai_mut(bearer, "cover target").primary_target =
        Some(AiEntityHandle::new(target.index()));
    engine.enemy_ai_mut(owner, "cover archer").is_archer_unit = true;
    engine.ai.standard_view_polygon_radius = 1;
    assert_eq!(
        engine.execute_ai_battle_cover(
            &crate::sim_rng::test_context(),
            &assets,
            owner,
            bearer.index()
        ),
        std::ops::ControlFlow::Continue(crate::ai::Decision::Shoot)
    );
    let cover = engine.ai(owner, "cover candidate").seek_position;
    assert_eq!(cover.x.to_bits(), 0x4488_bad1);
    assert_eq!(cover.y.to_bits(), 0x4268_b9b6);
}

#[test]
fn live_already_in_cover_decision_does_not_require_a_route() {
    let (mut engine, mut assets, ids) = fixture(&[(1123.7424, 396.0593), (1144.9557, 408.22668)]);
    let (owner, bearer) = (ids[0], ids[1]);
    engine
        .world
        .entities
        .get_mut(bearer)
        .unwrap()
        .element_data_mut()
        .set_direction_instantly(7);
    engine
        .world
        .entities
        .get_mut(owner)
        .unwrap()
        .ai_actor_data_mut()
        .unwrap()
        .number_of_arrows = 1;
    let ai = engine.enemy_ai_mut(owner, "already covered archer");
    ai.base.current_state = AiState::Attacking;
    ai.is_archer_unit = true;
    crate::engine::test_support::actors::edit_enemy_profile(&mut assets, ai, |profile| {
        profile.courage = 100
    });
    ai.shield_bearer_before_me = Some(AiEntityHandle::new(bearer.index()));
    ai.forced_next_battle_decision = crate::ai::Decision::None;
    // No navigation layers are available; this branch needs only the current
    // actor positions, so it must retain cover without attempting movement.
    engine.world.fast_grid_mut().level_mut().layers.clear();
    let (decision, cover) = engine.choose_live_battle_decision(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        crate::ai_enemy::BattleDecisionInputs {
            friends_lower_company: 0,
            soldiers_lower_pride: false,
            simple_soldiers_near: false,
            alerting_soldier_near: false,
            min_square_enemy_distance: 0,
            num_enemies_i_can_see: 1,
            friends_nearer_to_enemy: 0,
        },
    );
    assert_eq!(decision, crate::ai::Decision::Shoot);
    assert_eq!(cover, 0);
    assert_eq!(
        engine
            .world
            .entities
            .expect_enemy_ai(owner, format_args!("retained cover"))
            .shield_bearer_before_me,
        Some(AiEntityHandle::new(bearer.index()))
    );
}

#[test]
fn live_primary_selection_scores_raw_door_position_and_live_multiplicity() {
    let (mut engine, _, ids) = fixture(&[(100.0, 100.0), (160.0, 100.0), (259.0, 100.0)]);
    engine.scripts.mission = Some(crate::engine::test_support::asm::empty_mission_script(
        "selector_door.scs",
    ));
    let (owner, near, passing) = (ids[0], ids[1], ids[2]);
    let sector = engine
        .world
        .entities
        .get(passing)
        .unwrap()
        .element_data()
        .sector()
        .unwrap();
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            point_in: MapPoint::new(277.0, 100.0),
            point_out: MapPoint::new(277.0, 100.0),
            sector_in: crate::sector::SectorNumber::new(1),
            sector_out: crate::sector::SectorNumber::new(1),
            sector_in_index: sector.arena_index(),
            sector_out_index: sector.arena_index(),
            ..Default::default()
        });
    let mut pass = crate::sequence::SequenceElement::new_movement(
        1,
        crate::element::Command::PassDoor,
        Some(passing),
        crate::order::OrderType::WalkingUpright,
    );
    if let crate::sequence::SequenceElementData::Movement {
        gate_id, direction, ..
    } = &mut pass.data
    {
        *gate_id = Some(crate::gate::DoorIndex::new(0).unwrap());
        *direction = 1;
    } else {
        unreachable!()
    }
    let sequence = engine.orders.sequence_manager.insert_element(pass);
    engine
        .orders
        .sequence_manager
        .start_sequence_level(sequence);
    engine.select_sequence_element(passing, Some((sequence, 0)));
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &mut Vec::new(),
        sequence,
        0,
    );
    assert_eq!(engine.live_ai_position(passing).x, 277.0);
    engine.enemy_ai_mut(owner, "selector list").list_them = vec![near.index(), passing.index()];
    engine
        .ai
        .global
        .primary_target_multiplicity_scratch
        .insert(near.index(), 1);
    let flags = PrimaryTargetFlags::VIPS_ALLOWED | PrimaryTargetFlags::UNOCCUPIED_PREFERRED;
    assert_eq!(
        engine.select_live_ai_primary_target(owner, flags),
        Some(AiEntityHandle::new(passing.index()))
    );
    engine
        .ai
        .global
        .primary_target_multiplicity_scratch
        .insert(near.index(), 0);
    assert_eq!(
        engine.select_live_ai_primary_target(owner, flags),
        Some(AiEntityHandle::new(near.index()))
    );
    engine
        .enemy_ai_mut(owner, "empty selector list")
        .list_them
        .clear();
    assert_eq!(engine.select_live_ai_primary_target(owner, flags), None);
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
        .enemy_ai_mut(orphan, "seeking orphan")
        .base
        .current_state = AiState::Seeking;
    assert_eq!(engine.live_archers_needing_protection(&assets, owner), 1);
    engine
        .enemy_ai_mut(orphan, "default orphan")
        .base
        .current_state = AiState::Default;
    assert_eq!(engine.live_archers_needing_protection(&assets, owner), 0);
    engine
        .enemy_ai_mut(orphan, "seeking orphan")
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
    let ai = engine.enemy_ai_mut(owner, "linked shield");
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingPhalanx;
    ai.archer_behind_me = Some(AiEntityHandle::new(orphan.index()));
    ai.tower_guard = true;
    let ai = engine.enemy_ai_mut(orphan, "protected archer");
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
        .enemy_ai_mut(farther, "farther shield")
        .base
        .current_substate = Substate::AttackingProtectingWithShield;
    let sector = crate::ai::SectorHandle::new(0).unwrap();
    let ai = engine.enemy_ai_mut(nearest, "running shield");
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
    let ai = engine.ai(owner, "close formation destination");
    assert!(ai.already_on_point);
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
            .enemy_ai_mut(pair[0], "right chain")
            .right_combat_neighbour = Some(AiEntityHandle::new(pair[1].index()));
        engine
            .enemy_ai_mut(pair[1], "left chain")
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
    let (mut engine, assets, ids) = fixture(&[(100.0, 100.0)]);
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
    engine.launch_ai_raise_shield(&crate::sim_rng::test_context(), &assets, owner, stored);
    let element = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .flat_map(|sequence| &sequence.elements)
        .find(|element| {
            element.owner == Some(owner) && element.command == crate::element::Command::RaiseShield
        })
        .expect("live shield launch registers the command");
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
    let ai = engine.enemy_ai_mut(owner, "periodic shield owner");
    ai.base.current_state = AiState::Attacking;
    ai.base.current_substate = Substate::AttackingReactiontime;
    ai.base.stuck_counter = 2;
    ai.list_them = vec![enemy.index()];
    let npc = engine.ai_actor_mut(owner, "periodic visible enemy");
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
        .enemy_ai_mut(neighbour, "periodic phalanx anchor")
        .base
        .current_substate = Substate::AttackingPhalanx;
    let selected = engine.orders.sequence_manager.insert_element(
        crate::sequence::SequenceElement::new_generic(1, command, Some(owner)),
    );
    engine
        .orders
        .sequence_manager
        .start_sequence_level(selected);
    engine.select_sequence_element(owner, Some((selected, 0)));
    engine.element_in_progress(
        &crate::sim_rng::test_context(),
        &LevelAssets::new(),
        &mut Vec::new(),
        selected,
        0,
    );
    engine.install_test_order(owner, crate::order::OrderType::WaitingUpright);
    (engine, assets, owner)
}

#[test]
fn periodic_phalanx_move_is_registered_before_idle_stuck_check() {
    let (mut engine, assets, owner) =
        periodic_phalanx_fixture(500.0, crate::element::Command::Wait);
    engine.tick_periodic_ai_for_npc(&crate::sim_rng::test_context(), owner, &assets);
    let ai = engine.enemy_ai(owner, "periodic movement result");
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
        .enemy_ai_mut(owner, "attentive counter")
        .base
        .stuck_counter = 0;
    engine.tick_periodic_ai_for_npc(&crate::sim_rng::test_context(), owner, &assets);
    let ai = engine.enemy_ai(owner, "attentive periodic result");
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
    // Script lock discards the synchronous arrival decision so the watchdog
    // observes the movement result before any face-and-raise sequence.
    engine.ai_mut(owner, "locked arrival").script_locked = true;
    engine.tick_periodic_ai_for_npc(&crate::sim_rng::test_context(), owner, &assets);
    let ai = engine.enemy_ai(owner, "already-on-point result");
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
            .ai_log
            .iter()
            .any(|line| line.line_type == crate::ai::LogLineType::Event
                && line.info == crate::ai::StimulusType::EventReachPoint as u16)
    );
    assert!(ai.base.stimulus_queue.is_empty());
    assert!(
        ai.base
            .ai_log
            .iter()
            .any(|line| line.line_type == crate::ai::LogLineType::EventRefused && line.info == 2)
    );
    assert!(
        engine
            .orders
            .sequence_manager
            .element_is_about_to_be_launched(owner, crate::element::Command::EnterAttentiveMode,),
        "the state change still registers its attentive transition"
    );
    assert_eq!(
        ai.base.stuck_counter, 0,
        "the watchdog sees the pending attentive transition even though GoTo registers no Move"
    );
}
