use super::*;

// ── QA macro playback / abort system tests ─────────────────────────

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
    // Aborting writes the cleared slot back onto the owner's PcData, so the
    // quick-action owner must be a real PC entity.
    let pc = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

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
    // Deleting / tetris-shifting rewrites the owners' PcData slots, so the
    // quick-action owners must be real PC entities.
    let pc_a = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let pc_b = engine.add_entity(make_test_pc(crate::element::Posture::Upright));

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
    // Macro playback and the post-launch tetris shift both write back into
    // the owners' PcData slots, so the owners must be real PC entities. The
    // replayed Move dispatch also reads each mover's collision move box.
    let pc_a = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    let pc_b = engine.add_entity(make_test_pc(crate::element::Posture::Upright));
    for (i, pc) in [pc_a, pc_b].into_iter().enumerate() {
        use crate::coordinates::{MapVec, MoveBox};
        let element = engine.get_entity_mut(pc).unwrap().element_data_mut();
        element
            .sprite
            .position_iface
            .set_move_box(MoveBox::from_corners(
                MapVec::new(-5.0, -5.0),
                MapVec::new(5.0, 5.0),
            ));
        // Re-derive the map-space move box from the position: the group-move
        // destination authorization reads it.
        element.set_position_map(crate::coordinates::MapPoint::new(
            10.0 + i as f32 * 20.0,
            10.0,
        ));
    }

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
    use crate::element::EntityId;
    use crate::player_command::PlayerCommand;

    let mut engine = EngineInner::new();
    let pc = EntityId::Pc(crate::entity_id::PcId(50));
    engine.world.pc_ids.push(pc);

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
