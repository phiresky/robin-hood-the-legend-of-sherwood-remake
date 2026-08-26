use super::*;

// ── QA macro playback / abort system tests ─────────────────────────

/// Add a live PC entity to the engine.
///
/// The macro/quick-action paths write back through the owner's `PcData`, so
/// these fixtures need a real entity rather than a synthetic `PcId` handle.
#[cfg(test)]
fn add_test_pc(engine: &mut EngineInner) -> crate::element::EntityId {
    let pc = engine.add_entity(Entity::Pc(crate::element::ActorPc {
        element: crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            active: true,
            posture: crate::element::Posture::Upright,
            ..Default::default()
        },
        actor: Default::default(),
        human: Default::default(),
        pc: crate::element::PcData {
            life_points: 100,
            ..Default::default()
        },
    }));
    // Macro playback dispatches real group-move commands, whose formation
    // geometry reads the mover's map position and move box. A default
    // entity has an empty (hyperspace) box, so give it real geometry.
    let entity = engine.get_entity_mut(pc).unwrap();
    entity
        .position_iface_mut()
        .set_move_box(crate::coordinates::MoveBox::from_corners(
            crate::coordinates::MapVec::new(-5.0, -5.0),
            crate::coordinates::MapVec::new(5.0, 5.0),
        ));
    entity
        .element_data_mut()
        .set_position_map(crate::coordinates::MapPoint::new(10.0, 10.0));
    entity
        .element_data_mut()
        .set_sector(crate::position_interface::SectorHandle::new(1));
    pc
}

#[cfg(test)]
fn add_group_move_test_sector(engine: &mut EngineInner) -> crate::fast_find_grid::SectorIndex {
    use crate::coordinates::MapBBox;
    use crate::fast_find_grid::GridSector;
    use crate::sector::{SectorNumber, SectorType};

    engine.world.fast_grid_mut().size_map(64, 64);
    engine.world.fast_grid_mut().allocate_layers(1);
    let min = MapPoint::new(0.0, 0.0);
    let max = MapPoint::new(1000.0, 1000.0);
    let raw = engine.world.fast_grid_mut().add_sector(
        GridSector {
            points: vec![
                min,
                MapPoint::new(max.x, min.y),
                max,
                MapPoint::new(min.x, max.y),
            ],
            bounding_box: MapBBox::from_corners(min, max),
            sector_type: SectorType::MOTION | SectorType::AREA,
            layer: 0,
            sector_number: SectorNumber::new(1),
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
    crate::fast_find_grid::SectorIndex::new(raw).expect("test sector index")
}

#[cfg(test)]
fn arm_group_move_recording(engine: &mut EngineInner, pcs: &[crate::element::EntityId]) {
    for &pc in pcs {
        engine
            .players
            .macro_store
            .get_or_insert(pc)
            .begin_recording(0);
    }
    engine.players.qa_recording_slot = 0;
    engine.players.qa_recording_for = pcs.to_vec();
}

/// Seed a PC's macro slot with a recorded "move to (x,y)" step and a
/// wired titbit.  Used by the playback/abort/tetris tests below.
#[cfg(test)]
fn seed_macro_slot(
    engine: &mut EngineInner,
    pc: crate::element::EntityId,
    slot: u8,
    steps: Vec<(f32, f32)>,
) -> crate::titbit::TitbitId {
    use crate::coordinates::WorldPoint3D;
    use crate::macro_store::{QaReplayCommand, QuickActionStep};
    use crate::titbit::{ElementHandle, INVALID_ID, QuickAction, TitbitKind};

    let pc_handle = ElementHandle(pc.index());
    let titbit_id = engine.feedback.titbit_manager.add_titbit(
        WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        0,
        TitbitKind::QuickAction,
        pc_handle,
        QuickAction::Walk as u16,
        pc_handle,
        false,
        INVALID_ID,
        true,
        Some(0.0),
        Some(0),
    );

    let state = engine.players.macro_store.get_or_insert(pc);
    state.begin_recording(slot);
    for (x, y) in steps {
        let pos = crate::coordinates::MapPoint::new(x, y);
        state.append_if_recording(QuickActionStep {
            action: crate::profiles::Action::NoAction,
            position: pos,
            replay: QaReplayCommand::Move {
                destination: pos,
                running: false,
                route: None,
            },
        });
    }
    state.stop_recording();
    let titbit = crate::titbit::TitbitId::new(titbit_id).unwrap();
    state.set_slot_titbit(slot as usize, titbit);
    titbit
}

#[test]
fn stop_recording_macro_restores_occupied_slot_before_refreshing_portrait() {
    let mut engine = EngineInner::new();
    let pc = add_test_pc(&mut engine);
    let slot = 1;
    let titbit = seed_macro_slot(&mut engine, pc, slot, vec![(10.0, 20.0)]);

    engine
        .players
        .macro_store
        .get_mut(pc)
        .unwrap()
        .begin_recording(slot);
    engine
        .get_entity_mut(pc)
        .unwrap()
        .pc_data_mut()
        .unwrap()
        .portrait
        .quick_icons[slot as usize] = Default::default();
    engine.players.qa_recording_slot = slot;
    engine.players.qa_recording_for = vec![pc];

    engine.stop_recording_macro();

    let state = engine.players.macro_store.get(pc).unwrap();
    assert!(state.has_macro(slot as usize));
    assert_eq!(state.get_slot_titbit(slot as usize), Some(titbit));
    let icon = engine
        .get_entity(pc)
        .unwrap()
        .pc_data()
        .unwrap()
        .portrait
        .quick_icons[slot as usize];
    assert_eq!(
        icon.titbit_id,
        u32::from(engine.feedback.titbit_manager.get_phase(titbit))
    );
    assert_eq!(
        icon.running,
        engine.feedback.titbit_manager.is_running_for_qa(titbit)
    );
    assert!(engine.players.qa_recording_for.is_empty());
}

#[test]
fn stop_recording_macro_refreshes_empty_and_recorded_portraits() {
    use crate::coordinates::MapPoint;
    use crate::macro_store::{QaReplayCommand, QuickActionStep};

    let mut engine = EngineInner::new();
    let empty_pc = add_test_pc(&mut engine);
    let recorded_pc = add_test_pc(&mut engine);
    let slot = 2;
    let titbit = seed_macro_slot(&mut engine, recorded_pc, slot, vec![(1.0, 1.0)]);

    engine
        .players
        .macro_store
        .get_or_insert(empty_pc)
        .begin_recording(slot);
    let state = engine.players.macro_store.get_mut(recorded_pc).unwrap();
    state.begin_recording(slot);
    let destination = MapPoint::new(30.0, 40.0);
    state.append_if_recording(QuickActionStep {
        action: crate::profiles::Action::NoAction,
        position: destination,
        replay: QaReplayCommand::Move {
            destination,
            running: false,
            route: None,
        },
    });
    state.set_slot_titbit(slot as usize, titbit);
    engine.players.qa_recording_slot = slot;
    engine.players.qa_recording_for = vec![empty_pc, recorded_pc];

    engine.stop_recording_macro();

    assert!(
        !engine
            .players
            .macro_store
            .get(empty_pc)
            .unwrap()
            .has_macro(slot as usize)
    );
    let empty_icon = engine
        .get_entity(empty_pc)
        .unwrap()
        .pc_data()
        .unwrap()
        .portrait
        .quick_icons[slot as usize];
    assert_eq!(empty_icon.titbit_id, u32::MAX);
    assert!(!empty_icon.running);

    assert!(
        engine
            .players
            .macro_store
            .get(recorded_pc)
            .unwrap()
            .has_macro(slot as usize)
    );
    let recorded_icon = engine
        .get_entity(recorded_pc)
        .unwrap()
        .pc_data()
        .unwrap()
        .portrait
        .quick_icons[slot as usize];
    assert_eq!(
        recorded_icon.titbit_id,
        u32::from(engine.feedback.titbit_manager.get_phase(titbit))
    );
    assert_eq!(
        recorded_icon.running,
        engine.feedback.titbit_manager.is_running_for_qa(titbit)
    );
    assert!(engine.players.qa_recording_for.is_empty());
}

#[test]
fn recorded_single_group_move_keeps_adjusted_destination_and_replays_exact_seek() {
    use crate::element::Command;
    use crate::macro_store::QaReplayCommand;
    use crate::player_command::PlayerCommand;
    use crate::sequence::SequenceElementData;

    let sim = crate::sim_rng::test_context();
    let mut display = HostDisplayState::default();
    let mut input = crate::engine::InputState::default();
    let assets = crate::engine::LevelAssets::new();
    let mut engine = EngineInner::new();
    let exact_sector = add_group_move_test_sector(&mut engine);
    let pc = add_test_pc(&mut engine);
    {
        let entity = engine.get_entity_mut(pc).expect("test PC");
        entity
            .position_iface_mut()
            .set_move_box(crate::coordinates::MoveBox::from_corners(
                crate::coordinates::MapVec::new(-4.0, -2.0),
                crate::coordinates::MapVec::new(12.0, 2.0),
            ));
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(100.0, 100.0));
    }
    arm_group_move_recording(&mut engine, &[pc]);

    engine.apply_command(
        &sim,
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::GroupMove {
            actors: vec![pc],
            destination: MapPoint::new(500.0, 500.0),
            running: true,
            show_marker: true,
            goal_override: Some((crate::sector::SectorNumber::new(1), 0)),
            goal_sector_index_override: Some(exact_sector),
            door_route_override: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: Vec::new(),
        },
    );

    assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
    let step = engine
        .players
        .macro_store
        .get(pc)
        .and_then(|state| state.slot(0))
        .and_then(|slot| slot.steps.first())
        .copied()
        .expect("one recorded group-move step");
    let QaReplayCommand::Move {
        destination,
        running,
        route: Some(route),
    } = step.replay
    else {
        panic!("group move did not retain its resolved route")
    };
    assert_eq!(destination, MapPoint::new(504.0, 500.0));
    assert!(running);
    assert_eq!(route.goal_sector, crate::sector::SectorNumber::new(1));
    assert_eq!(route.goal_sector_index, exact_sector);
    assert_eq!(route.goal_layer, 0);

    engine.apply_command(
        &sim,
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::StartMacro {
            pc: Some(pc),
            slot: 0,
        },
    );
    let sequence = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .next()
        .expect("recorded group move launched one seek sequence");
    assert_eq!(sequence.elements[0].command, Command::Seek);
    let SequenceElementData::Movement {
        destination,
        sector,
        layer,
        post_seek_sequence,
        ..
    } = &sequence.elements[0].data
    else {
        panic!("recorded group move did not replay as movement")
    };
    assert_eq!(*destination, MapPoint::new(504.0, 500.0));
    assert_eq!(
        sector.expect("exact replay goal").arena_index(),
        Some(exact_sector)
    );
    assert_eq!(*layer, 0);
    let post_seek = post_seek_sequence.as_ref().expect("arrival continuation");
    assert_eq!(
        post_seek.elements[0].command,
        Command::SpeakHeroReachDestination
    );
}

#[test]
fn recorded_multi_pc_group_move_keeps_actor_order_and_individual_slots_without_launching() {
    use crate::macro_store::QaReplayCommand;
    use crate::player_command::PlayerCommand;

    let sim = crate::sim_rng::test_context();
    let mut display = HostDisplayState::default();
    let mut input = crate::engine::InputState::default();
    let assets = crate::engine::LevelAssets::new();
    let mut engine = EngineInner::new();
    let exact_sector = add_group_move_test_sector(&mut engine);
    let pc_a = add_test_pc(&mut engine);
    let pc_b = add_test_pc(&mut engine);
    engine
        .get_entity_mut(pc_a)
        .expect("first PC")
        .element_data_mut()
        .set_position_map(MapPoint::new(100.0, 100.0));
    engine
        .get_entity_mut(pc_b)
        .expect("second PC")
        .element_data_mut()
        .set_position_map(MapPoint::new(120.0, 100.0));
    arm_group_move_recording(&mut engine, &[pc_a, pc_b]);

    engine.apply_command(
        &sim,
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::GroupMove {
            actors: vec![pc_a, pc_b],
            destination: MapPoint::new(500.0, 500.0),
            running: false,
            show_marker: true,
            goal_override: Some((crate::sector::SectorNumber::new(1), 0)),
            goal_sector_index_override: Some(exact_sector),
            door_route_override: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: Vec::new(),
        },
    );

    assert_eq!(engine.orders.sequence_manager.sequence_count(), 0);
    for (pc, expected) in [
        (pc_a, MapPoint::new(490.0, 500.0)),
        (pc_b, MapPoint::new(510.0, 500.0)),
    ] {
        let steps = &engine
            .players
            .macro_store
            .get(pc)
            .and_then(|state| state.slot(0))
            .expect("recorded slot")
            .steps;
        assert_eq!(steps.len(), 1);
        let QaReplayCommand::Move {
            destination,
            running,
            route: Some(route),
        } = steps[0].replay
        else {
            panic!("per-PC group move did not retain its resolved route")
        };
        assert_eq!(destination, expected);
        assert!(!running);
        assert_eq!(route.goal_sector_index, exact_sector);
    }
    let stop_count = engine
        .orders
        .messenger
        .drain()
        .into_iter()
        .filter(|message| {
            matches!(
                message.msg_type,
                crate::messenger::MessageType::Pc(
                    crate::messenger::PcMessage::StopRecordingMacro,
                    None
                )
            )
        })
        .count();
    assert_eq!(stop_count, 1, "one global stop follows every recorded PC");
}

#[test]
fn group_move_recording_suppresses_only_armed_actor_and_launches_live_sibling() {
    use crate::player_command::PlayerCommand;

    let sim = crate::sim_rng::test_context();
    let mut display = HostDisplayState::default();
    let mut input = crate::engine::InputState::default();
    let assets = crate::engine::LevelAssets::new();
    let mut engine = EngineInner::new();
    let exact_sector = add_group_move_test_sector(&mut engine);
    let recording_pc = add_test_pc(&mut engine);
    let live_pc = add_test_pc(&mut engine);
    engine
        .get_entity_mut(recording_pc)
        .expect("recording PC")
        .element_data_mut()
        .set_position_map(MapPoint::new(100.0, 100.0));
    engine
        .get_entity_mut(live_pc)
        .expect("live PC")
        .element_data_mut()
        .set_position_map(MapPoint::new(120.0, 100.0));
    arm_group_move_recording(&mut engine, &[recording_pc]);

    engine.apply_command(
        &sim,
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::GroupMove {
            actors: vec![recording_pc, live_pc],
            destination: MapPoint::new(500.0, 500.0),
            running: false,
            show_marker: false,
            goal_override: Some((crate::sector::SectorNumber::new(1), 0)),
            goal_sector_index_override: Some(exact_sector),
            door_route_override: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: Vec::new(),
        },
    );

    assert_eq!(
        engine
            .players
            .macro_store
            .get(recording_pc)
            .and_then(|state| state.slot(0))
            .expect("recorded slot")
            .steps
            .len(),
        1
    );
    assert_eq!(engine.orders.sequence_manager.sequence_count(), 1);
    let launched = engine
        .orders
        .sequence_manager
        .sequences_iter()
        .next()
        .expect("live sibling movement");
    assert!(
        launched.elements.iter().all(|element| {
            element.owner != Some(recording_pc) && element.owner == Some(live_pc)
        })
    );
}

/// `EngineInner::has_quick_action` reports whether a PC has a macro in a slot.
#[test]
fn has_quick_action_reads_macro_store() {
    use crate::element::EntityId;

    let mut engine = EngineInner::new();
    let pc = EntityId::Pc(crate::entity_id::PcId(10));

    assert!(!engine.has_quick_action(pc, 0));

    seed_macro_slot(&mut engine, pc, 1, vec![(100.0, 100.0)]);

    assert!(!engine.has_quick_action(pc, 0));
    assert!(engine.has_quick_action(pc, 1));
    assert!(!engine.has_quick_action(pc, 2));
}

/// `EngineInner::abort_quick_action` drops the slot's titbit and clears the
/// slot.
#[test]
fn abort_quick_action_clears_slot_and_titbit() {
    let mut engine = EngineInner::new();
    let pc = add_test_pc(&mut engine);

    // Empty slot → false.
    assert!(!engine.abort_quick_action(pc, 0));

    seed_macro_slot(&mut engine, pc, 2, vec![(1.0, 2.0), (3.0, 4.0)]);
    assert!(engine.has_quick_action(pc, 2));
    let titbit_count_before = engine.feedback.titbit_manager.titbits().len();
    assert_eq!(titbit_count_before, 1);

    // Aborting returns true and fully clears state.
    assert!(engine.abort_quick_action(pc, 2));
    assert!(!engine.has_quick_action(pc, 2));
    assert!(engine.feedback.titbit_manager.titbits().is_empty());

    // A second abort is a no-op.
    assert!(!engine.abort_quick_action(pc, 2));
}

/// `DeleteMacro` PlayerCommand: single-PC variant drops one slot
/// without tetris; all-PC variant drops + collapses.
#[test]
fn delete_macro_command_matches_original_single_vs_all() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut display = HostDisplayState::default();
    use crate::player_command::PlayerCommand;

    let mut engine = EngineInner::new();
    let pc_a = add_test_pc(&mut engine);
    let pc_b = add_test_pc(&mut engine);

    // Both PCs have macros in slots 0 and 1; slot 2 is empty.
    seed_macro_slot(&mut engine, pc_a, 0, vec![(1.0, 1.0)]);
    seed_macro_slot(&mut engine, pc_a, 1, vec![(2.0, 2.0)]);
    seed_macro_slot(&mut engine, pc_b, 0, vec![(3.0, 3.0)]);
    seed_macro_slot(&mut engine, pc_b, 1, vec![(4.0, 4.0)]);

    let mut input = crate::engine::InputState::default();
    let assets = crate::engine::LevelAssets::new();

    // Single-PC delete: only pc_a slot 0 cleared; no tetris → pc_a slot 1
    // stays in slot 1.
    engine.apply_command(
        sim,
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::DeleteMacro {
            pc: Some(pc_a),
            slot: 0,
        },
    );
    assert!(!engine.has_quick_action(pc_a, 0));
    assert!(engine.has_quick_action(pc_a, 1));
    assert!(engine.has_quick_action(pc_b, 0));
    assert!(engine.has_quick_action(pc_b, 1));

    // All-PC delete on slot 0: pc_b slot 0 cleared, tetris collapses
    // remaining slots so pc_a/pc_b slot 0 now hold what used to be slot 1.
    engine.apply_command(
        sim,
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::DeleteMacro { pc: None, slot: 0 },
    );
    assert!(engine.has_quick_action(pc_a, 0)); // was pc_a slot 1
    assert!(engine.has_quick_action(pc_b, 0)); // was pc_b slot 1
    assert!(!engine.has_quick_action(pc_a, 1));
    assert!(!engine.has_quick_action(pc_b, 1));
}

/// `StartMacro` replays a move-only macro and fires the dotted-chain
/// commands through `apply_command`.  After playback the slot is empty
/// and its titbit is gone.  For the all-PC variant on a slot where every
/// PC had a macro, tetris collapses the strip.
#[test]
fn start_macro_plays_back_move_steps_and_tetris_collapses() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut display = HostDisplayState::default();
    use crate::player_command::PlayerCommand;

    let mut engine = EngineInner::new();
    let pc_a = add_test_pc(&mut engine);
    let pc_b = add_test_pc(&mut engine);

    // Both PCs record a one-step move macro at slot 0; pc_a has a slot-1
    // macro too.
    seed_macro_slot(&mut engine, pc_a, 0, vec![(50.0, 60.0)]);
    seed_macro_slot(&mut engine, pc_b, 0, vec![(70.0, 80.0)]);
    seed_macro_slot(&mut engine, pc_a, 1, vec![(90.0, 100.0)]);

    let mut input = crate::engine::InputState::default();
    let assets = crate::engine::LevelAssets::new();

    // Sanity: titbit manager holds all three macro titbits.
    assert_eq!(engine.feedback.titbit_manager.titbits().len(), 3);

    // All-PC StartMacro on slot 0: both PCs launch → slot 0 emptied for
    // both, then tetris shifts slot 1 → slot 0.
    engine.apply_command(
        sim,
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::StartMacro { pc: None, slot: 0 },
    );

    // pc_a: slot 0 now holds what was slot 1 (90, 100); slot 1 is empty.
    // pc_b: all slots empty.
    assert!(engine.has_quick_action(pc_a, 0));
    assert!(!engine.has_quick_action(pc_a, 1));
    assert!(!engine.has_quick_action(pc_b, 0));
    assert!(!engine.has_quick_action(pc_b, 1));

    // The launched macros' titbits are gone; only pc_a's (was-slot-1)
    // titbit remains.
    assert_eq!(engine.feedback.titbit_manager.titbits().len(), 1);
}

/// `StartMacro` on an empty slot is a no-op: no dispatch, no tetris.
#[test]
fn start_macro_empty_slot_is_noop() {
    let sim_context = crate::sim_rng::test_context();
    let sim = &sim_context;
    let mut display = HostDisplayState::default();
    use crate::player_command::PlayerCommand;

    let mut engine = EngineInner::new();
    let pc = add_test_pc(&mut engine);

    // pc has a macro only in slot 2 — starting slot 0 should NOT tetris,
    // because no PC had a slot-0 macro to launch.
    seed_macro_slot(&mut engine, pc, 2, vec![(1.0, 1.0)]);

    let mut input = crate::engine::InputState::default();
    let assets = crate::engine::LevelAssets::new();

    engine.apply_command(
        sim,
        &mut display,
        &mut input,
        &assets,
        &PlayerCommand::StartMacro { pc: None, slot: 0 },
    );

    // Slot 2 should still hold the macro — no tetris ran because the
    // start was a no-op.
    assert!(engine.has_quick_action(pc, 2));
    assert!(!engine.has_quick_action(pc, 0));
    assert!(!engine.has_quick_action(pc, 1));
}

#[test]
fn sherwood_harvest_detaches_production_sector_and_clears_points() {
    let mut engine = EngineInner::new();
    let mut sector =
        crate::sector_production::SectorProduction::new(crate::sector_production::Type::MakeArrow);
    sector.script_zone = Some(3);
    sector
        .production_points
        .push(crate::sector_production::Point {
            x: 12.0,
            y: 34.0,
            layer: 2,
            sector: 7,
            obstacle: 0xFFFF,
        });
    engine.mission_domain.campaign.production_sectors = vec![sector];

    engine.harvest_production_sector_state(&LevelAssets::new());

    let sector = &engine.mission_domain.campaign.production_sectors[0];
    assert_eq!(sector.script_zone, None);
    assert!(sector.production_points.is_empty());
}
