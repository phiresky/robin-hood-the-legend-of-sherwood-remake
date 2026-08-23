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

/// Seed a PC's macro slot with a recorded "move to (x,y)" step and a
/// wired titbit.  Used by the playback/abort/tetris tests below.
#[cfg(test)]
fn seed_macro_slot(
    engine: &mut EngineInner,
    pc: crate::element::EntityId,
    slot: u8,
    steps: Vec<(f32, f32)>,
) {
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
            },
        });
    }
    state.stop_recording();
    state.set_slot_titbit(
        slot as usize,
        crate::titbit::TitbitId::new(titbit_id).unwrap(),
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
