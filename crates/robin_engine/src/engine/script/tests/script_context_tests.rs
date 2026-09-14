use super::*;
use crate::engine::test_support::asm::{STARTUP_CLASS, empty_mission_script};
use crate::scb::{ClassEntry, SCB_VERSION, ScbFile};

#[test]
fn fresh_callback_driver_obeys_live_vm_depth_and_ignores_receiver_guards() {
    use crate::engine::test_support::asm::{
        empty_startup_class, q_aff0_iconstant, q_begin_function, q_end_function, q_return_val,
    };
    let mut class = empty_startup_class("callback_depth.scs".into());
    class.functions.push(crate::scb::Function {
        name: "FilterAIEvent".into(),
        address: 0,
        num_parameters: 0,
        size_of_return_value: 4,
        size_of_parameters: 0,
        size_of_volatile: 0,
        size_of_temporary: 4,
    });
    class.quads = vec![
        q_begin_function(0, 4),
        q_aff0_iconstant(0xc000, 17),
        q_return_val(0xc000),
        q_end_function(),
    ];
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(
        MissionScript::from_scb(ScbFile {
            version: SCB_VERSION,
            classes: vec![class],
        })
        .unwrap(),
    );
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let sim = crate::sim_rng::test_context();
    let receiver = crate::natives::ScriptCallFrame::actor(77);
    let outer = crate::natives::ScriptCallFrame::default();
    let limit = usize::from(crate::natives::MAX_NESTED_CALL_DEPTH);
    let script = engine.scripts.mission.as_mut().unwrap();
    script.push_active_driver_frame(receiver, false);
    for _ in 0..limit - 1 {
        script.push_active_driver_frame(outer, true);
    }

    // Direct gameplay callbacks re-enter through this public driver with a
    // fresh local Vec while the outer activation guards remain installed.
    let invoke = |engine: &mut EngineInner| {
        engine.call_script_vm(
            &sim,
            &assets,
            ScriptVmKey::Global,
            "FilterAIEvent",
            &[],
            outer,
        )
    };
    assert_eq!(invoke(&mut engine).unwrap(), 17);
    let script = engine.scripts.mission.as_mut().unwrap();
    assert_eq!(script.active_vm_depth(), limit - 1);
    assert_eq!(script.active_call_frame_count(), limit);
    script.push_active_driver_frame(outer, true);
    let error = invoke(&mut engine).expect_err("live caller activations must exhaust the limit");
    assert!(error.contains("depth limit"), "{error}");
    let script = engine.scripts.mission.as_mut().unwrap();
    assert_eq!(script.active_vm_depth(), limit);
    script.pop_active_driver_frame(outer);
    assert_eq!(
        invoke(&mut engine).unwrap(),
        17,
        "unwinding reopens one slot"
    );

    let script = engine.scripts.mission.as_mut().unwrap();
    for _ in 0..limit - 1 {
        script.pop_active_driver_frame(outer);
    }
    script.pop_active_driver_frame(receiver);
    script.assert_no_active_call_frames();
}

#[test]
fn repeated_patch_target_skips_one_shot_vm_and_respects_config_and_locks() {
    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.control.sim_config.reversible_background_patches = true;
    let mut patch = crate::patch::Patch::new();
    patch.active = true;
    patch.applied = true;
    patch.repeat_activation = Some((42, "ActivatedBySword".into()));
    engine.script_domains.interactables.patches.push(patch);
    let activate = |engine: &mut EngineInner| {
        engine.call_script_vm_inner(
            &sim,
            &assets,
            ScriptVmKey::Target(42),
            "ActivatedBySword",
            &[1],
            crate::natives::ScriptCallFrame::default(),
            &mut Vec::new(),
        )
    };
    // No VM is installed: succeeding proves mission-only effects cannot
    // run again, even for targets whose scripts set a one-shot guard.
    assert_eq!(activate(&mut engine).unwrap(), 1);
    assert!(!engine.script_domains.interactables.patches[0].applied);
    assert_eq!(activate(&mut engine).unwrap(), 1);
    assert!(engine.script_domains.interactables.patches[0].applied);
    engine.script_domains.interactables.patches[0].locked = true;
    assert_eq!(activate(&mut engine).unwrap(), 1);
    assert!(engine.script_domains.interactables.patches[0].applied);
    engine.control.sim_config.reversible_background_patches = false;
    assert!(
        activate(&mut engine).is_err(),
        "parity mode must dispatch the original VM"
    );
}

#[test]
fn target_callback_discovers_two_patch_bindings_and_restores_them_from_save() {
    use crate::vm::{Opcode, Quad};
    let instruction = |opcode: Opcode, symbol: u16, immediate: i32| {
        let mut operands = [0; 8];
        operands[0..2].copy_from_slice(&symbol.to_le_bytes());
        operands[4..8].copy_from_slice(&immediate.to_le_bytes());
        Quad {
            operation: opcode as u8,
            operands,
        }
    };
    let mut begin = instruction(Opcode::BeginFunction, 0, 0);
    begin.operands[2..4].copy_from_slice(&1u16.to_le_bytes());
    let mut quads = vec![begin];
    for index in 0..2 {
        quads.push(instruction(
            Opcode::Aff0IConstant,
            0xC000,
            crate::natives::ScriptHandleCodec::patch_handle_from_index(index),
        ));
        quads.push(instruction(Opcode::NativeParam, 0xC000, 0));
        let mut native = instruction(Opcode::NativeCall, 0, 0);
        native.operands[0..4]
            .copy_from_slice(&(crate::natives::NativeFn::ApplyPatch as u32).to_le_bytes());
        quads.push(native);
    }
    quads.push(instruction(Opcode::Aff0IConstant, 0xC000, 1));
    quads.push(instruction(Opcode::ReturnVal, 0xC000, 0));
    quads.push(instruction(Opcode::EndFunction, 0, 0));
    let class = ClassEntry {
        source_file: "patch_trigger_test.scs".into(),
        class_name: STARTUP_CLASS.into(),
        size_of_member_variables: 0,
        member_variables: Vec::new(),
        functions: vec![crate::scb::Function {
            name: "ActivatedBySword".into(),
            address: 0,
            num_parameters: 1,
            size_of_return_value: 4,
            size_of_parameters: 4,
            size_of_volatile: 0,
            size_of_temporary: 4,
        }],
        quads,
    };
    let mut script = MissionScript::from_scb(ScbFile {
        version: SCB_VERSION,
        classes: vec![class],
    })
    .unwrap();
    let instance = script.manager.create_instance(STARTUP_CLASS).unwrap();
    script.target_instances.insert(42, instance);
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(script);
    engine.control.sim_config.reversible_background_patches = true;
    engine.script_domains.interactables.patches = (0..2)
        .map(|_| {
            let mut patch = crate::patch::Patch::new();
            patch.active = true;
            patch
        })
        .collect();
    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let activate = |engine: &mut EngineInner| {
        engine.call_script_vm_inner(
            &sim,
            &assets,
            ScriptVmKey::Target(42),
            "ActivatedBySword",
            &[1],
            crate::natives::ScriptCallFrame::default(),
            &mut Vec::new(),
        )
    };
    assert_eq!(activate(&mut engine).unwrap(), 1);
    for patch in &engine.script_domains.interactables.patches {
        assert!(patch.applied);
        assert_eq!(
            patch.repeat_activation,
            Some((42, "ActivatedBySword".into()))
        );
    }
    let saved = serde_json::to_string(&engine.script_domains.interactables.patches).unwrap();
    engine.script_domains.interactables.patches = serde_json::from_str(&saved).unwrap();
    // Removing the original callback proves repeats cannot rerun its
    // mission-only messages, even after a serialized save round trip.
    engine.scripts.mission = None;
    assert_eq!(activate(&mut engine).unwrap(), 1);
    assert!(
        engine
            .script_domains
            .interactables
            .patches
            .iter()
            .all(|patch| !patch.applied)
    );
    assert_eq!(activate(&mut engine).unwrap(), 1);
    assert!(
        engine
            .script_domains
            .interactables
            .patches
            .iter()
            .all(|patch| patch.applied)
    );
}

#[test]
fn put_actor_in_building_retains_exact_sector_across_special_layer() {
    let mut engine = EngineInner::new();
    engine.world.fast_grid_mut().size_map(8, 8);
    engine.world.fast_grid_mut().allocate_layers(8);
    let lift_layer = engine.world.fast_grid.lift_layer();
    let special_layer = engine.world.fast_grid.level.special_layer;
    assert_ne!(lift_layer, special_layer);

    let public = crate::sector::SectorNumber::new(353);
    let arena_raw = engine.world.fast_grid_mut().add_sector(
        crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::MOTION
                | crate::sector::SectorType::AREA
                | crate::sector::SectorType::BUILDING,
            layer: lift_layer,
            sector_number: public,
            door_index: None,
            lift_type: None,
            lift_direction: 0,
            force_crouched: false,
            building_index: crate::sector::BuildingIdx::new(0),
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        },
        lift_layer,
    );
    let arena = crate::fast_find_grid::SectorIndex::new(arena_raw)
        .expect("test building arena index is valid");
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            point_in: crate::coordinates::MapPoint::new(1222.0, 2078.0),
            layer_in: lift_layer,
            sector_in: public,
            sector_in_index: Some(arena),
            ..Default::default()
        });
    engine.script_domains.buildings.gates = vec![vec![
        crate::natives::ScriptHandleCodec::door_handle_from_index(0),
    ]];
    engine.script_domains.buildings.occupants = vec![Vec::new()];
    engine.ai.global.houses.push(crate::ai::House {
        building_index: crate::sector::BuildingIdx::new(0),
        ..Default::default()
    });

    let mut carried_element = {
        let mut initial_element = crate::element::ElementData::default();
        initial_element.kind = crate::element::ElementKind::ActorCivilian;
        initial_element.active = true;
        initial_element
    };
    carried_element.set_layer(2);
    carried_element.set_sector(crate::position_interface::SectorHandle::new(12));
    carried_element.set_position_map(crate::coordinates::MapPoint::new(80.0, 90.0));
    let carried_id = engine.add_test_entity(crate::element::Entity::Civilian(
        crate::element::ActorCivilian {
            element: carried_element,
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            civilian: Default::default(),
        },
    ));
    let actor_id = engine.add_test_entity(crate::element::Entity::Pc(crate::element::ActorPc {
        element: {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ActorPc;
            initial_element.active = true;
            initial_element
        },
        actor: Default::default(),
        human: Default::default(),
        pc: crate::element::PcData {
            carried: Some(carried_id),
            ..Default::default()
        },
    }));
    let actor_handle = crate::natives::ScriptHandleCodec::actor_handle(actor_id);
    engine.script_domains.buildings.occupants[0].push(actor_handle);
    engine.script_domains.buildings.actor_building.insert(
        actor_handle,
        crate::natives::ScriptHandleCodec::building_handle_from_index(0),
    );
    engine.put_actor_in_building(
        actor_handle,
        crate::natives::ScriptHandleCodec::building_handle_from_index(0),
    );

    let actor = engine
        .get_entity(actor_id)
        .expect("scripted building occupant remains live");
    assert_eq!(actor.element_data().layer(), special_layer);
    let sector = actor
        .element_data()
        .sector()
        .expect("scripted building occupant retains a sector");
    assert_eq!(sector.get(), u16::from(public));
    assert_eq!(sector.arena_index(), Some(arena));
    assert_eq!(
        crate::engine::ai::ai_view_position_sector(&engine, actor.element_data()),
        Some(sector),
        "AI Position consumes the retained building pointer without trying to recover it from the actor's different layer"
    );

    let carried = engine
        .get_entity(carried_id)
        .expect("carried actor remains live after recursive building entry");
    assert!(!carried.element_data().active);
    assert!(carried.element_data().hidden_in_building);
    assert_eq!(carried.element_data().layer(), 2);
    let carried_sector = carried.element_data().sector().unwrap();
    assert_eq!(carried_sector.get(), 12);
    assert_eq!(carried_sector.arena_index(), None);
    assert_eq!(
        carried.element_data().position_map(),
        crate::coordinates::MapPoint::new(80.0, 90.0),
        "putting an actor in a building changes only that actor's topology and position"
    );
    assert_eq!(
        engine.script_domains.buildings.occupants[0],
        [
            crate::natives::ScriptHandleCodec::actor_handle(actor_id),
            crate::natives::ScriptHandleCodec::actor_handle(carried_id),
        ],
        "scripted entry and its carried occupant must reach indoor enemy alerts in game order"
    );
}

#[test]
fn assign_post_engine_boundary_retains_exact_return_to_duty_sector() {
    let mut engine = EngineInner::new();
    let assets = LevelAssets::new();
    let owner = engine.add_test_entity(crate::engine::test_support::actors::make_test_ai_soldier(
        crate::element::Camp::Royalists,
    ));
    engine
        .world
        .entities
        .expect_ai_controller_mut(owner, format_args!("assigned post fixture"))
        .script_locked = true;
    let arena = crate::fast_find_grid::SectorIndex::new(97).unwrap();
    let exact = crate::position_interface::SectorHandle::new(97)
        .unwrap()
        .with_arena_index(arena);

    engine.execute_ai_assign_post(
        &crate::sim_rng::test_context(),
        &assets,
        owner,
        crate::ai::Position {
            x: 780.0,
            y: 995.0,
            sector: Some(exact),
            level: 3,
        },
        4,
    );
    let ai = engine
        .world
        .entities
        .expect_ai_controller(owner, format_args!("assigned post result"));

    assert_eq!(
        ai.initial_position,
        crate::ai::Position {
            x: 780.0,
            y: 995.0,
            sector: Some(exact),
            level: 3,
        }
    );
    assert_eq!(
        ai.initial_position.sector.unwrap().arena_index(),
        Some(arena)
    );
    assert_ne!(
        ai.initial_position.sector.unwrap().arena_index(),
        crate::fast_find_grid::SectorIndex::new(98),
        "same-public foreign topology must not replace the authored post"
    );
    assert_eq!(ai.initial_view_direction, 4);
    assert!(!ai.has_patrol_path);
}

#[test]
fn shipped_stare_natives_leave_view_direction_and_pending_work_untouched() {
    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::default();
    let mut engine = EngineInner::new();
    let owner = engine.add_test_entity(crate::element::Entity::Civilian(
        crate::element::ActorCivilian {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::ActorCivilian;
                initial_element.active = true;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            civilian: Default::default(),
        },
    ));
    let target = engine.add_test_entity(crate::element::Entity::Civilian(
        crate::element::ActorCivilian {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::ActorCivilian;
                initial_element.active = true;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            civilian: Default::default(),
        },
    ));
    {
        let entity = engine.get_entity_mut(owner).unwrap();
        entity
            .position_iface_mut()
            .set_direction_instantly(crate::position_interface::Direction::from_raw(13));
        let npc = entity.ai_actor_data_mut().unwrap();
        npc.eye_status = crate::element::EyeStatus::LookForward;
        npc.follow_target = None;
    }
    let pending =
        engine
            .orders
            .sequence_manager
            .launch_element(crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::Move,
                Some(owner),
            ));
    let actor = crate::natives::ScriptHandleCodec::actor_handle(owner);

    engine
        .execute_synchronous_script_request(
            &sim,
            &assets,
            crate::interp::SynchronousScriptRequest::StareActor {
                actor,
                target,
                turn_sprite: true,
                native_return: 0,
            },
            &mut Vec::new(),
        )
        .unwrap();
    engine
        .execute_synchronous_script_request(
            &sim,
            &assets,
            crate::interp::SynchronousScriptRequest::StareLocation {
                actor,
                target: crate::ai::Position {
                    x: 10.0,
                    y: 20.0,
                    sector: None,
                    level: 0,
                },
                turn_sprite: true,
                native_return: 0,
            },
            &mut Vec::new(),
        )
        .unwrap();

    let entity = engine.get_entity(owner).unwrap();
    assert_eq!(entity.element_data().direction(), 13);
    let npc = entity.ai_actor_data().unwrap();
    assert_eq!(npc.eye_status, crate::element::EyeStatus::LookForward);
    assert_eq!(npc.follow_target, None);
    assert_eq!(
        engine
            .orders
            .sequence_manager
            .get_element(pending, 0)
            .unwrap()
            .state,
        crate::sequence::SequenceState::Todo,
        "ROBINME SetViewTarget must not Halt queued sequence work"
    );
}

#[test]
fn mission_script_snapshot_round_trips_state_and_reattaches_program() {
    let mut script = empty_mission_script("script_context_test.scs");
    script
        .state
        .computed_locations
        .push(Some(crate::natives::ComputedScriptLocation {
            position: (12.5, -8.0),
            layer: Some(2),
            sector: Some(44),
            sector_handle: None,
            active: true,
        }));
    let mut recording = crate::sequence::RecordingSession::new();
    for expected_level in [2, 3] {
        let mut timer =
            crate::sequence::SequenceElement::new_generic(1, crate::element::Command::Timer, None);
        timer.set_property(
            crate::sequence::Field::Timer,
            crate::sequence::FieldValue::Integer(12),
        );
        recording.add_element(timer);
        assert_eq!(recording.advance_level(), expected_level);
    }
    script.state.sequence_recorder = Some(recording);
    let location_positions = std::sync::Arc::new(vec![(12.0, 34.0)]);
    script.attach_bindings(crate::natives::AttachedScriptBindings {
        script_location_count: 1,
        location_positions: location_positions.clone(),
        ..Default::default()
    });

    let hash_before = robin_util::state_hash::compute(&script);
    let program = script.manager.program.clone();
    let json = serde_json::to_string(&script).expect("serialize MissionScript");
    let value: serde_json::Value = serde_json::from_str(&json).expect("parse snapshot JSON");
    assert!(value.get("snapshot_version").is_none());
    let effect_keys = value["script_effects"]
        .as_object()
        .expect("ScriptEffects snapshot object")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(effect_keys, ["ordered"].into_iter().collect());
    assert!(value.get("bindings").is_none());

    let mut decoded: MissionScript =
        serde_json::from_str(&json).expect("deserialize MissionScript");
    assert_eq!(decoded.bindings.script_location_count, 0);
    decoded.attach_program(program);
    decoded.attach_bindings(crate::natives::AttachedScriptBindings {
        script_location_count: 1,
        location_positions: location_positions.clone(),
        ..Default::default()
    });
    assert!(std::sync::Arc::ptr_eq(
        &decoded.bindings.location_positions,
        &location_positions
    ));
    assert!(
        serde_json::to_value(&decoded.state)
            .unwrap()
            .get("globals")
            .is_none()
    );
    assert_eq!(decoded.state.computed_locations.len(), 1);
    assert!(decoded.state.sequence_recorder.is_some());
    assert_eq!(robin_util::state_hash::compute(&decoded), hash_before);
    let recording = decoded.state.sequence_recorder.as_mut().unwrap();
    assert_eq!(recording.command_level, 3);
    assert_eq!(recording.advance_level(), 3, "empty Then does not advance");
    let mut timer =
        crate::sequence::SequenceElement::new_generic(1, crate::element::Command::Timer, None);
    timer.set_property(
        crate::sequence::Field::Timer,
        crate::sequence::FieldValue::Integer(12),
    );
    recording.add_element(timer);
    assert_eq!(recording.advance_level(), 4);
}

#[test]
fn customize_minimap_accepts_vip_dots_and_gates_vip_multi_to_humans() {
    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine
        .world
        .entities
        .push(Some(crate::element::Entity::Soldier(
            crate::element::ActorSoldier {
                element: {
                    let mut initial_element = crate::element::ElementData::default();
                    initial_element.kind = crate::element::ElementKind::ActorSoldier;
                    initial_element
                },
                actor: crate::element::ActorData::default(),
                human: crate::element::HumanData::default(),
                npc: crate::element::NpcData::default(),
                soldier: crate::element::SoldierData::default(),
            },
        )));
    engine.world.entities.push(Some(crate::element::Entity::Fx(
        crate::element::ElementFx {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::Fx;
                initial_element.custom_minimap_dot =
                    crate::minimap::CustomDot::NotCustomized as u16;
                initial_element
            },
            fx: crate::element::FxData::default(),
        },
    )));
    let human_handle = crate::natives::ScriptHandleCodec::actor_handle_from_index(0);
    let non_human_handle = crate::natives::ScriptHandleCodec::actor_handle_from_index(1);

    engine.apply_host_commands(
        &sim,
        &LevelAssets::default(),
        vec![crate::natives::EngineCommand::CustomizeMinimapDisplay {
            actor_handle: human_handle,
            dot_type: crate::minimap::CustomDot::VipMulti as i32,
        }],
    );
    assert_eq!(
        engine
            .get_entity(engine.entity_id_for_index(0).expect("human entity"))
            .expect("human entity")
            .element_data()
            .custom_minimap_dot,
        crate::minimap::CustomDot::VipMulti as u16
    );

    engine.apply_host_commands(
        &sim,
        &LevelAssets::default(),
        vec![crate::natives::EngineCommand::CustomizeMinimapDisplay {
            actor_handle: non_human_handle,
            dot_type: crate::minimap::CustomDot::VipMulti as i32,
        }],
    );
    assert_eq!(
        engine
            .get_entity(engine.entity_id_for_index(1).expect("non-human entity"))
            .expect("non-human entity")
            .element_data()
            .custom_minimap_dot,
        crate::minimap::CustomDot::NotCustomized as u16
    );
}

#[test]
fn actor_location_changes_preserve_material_and_display_reference_state() {
    let sim = crate::sim_rng::test_context();
    let obstacle =
        crate::position_interface::ObstacleHandle::new(86).expect("test obstacle handle is valid");
    let plane = crate::position_interface::PlaneZCoeffs {
        az: 0.0,
        bz: 0.0,
        dz: 90.00101,
    };

    for (spawn_elevation_probe, expected_obstacle, expected_plane, expected_material) in [
        (
            Some((20.0, 30.0)),
            Some(obstacle),
            plane,
            crate::element::GameMaterial::Stone,
        ),
        (
            None,
            crate::position_interface::ObstacleHandle::new(0),
            crate::position_interface::PlaneZCoeffs {
                az: 0.0,
                bz: 0.0,
                dz: 10.0,
            },
            crate::element::GameMaterial::Stone,
        ),
    ] {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(8, 8);
        engine.world.fast_grid_mut().allocate_layers(1);
        let sector_number = crate::sector::SectorNumber::new(0);
        let sector_index = engine.world.fast_grid_mut().add_sector(
            crate::fast_find_grid::GridSector {
                points: Vec::new(),
                bounding_box: crate::coordinates::MapBBox::new(),
                sector_type: crate::sector::SectorType::MOTION | crate::sector::SectorType::AREA,
                layer: 0,
                sector_number,
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
            },
            0,
        );
        let sector = crate::position_interface::SectorHandle::new(0)
            .expect("test sector handle is valid")
            .with_arena_index(
                crate::fast_find_grid::SectorIndex::new(sector_index)
                    .expect("test arena sector index is valid"),
            );
        let mut replacement = crate::sight_obstacle::SightObstacle::new(
            1,
            crate::sight_obstacle::SIGHTOBSTACLE_SOLID
                | crate::sight_obstacle::SIGHTOBSTACLE_PROJECTION_AREA,
        );
        replacement.obstacle_points = vec![
            crate::sight_obstacle::ObstaclePoint {
                x: 0.0,
                y: 0.0,
                z_bottom: 0.0,
                z_top: 10.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 2000.0,
                y: 0.0,
                z_bottom: 0.0,
                z_top: 10.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 2000.0,
                y: 3000.0,
                z_bottom: 0.0,
                z_top: 10.0,
            },
            crate::sight_obstacle::ObstaclePoint {
                x: 0.0,
                y: 3000.0,
                z_bottom: 0.0,
                z_top: 10.0,
            },
        ];
        replacement.top_plane_points = [
            [0.0, 0.0, 10.0],
            [2000.0, 0.0, 10.0],
            [2000.0, 3000.0, 10.0],
        ];
        replacement.set_projection_area_ref(
            crate::position_interface::Layer::ZERO,
            crate::fast_find_grid::SectorIndex::new(sector_index)
                .expect("test arena sector index is valid"),
        );
        replacement.material = crate::element::GameMaterial::Wood as u8;
        replacement.rebuild_geometry();
        let assets = LevelAssets {
            environment: crate::engine::LevelEnvironmentAssets {
                static_sight_obstacles: std::sync::Arc::new(vec![replacement]),
                ..Default::default()
            },
            ..LevelAssets::default()
        };
        let mut element = {
            let mut initial_element = crate::element::ElementData::default();
            initial_element.kind = crate::element::ElementKind::ActorSoldier;
            initial_element.active = true;
            initial_element
        };
        element.set_obstacle_index(Some(obstacle), Some(plane));
        element.set_material(crate::element::GameMaterial::Stone);
        element.sprite.display_order_ref = None;
        element.sprite.behind_display_order_ref = true;
        engine
            .world
            .entities
            .push(Some(crate::element::Entity::Soldier(
                crate::element::ActorSoldier {
                    element,
                    actor: Default::default(),
                    human: Default::default(),
                    npc: Default::default(),
                    soldier: Default::default(),
                },
            )));
        let actor_handle = crate::natives::ScriptHandleCodec::actor_handle_from_index(0);

        engine.apply_host_commands(
            &sim,
            &assets,
            vec![crate::natives::EngineCommand::SetActorLocation {
                actor_handle,
                x: 1000.0,
                y: 2000.0,
                dest_layer_sector: Some((0, sector)),
                spawn_elevation_probe,
            }],
        );

        let entity = engine
            .get_entity(engine.entity_id_for_index(0).expect("test actor entity"))
            .expect("test actor survives location command");
        assert_eq!(entity.position_iface().get_obstacle(), expected_obstacle);
        assert_eq!(entity.position_iface().get_plane(), Some(&expected_plane));
        assert_eq!(entity.position_iface().get_material(), expected_material);
        assert_eq!(entity.sprite().display_order_ref, None);
        assert!(entity.sprite().behind_display_order_ref);
        if spawn_elevation_probe.is_some() {
            assert_eq!(entity.position_iface().get_position().z, 10.0);
            assert_eq!(entity.position_iface().map_position().y, 2000.0);
        }
    }
}

#[test]
fn direct_popup_native_refreshes_a_new_arrow_before_returning() {
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        Animation, ElementData, ElementKind, ElementProjectile, Entity, ObjectData, ObjectType,
        ProjectileData, TrajectoryPoint,
    };

    let sim = crate::sim_rng::test_context();
    let mut engine = EngineInner::new();
    engine.control.frame_counter = 48479;

    let mut element = {
        let mut initial_element = ElementData::default();
        initial_element.kind = ElementKind::ObjectProjectile;
        initial_element.active = true;
        initial_element
    };
    element.set_position(WorldPoint3D::ZERO);
    let arrow = engine.add_test_entity(Entity::Projectile(ElementProjectile {
        element,
        object: ObjectData {
            object_type: ObjectType::Arrow,
            animation: Animation::ObjectFlying,
            ..Default::default()
        },
        projectile: ProjectileData {
            flying: true,
            trajectory: vec![TrajectoryPoint {
                position: WorldPoint3D::new(10.0, 0.0, 100.0),
                time: 4,
            }],
            ..Default::default()
        },
    }));

    engine.apply_host_commands(
        &sim,
        &LevelAssets::default(),
        vec![crate::natives::EngineCommand::DisplayPopupText { text_id: 11 }],
    );

    let Entity::Projectile(arrow) = engine
        .get_entity(arrow)
        .expect("direct popup must retain its arrow")
    else {
        panic!("direct popup arrow changed entity kind");
    };
    assert_eq!(
        (
            arrow.element.sprite.current_row,
            arrow.element.sprite.current_frame
        ),
        (4, 8)
    );
    assert_eq!(engine.control.popup_scroll_last_display_frame, Some(48479));
    assert_eq!(
        engine.feedback.pending_side_effects.pending_popup_texts,
        vec![11]
    );
}

#[test]
fn patch_background_effects_invalidate_canonical_side_effects_immediately() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
    engine
        .script_domains
        .interactables
        .patches
        .push(crate::patch::Patch {
            integrate_in_background: true,
            ..Default::default()
        });
    let patch_index = crate::patch::PatchIndex::new(0).expect("zero is a valid patch index");

    engine.process_patch_effects(
        sim,
        &LevelAssets::default(),
        patch_index,
        vec![crate::patch::PatchEffect::SwapBackground { applied: true }],
    );
    assert!(engine.feedback.pending_side_effects.invalidate_background);

    engine.feedback.pending_side_effects.invalidate_background = false;
    engine.process_patch_effects(
        sim,
        &LevelAssets::default(),
        patch_index,
        vec![crate::patch::PatchEffect::RestoreBackground],
    );
    assert!(engine.feedback.pending_side_effects.invalidate_background);
}

#[test]
fn external_this_actor_success_keeps_canonical_entity_ownership() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.world.entities.push(None);
    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
    engine.attach_script_bindings(&LevelAssets::new());

    let result =
        engine.call_external_native_with_this(sim, &LevelAssets::new(), "ThisActor", &[], Some(99));

    assert_eq!(result, Ok(99));
    assert_eq!(engine.world.entities.len(), 1);
    let script = engine
        .scripts
        .mission
        .as_ref()
        .expect("script remains installed");
    assert_eq!(script.active_call_frame_count(), 0);
}

#[test]
fn post_initialize_game_latch_covers_disabled_missing_vm_and_missing_function() {
    let assets = LevelAssets::new();
    for with_mission in [false, true] {
        for script_enabled in [false, true] {
            for already_initialized in [false, true] {
                let mut engine = EngineInner::new();
                if with_mission {
                    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
                    engine.attach_script_bindings(&assets);
                }
                engine.control.sim_config.script_enabled = script_enabled;
                engine.script_domains.mission_ui.game_post_initialized = already_initialized;
                engine.control.arrow_refresh_pending = true;
                engine.feedback.pending_side_effects.set_draw_hidden = Some(true);
                let before_rng = engine.rng_seed();
                let runs_vm_stage = script_enabled && !already_initialized && with_mission;
                let effects = engine.perform_frame_post_initialize(&assets);
                assert_eq!(effects.is_some(), script_enabled && !already_initialized);
                assert_eq!(
                    engine.script_domains.mission_ui.game_post_initialized,
                    already_initialized || script_enabled
                );
                assert_eq!(engine.control.arrow_refresh_pending, !runs_vm_stage);
                assert_eq!(engine.rng_seed(), before_rng);
                if runs_vm_stage {
                    assert_eq!(effects.unwrap().set_draw_hidden, Some(true));
                } else {
                    if let Some(effects) = effects {
                        assert_eq!(effects.set_draw_hidden, None);
                    }
                    assert_eq!(
                        engine.feedback.pending_side_effects.set_draw_hidden,
                        Some(true)
                    );
                }
                assert!(engine.perform_frame_post_initialize(&assets).is_none());
            }
        }
    }
}

#[test]
fn post_initialize_game_latch_survives_snapshots_without_a_script_mirror() {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            let assets = LevelAssets::new();
            for with_mission in [false, true] {
                let mut engine = EngineInner::new();
                if with_mission {
                    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
                    engine.attach_script_bindings(&assets);
                }
                let rollback = engine.clone();
                let before = crate::replay::state_hash(&engine);
                engine.perform_frame_post_initialize(&assets);
                let after = crate::replay::state_hash(&engine);
                assert_ne!(before, after);
                let json = serde_json::to_value(&engine).unwrap();
                assert_eq!(
                    json["script_domains"]["mission_ui"]["game_post_initialized"],
                    true
                );
                if with_mission {
                    assert!(json["scripts"]["mission"].get("post_initialized").is_none());
                }
                let restored_json: EngineInner = serde_json::from_value(json).unwrap();
                let bytes = crate::engine::snapshot::encode_native_engine_inner(&engine);
                let restored_native =
                    crate::engine::snapshot::decode_native_engine_inner(&bytes).unwrap();
                for mut restored in [restored_json, restored_native] {
                    assert_eq!(crate::replay::state_hash(&restored), after);
                    assert!(restored.perform_frame_post_initialize(&assets).is_none());
                    assert_eq!(crate::replay::state_hash(&restored), after);
                }
                engine = rollback;
                assert_eq!(crate::replay::state_hash(&engine), before);
                assert!(!engine.script_domains.mission_ui.game_post_initialized);
                engine.perform_frame_post_initialize(&assets);
                assert_eq!(crate::replay::state_hash(&engine), after);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn native_globals_are_canonical_across_json_native_snapshots_and_rollback() {
    // The native engine codec requires more than libtest's default stack.
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            let sim = crate::sim_rng::test_context();
            let assets = LevelAssets::new();
            let mut engine = EngineInner::new();
            engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
            engine.attach_script_bindings(&assets);
            assert_eq!(
                engine.call_external_native(&sim, &assets, "InitGlobal", &[0, 7]),
                Ok(0)
            );
            assert_eq!(
                engine.call_external_native(&sim, &assets, "GetGlobal", &[1]),
                Ok(0)
            );
            assert_eq!(
                engine.call_external_native(&sim, &assets, "SetGlobal", &[15, 9]),
                Ok(0)
            );
            assert_eq!(engine.scripts.globals.len(), 16);
            let expected = engine.scripts.globals.clone();
            let before = crate::replay::state_hash(&engine);
            let program = engine
                .scripts
                .mission
                .as_ref()
                .unwrap()
                .manager
                .program
                .clone();
            let json = serde_json::to_value(&engine).unwrap();
            assert_eq!(json["scripts"]["globals"], serde_json::json!(expected));
            assert!(json["scripts"]["mission"]["state"].get("globals").is_none());
            let restored_json: EngineInner = serde_json::from_value(json).unwrap();
            let native = crate::engine::snapshot::encode_native_engine_inner(&engine);
            let restored_native =
                crate::engine::snapshot::decode_native_engine_inner(&native).unwrap();
            for mut restored in [restored_json, restored_native] {
                assert_eq!(restored.scripts.globals, expected);
                assert_eq!(crate::replay::state_hash(&restored), before);
                restored
                    .scripts
                    .mission
                    .as_mut()
                    .unwrap()
                    .attach_program(program.clone());
                restored.attach_script_bindings(&assets);
                assert_eq!(
                    restored.call_external_native(&sim, &assets, "GetGlobal", &[15]),
                    Ok(9)
                );
                assert_eq!(
                    restored.call_external_native(&sim, &assets, "SetGlobal", &[14, 13]),
                    Ok(0)
                );
                assert_eq!(restored.scripts.globals[14], 13);
            }
            let rollback = engine.clone();
            assert_eq!(
                engine.call_external_native(&sim, &assets, "SetGlobal", &[15, 17]),
                Ok(0)
            );
            assert_ne!(crate::replay::state_hash(&engine), before);
            engine = rollback;
            assert_eq!(crate::replay::state_hash(&engine), before);
            assert_eq!(engine.scripts.globals, expected);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn external_remove_all_subordinates_finishes_clear_before_returning() {
    fn soldier() -> crate::element::Entity {
        crate::element::Entity::Soldier(crate::element::ActorSoldier {
            element: {
                let mut initial_element = crate::element::ElementData::default();
                initial_element.kind = crate::element::ElementKind::ActorSoldier;
                initial_element
            },
            actor: crate::element::ActorData::default(),
            human: crate::element::HumanData::default(),
            npc: crate::element::NpcData {
                ai: crate::element::AiActorData {
                    ai_brain: crate::element::AiBrain::Enemy(Box::new(
                        crate::ai_enemy::EnemyAi::new(0),
                    )),
                    ..Default::default()
                },
                ..Default::default()
            },
            soldier: crate::element::SoldierData::default(),
        })
    }

    let sim = crate::sim_rng::test_context();
    let assets = LevelAssets::new();
    let mut engine = EngineInner::new();
    engine.world.entities.push(Some(soldier()));
    engine.world.entities.push(Some(soldier()));
    let chief = crate::element::EntityId::Soldier(crate::element::SoldierId(0));
    let member = crate::element::EntityId::Soldier(crate::element::SoldierId(1));

    {
        let chief_ai = engine
            .get_entity_mut(chief)
            .and_then(crate::element::Entity::ai_controller_mut)
            .expect("chief has AI");
        chief_ai.theoretical_patrol.push(member);
        chief_ai.patrol.push(member);
        chief_ai.missed_patrol_members.push(member);
    }
    {
        let member_ai = engine
            .get_entity_mut(member)
            .and_then(crate::element::Entity::ai_controller_mut)
            .expect("member has AI");
        member_ai.patrol_chief = Some(chief);
        // Non-default members are unlinked synchronously without entering
        // forced return to duty, keeping this test focused on the VM barrier.
        member_ai.current_state = crate::ai::AiState::Seeking;
    }

    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
    engine.attach_script_bindings(&assets);
    let chief_handle = crate::natives::ScriptHandleCodec::actor_handle_from_index(0);
    assert_eq!(
        engine.call_external_native(&sim, &assets, "RemoveAllSubordinates", &[chief_handle],),
        Ok(0)
    );

    let chief_ai = engine
        .get_entity(chief)
        .and_then(crate::element::Entity::ai_controller)
        .expect("chief remains an NPC");
    assert!(chief_ai.theoretical_patrol.is_empty());
    assert!(chief_ai.patrol.is_empty());
    assert!(chief_ai.missed_patrol_members.is_empty());
    assert_eq!(
        engine
            .get_entity(member)
            .and_then(crate::element::Entity::ai_controller)
            .expect("member remains an NPC")
            .patrol_chief,
        None
    );
}

#[test]
fn native_mutation_writes_the_canonical_script_domains_in_place() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::interp::{HostFunctions, NativeStack};
    use crate::natives::NativeFn;

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
    engine
        .script_domains
        .interactables
        .doors
        .push(crate::gate::Door {
            locked_pc: true,
            ..Default::default()
        });
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let canonical_domains = std::ptr::addr_of_mut!(engine.script_domains);
    let canonical_entities = std::ptr::from_ref(&engine.world.entities);
    let door = crate::natives::ScriptHandleCodec::door_handle_from_index(0);

    let result =
        engine.with_script_session(sim, &assets, |script, script_domains, capabilities| {
            assert_eq!(
                std::ptr::from_mut(script_domains),
                canonical_domains,
                "the native capability must borrow EngineInner's allocation"
            );
            assert_eq!(
                capabilities.entities_owner_ptr(),
                canonical_entities,
                "the entity capability must borrow EngineInner's canonical allocation"
            );
            let mut stack = NativeStack::default();
            stack.push_i32(door);
            stack.push_i32(0);
            let mut context = crate::natives::NativeContext::with_bindings(
                &mut script.script_effects,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
            );
            HostFunctions::call(&mut context, NativeFn::SetDoorLockedPC as u32, &mut stack)
                .expect_return("SetDoorLockedPC is synchronous")
        });

    assert_eq!(result, Some(0));
    assert!(!engine.script_domains.interactables.doors[0].locked_pc);
}

#[test]
fn native_ai_mutation_writes_engine_inner_directly() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    use crate::interp::{HostFunctions, NativeStack};
    use crate::natives::NativeFn;

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
    engine.ai.global.next_repulsive_point_id = 9;
    engine
        .ai
        .global
        .repulsive_points
        .push(crate::ai::RepulsivePoint::new(
            8,
            crate::ai::Position::default(),
            10.0,
            20.0,
            0,
        ));
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let canonical_ai_global = std::ptr::addr_of_mut!(engine.ai.global);

    let result = engine.with_script_session(sim, &assets, |script, script_domains, queries| {
        let mut context = crate::natives::NativeContext::with_bindings(
            &mut script.script_effects,
            &mut script.state,
            script_domains,
            &script.bindings,
            queries,
        );
        assert_eq!(
            std::ptr::from_mut(context.ai_global_mut()),
            canonical_ai_global,
            "the native capability must borrow EngineInner's AI allocation"
        );
        let mut stack = NativeStack::default();
        stack.push_i32(8);
        HostFunctions::call(
            &mut context,
            NativeFn::DeleteRepulsivePoint as u32,
            &mut stack,
        )
        .expect_return("DeleteRepulsivePoint is synchronous")
    });

    assert_eq!(result, Some(0));
    assert!(engine.ai.global.repulsive_points.is_empty());
    assert_eq!(engine.ai.global.next_repulsive_point_id, 9);
}

#[test]
#[should_panic(expected = "native dispatch requires live level attachments")]
fn external_native_rejects_a_detached_live_script() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));

    let _ =
        engine.call_external_native_with_this(sim, &LevelAssets::new(), "ThisActor", &[], Some(99));
}

#[test]
fn script_session_normal_return_restores_state_and_hash() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.world.entities.push(None);
    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let hash_before = robin_util::state_hash::compute(&engine);
    let canonical_entities = std::ptr::from_ref(&engine.world.entities);

    let result = engine.with_script_session(sim, &assets, |_script, _, capabilities| {
        assert_eq!(capabilities.entities_owner_ptr(), canonical_entities);
        73
    });

    assert_eq!(result, Some(73));
    assert_eq!(engine.world.entities.len(), 1);
    let script = engine.scripts.mission.as_ref().unwrap();
    assert_eq!(script.active_call_frame_count(), 0);
    assert_eq!(robin_util::state_hash::compute(&engine), hash_before);
}

#[test]
fn script_callback_error_keeps_canonical_owners_in_place() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.world.entities.push(None);
    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);

    let result: Result<(), &'static str> = engine
        .with_script_session(sim, &assets, |_script, _, _capabilities| {
            Err("simulated script error")
        })
        .unwrap();

    assert_eq!(result, Err("simulated script error"));
    assert_eq!(engine.world.entities.len(), 1);
    let script = engine.scripts.mission.as_ref().unwrap();
    assert_eq!(script.active_call_frame_count(), 0);
}

#[test]
#[should_panic(expected = "simulated script panic")]
fn script_callback_unwind_keeps_canonical_owners_in_place() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    struct VerifyRestoredOnUnwind(*const EngineInner);

    impl Drop for VerifyRestoredOnUnwind {
        fn drop(&mut self) {
            // SAFETY: the pointer targets the engine local below, which
            // outlives this verifier. All callback capability borrows have
            // ended before unwinding reaches this Drop implementation.
            let engine = unsafe { &*self.0 };
            assert_eq!(engine.world.entities.len(), 1);
            let script = engine.scripts.mission.as_ref().unwrap();
            assert_eq!(script.active_call_frame_count(), 0);
            assert!(
                engine.script_domains.mission_ui.outline_display,
                "canonical domain mutation survives callback unwind"
            );
            assert!(
                engine.ai.global.golden_eye_mode,
                "canonical AI-global mutation survives callback unwind"
            );
        }
    }

    let mut engine = EngineInner::new();
    engine.mission_domain.campaign = crate::campaign::Campaign::default();
    engine.world.entities.push(None);
    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));
    let assets = LevelAssets::new();
    engine.attach_script_bindings(&assets);
    let _verify = VerifyRestoredOnUnwind(&engine);

    let _ = engine.with_script_session(sim, &assets, |script, script_domains, capabilities| {
        script_domains.mission_ui.outline_display = true;
        {
            let mut context = crate::natives::NativeContext::with_bindings(
                &mut script.script_effects,
                &mut script.state,
                script_domains,
                &script.bindings,
                capabilities,
            );
            context.ai_global_mut().golden_eye_mode = true;
        }
        panic!("simulated script panic");
    });
}

#[test]
fn external_native_early_returns_without_touching_callback_state() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut engine = EngineInner::new();
    engine.world.entities.push(None);
    engine.scripts.mission = Some(empty_mission_script("script_context_test.scs"));

    let result = engine.call_external_native_with_this(
        sim,
        &LevelAssets::new(),
        "NotAnOriginalNative",
        &[],
        Some(99),
    );

    assert_eq!(result, Err("unknown native: NotAnOriginalNative".into()));
    assert_eq!(engine.world.entities.len(), 1);
    let script = engine
        .scripts
        .mission
        .as_ref()
        .expect("script remains installed");
    assert_eq!(script.active_call_frame_count(), 0);
}
