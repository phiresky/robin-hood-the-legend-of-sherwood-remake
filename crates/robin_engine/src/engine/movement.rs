//! Movement ticking, pathfinding dispatch, and order processing.

use super::*;
use crate::coordinates::{MapBBox, MapPoint, MapVec};
use crate::element::{ActiveDoorPass, EntityId};
use crate::entities::EntitySlots;
use crate::movement::ActiveMovement;
use crate::order::OrderType;
use crate::position_interface::vector_to_sector_0_to_15;
use crate::sprite::{FrameProgression, MotionMethod, MotionOrderContext, MotionState};

mod combat_motion;
mod door_traversal;
mod elevation;
mod formation;
mod rider_charge;
mod routing;

pub(in crate::engine) use formation::PlannedRecordedGroupMoveOutcome;

use combat_motion::{
    combat_directional_animation, combat_movement_angle, executes_sword_movement_action,
    is_sword_motion_context, is_sword_movement_nonanimation, sword_movement_dispatch_action,
};
use rider_charge::{is_galopp_decision_frame, rider_charge_point_in_quad};

#[inline]
fn debug_post_seek_handoff_enabled() -> bool {
    std::env::var_os("PARITY_DEBUG_POST_SEEK_HANDOFF").is_some()
}

#[derive(Debug, Clone, Copy)]
enum MovementPopGoalOwnerKind {
    Pc,
    Soldier,
    Civilian,
}

#[derive(Debug, Clone, Copy)]
struct MovementPopGoalOwnerDebugConfig {
    frame: u32,
    kind: MovementPopGoalOwnerKind,
    index: u32,
}

fn movement_pop_goal_owner_debug_config() -> Option<&'static MovementPopGoalOwnerDebugConfig> {
    static CONFIG: std::sync::OnceLock<Option<MovementPopGoalOwnerDebugConfig>> =
        std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            std::env::var_os("PARITY_DEBUG_GOAL_OWNER_HANDOFF")?;
            let frame = std::env::var("PARITY_DEBUG_GOAL_OWNER_FRAME").unwrap_or_else(|_| {
                panic!(
                    "PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER_FRAME=FRAME"
                )
            });
            let frame = frame.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid PARITY_DEBUG_GOAL_OWNER_FRAME={frame:?}: {error}")
            });
            let owner = std::env::var("PARITY_DEBUG_GOAL_OWNER").unwrap_or_else(|_| {
                panic!(
                    "PARITY_DEBUG_GOAL_OWNER_HANDOFF requires PARITY_DEBUG_GOAL_OWNER=pc|soldier|civilian:INDEX"
                )
            });
            let (kind, index) = owner.split_once(':').unwrap_or_else(|| {
                panic!("PARITY_DEBUG_GOAL_OWNER must look like pc|soldier|civilian:INDEX")
            });
            let kind = match kind {
                "pc" => MovementPopGoalOwnerKind::Pc,
                "soldier" => MovementPopGoalOwnerKind::Soldier,
                "civilian" => MovementPopGoalOwnerKind::Civilian,
                unsupported => {
                    panic!("PARITY_DEBUG_GOAL_OWNER has unsupported kind {unsupported:?}")
                }
            };
            let index = index.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid PARITY_DEBUG_GOAL_OWNER={owner:?}: {error}")
            });
            Some(MovementPopGoalOwnerDebugConfig { frame, kind, index })
        })
        .as_ref()
}

fn movement_pop_goal_owner_debug_matches(frame: u32, owner: EntityId) -> bool {
    let Some(config) = movement_pop_goal_owner_debug_config() else {
        return false;
    };
    config.frame == frame
        && config.index == owner.index()
        && matches!(
            (config.kind, owner),
            (MovementPopGoalOwnerKind::Pc, EntityId::Pc(_))
                | (MovementPopGoalOwnerKind::Soldier, EntityId::Soldier(_))
                | (MovementPopGoalOwnerKind::Civilian, EntityId::Civilian(_))
        )
}

/// Per-owner lift translation snapshot consumed by movement Execute's live
/// animation derivation. Covers the lift cases of
/// `DetermineMovementAnimation`.
#[derive(Debug, Clone, Copy)]
enum LiftAnimContext {
    /// Upright posture in a lift sector.  Upwards and downwards animations
    /// are asserted equal for upright posture, so a single mapping covers
    /// both directions.
    Upright(crate::sector::LiftType),
    /// On-ladder / on-wall posture in a ladder or wall lift sector.  The
    /// per-frame upwards-vs-downwards pick comes from the dot product of
    /// the ladder vector (low point minus high point) with the actor's
    /// movement vector.  `ladder_dx` / `ladder_dy` is that ladder vector
    /// in map coordinates.
    OnClimb {
        lift_type: crate::sector::LiftType,
        lift_direction: i16,
        ladder_dx: f32,
        ladder_dy: f32,
    },
}

/// Original `RHElementActor::InstructOwner(RHCOMMAND_MOVE)` direct-dispatch
/// predicate. `RHMOVE_LINE` changes post-processing of the resulting path; it
/// does not by itself bypass `RHPathFinder::AddPathRequest`.
#[inline]
fn movement_flags_force_direct_dispatch(flags: crate::sequence::MoveFlags) -> bool {
    flags.contains(crate::sequence::MoveFlags::MAP)
        || flags.contains(crate::sequence::MoveFlags::STRAIGHT)
}

/// `RHPathFinder::AddPathRequest` runs this gate for every request it receives;
/// command type and actor posture do not provide bypasses.
#[inline]
fn path_request_needs_source_extraction(direct_dispatch: bool, source_authorized: bool) -> bool {
    !direct_dispatch && !source_authorized
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActorPostSeekInteraction {
    Hit,
    Tie,
    Untie,
}

/// Identify an actor-owned interaction whose init-time 40-unit validity
/// guard immediately follows a completed entity Seek.
///
/// The copied terminal movement can lose its own target, but the Original
/// retains one `mpSeekTarget` pointer on the actor. Scope the early handoff to
/// a post-seek interaction with that exact antagonist; an unrelated tail must
/// remain on the ordinary sequence-manager path.
fn actor_post_seek_interaction(
    actor: &crate::element::ActorData,
) -> Option<ActorPostSeekInteraction> {
    let element = actor
        .post_seek_sequence
        .as_ref()
        .and_then(|sequence| sequence.elements.first())?;
    let antagonist = match &element.data {
        crate::sequence::SequenceElementData::Interaction { antagonist } => *antagonist,
        _ => None,
    }?;
    if actor.seek_target != Some(antagonist) {
        return None;
    }
    match element.command {
        crate::element::Command::HitCmd => Some(ActorPostSeekInteraction::Hit),
        crate::element::Command::TieCmd => Some(ActorPostSeekInteraction::Tie),
        crate::element::Command::Untie => Some(ActorPostSeekInteraction::Untie),
        _ => None,
    }
}

/// Original Hit and Tie initialization compare the raw map-space
/// `SBGeoVector2D::SquareNorm()` with 1600
/// (`RHelementactorhuman.cpp:6653-6678`, `RHelementactorpc.cpp:7157-7181`).
/// Keep the strict comparison: a victim at exactly 40 units is valid.
fn interaction_exceeds_init_range(owner: MapPoint, victim: MapPoint) -> bool {
    let dx = victim.x - owner.x;
    let dy = victim.y - owner.y;
    dx * dx + dy * dy > 1600.0
}

#[cfg(test)]
mod post_seek_hit_handoff_tests {
    use super::*;

    #[test]
    fn hit_init_range_uses_raw_map_square_norm_and_includes_exact_boundary() {
        let owner = MapPoint::new(1_099.375_2, 1_823.835_4);
        let nescafe_target = MapPoint::new(1055.0, 1790.0);
        assert!(interaction_exceeds_init_range(owner, nescafe_target));

        let owner = MapPoint::new(1_483.855_8, 2720.03);
        let cyrdach_target = MapPoint::new(1470.0, 2759.0);
        assert!(interaction_exceeds_init_range(owner, cyrdach_target));

        assert!(!interaction_exceeds_init_range(
            MapPoint::ZERO,
            MapPoint::new(40.0, 0.0)
        ));
        assert!(interaction_exceeds_init_range(
            MapPoint::ZERO,
            MapPoint::new(f32::from_bits(40.0f32.to_bits() + 1), 0.0)
        ));
    }
}

/// Input passed to a ladder/wall lift's action translation.
///
/// Original `DetermineMovementAnimation` passes an authored upright walk/run
/// action through verbatim: the lift itself maps `RunningUpright` to its fast
/// climb row, independently of `RHMOVE_FAST`. Rust can also reach this point
/// with a carried movement variant, where the element's speed flag remains the
/// useful normalization signal.
#[inline]
fn climb_lift_translation_input(action: OrderType, is_fast: bool) -> OrderType {
    match action {
        OrderType::WalkingUpright | OrderType::RunningUpright => action,
        OrderType::WalkingWithSword
        | OrderType::RunningWithSword
        | OrderType::WalkingWithShield
        | OrderType::WalkingCrouched
        | OrderType::WalkingWithCorpse => {
            if is_fast {
                OrderType::RunningUpright
            } else {
                OrderType::WalkingUpright
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod group_move_authorization_tests {
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
        let mut element = crate::element::ElementData {
            kind: crate::element::ElementKind::ActorPc,
            ..Default::default()
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
        // The recorded movement level is an RHPosition property, independent
        // from the retained RHSector pointer's own topology layer.
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
        let live_box =
            MapBBox::from_coords(actor.x - 6.0, actor.y - 4.0, actor.x + 6.0, actor.y + 4.0);

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
        // RecordGroupMove's patch-aware pSectorGoal remains sector 288/L4.
        // Losing the recorded identity turns the command into a same-sector
        // move; preserving it reaches gate A*, whose failure leaves the old
        // Wait sequence installed just as AppendMoveToSequence does.
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
            &[exact_door],
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
}

/// Apply the lift-sector portion of Original
/// `RHElementActor::DetermineMovementAnimation`.
///
/// The current actor sector is authoritative. In particular, an actor leaving
/// a lift translates its movement action before the door callback changes the
/// sector, while an actor approaching the lift from outside does not.
pub(super) fn grid_sector_for_position_handle(
    level: &crate::fast_find_grid::LevelGrid,
    sector: crate::position_interface::SectorHandle,
) -> Option<&crate::fast_find_grid::GridSector> {
    match sector.arena_index() {
        Some(index) => Some(level.sectors.get(usize::from(index)).unwrap_or_else(|| {
            panic!(
                "sector {} carries missing exact arena index {}",
                sector.get(),
                index.get()
            )
        })),
        None => {
            let number = crate::sector::SectorNumber::new(i16::from(sector));
            level
                .sector_number_map
                .get(&number)
                .and_then(|&index| level.sectors.get(index))
        }
    }
}

pub(super) fn lift_endpoint_points_for_sector(
    sector: &crate::fast_find_grid::GridSector,
) -> (MapPoint, MapPoint) {
    let low = sector.low_exit_point.unwrap_or_else(|| {
        panic!(
            "DetermineMovementAnimation: lift sector {} missing low exit point",
            sector.sector_number
        )
    });
    let high = sector.high_exit_point.unwrap_or_else(|| {
        panic!(
            "DetermineMovementAnimation: lift sector {} missing high exit point",
            sector.sector_number
        )
    });
    (low, high)
}

pub(super) fn determine_lift_movement_animation_for(
    entity: &crate::element::Entity,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    posture_after: crate::element::Posture,
    action: OrderType,
    destination: MapPoint,
) -> OrderType {
    use crate::element::Posture;

    let elem = entity.element_data();
    let posture = if posture_after == Posture::Undefined {
        elem.posture
    } else {
        posture_after
    };
    let Some(sector_handle) = elem.sector() else {
        return action;
    };
    // Original samples the actor's live `RHSector*`. Public sector numbers
    // are not unique, so retain the arena identity carried by RHposition;
    // number lookup is only the compatibility path for identity-less saves.
    let Some(sector) = grid_sector_for_position_handle(&fast_grid.level, sector_handle) else {
        return action;
    };
    let Some(lift_type) = sector.lift_type else {
        return action;
    };

    match posture {
        Posture::Upright => lift_type.translate_upright_action(action),
        Posture::OnWall | Posture::OnLadder => {
            if !matches!(
                (posture, lift_type),
                (Posture::OnWall, crate::sector::LiftType::Wall)
                    | (Posture::OnLadder, crate::sector::LiftType::Ladder)
            ) {
                tracing::warn!(
                    ?posture,
                    ?lift_type,
                    sector = %sector.sector_number,
                    "DetermineMovementAnimation: climb posture does not match lift sector"
                );
                return action;
            }
            let (low, high) = lift_endpoint_points_for_sector(sector);
            let position = elem.position_map();
            let ladder_dx = low.x - high.x;
            let ladder_dy = low.y - high.y;
            let movement_dx = destination.x - position.x;
            let movement_dy = destination.y - position.y;
            let going_down = ladder_dx * movement_dx + ladder_dy * movement_dy >= 0.0;
            lift_type.translate_climb_action(action, going_down)
        }
        // Original's default posture arm still applies the lift's upright
        // action translation (RHelementactor.cpp:4735-4745). This matters for
        // resumed PassDoor elements whose serialized transition result is a
        // non-movement posture such as Lying: while the live actor is already
        // upright in the lift, that dormant result remains stamped on the
        // element and the stairs action must still be selected.
        Posture::CarryingCorpse
        | Posture::Crouched
        | Posture::CarryingOnShoulders
        | Posture::HelpingToClimb
        | Posture::SimulatingBeggar => action,
        _ => lift_type.translate_upright_action(action),
    }
}

#[cfg(test)]
mod exact_lift_sector_tests {
    use super::*;
    use crate::element::{ActorData, ActorPc, ElementData, ElementKind, Entity, HumanData, PcData};
    use crate::fast_find_grid::{FastFindGrid, GridSector, SectorIndex};
    use crate::sector::{LiftType, SectorNumber, SectorType};

    fn sector(number: SectorNumber, lift: Option<LiftType>) -> GridSector {
        GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: lift.map_or(SectorType::AREA, |_| SectorType::LIFT),
            layer: 3,
            sector_number: number,
            door_index: None,
            lift_type: lift,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: lift.map(|_| MapPoint::new(2279.0, 1300.0)),
            high_exit_point: lift.map(|_| MapPoint::new(2279.0, 1200.0)),
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        }
    }

    fn pc126(sector: crate::position_interface::SectorHandle) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            posture: crate::element::Posture::OnWall,
            ..ElementData::default()
        };
        element.set_position_map(MapPoint::new(2_278.88, 1257.0005));
        element.sprite.position_iface.set_sector_topology(
            crate::position_interface::SectorHandle::new(sector.get()),
            sector.arena_index(),
        );
        Entity::Pc(ActorPc {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    #[test]
    fn pc126_on_wall_uses_exact_duplicate_sector_for_climbing_down() {
        let public = SectorNumber::new(62);
        let ordinary_index = SectorIndex::new(0).unwrap();
        let wall_index = SectorIndex::new(1).unwrap();
        let mut grid = FastFindGrid::default();
        let level = std::sync::Arc::make_mut(&mut grid.level);
        level.sectors.push(sector(public, None));
        level.sectors.push(sector(public, Some(LiftType::Wall)));
        level
            .sector_number_map
            .insert(public, usize::from(ordinary_index));

        let exact_wall = crate::position_interface::SectorHandle::new(62)
            .unwrap()
            .with_arena_index(wall_index);
        assert_eq!(
            determine_lift_movement_animation_for(
                &pc126(exact_wall),
                &grid,
                crate::element::Posture::OnWall,
                OrderType::WalkingUpright,
                MapPoint::new(2279.0, 1269.0),
            ),
            OrderType::ClimbingWallDown,
            "Pc126's gate-73 approach must use the exact Wall sector's downward action"
        );

        let exact_ordinary = crate::position_interface::SectorHandle::new(62)
            .unwrap()
            .with_arena_index(ordinary_index);
        assert_eq!(
            determine_lift_movement_animation_for(
                &pc126(exact_ordinary),
                &grid,
                crate::element::Posture::OnWall,
                OrderType::WalkingUpright,
                MapPoint::new(2279.0, 1269.0),
            ),
            OrderType::WalkingUpright,
            "an exact ordinary duplicate must not acquire Wall movement"
        );

        assert_eq!(
            determine_lift_movement_animation_for(
                &pc126(crate::position_interface::SectorHandle::new(62).unwrap()),
                &grid,
                crate::element::Posture::OnWall,
                OrderType::WalkingUpright,
                MapPoint::new(2279.0, 1269.0),
            ),
            OrderType::WalkingUpright,
            "identity-less legacy positions retain the public-number fallback"
        );
    }

    #[test]
    #[should_panic(expected = "sector 62 carries missing exact arena index 9")]
    fn exact_sector_identity_never_falls_back_when_its_arena_object_is_missing() {
        let grid = FastFindGrid::default();
        let missing = crate::position_interface::SectorHandle::new(62)
            .unwrap()
            .with_arena_index(SectorIndex::new(9).unwrap());
        let _ = grid_sector_for_position_handle(&grid.level, missing);
    }
}

#[cfg(test)]
mod line_crossing_eligibility_tests {
    use super::actor_line_crossing_eligible;
    use crate::element::Posture;

    #[test]
    fn wall_and_ladder_climbers_still_check_elevation_lines() {
        assert!(actor_line_crossing_eligible(Posture::OnWall, false, true));
        assert!(actor_line_crossing_eligible(Posture::OnLadder, false, true));
        assert!(!actor_line_crossing_eligible(Posture::Flying, false, true));
        assert!(!actor_line_crossing_eligible(Posture::OnWall, true, true));
        assert!(!actor_line_crossing_eligible(Posture::OnWall, false, false));
    }
}

/// Mobile geometry sampled at one actor's live creation-order slot.
///
/// Unlike the other immutable movement preparation, this must not escape the
/// owner boundary: an actor before a mobile sees its previous position and an
/// actor after the mobile sees the geometry translated by that master's
/// `Hourglass`.
struct LiveMobileGeometry {
    mobile_lines_by_layer: std::collections::BTreeMap<u16, Vec<crate::fast_find_grid::GridLine>>,
    mobile_points_by_layer: std::collections::BTreeMap<u16, Vec<crate::repulsive::RepulsivePoint>>,
    mobile_polygons_by_layer:
        std::collections::BTreeMap<u16, Vec<Vec<crate::coordinates::MapPoint>>>,
}

#[derive(Clone, Copy, Debug)]
struct RiderChargeExecution {
    /// Identity of the same order object after Execute returned. Rider charge
    /// may legitimately assign that object a fresh ID on its last animation
    /// frame. `None` means a synchronous callback replaced the entry object
    /// while Execute was still running.
    completion_order_id: Option<std::num::NonZeroU32>,
}

#[cfg(test)]
thread_local! {
    static LAST_MOBILE_CROSSING_INCREMENT: std::cell::Cell<Option<MapVec>> = const { std::cell::Cell::new(None) };
    static POST_EXECUTE_CROSSING_OBSERVER: std::cell::RefCell<Option<Box<dyn FnMut(&mut EngineInner, EntityId)>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn take_last_mobile_crossing_increment() -> Option<MapVec> {
    LAST_MOBILE_CROSSING_INCREMENT.with(|increment| increment.take())
}

#[cfg(test)]
pub(super) fn set_post_execute_crossing_observer(
    observer: Option<Box<dyn FnMut(&mut EngineInner, EntityId)>>,
) {
    POST_EXECUTE_CROSSING_OBSERVER.with(|slot| *slot.borrow_mut() = observer);
}

#[cfg(test)]
fn observe_post_execute_crossing(engine: &mut EngineInner, entity_id: EntityId) {
    POST_EXECUTE_CROSSING_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow_mut().as_mut() {
            observer(engine, entity_id);
        }
    });
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MovementOwnerSelection {
    pub seq_id: crate::sequence::SequenceId,
    pub elem_idx: usize,
    pub order_id: std::num::NonZeroU32,
}

fn order_uses_distance_motion(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::WalkingUpright
            | OrderType::WalkingCrouched
            | OrderType::WalkingAlerted
            | OrderType::RunningUpright
            | OrderType::WalkingWithSword
            | OrderType::RunningWithSword
            | OrderType::WalkingStairs
            | OrderType::WalkingStairsAlerted
            | OrderType::RunningStairs
            | OrderType::WalkingSword
            | OrderType::WalkingBackwardsSword
            | OrderType::StrafingRightSword
            | OrderType::StrafingLeftSword
            | OrderType::WalkingShield
            | OrderType::WalkingBackwardsShield
            | OrderType::StrafingRightShield
            | OrderType::StrafingLeftShield
            | OrderType::WalkingWithCorpse
            | OrderType::WalkingCarryingOnShoulders
            | OrderType::ClimbingWallUp
            | OrderType::ClimbingWallDown
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast
            | OrderType::ClimbingLadderUp
            | OrderType::ClimbingLadderDown
            | OrderType::ClimbingLadderUpAlerted
            | OrderType::ClimbingLadderDownAlerted
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
    )
}

#[inline]
fn refresh_pc_walking_shield_after_execute(
    entity: &mut crate::element::Entity,
    profiles: &crate::profiles::ProfileManager,
    order_action: OrderType,
) {
    if entity.is_pc() && order_action == OrderType::WalkingWithShield {
        crate::bow_shot::refresh_retained_shield_obstacle(entity, profiles);
    }
}

/// Keep the split door-pass walk mirror aligned with the concrete order that
/// has reached the actor slot.
///
/// Original stores the translated door route and posture-transition copies in
/// one order list. Once a transition retires, the following walk/run order is
/// immediately authoritative. Rust keeps the untranslated tail in
/// `ActiveDoorPass`; without this rebind a transition written into
/// `current_action` by MakeFast/MakeSlow can continue supplying the sprite row
/// while the concrete successor is already executing.
fn synchronize_selected_door_pass_walk_action(
    current_action: &mut OrderType,
    selected_action: OrderType,
) {
    if order_uses_distance_motion(selected_action) {
        *current_action = selected_action;
    }
}

/// Mirror `RHSprite::PerformMotion(TILL_LAST_FRAME)` deleting every following
/// order when a transition loops short of its goal and cannot find a later
/// distinct animation with a nonzero destination.
///
/// Original's translated door route lives in that one order list
/// (`RHsprite.cpp:1844-1881`). Rust stores its untranslated suffix separately,
/// so the same deletion must also empty `ActiveDoorPass::steps` or a discarded
/// zero-destination `PASSING_DOOR` action point will be materialized again.
fn discard_lazy_door_pass_following_orders(pass: Option<&mut ActiveDoorPass>) {
    if let Some(pass) = pass {
        pass.steps.clear();
    }
}

/// Materialize the zero-destination action points which precede the next
/// authored door walk.
///
/// Original translates the complete door route up front. Rust normally
/// materializes these steps one at a time, but a TillLastFrame continuation
/// has to be inserted relative to the complete translated route. Moving this
/// prefix into the concrete order queue first preserves Original's
/// `Select -> PassingDoor -> copied walk` ordering.
fn materialize_door_action_point_prefix(
    pass: &mut ActiveDoorPass,
    next_order_id: &mut u32,
) -> Vec<crate::order::Order> {
    let mut orders = Vec::new();
    while let Some(step) = pass.steps.front() {
        let mut order = match step {
            crate::element::DoorPassStep::Select { speed } => {
                let mut order = crate::order::Order::new(
                    OrderType::Select,
                    0.0,
                    0.0,
                    crate::order::alloc_order_id(next_order_id),
                );
                order.compute_direction = true;
                order.tolerance = *speed;
                order
            }
            crate::element::DoorPassStep::PassingDoor => crate::order::Order::new(
                OrderType::PassingDoor,
                0.0,
                0.0,
                crate::order::alloc_order_id(next_order_id),
            ),
            _ => break,
        };
        // These action points now have concrete successors in the same order
        // list, so generic DoNextOrder owns their completion. ResumeDoorPass
        // would incorrectly materialize another lazy step alongside them.
        order.completion = crate::order::OrderCompletion::AdvanceElement;
        pass.steps.pop_front();
        orders.push(order);
    }
    orders
}

/// Insert a materialized lazy door-pass step at the same side of a copied
/// transition-distance continuation as Original's single translated order
/// list.
///
/// `PerformMotion(TILL_LAST_FRAME)` inserts the copied continuation immediately
/// before the first later, distinct animation with a nonzero destination. Rust
/// stores the orders preceding that animation (PassingDoor and zero-target
/// posture transitions) in `ActiveDoorPass`, so those steps must be inserted
/// before the concrete continuation. The matching authored walk belongs after
/// it.
fn insert_door_pass_successor(
    element: &mut crate::sequence::SequenceElement,
    order: crate::order::Order,
) {
    let continuation = element
        .orders
        .iter()
        .position(|queued| queued.transition_distance_continuation);
    let Some(continuation) = continuation else {
        element.push_order(order);
        return;
    };
    let copied = &element.orders[continuation];
    let is_matching_authored_walk =
        (order.target_x != 0.0 || order.target_y != 0.0) && order.order_type == copied.order_type;
    let insertion = continuation + usize::from(is_matching_authored_walk);
    element.insert_order(insertion, order);
}

fn completed_door_pass_to_commit(
    discarded_following_orders: bool,
    completed: Option<(crate::gate::DoorIndex, bool)>,
) -> Option<(crate::gate::DoorIndex, bool)> {
    (!discarded_following_orders).then_some(completed).flatten()
}

/// Ignore a stale split door-route mirror when a distinct concrete transition
/// has reached the actor slot.
///
/// Original keeps the whole translated route in one order list, so an
/// explicit transition inserted by MakeFast/MakeSlow is authoritative. Rust's
/// mirror remains useful for concrete distance motion and for door-authored
/// transitions where it already agrees with the selected order.
fn door_pass_sprite_animation_override(
    selected_action: OrderType,
    current_action: Option<OrderType>,
) -> Option<OrderType> {
    current_action.filter(|current| {
        order_uses_distance_motion(selected_action) || *current == selected_action
    })
}

/// Movement Execute arms which call `Turn()` immediately before entering
/// `RHSprite::PerformMotion`.
///
/// This distinction remains observable under `FreezeAll`: the sprite returns
/// `RHMOTION_IN_PROGRESS` before animation or displacement, but the actor-side
/// turn has already happened (`RHelementactor.cpp:1142-1341`).
fn order_turns_before_motion(order: OrderType) -> bool {
    order_uses_distance_motion(order)
        || matches!(
            order,
            OrderType::TransitionWalkingUprightWaitingUpright
                | OrderType::TransitionRunningUprightWaitingUpright
                | OrderType::TransitionWaitingUprightWalkingUpright
                | OrderType::TransitionWaitingUprightRunningUpright
                | OrderType::TransitionWalkingUprightRunningUpright
                | OrderType::TransitionRunningUprightWalkingUpright
                | OrderType::TransitionWaitingCrouchedWalkingCrouched
                | OrderType::TransitionWalkingCrouchedWaitingCrouched
                | OrderType::TransitionWalkingCrouchedWalkingUpright
                | OrderType::TransitionWalkingUprightWalkingCrouched
                | OrderType::TransitionWalkingCrouchedRunningUpright
                | OrderType::TransitionRunningUprightWalkingCrouched
        )
}

/// Match `RHSprite::PerformMotion`: scale the sprite-frame distance by the
/// movement element's speed factor before applying the turn slowdown and its
/// minimum useful step. Direct transition orders call `PerformMotion` without
/// the element speed factor, while seek transitions route through
/// `RHElementActor::PerformSeek`, which passes the element factor explicitly.
pub(super) fn scaled_motion_distance(
    frame_distance: f32,
    speed_factor: f32,
    apply_speed_factor: bool,
    direction_differs_from_goal: bool,
) -> f32 {
    let mut distance = frame_distance
        * if apply_speed_factor {
            speed_factor
        } else {
            1.0
        };
    if direction_differs_from_goal && distance > 0.0 {
        distance *= 0.6;
        if distance < 0.7 {
            distance = 0.7;
        }
    }
    distance
}

fn climb_lift_type(action: OrderType) -> Option<crate::sector::LiftType> {
    use crate::sector::LiftType;

    match action {
        OrderType::TransitionWaitingUprightClimbingWallUp
        | OrderType::ClimbingWallUp
        | OrderType::ClimbingWallDown
        | OrderType::TransitionClimbingWallUpWaitingCrouched
        | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
        | OrderType::TransitionWaitingCrouchedClimbingWallDown
        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
        | OrderType::TransitionClimbingWallDownWaitingUpright
        | OrderType::ClimbingWallUpFast
        | OrderType::ClimbingWallDownFast => Some(LiftType::Wall),
        OrderType::TransitionWaitingUprightClimbingLadderUp
        | OrderType::TransitionWaitingUprightClimbingLadderUpAlerted
        | OrderType::TransitionClimbingLadderUpWaitingCrouched
        | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
        | OrderType::TransitionWaitingCrouchedClimbingLadderDown
        | OrderType::TransitionWaitingUprightClimbingLadderDownAlerted
        | OrderType::TransitionClimbingLadderDownWaitingUpright
        | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
        | OrderType::ClimbingLadderUp
        | OrderType::ClimbingLadderDown
        | OrderType::ClimbingLadderUpFast
        | OrderType::ClimbingLadderDownFast => Some(LiftType::Ladder),
        _ => None,
    }
}

/// Lift-wall and ladder orders are literal `RHElementActor::Execute` dispatch
/// actions, including their start/landing transitions. A split Rust door
/// route can retain a stale mirror of the preceding climb action, but that
/// mirror must not make the selected transition fall back to an action-state
/// walk animation.
#[inline]
fn literal_lift_sprite_action(action: OrderType) -> Option<OrderType> {
    climb_lift_type(action).map(|_| action)
}

#[inline]
fn door_type_uses_lift_climb_direction(door_type: crate::gate::DoorType) -> bool {
    matches!(
        door_type,
        crate::gate::DoorType::LiftHigh
            | crate::gate::DoorType::LiftHighCrenel
            | crate::gate::DoorType::LiftLow
    )
}

fn is_fast_climb_action(action: OrderType) -> bool {
    matches!(
        action,
        OrderType::RunningStairs
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
    )
}

/// Fast ladder/wall Execute arms return immediately when their first
/// `PerformMotion` call terminates. `RunningStairs` also executes two motion
/// calls per tick, but its loop deliberately has no such early return.
fn fast_climb_stops_after_first_termination(action: OrderType) -> bool {
    matches!(
        action,
        OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
    )
}

fn is_authored_climb_action(action: OrderType) -> bool {
    matches!(
        action,
        OrderType::ClimbingWallUp
            | OrderType::ClimbingWallDown
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast
            | OrderType::ClimbingLadderUp
            | OrderType::ClimbingLadderDown
            | OrderType::ClimbingLadderUpAlerted
            | OrderType::ClimbingLadderDownAlerted
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
    )
}

fn sprite_motion_order_for_nonanimation(order: OrderType) -> OrderType {
    match order {
        // legacy implementation RHNONANIMATION_CLIMBING_*_FAST tokens are dispatch /
        // pathfinder speed tokens. RHElementActor handles them by
        // playing the normal climb animation row with RHMOTIONMETHOD_RUN.
        OrderType::RunningStairs => OrderType::WalkingStairs,
        OrderType::ClimbingWallUpFast => OrderType::ClimbingWallUp,
        OrderType::ClimbingWallDownFast => OrderType::ClimbingWallDown,
        OrderType::ClimbingLadderUpFast => OrderType::ClimbingLadderUp,
        OrderType::ClimbingLadderDownFast => OrderType::ClimbingLadderDown,
        other => other,
    }
}

/// Whether an actor climb order applies the lift's fixed facing.
///
/// Original `RHElementActor::Execute` does this only inside
/// `IsInitialisation()`. A climb order reached recursively after a door step
/// can therefore start without replacing the facing inherited from that
/// transition.
fn initialising_climb_uses_lift_direction(
    action: OrderType,
    lift_type: crate::sector::LiftType,
    initialising: bool,
) -> bool {
    initialising
        && matches!(
            (action, lift_type),
            (
                OrderType::ClimbingWallUp
                    | OrderType::ClimbingWallDown
                    | OrderType::ClimbingWallUpFast
                    | OrderType::ClimbingWallDownFast,
                crate::sector::LiftType::Wall
            ) | (
                OrderType::ClimbingLadderUp
                    | OrderType::ClimbingLadderDown
                    | OrderType::ClimbingLadderUpFast
                    | OrderType::ClimbingLadderDownFast,
                crate::sector::LiftType::Ladder
            )
        )
}

/// Posture owned eagerly by a lift animation while executing a door-pass
/// step. Wall-exit transitions are different from the climb rows: Original
/// `RHElementActor::Execute` only inherits `OnWall` when the transition is
/// initialized, then its raw `DONE` edge is allowed to publish the landing
/// posture while the animation wrapper remains installed.
fn door_pass_eager_posture(
    action: OrderType,
    has_door_pass_animation: bool,
    execute_order_initialising: bool,
    decorative_building_trap_at_destination: bool,
) -> Option<crate::element::Posture> {
    use crate::element::Posture;

    if !has_door_pass_animation || decorative_building_trap_at_destination {
        return None;
    }
    match action {
        OrderType::ClimbingWallUp
        | OrderType::ClimbingWallDown
        | OrderType::ClimbingWallUpFast
        | OrderType::ClimbingWallDownFast => Some(Posture::OnWall),
        OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
            if execute_order_initialising =>
        {
            Some(Posture::Flying)
        }
        OrderType::TransitionClimbingWallUpWaitingCrouched
        | OrderType::TransitionClimbingWallDownWaitingUpright
            if execute_order_initialising =>
        {
            Some(Posture::OnWall)
        }
        OrderType::ClimbingLadderUp
        | OrderType::ClimbingLadderDown
        | OrderType::ClimbingLadderUpFast
        | OrderType::ClimbingLadderDownFast => Some(Posture::OnLadder),
        _ => None,
    }
}

/// Whether a terminal translated door transition still has an authoritative
/// PassDoor owner when the runtime `ActiveDoorPass` mirror is absent.
///
/// Restored Original saves can carry the complete translated order chain in
/// the serialized PassDoor sequence without reconstructing that Rust-only
/// mirror. The caller treats that serialized chain as ownership while the
/// actor's saved PositionInterface door supplies any geometry. The crenel
/// climb-up exit is also recoverable without either representation because its
/// Original Execute arm only publishes the PC's crouched/waiting state.
pub(super) fn pass_door_transition_completion_has_owner(
    command: crate::element::Command,
    has_materialized_or_restored_door_pass: bool,
    action: OrderType,
    is_pc: bool,
) -> bool {
    has_materialized_or_restored_door_pass
        || (command == crate::element::Command::PassDoor
            && is_pc
            && action == OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel)
}

#[cfg(test)]
mod door_pass_posture_tests {
    use super::*;
    use crate::element::{Command, Posture};
    use crate::order::Order;
    use crate::sequence::SequenceElement;

    #[test]
    fn recursively_reached_wall_exit_preserves_its_done_posture() {
        let transition = OrderType::TransitionClimbingWallUpWaitingCrouched;

        assert_eq!(
            door_pass_eager_posture(transition, true, true, false),
            Some(Posture::OnWall),
            "an initializing wall-exit transition inherits the climb posture"
        );
        assert_eq!(
            door_pass_eager_posture(transition, true, false, false),
            None,
            "a recursively reached Execute must not stomp the crouched posture published by DONE"
        );
        assert_eq!(
            door_pass_eager_posture(OrderType::ClimbingWallUp, true, false, false),
            Some(Posture::OnWall),
            "ordinary wall-climb rows continue owning their posture on every Execute"
        );
    }

    #[test]
    fn crenel_and_sibling_lift_transitions_keep_their_selected_sprite_action() {
        use OrderType as OT;

        // Interactive session 002, PC 126, frame 707: the selected crenel
        // transition is action 255 while the split door mirror still names
        // the preceding climb. Original dispatches action 255 literally.
        assert_eq!(
            literal_lift_sprite_action(OT::TransitionClimbingWallUpWaitingCrouchedCrenel),
            Some(OT::TransitionClimbingWallUpWaitingCrouchedCrenel)
        );

        for sibling in [
            OT::TransitionWaitingUprightClimbingWallUp,
            OT::TransitionClimbingWallUpWaitingCrouched,
            OT::TransitionWaitingCrouchedClimbingWallDown,
            OT::TransitionWaitingCrouchedClimbingWallDownCrenel,
            OT::TransitionClimbingWallDownWaitingUpright,
            OT::TransitionWaitingUprightClimbingLadderUp,
            OT::TransitionWaitingUprightClimbingLadderUpAlerted,
            OT::TransitionClimbingLadderUpWaitingCrouched,
            OT::TransitionClimbingLadderUpWaitingUprightAlerted,
            OT::TransitionWaitingCrouchedClimbingLadderDown,
            OT::TransitionWaitingUprightClimbingLadderDownAlerted,
            OT::TransitionClimbingLadderDownWaitingUpright,
            OT::TransitionClimbingLadderDownWaitingUprightAlerted,
        ] {
            assert_eq!(literal_lift_sprite_action(sibling), Some(sibling));
        }

        assert_eq!(
            door_pass_sprite_animation_override(
                OT::TransitionClimbingWallUpWaitingCrouchedCrenel,
                Some(OT::ClimbingWallUp),
            ),
            None,
            "a stale split-route mirror must not replace the selected transition"
        );
        assert_eq!(literal_lift_sprite_action(OT::PassingDoor), None);
        assert_eq!(literal_lift_sprite_action(OT::WalkingUpright), None);
    }

    #[test]
    fn restored_pass_door_completion_accepts_serialized_chain_ownership() {
        assert!(pass_door_transition_completion_has_owner(
            Command::PassDoor,
            false,
            OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
            true,
        ));
        assert!(pass_door_transition_completion_has_owner(
            Command::PassDoor,
            true,
            OrderType::TransitionWaitingCrouchedClimbingLadderDown,
            true,
        ));

        assert!(
            !pass_door_transition_completion_has_owner(
                Command::PassDoor,
                false,
                OrderType::TransitionClimbingWallDownWaitingUpright,
                true,
            ),
            "door-dependent transition completion still requires either the materialized pass or its restored serialized chain"
        );
        assert!(
            !pass_door_transition_completion_has_owner(
                Command::Move,
                false,
                OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
                true,
            ),
            "an unrelated movement sequence must not acquire PassDoor completion semantics"
        );
    }

    #[test]
    fn lazy_door_steps_keep_original_position_around_copied_continuation() {
        let owner = EntityId::Pc(crate::entity_id::PcId(1));
        let mut element = SequenceElement::new_movement(
            1,
            Command::PassDoor,
            Some(owner),
            OrderType::WalkingUpright,
        );
        element.orders.clear();
        element.orders.push_back(Order::new(
            OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
            20.0,
            30.0,
            std::num::NonZeroU32::new(1).unwrap(),
        ));
        let mut copied_walk = Order::new(
            OrderType::WalkingUpright,
            20.0,
            30.0,
            std::num::NonZeroU32::new(2).unwrap(),
        );
        copied_walk.transition_distance_continuation = true;
        element.orders.push_back(copied_walk);

        insert_door_pass_successor(
            &mut element,
            Order::new(
                OrderType::PassingDoor,
                0.0,
                0.0,
                std::num::NonZeroU32::new(3).unwrap(),
            ),
        );
        insert_door_pass_successor(
            &mut element,
            Order::new(
                OrderType::TransitionCrouchingUp,
                0.0,
                0.0,
                std::num::NonZeroU32::new(4).unwrap(),
            ),
        );
        insert_door_pass_successor(
            &mut element,
            Order::new(
                OrderType::WalkingUpright,
                20.0,
                30.0,
                std::num::NonZeroU32::new(5).unwrap(),
            ),
        );

        assert_eq!(
            element
                .orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![
                OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel,
                OrderType::PassingDoor,
                OrderType::TransitionCrouchingUp,
                OrderType::WalkingUpright,
                OrderType::WalkingUpright,
            ],
            "zero-target door steps precede the copied walk, while the authored matching walk follows it"
        );
        assert!(element.orders[3].transition_distance_continuation);
        assert!(!element.orders[4].transition_distance_continuation);
    }

    #[test]
    fn materialized_door_action_points_precede_copied_walk() {
        let mut pass = ActiveDoorPass {
            door_index: crate::gate::DoorIndex::new(43).expect("valid door index"),
            direct: true,
            position_direct: true,
            steps: [
                crate::element::DoorPassStep::Select { speed: 2.0 },
                crate::element::DoorPassStep::PassingDoor,
                crate::element::DoorPassStep::Walk {
                    destination: MapPoint::new(20.0, 30.0),
                    action: OrderType::RunningUpright,
                    reverse: false,
                    compute_direction: true,
                    tolerance: 0.0,
                },
                crate::element::DoorPassStep::PassingDoor,
            ]
            .into(),
            triggers_fired: 0,
            current_action: OrderType::TransitionWalkingUprightRunningUpright,
            current_reverse: false,
            saved_action_state: None,
        };
        let mut next_order_id = 1;

        let orders = materialize_door_action_point_prefix(&mut pass, &mut next_order_id);

        assert_eq!(
            orders
                .iter()
                .map(|order| order.order_type)
                .collect::<Vec<_>>(),
            vec![OrderType::Select, OrderType::PassingDoor]
        );
        assert!(
            orders
                .iter()
                .all(|order| order.completion == crate::order::OrderCompletion::AdvanceElement)
        );
        assert!(matches!(
            pass.steps.front(),
            Some(crate::element::DoorPassStep::Walk {
                destination,
                action: OrderType::RunningUpright,
                ..
            }) if *destination == MapPoint::new(20.0, 30.0)
        ));
        assert_eq!(pass.steps.len(), 2, "the suffix remains lazily translated");
    }
}

pub(super) fn door_click_polygon_at(doors: &[crate::gate::Door], click: MapPoint) -> Option<u32> {
    doors
        .iter()
        .enumerate()
        .find(|(_, door)| door.is_door() && door.click_polygon_contains(click.x, click.y))
        .map(|(idx, _)| idx as u32)
}

pub(super) fn movement_execute_state_effect(
    order: OrderType,
    motion: MotionState,
) -> Option<(crate::element::Posture, crate::element::ActionState)> {
    use crate::element::{ActionState as AS, Posture as P};
    use crate::order::OrderType as OT;
    use crate::sprite::MotionState as MS;

    match (order, motion) {
        (
            OT::TransitionWalkingUprightWaitingUpright
            | OT::TransitionRunningUprightWaitingUpright
            | OT::TransitionWaitingUprightWalkingUpright
            | OT::TransitionSpecialWaitingUpright,
            MS::Done | MS::Terminated,
        ) => Some((P::Upright, AS::Waiting)),
        (OT::TransitionWaitingUprightSpecial, MS::Done | MS::Terminated) => {
            Some((P::Leisure, AS::Waiting))
        }
        (OT::TransitionWaitingUprightBoredWaitingUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Waiting))
        }
        (OT::TransitionWaitingUprightWaitingUprightBored, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Bored))
        }
        (
            OT::TransitionCrouchingUp
            | OT::TransitionSittingWaitingUpright
            | OT::TransitionLeaningOutWaitingAlerted
            | OT::LoweringShield,
            MS::Done | MS::Terminated,
        ) => Some((P::Upright, AS::Waiting)),
        (OT::TransitionCrouchingDown, MS::Done | MS::Terminated) => {
            Some((P::Crouched, AS::Waiting))
        }
        (OT::TransitionWalkingCrouchedWaitingCrouched, MS::Done | MS::Terminated) => {
            Some((P::Crouched, AS::Waiting))
        }
        (
            OT::TransitionWaitingCrouchedWalkingCrouched
            | OT::TransitionWalkingUprightWalkingCrouched
            | OT::TransitionRunningUprightWalkingCrouched,
            MS::Done | MS::Terminated,
        ) => Some((P::Crouched, AS::Moving)),
        (OT::TransitionWalkingCrouchedWalkingUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Moving))
        }
        (OT::TransitionWalkingCrouchedRunningUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::MovingFast))
        }
        (
            OT::TransitionWaitingUprightRunningUpright | OT::TransitionWalkingUprightRunningUpright,
            MS::Done | MS::Terminated,
        ) => Some((P::Upright, AS::MovingFast)),
        (OT::TransitionRunningUprightWalkingUpright, MS::Done | MS::Terminated) => {
            Some((P::Upright, AS::Moving))
        }
        (
            OT::WalkingUpright | OT::WalkingAlerted | OT::WalkingStairs | OT::RunningStairs,
            MS::Start,
        ) => Some((P::Upright, AS::Moving)),
        // The crouched walk starts the actor moving without standing it
        // up; only the PC executes this animation.
        (OT::WalkingCrouched, MS::Start) => Some((P::Crouched, AS::Moving)),
        // Unlike the neighboring walk/stairs arms, Original stamps this
        // state unconditionally after PerformSeek/PerformMotion. A fresh
        // short run can therefore return Terminated without ever exposing
        // Start and must still leave the actor MovingFast.
        (OT::RunningUpright, _) => Some((P::Upright, AS::MovingFast)),
        (OT::WalkingWithSword, MS::Start) => Some((P::Upright, AS::MovingSword)),
        (OT::RunningWithSword, MS::Start) => Some((P::Upright, AS::MovingFastSword)),
        // The PC WalkingWithShield Execute arm stamps MovingShield after
        // every PerformSeek/PerformMotion result, then replaces it with
        // HoldingShield when that result is TERMINATED.
        (OT::WalkingWithShield, MS::Terminated) => Some((P::Upright, AS::HoldingShield)),
        (OT::WalkingWithShield, _) => Some((P::Upright, AS::MovingShield)),
        (OT::WalkingWithCorpse, MS::Start) => Some((P::CarryingCorpse, AS::Moving)),
        (OT::WalkingWithCorpse, MS::Terminated) => Some((P::CarryingCorpse, AS::Waiting)),
        (OT::ClimbingWallUp | OT::ClimbingWallDown, MS::Start) => Some((P::OnWall, AS::Moving)),
        (
            OT::TransitionWaitingUprightClimbingLadderUp
            | OT::TransitionWaitingUprightClimbingLadderUpAlerted,
            MS::Done | MS::Terminated,
        ) => Some((P::OnLadder, AS::Moving)),
        _ => None,
    }
}

fn take_transition_distance_first_execute(transition_distance_continuation: &mut bool) -> bool {
    std::mem::take(transition_distance_continuation)
}

fn take_deferred_movement_state_start(deferred_movement_state_start: &mut bool) -> bool {
    std::mem::take(deferred_movement_state_start)
}

fn should_defer_pc_movement_state_start(is_pc: bool, entity_target_seek: bool) -> bool {
    is_pc && !entity_target_seek
}

/// The shipped game clears an anti-vibration deviation latch when an in-place
/// movement startup transition starts. Both PCs and NPCs distinguish the two
/// upright handoffs: a waiting-to-walking/running startup retires the
/// preceding movement's latch, while a walking-to-waiting exit preserves it
/// for the following `Turn`.
#[inline]
fn should_clear_deviated_for_aligned_transition_start(
    _is_pc: bool,
    execute_order_initialising: bool,
    is_transition_anim: bool,
    order_action: OrderType,
    deviated: bool,
    position: MapPoint,
    goal: MapPoint,
) -> bool {
    execute_order_initialising
        && is_transition_anim
        && matches!(
            order_action,
            OrderType::TransitionWaitingUprightWalkingUpright
                | OrderType::TransitionWaitingUprightRunningUpright
        )
        && deviated
        && position == goal
}

fn actor_line_crossing_eligible(
    posture: crate::element::Posture,
    human_is_carried: bool,
    inside_map: bool,
) -> bool {
    posture != crate::element::Posture::Flying && !human_is_carried && inside_map
}

#[inline]
fn stationary_motion_waits(speed: f32, tolerance_arrival: bool, distance: f32) -> bool {
    speed <= 0.0 && !tolerance_arrival && (distance > f32::EPSILON || !distance.is_finite())
}

#[inline]
fn motion_recomputes_exact_position(
    is_transition: bool,
    has_map_target: bool,
    speed: f32,
    distance: f32,
) -> bool {
    is_transition && has_map_target && speed > 0.0 && distance <= f32::EPSILON
}

/// Mirror the forecast update at the end of Original's nonzero
/// `RHSprite::PerformMotion` displacement. Transition-distance orders use a
/// separate commit path from ordinary walking in Rust, but Original runs both
/// through the same forecast update before its arrival check.
fn refresh_motion_forecast(
    sprite: &mut crate::sprite::Sprite,
    speed: f32,
    split_motion_speeds: Option<(f32, f32)>,
) {
    if sprite.position_iface.is_blocked() {
        return;
    }

    // Fast movement executes PerformMotion twice. Each nonzero call updates
    // the forecast, so the second distance wins when it moved; otherwise the
    // first call's forecast remains live.
    let forecast_distance = match split_motion_speeds {
        Some((_, second)) if second != 0.0 => second,
        Some((first, _)) => first,
        None => speed,
    };
    if forecast_distance == 0.0 {
        return;
    }

    let wait = sprite.wait_time(sprite.current_row, sprite.current_frame);
    sprite
        .position_iface
        .update_forecasted_movement(forecast_distance, wait + 1);
}

/// Original only performs the exact zero-tolerance goal snap from the
/// post-movement arrival branches. An order which starts at its goal is
/// consumed without rewriting the actor's coordinates.
#[inline]
fn should_snap_arrival(
    arrived_after_committed_step: bool,
    tolerance_arrival: bool,
    order_tolerance: f32,
    deviated: bool,
) -> bool {
    arrived_after_committed_step && !tolerance_arrival && order_tolerance == 0.0 && !deviated
}

/// Whether `PerformSeek` exposes a wrapped `PerformMotion` termination to
/// `RHElementActorHuman::Execute`. A successful terminal entity seek without
/// a post-seek sequence deliberately converts it back to IN_PROGRESS so it
/// can wait/refresh in place.
fn perform_seek_exposes_motion_termination(
    starts_post_seek: bool,
    final_entity_seek_arrival: Option<bool>,
) -> bool {
    starts_post_seek || final_entity_seek_arrival != Some(true)
}

fn both_sword_ranges_contain_distance(
    distance: f32,
    my_maximal: u16,
    my_uber: u16,
    opponent_maximal: u16,
    opponent_uber: u16,
) -> bool {
    let between =
        |maximal: u16, uber: u16| f32::from(maximal) < distance && distance <= f32::from(uber);
    between(my_maximal, my_uber) && between(opponent_maximal, opponent_uber)
}

/// Does the step this Execute is about to commit satisfy `IsGoalReached`?
///
/// `PerformMotion` moves first and only then asks the position interface
/// whether the goal is reached, so a call that would otherwise return `START`
/// can return `TERMINATED` instead. Rust stages the physical step until after
/// the sprite call, so the answer has to be projected on a throwaway copy of
/// the position interface, anti-collision and all. Comparing the straight-line
/// distance against the step length is not a substitute: the predicate is a
/// tolerance-compared dot product against the movement increment, and a step
/// deviated around another actor both leaves that line and rebuilds the
/// increment it is measured against.
#[allow(clippy::too_many_arguments)]
fn projected_step_reaches_goal(
    position_iface: &crate::position_interface::PositionInterface,
    mover_snapshot: Option<&super::anti_collision::ActorSnapshot>,
    neighbours: &[Option<super::anti_collision::ActorSnapshot>],
    static_repulsive_points: &[crate::ai::RepulsivePoint],
    mobile: &LiveMobileGeometry,
    grid: &crate::fast_find_grid::FastFindGrid,
    goal: MapPoint,
    target: Option<crate::position_interface::TargetInfo>,
    speed: f32,
) -> bool {
    if speed == 0.0 {
        return false;
    }
    let mut projected = position_iface.clone();
    let increment = projected.get_increment_map();
    let anti_on = projected.is_anti_collision_on();
    let (dx_step, dy_step, recovered_from_deviation, rebuild_after_deviation) =
        if anti_on && let Some(mover) = mover_snapshot.filter(|snapshot| snapshot.active) {
            let move_box = *projected.get_move_box();
            let half_diagonal = projected.get_half_diagonal();
            let was_deviated = projected.is_deviated();
            let mut state = super::anti_collision::AntiCollisionState {
                pi: &mut projected,
                move_box,
                half_diagonal,
                goal_map: goal,
            };
            let (dx_step, dy_step) = super::anti_collision::apply_anti_collision_step(
                mover,
                neighbours,
                static_repulsive_points,
                mobile
                    .mobile_points_by_layer
                    .get(&mover.layer)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                mobile
                    .mobile_lines_by_layer
                    .get(&mover.layer)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                mobile
                    .mobile_polygons_by_layer
                    .get(&mover.layer)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                Some(grid),
                Some(&mut state),
                increment.x,
                increment.y,
                speed,
                anti_on,
            );
            (
                dx_step,
                dy_step,
                was_deviated && !state.pi.is_deviated(),
                state.pi.is_deviated() && state.pi.blocked_count == 0,
            )
        } else {
            (increment.x * speed, increment.y * speed, false, false)
        };
    let mut projected_position = projected.map_position();
    projected_position.x += dx_step;
    projected_position.y += dy_step;
    projected.set_map_position(projected_position);
    // A committed deviation invalidates the cached increment and rebuilds it
    // from the new position toward the same goal, and the arrival predicate
    // that follows reads the rebuilt vector. Skipping the rebuild leaves the
    // dot product measuring against the pre-deviation heading, which is how a
    // sidestepped walker looked as if it had already arrived.
    if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
        projected.reset_increment_computed();
        projected.compute_increment_all(false);
    } else if recovered_from_deviation {
        projected.reset_increment_computed();
        projected.compute_increment_all(true);
    }
    projected.is_goal_reached(grid, target)
}

/// Publish an interleaved motion call's authoritative commit to the serial
/// anti-collision snapshot. Goal snapping can make this differ from the raw
/// requested displacement; offset repulsive geometry must follow the stored
/// position, not the discarded overshoot.
fn sync_snapshot_after_committed_step(
    snapshot: &mut super::anti_collision::ActorSnapshot,
    pre_position: MapPoint,
    post_position: MapPoint,
) {
    super::anti_collision::sync_snapshot_after_move(
        snapshot,
        post_position,
        post_position - pre_position,
    );
}

/// Motion state observed by the Original Execute arm after `PerformSeek`.
///
/// Entity-target `PerformSeek` consumes non-terminal sprite results and returns
/// `IN_PROGRESS`; point seeks return the raw result. This matters because the
/// caller's Execute switch must not observe either a raw `START` or `DONE`
/// while the seek wrapper remains active. Running upright is deliberately
/// excluded: its Original Execute arm sets `MOVING_FAST` unconditionally after
/// `PerformSeek`, irrespective of the returned motion state.
///
/// The ordinary arrival branch outranks the sprite's own result: once the
/// committed step satisfies the goal predicate, `PerformMotion` returns
/// `TERMINATED` whatever it was about to report, so a walk that merely
/// continued (`IN_PROGRESS`) or finished its action frame (`DONE`) still
/// reaches the Execute arm as a termination.
fn movement_execute_visible_motion(
    order: OrderType,
    motion: MotionState,
    reaches_goal_this_step: bool,
    entity_target_seek: bool,
) -> MotionState {
    if reaches_goal_this_step {
        return MotionState::Terminated;
    }
    if entity_target_seek
        && !matches!(motion, MotionState::Terminated)
        && !matches!(order, OrderType::RunningUpright)
    {
        return MotionState::InProgress;
    }
    motion
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct MovementOwnerMotion {
    pub initial: Option<MotionState>,
    pub post_completion_override: Option<MotionState>,
}

fn committed_arrival_post_completion_override(
    raw_sprite_motion: MotionState,
    visible_execute_motion: MotionState,
    reaches_goal_this_step: bool,
) -> Option<MotionState> {
    (reaches_goal_this_step && raw_sprite_motion != visible_execute_motion)
        .then_some(visible_execute_motion)
}

fn cancel_aborted_order_pop(
    order_pops: &mut Vec<(crate::sequence::SequenceId, usize)>,
    seq_id: crate::sequence::SequenceId,
    elem_idx: usize,
) {
    order_pops.retain(|&(queued_seq, queued_idx)| queued_seq != seq_id || queued_idx != elem_idx);
}

/// Original `mulWaitTime--` uses an unsigned 32-bit counter. A stationary
/// entity seek deliberately wraps zero to `UINT_MAX`; the signed refresh gate
/// then continues to regard the wrapped values as elapsed.
#[inline]
fn age_seek_refresh_wait(wait: u32) -> u32 {
    wait.wrapping_sub(1)
}

/// Number of times the Original actor `Execute` arm invokes `PerformSeek` for
/// one execution of this movement order when `RHMOVE_SEEK` is set.
///
/// The flag alone is not enough: authored wall and ladder orders retain it
/// while their Execute arms call `PerformMotion` directly. Conversely,
/// `RHNONANIMATION_RUNNING_STAIRS` literally calls `PerformSeek` twice.
#[inline]
pub(super) fn perform_seek_calls_per_execute(order: OrderType) -> u32 {
    match order {
        OrderType::TransitionWalkingUprightWaitingUpright
        | OrderType::TransitionRunningUprightWaitingUpright
        | OrderType::TransitionWaitingUprightWalkingUpright
        | OrderType::TransitionWaitingUprightRunningUpright
        | OrderType::TransitionWalkingUprightRunningUpright
        | OrderType::TransitionRunningUprightWalkingUpright
        | OrderType::TransitionWaitingCrouchedWalkingCrouched
        | OrderType::TransitionWalkingCrouchedWaitingCrouched
        | OrderType::TransitionWalkingUprightWalkingCrouched
        | OrderType::TransitionWalkingCrouchedWalkingUpright
        | OrderType::TransitionRunningUprightWalkingCrouched
        | OrderType::TransitionWalkingCrouchedRunningUpright
        | OrderType::WalkingUpright
        | OrderType::RunningUpright
        | OrderType::WalkingCrouched
        | OrderType::WalkingAlerted
        | OrderType::WalkingStairs
        | OrderType::WalkingStairsAlerted
        | OrderType::WalkingCarryingOnShoulders
        | OrderType::WalkingWithCorpse
        | OrderType::WalkingWithSword
        | OrderType::RunningWithSword
        | OrderType::WalkingWithShield => 1,
        OrderType::RunningStairs => 2,
        _ => 0,
    }
}

fn original_final_path_metadata(
    raw_waypoint_count: usize,
    tolerance: f32,
    antagonist: Option<EntityId>,
) -> (f32, Option<EntityId>) {
    if raw_waypoint_count > 1 {
        (tolerance, antagonist)
    } else {
        (0.0, None)
    }
}

/// Prepare the raw pathfinder points for movement-order post-processing.
///
/// `RHEngine::ProcessPathRequests` starts its order loop at index one whenever
/// `bUseFirstPoint` is false (`RHengine.cpp:8410-8423`).  Do not re-check that
/// the first point equals the request source here: legacy floating-point
/// equality is not the gate, and a source poisoned with NaNs must still be
/// skipped rather than becoming a live movement order.
///
/// Returns the raw count because final-order tolerance and antagonist
/// metadata depend on the pre-skip path exactly as they do in Original.
fn prepare_path_waypoints_for_postprocess(
    waypoints: &mut Vec<MapPoint>,
    use_first_point: bool,
) -> usize {
    let raw_waypoint_count = waypoints.len();
    if !use_first_point && waypoints.len() > 1 {
        waypoints.remove(0);
    }
    raw_waypoint_count
}

fn is_in_place_movement_transition(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::TransitionWaitingUprightSpecial
            | OrderType::TransitionSpecialWaitingUpright
            | OrderType::TransitionWaitingUprightBoredWaitingUpright
            | OrderType::TransitionWaitingUprightWaitingUprightBored
            | OrderType::TransitionCrouchingUp
            | OrderType::TransitionCrouchingDown
            | OrderType::TransitionSittingWaitingUpright
            | OrderType::TransitionLeaningOutWaitingAlerted
            | OrderType::TransitionClimbingWallDownWaitingUpright
            | OrderType::StandingUp
            | OrderType::StandingUpSword
            | OrderType::StandingUpBow
            | OrderType::LoweringShield
    )
}

/// Whether an outgoing movement is still in one of the generated locomotion
/// transitions that owns the previously published waypoint.
///
/// A replacement instructed while the actor is in a concrete walk/run order
/// does not inherit that waypoint: Original clears it at the replacement
/// arbitration boundary and leaves it zero until the new movement executes.
/// The transition case is different because the transition itself continues
/// to own the live movement forecast across the hand-off.
fn movement_transition_retains_goal(order: OrderType) -> bool {
    matches!(
        order,
        OrderType::TransitionWalkingUprightWaitingUpright
            | OrderType::TransitionRunningUprightWaitingUpright
            | OrderType::TransitionWaitingUprightWalkingUpright
            | OrderType::TransitionWaitingUprightRunningUpright
            | OrderType::TransitionWalkingUprightRunningUpright
            | OrderType::TransitionRunningUprightWalkingUpright
            | OrderType::TransitionWaitingCrouchedWalkingCrouched
            | OrderType::TransitionWalkingCrouchedWaitingCrouched
            | OrderType::TransitionWalkingUprightWalkingCrouched
            | OrderType::TransitionWalkingCrouchedWalkingUpright
            | OrderType::TransitionRunningUprightWalkingCrouched
            | OrderType::TransitionWalkingCrouchedRunningUpright
    )
}

#[cfg(test)]
mod movement_goal_replacement_tests {
    use super::*;

    #[test]
    fn only_live_locomotion_transitions_retain_the_outgoing_goal() {
        assert!(movement_transition_retains_goal(
            OrderType::TransitionWaitingUprightRunningUpright
        ));
        assert!(movement_transition_retains_goal(
            OrderType::TransitionRunningUprightWaitingUpright
        ));
        assert!(!movement_transition_retains_goal(OrderType::RunningUpright));
        assert!(!movement_transition_retains_goal(OrderType::WalkingUpright));
    }
}

/// Result of [`EngineInner::advance_door_pass`].
///
/// Outcomes from draining the order list after a walk step terminates.
#[derive(Debug, Clone)]
pub(super) enum DoorPassAdvance {
    /// No active door pass existed when the state machine was asked to
    /// advance. This is a caller bug or a stale animation callback; it
    /// must not be treated as a completed pass.
    NoActive,
    /// A new `Walk` step is ready — the caller must push a walking
    /// order onto the actor's current sequence element to install the
    /// destination.  Movement tick resumes once the order is queued.
    Continue {
        destination: MapPoint,
        action: OrderType,
        reverse: bool,
        compute_direction: bool,
        /// Walk-step tolerance copied from the source
        /// [`DoorPassStep::Walk`].  Populated for the ladder/wall
        /// translators and `0.0` for stairs/building/default.
        tolerance: f32,
    },
    /// A `Transition` step was popped — the caller must push the
    /// included [`crate::order::Order`] onto the actor's current
    /// sequence element and *not* clear `active_door_pass` or signal
    /// arrival.  Door-pass advancement resumes when the transition
    /// animation completes (via [`crate::order::OrderCompletion::ResumeDoorPass`]).
    Paused {
        transition_order: crate::order::Order,
    },
    /// A non-animation `PassingDoor` action point is ready. It must be
    /// installed as the next real actor order so it consumes its own owner
    /// slot, just like the Original order chain.
    ActionPoint { order: crate::order::Order },
    /// No more steps remain; the door pass is complete and the caller
    /// should tear down path / active-movement state.
    Done {
        completed: Option<(crate::gate::DoorIndex, bool)>,
    },
}

// ─── Group-move formation helper ─────────────────────────────────────

/// Compute per-character destination points for a "mercenary"-style group
/// move around `click_point`.
///
/// The group's centroid is calculated, then each character's destination
/// is its current position translated so that the centroid lands on the
/// click point — preserving the relative formation of the group.
///
/// Returns a vector with the same length as `pc_positions`, each entry
/// being the destination for the PC at the matching index.  Returns an
/// empty vector if `pc_positions` is empty.
pub(crate) fn mercenary_formation_destinations(
    pc_positions: &[MapPoint],
    click_point: MapPoint,
) -> Vec<MapPoint> {
    if pc_positions.is_empty() {
        return Vec::new();
    }

    let n = pc_positions.len() as f32;
    let cx = pc_positions.iter().map(|p| p.x).sum::<f32>() / n;
    let cy = pc_positions.iter().map(|p| p.y).sum::<f32>() / n;

    pc_positions
        .iter()
        .map(|p| MapPoint::new(p.x - cx + click_point.x, p.y - cy + click_point.y))
        .collect()
}

/// Shape of the goal passed to [`EngineInner::build_gate_movement_sequence`].
///
/// Unifies the three goal flavours (point, door, line) into a single
/// builder; the function switches on this enum to pick the right
/// trailing-step shape.
#[derive(Debug, Clone, Copy)]
pub(crate) enum GoalShape {
    /// Point-goal. The actor walks to this map point after the last gate,
    /// retaining the caller's arrival tolerance (notably for AI `GoNear`).
    Point { point: MapPoint, tolerance: f32 },
    /// Entity-target seek goal.  The trailing MOVE keeps the target
    /// element, SEEK flag, and tolerance so arrival uses the same
    /// live target-distance predicate as a plain `Command::Seek`.
    Seek {
        point: MapPoint,
        target: EntityId,
        tolerance: f32,
    },
    /// Direct entity-target route built by `RHElementTarget::MouseClicked`.
    /// Unlike `Seek`, the target pointer is retained on each ordinary MOVE
    /// but `RHMOVE_SEEK` is not set and the actor's seek-refresh state is not
    /// touched.
    Target {
        point: MapPoint,
        target: EntityId,
        tolerance: f32,
    },
    /// Door-goal.  The gate path's final element is the goal door
    /// itself.  `far_side_point` describes the point the actor lands
    /// at after passing through.  When the far-side sector is a
    /// building, a `CHANGE_POSITION` teleport is emitted.
    Door {
        /// Index of the goal door in `self.script_domains.interactables.doors`.
        door_index: crate::gate::DoorIndex,
        /// The approach point (near side of the goal door).
        far_side_point: MapPoint,
        /// Far-side layer.
        far_side_layer: u16,
        /// True iff the goal-sector (far side) is a building.  When
        /// true the trailing step is a `CHANGE_POSITION` teleport after
        /// a random wait, not a plain walk to the far-side point.
        far_side_is_building: bool,
    },
    /// Line-goal.  The final MOVE uses the line's midpoint as its
    /// waypoint and carries `MoveFlags::LINE` + the line id so the
    /// actor's arrival check snaps to line tolerance.
    Line {
        /// Index of the goal line in `fast_grid.level.jump_lines`.
        line_index: crate::jump_line::JumpLineIndex,
        /// Midpoint of the line.  Used as the path target point during
        /// gate routing.
        midpoint: MapPoint,
        /// Arrival tolerance passed to the final line move.
        tolerance: f32,
    },
}

#[inline]
pub(crate) fn building_exit_wait_frames(sim: &crate::sim_rng::SimulationContext) -> u32 {
    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::RuntimeBuildingExitWait, 0..16)
        + crate::sim_rng::u32(sim, crate::sim_rng::RngSite::RuntimeBuildingExitWait, 0..16)
}

fn route_sector_by_exact_handle(
    engine: &EngineInner,
    sector: crate::position_interface::SectorHandle,
) -> Option<&crate::fast_find_grid::GridSector> {
    grid_sector_for_position_handle(&engine.world.fast_grid.level, sector)
}

fn ai_move_goal_door(
    engine: &EngineInner,
    goal_sector: crate::position_interface::SectorHandle,
    goal_sector_index: Option<crate::fast_find_grid::SectorIndex>,
) -> Option<crate::gate::DoorIndex> {
    let exact_goal_sector =
        goal_sector_index.map_or(goal_sector, |index| goal_sector.with_arena_index(index));
    route_sector_by_exact_handle(engine, exact_goal_sector)
        .filter(|sector| sector.sector_type.is_door())
        .and_then(|sector| sector.door_index)
        .map(crate::gate::DoorIndex)
}

/// Timeout queue entry for a Move/Seek element whose pathfind failed.
/// When the pathfinder returns no path, the request is stamped with the
/// current universal frame counter and pushed onto this list.  After
/// 100 frames the element transitions to `Impossible` (and, for PCs,
/// the "unable to do something" speech line fires).
///
/// This is **not** a retry queue: the path is not re-dispatched during the
/// 100-frame window. The element sits waiting (no orders, so the actor's idle
/// animation drives) until it is cancelled (halt / postpone) or the timeout
/// elapses.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct FailedPathRequest {
    pub(crate) owner: EntityId,
    pub(crate) seq_id: crate::sequence::SequenceId,
    pub(crate) elem_idx: usize,
    /// Universal frame counter at failure time.  Ages out at
    /// `first_fail_frame + 100`.
    pub(crate) first_fail_frame: u32,
    /// Exact `RHpathRequest` payload retained by the Original timeout list.
    pub(crate) request: PendingPathRequest,
}

impl FailedPathRequest {
    pub(crate) fn from_pending(request: PendingPathRequest, first_fail_frame: u32) -> Self {
        Self {
            owner: request.owner,
            seq_id: request.seq_id,
            elem_idx: request.elem_idx,
            first_fail_frame,
            request,
        }
    }
}

/// Snapshot of one legacy `RHpathRequest` waiting for A*.
///
/// Direct / straight moves never enter this queue. Requests that do need A*
/// snapshot their dispatch inputs here, then [`PathScheduleContext`] resolves at
/// most one request at the original `RHEngine::ProcessPathRequests` point per
/// frame.
#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct PendingPathRequest {
    /// This request was decoded from an Original v48 pending-path FIFO.
    ///
    /// Its movement element owns the exact serialized pre-path order queue,
    /// so completion must reuse the saved last waiting order in place.
    #[serde(default)]
    pub(crate) restored_from_v48: bool,
    pub(crate) owner: EntityId,
    pub(crate) seq_id: crate::sequence::SequenceId,
    pub(crate) elem_idx: usize,
    pub(crate) source: MapPoint,
    pub(crate) dest: MapPoint,
    pub(crate) layer: u16,
    /// Original `RHpathRequest::uwArea`; despite the name this is the actor's
    /// sector number and is converted to a graph-area index during A*.
    pub(crate) sector: u16,
    /// Exact serialized `RHpathRequest::uwSector`. Original request creation
    /// does not initialize this member and pathfinding never reads it, but v48
    /// saves nevertheless contain it.
    pub(crate) legacy_sector: u16,
    pub(crate) half_diagonal_idx: u16,
    pub(crate) use_first_point: bool,
    pub(crate) move_action: OrderType,
    pub(crate) speed: crate::pathfinder::PathFinderSpeed,
    pub(crate) reverse: bool,
    pub(crate) tolerance: f32,
    pub(crate) antagonist: Option<EntityId>,
    pub(crate) is_pass_door: bool,
    pub(crate) elem_flags: crate::sequence::MoveFlags,
    pub(crate) sword_movement_context: bool,
    pub(crate) is_fast: bool,
}

#[cfg(test)]
impl PendingPathRequest {
    pub(crate) fn test_request(
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> Self {
        Self {
            restored_from_v48: false,
            owner,
            seq_id,
            elem_idx,
            source: MapPoint::new(10.0, 10.0),
            dest: MapPoint::new(20.0, 20.0),
            layer: 0,
            sector: 0,
            legacy_sector: 0,
            half_diagonal_idx: 0,
            use_first_point: false,
            move_action: OrderType::WalkingUpright,
            speed: crate::pathfinder::PathFinderSpeed::Medium,
            reverse: false,
            tolerance: 0.0,
            antagonist: None,
            is_pass_door: false,
            elem_flags: crate::sequence::MoveFlags::empty(),
            sword_movement_context: false,
            is_fast: false,
        }
    }
}

fn parity_path_request_state(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    request: &PendingPathRequest,
) -> crate::pathfinder::ParityPathRequest {
    let half_diagonal = fast_grid
        .try_move_box_half_diagonal(usize::from(request.half_diagonal_idx))
        .unwrap_or_else(|| {
            panic!(
                "path request for {:?} references missing half-diagonal index {}",
                request.owner, request.half_diagonal_idx
            )
        });
    crate::pathfinder::ParityPathRequest {
        actor: request.owner,
        antagonist: request.antagonist,
        layer: request.layer,
        area: request.sector,
        source: request.source,
        goal: request.dest,
        half_diagonal_index: request.half_diagonal_idx,
        half_diagonal,
        animation: request.move_action as u32,
        reverse: request.reverse,
        speed: request.speed as u8,
        tolerance: request.tolerance,
        use_first_point: request.use_first_point,
    }
}

#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
struct ProcessedPathRequest {
    request: PendingPathRequest,
    waypoints: Option<Vec<MapPoint>>,
}

#[derive(Debug)]
pub(crate) struct ParityPendingPathRequest {
    pub(crate) request: crate::pathfinder::ParityPathRequest,
    pub(crate) sequence_id: crate::sequence::SequenceId,
    pub(crate) element_index: usize,
    pub(crate) in_flight: bool,
    pub(crate) waypoints: Option<Vec<MapPoint>>,
}

/// Legacy path-request ordering plus the pathfinder's in-flight result.
///
/// `RHPathFinder::AddPathRequest` leaves queues of length zero or one alone.
/// From length two onward it stably sorts by speed, except that the in-flight
/// entry cannot be displaced. The original WAITING branch starts work but
/// returns no result; a later READY call delivers it and starts the next
/// request. `in_flight` preserves that one-call latency.
#[derive(
    Debug,
    Clone,
    Default,
    serde::Serialize,
    serde::Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub(crate) struct PendingPathRequestQueue {
    waiting: Vec<PendingPathRequest>,
    in_flight: Option<ProcessedPathRequest>,
    /// Original `RHPathFinder::mbIgnoreNextPath`. Cancelling the logical list
    /// head does not remove it; its eventual result remains observable but is
    /// delivered with `valid=false` and consumes the call's one result slot.
    #[serde(default)]
    ignore_next_path: bool,
}

impl PendingPathRequestQueue {
    /// Restore the exact post-save FIFO. The Original writer excludes an
    /// ignored/in-flight head and writes the remaining list in order, so every
    /// deserialized request is waiting and no completion result is present.
    pub(crate) fn restore_v48_waiting(waiting: Vec<PendingPathRequest>) -> Self {
        Self {
            waiting,
            in_flight: None,
            ignore_next_path: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn v48_waiting(&self) -> &[PendingPathRequest] {
        &self.waiting
    }

    pub(crate) fn has_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    pub(crate) fn parity_state(
        &self,
        fast_grid: &crate::fast_find_grid::FastFindGrid,
    ) -> (bool, Vec<ParityPendingPathRequest>) {
        let mut requests =
            Vec::with_capacity(self.waiting.len() + usize::from(self.in_flight.is_some()));
        if let Some(processed) = &self.in_flight {
            requests.push(ParityPendingPathRequest {
                request: parity_path_request_state(fast_grid, &processed.request),
                sequence_id: processed.request.seq_id,
                element_index: processed.request.elem_idx,
                in_flight: true,
                waypoints: Some(processed.waypoints.clone().unwrap_or_default()),
            });
        }
        requests.extend(self.waiting.iter().map(|request| ParityPendingPathRequest {
            request: parity_path_request_state(fast_grid, request),
            sequence_id: request.seq_id,
            element_index: request.elem_idx,
            in_flight: false,
            waypoints: None,
        }));
        (self.ignore_next_path, requests)
    }

    fn enqueue(&mut self, request: PendingPathRequest) {
        let total = self.waiting.len() + usize::from(self.in_flight.is_some());
        if total < 2 {
            self.waiting.push(request);
            return;
        }

        // With no in-flight request, waiting[0] is the original list's
        // special first entry and is compared only after index 1. Once a
        // request is in flight, every waiting entry is priority-sortable.
        let first_sortable = usize::from(self.in_flight.is_none());
        let speed = request.speed as u8;
        if let Some(index) = (first_sortable..self.waiting.len())
            .rev()
            .find(|&index| self.waiting[index].speed as u8 <= speed)
        {
            self.waiting.insert(index + 1, request);
        } else {
            self.waiting.insert(0, request);
        }
    }

    fn take_completed(&mut self) -> Option<(ProcessedPathRequest, bool)> {
        let processed = self.in_flight.take()?;
        let valid = !std::mem::take(&mut self.ignore_next_path);
        Some((processed, valid))
    }

    fn pop_to_start(&mut self) -> Option<PendingPathRequest> {
        (!self.waiting.is_empty()).then(|| self.waiting.remove(0))
    }

    fn set_in_flight(&mut self, request: PendingPathRequest, waypoints: Option<Vec<MapPoint>>) {
        debug_assert!(self.in_flight.is_none());
        self.in_flight = Some(ProcessedPathRequest { request, waypoints });
    }

    pub(super) fn retain_not_owned_by(&mut self, owner: EntityId) {
        let in_flight_is_owner = self
            .in_flight
            .as_ref()
            .is_some_and(|processed| processed.request.owner == owner);
        let waiting_head_is_owner = self.in_flight.is_none()
            && self
                .waiting
                .first()
                .is_some_and(|request| request.owner == owner);

        // Entity teardown follows the same path cancellation timing as an
        // interrupted movement element: the logical head stays in the queue,
        // is delivered invalid, and consumes this barrier's result slot.
        // Only later requests for the removed owner disappear immediately.
        if in_flight_is_owner || waiting_head_is_owner {
            self.ignore_next_path = true;
        }
        let first_waiting = usize::from(waiting_head_is_owner);
        self.waiting = self
            .waiting
            .drain(..)
            .enumerate()
            .filter_map(|(index, request)| {
                (index < first_waiting || request.owner != owner).then_some(request)
            })
            .collect();
    }

    /// Mirror `RHPathFinder::CancelPathRequest`: cancelling the list head
    /// marks its eventual result stale instead of removing it, while later
    /// requests for the same actor are deleted immediately. The retained head
    /// still occupies one `ProcessPathRequests` result slot.
    pub(super) fn cancel_for_owner(&mut self, owner: EntityId) {
        let head_owner = self
            .in_flight
            .as_ref()
            .map(|processed| processed.request.owner)
            .or_else(|| self.waiting.first().map(|request| request.owner));
        if head_owner == Some(owner) {
            self.ignore_next_path = true;
        }

        // The Original scans from logical list index 1 and deletes only the
        // first later request for this actor. With an in-flight head every
        // waiting entry starts at logical index 1; otherwise waiting[0] is the
        // retained head.
        let first_waiting = usize::from(self.in_flight.is_none());
        if let Some(relative) = self
            .waiting
            .get(first_waiting..)
            .and_then(|waiting| waiting.iter().position(|request| request.owner == owner))
        {
            self.waiting.remove(first_waiting + relative);
        }
    }

    fn first_for_owner_mut(&mut self, owner: EntityId) -> Option<&mut PendingPathRequest> {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|processed| processed.request.owner == owner)
        {
            return self
                .in_flight
                .as_mut()
                .map(|processed| &mut processed.request);
        }
        self.waiting
            .iter_mut()
            .find(|request| request.owner == owner)
    }

    /// Mirror `RHPathFinder::MakeFast` on the first request for this actor.
    pub(super) fn make_fast(&mut self, owner: EntityId, pathfinder_index: u16) {
        let Some(request) = self.first_for_owner_mut(owner) else {
            return;
        };
        request.move_action = match request.move_action {
            OrderType::RunningWithSword
            | OrderType::RunningUpright
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast => request.move_action,
            OrderType::WalkingUpright
            | OrderType::WalkingCrouched
            | OrderType::WalkingWithShield => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::RunningUpright
            }
            OrderType::WalkingWithSword => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::RunningWithSword
            }
            OrderType::ClimbingLadderUp => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingLadderUpFast
            }
            OrderType::ClimbingLadderDown => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingLadderDownFast
            }
            OrderType::ClimbingWallUp => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingWallUpFast
            }
            OrderType::ClimbingWallDown => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingWallDownFast
            }
            action => panic!(
                "RHPathFinder::MakeFast received unsupported pending action {action:?} for {owner:?}"
            ),
        };
    }

    /// Mirror `RHPathFinder::MakeSlow` on the first request for this actor.
    pub(super) fn make_slow(&mut self, owner: EntityId, pathfinder_index: u16) {
        let Some(request) = self.first_for_owner_mut(owner) else {
            return;
        };
        request.move_action = match request.move_action {
            OrderType::WalkingUpright
            | OrderType::WalkingCrouched
            | OrderType::ClimbingLadderUp
            | OrderType::ClimbingLadderDown
            | OrderType::ClimbingWallUp
            | OrderType::ClimbingWallDown => request.move_action,
            OrderType::RunningUpright => OrderType::WalkingUpright,
            OrderType::RunningWithSword => OrderType::WalkingWithSword,
            OrderType::ClimbingLadderUpFast => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingLadderUp
            }
            OrderType::ClimbingLadderDownFast => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingLadderDown
            }
            OrderType::ClimbingWallUpFast => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingWallUp
            }
            OrderType::ClimbingWallDownFast => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::ClimbingWallDown
            }
            action => panic!(
                "RHPathFinder::MakeSlow received unsupported pending action {action:?} for {owner:?}"
            ),
        };
    }

    /// Mirror `RHPathFinder::MakeUpright` on the first request for this actor.
    pub(super) fn make_upright(&mut self, owner: EntityId, pathfinder_index: u16) {
        let Some(request) = self.first_for_owner_mut(owner) else {
            return;
        };
        request.move_action = match request.move_action {
            OrderType::WalkingUpright
            | OrderType::RunningUpright
            | OrderType::ClimbingLadderUp
            | OrderType::ClimbingLadderDown
            | OrderType::ClimbingWallUp
            | OrderType::ClimbingWallDown
            | OrderType::ClimbingLadderUpFast
            | OrderType::ClimbingLadderDownFast
            | OrderType::ClimbingWallUpFast
            | OrderType::ClimbingWallDownFast => request.move_action,
            OrderType::WalkingCrouched => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::WalkingUpright
            }
            action => panic!(
                "RHPathFinder::MakeUpright received unsupported pending action {action:?} for {owner:?}"
            ),
        };
    }

    /// Mirror `RHPathFinder::MakeCrouched` on the first request for this actor.
    pub(super) fn make_crouched(&mut self, owner: EntityId, pathfinder_index: u16) {
        let Some(request) = self.first_for_owner_mut(owner) else {
            return;
        };
        request.move_action = match request.move_action {
            OrderType::WalkingUpright | OrderType::RunningUpright => {
                request.half_diagonal_idx = pathfinder_index;
                OrderType::WalkingCrouched
            }
            action => panic!(
                "RHPathFinder::MakeCrouched received unsupported pending action {action:?} for {owner:?}"
            ),
        };
    }

    pub(super) fn clear(&mut self) {
        self.waiting.clear();
        self.in_flight = None;
        self.ignore_next_path = false;
    }
}

/// Disjoint owner borrows for the once-per-frame path scheduling barrier.
///
/// This context deliberately cannot reach scripts, campaign, player state, or
/// feedback. It only advances the pathfinder queue and classifies expired
/// failures; the root tick performs the cross-owner consequences (path order
/// installation and hero speech) immediately after each returned item.
pub(super) struct PathScheduleContext<'a> {
    frame_counter: u32,
    entities: &'a crate::entities::Entities,
    fast_grid: &'a crate::fast_find_grid::FastFindGrid,
    pathfinder: &'a mut crate::pathfinder::PathFinder,
    pending_path_requests: &'a mut PendingPathRequestQueue,
    failed_path_requests: &'a mut Vec<FailedPathRequest>,
    sequence_manager: &'a crate::sequence::SequenceManager,
}

pub(super) enum CompletedPathWork {
    Ready {
        request: PendingPathRequest,
        waypoints: Vec<MapPoint>,
    },
    Failed(PendingPathRequest),
}

pub(super) struct ExpiredPathWork {
    pub(super) request: FailedPathRequest,
    pub(super) owner_is_pc: bool,
    pub(super) age: u32,
}

impl<'a> PathScheduleContext<'a> {
    pub(super) fn new(
        frame_counter: u32,
        entities: &'a crate::entities::Entities,
        fast_grid: &'a crate::fast_find_grid::FastFindGrid,
        pathfinder: &'a mut crate::pathfinder::PathFinder,
        pending_path_requests: &'a mut PendingPathRequestQueue,
        failed_path_requests: &'a mut Vec<FailedPathRequest>,
        sequence_manager: &'a crate::sequence::SequenceManager,
    ) -> Self {
        Self {
            frame_counter,
            entities,
            fast_grid,
            pathfinder,
            pending_path_requests,
            failed_path_requests,
            sequence_manager,
        }
    }

    /// Execute one Original `RHPathFinder::ProcessPathRequests` barrier.
    ///
    /// The pathfinder starts its successor before returning a completed head
    /// to `RHEngine`, including from the recursive synchronous `WAITING` arm
    /// (`original-code/RHpathfinder.cpp:724-910`). Keep completion
    /// classification and successor start inside one context borrow so result
    /// application cannot enqueue, cancel, or otherwise change which request
    /// becomes in-flight first.
    pub(super) fn process_requests(
        &mut self,
        assets: &LevelAssets,
        synchronous_pathfinding: bool,
    ) -> Option<CompletedPathWork> {
        if self.pending_path_requests.has_in_flight() {
            let completed = self.take_completed();
            self.start_next(assets);
            return completed;
        }

        self.start_next(assets);
        if !synchronous_pathfinding {
            return None;
        }

        // Original's deterministic WAITING arm computes the first request,
        // recursively enters READY, starts/computes one successor, and only
        // then returns the first completion to RHEngine.
        let completed = self.take_completed();
        self.start_next(assets);
        completed
    }

    /// Take the one result made ready by the previous scheduling operation.
    /// Stale results are discarded without handing this barrier's completion
    /// slot to a later request.
    fn take_completed(&mut self) -> Option<CompletedPathWork> {
        let (processed, valid) = self.pending_path_requests.take_completed()?;
        let request = processed.request;
        if crate::pathfinder::parity_path_capture_is_active() {
            crate::pathfinder::record_parity_path_event(
                crate::pathfinder::ParityPathEvent::Completed {
                    request: parity_path_request_state(self.fast_grid, &request),
                    valid,
                    // Original records the raw path even when cancellation
                    // makes the delivery invalid. A failed A* request has an
                    // empty raw path but remains a valid delivery.
                    waypoints: processed.waypoints.clone().unwrap_or_default(),
                },
            );
        }
        if !valid {
            return None;
        }
        let still_live = self
            .sequence_manager
            .get_element(request.seq_id, request.elem_idx)
            .is_some_and(|elem| {
                elem.owner == Some(request.owner)
                    && elem.state == crate::sequence::SequenceState::InProgress
                    && elem.command == crate::element::Command::MoveWaiting
            });
        if !still_live {
            return None;
        }

        Some(match processed.waypoints {
            Some(waypoints) => CompletedPathWork::Ready { request, waypoints },
            None => CompletedPathWork::Failed(request),
        })
    }

    /// Start at most one queued request. Rust computes A* synchronously, but
    /// the result remains parked until the next scheduling operation consumes
    /// it (or the recursive deterministic `WAITING` arm above consumes it).
    fn start_next(&mut self, assets: &LevelAssets) {
        // `RHPathFinder::ProcessPathRequests` never inspects the requesting
        // sequence element (`original-code/RHpathfinder.cpp:806-820,891-901`).
        // Every entry that is still in `mListPathRequests` is started and,
        // one call later, delivered — including entries whose element has
        // since been interrupted. Only `CancelPathRequest` removes an entry,
        // and it removes at most the first *later* request for the actor
        // while the logical head merely gets `mbIgnoreNextPath`
        // (`original-code/RHpathfinder.cpp:538-598`). Skipping a request here
        // because its element died would hand the freed result slot to the
        // next queued request a frame early.
        let retained_cancelled_head = self.pending_path_requests.ignore_next_path;
        let Some(request) = self.pending_path_requests.pop_to_start() else {
            return;
        };
        // Original FindPathNodes observes mbIgnoreNextPath and exits before
        // expanding its first node. The retained head therefore delivers an
        // invalid completion with an empty raw path; it must not calculate a
        // route merely because Rust runs pathfinding synchronously.
        let waypoints = retained_cancelled_path_result(retained_cancelled_head).or_else(|| {
            self.pathfinder.find_path(
                assets.pathfinder_graph.as_ref(),
                self.fast_grid,
                request.layer,
                request.sector,
                request.half_diagonal_idx,
                request.source,
                request.dest,
                request.use_first_point,
            )
        });
        self.pending_path_requests.set_in_flight(request, waypoints);
    }

    /// Remove stale failures and return the next expired live entry.
    ///
    /// Returning one item at a time lets the root coordinator close hero
    /// speech, `element_impossible`, and the owner's synchronous condolation
    /// boundary before this method inspects the following entry, matching the
    /// mutable Original list walk at `RHengine.cpp:8487-8509`.
    pub(super) fn take_next_expired_failure(&mut self) -> Option<ExpiredPathWork> {
        let mut index = 0;
        while index < self.failed_path_requests.len() {
            let request = &self.failed_path_requests[index];
            let still_live = self
                .sequence_manager
                .get_element(request.seq_id, request.elem_idx)
                .is_some_and(|element| {
                    element.owner == Some(request.owner)
                        && element.state == crate::sequence::SequenceState::InProgress
                        && element.command == crate::element::Command::MoveWaiting
                });
            if !still_live {
                self.failed_path_requests.remove(index);
                continue;
            }

            let age = self.frame_counter.saturating_sub(request.first_fail_frame);
            if age <= 100 {
                index += 1;
                continue;
            }

            let owner_id = request.owner;
            let owner_is_pc = self
                .entities
                .get(owner_id)
                .unwrap_or_else(|| panic!(
                    "expired path request for {:?} retains a live sequence element but its owner entity is missing",
                    owner_id
                ))
                .is_pc();
            let request = self.failed_path_requests.remove(index);
            return Some(ExpiredPathWork {
                request,
                owner_is_pc,
                age,
            });
        }
        None
    }
}

#[inline]
fn retained_cancelled_path_result(retained_cancelled_head: bool) -> Option<Vec<MapPoint>> {
    retained_cancelled_head.then(Vec::new)
}

/// Outcome of [`EngineInner::try_dispatch_move_path`], the unified
/// pathfind-and-populate pipeline invoked from the hourglass Move
/// dispatch.
#[derive(Debug)]
pub(crate) enum MovePathOutcome {
    /// Path found, orders populated, actor's `active_movement` + action
    /// state set, element transitioned to `InProgress`.  Caller has
    /// nothing left to do.
    Success,
    /// The move requires A* and has entered the legacy one-completion-per-
    /// frame request queue.
    Pending,
    /// Dispatch could not submit the move (for example, source extraction
    /// failed). The caller applies the existing failure handling.
    Failed,
    /// The entity slot is empty or the element vanished mid-dispatch.
    /// Caller should mark the element `Impossible`.
    ActorGone,
    /// The actor's current state forbids the move outright (contest
    /// archer). The refusal bark has already been played; the caller marks
    /// the element `Impossible`.
    Refused,
}

impl GoalShape {
    /// The point used for pathfinding / the final MOVE's destination.
    pub(crate) fn goal_point(&self) -> MapPoint {
        match *self {
            GoalShape::Point { point, .. } => point,
            GoalShape::Seek { point, .. } => point,
            GoalShape::Target { point, .. } => point,
            GoalShape::Door { far_side_point, .. } => far_side_point,
            GoalShape::Line { midpoint, .. } => midpoint,
        }
    }
}

/// Source adaptation when an actor is currently straddling a gate.
///
/// When the actor's current door is non-null, the path source is
/// rewritten to the gate's far-side point / sector / layer based on the
/// actor's door direction.
///
/// Returns `None` when the actor is not in a gate (callers should use
/// the raw `position_map` / `sector` / `layer`).
pub(crate) fn adapt_source_to_current_door(
    doors: &[crate::gate::Door],
    door_handle: crate::position_interface::DoorHandle,
    door_direction: bool,
) -> Option<(MapPoint, u16, u16)> {
    adapt_source_to_current_door_with_identity(doors, door_handle, door_direction)
        .map(|(point, sector, layer)| (point, u16::from(sector), layer))
}

/// Identity-preserving form of [`adapt_source_to_current_door`]. Original
/// copies the complete endpoint RHposition, including its RHSector pointer.
pub(crate) fn adapt_source_to_current_door_with_identity(
    doors: &[crate::gate::Door],
    door_handle: crate::position_interface::DoorHandle,
    door_direction: bool,
) -> Option<(MapPoint, crate::position_interface::SectorHandle, u16)> {
    let door = doors.get(usize::from(door_handle))?;
    // door_direction true → use the "in" side of the door as the
    // source; false → use the "out" side.
    if door_direction {
        let handle = crate::position_interface::SectorHandle::new(u16::from(door.sector_in))?;
        Some((
            door.point_in,
            door.sector_in_index
                .map_or(handle, |index| handle.with_arena_index(index)),
            door.layer_in,
        ))
    } else {
        let handle = crate::position_interface::SectorHandle::new(u16::from(door.sector_out))?;
        Some((
            door.point_out,
            door.sector_out_index
                .map_or(handle, |index| handle.with_arena_index(index)),
            door.layer_out,
        ))
    }
}

/// Legacy `GetDoor()` source state for route construction.
///
/// Rust keeps an executing translated pass in `ActorData` rather than always
/// mirroring it into `PositionInterface`. Prefer that live pass until its
/// first `PassingDoor` callback, but only while the pass still owns the
/// installed actor order. A postponed pass can remain in this Rust-only slot
/// while an unrelated command is installed; Original's `GetDoor()` then
/// reflects only `PositionInterface` and must not be reconstructed from the
/// dormant pass. Original `RHElementActor::PassDoor` clears `GetDoor()` at the
/// callback even though the translated movement element can keep executing
/// its far-side walk, so later commands must use the live position/sector
/// instead of adapting through the completed gate.
///
/// The direction reported here is the pass's live traversal direction, not the
/// movement element's retained `mswDirection`. `RHSequence::AppendMoveToSequence`
/// reads `pTarget->GetDoorDirection()` (`original-code/RHsequence.cpp:369`),
/// which is `RHPositionInterface::mbDoorDirection`
/// (`original-code/RHpositioninterface.h:297-298`). That field is written by
/// `SetDoor( pDoor, <live direction> )` inside `RHElementActor::Translate`
/// (`original-code/RHelementactor.cpp:4035`, `:4053`, `:4080`, `:4110`,
/// `:4165`, `:4199`, `:4242`, `:4295`), where the direction comes from the
/// `GetSector() == pSectorIn` test performed at launch — the same test
/// `PassDoor::dispatch` reproduces into `ActiveDoorPass::direct`.
/// `ActiveDoorPass::position_direct` mirrors the *element's* serialized
/// `mswDirection` (`original-code/RHSequenceElementMovement.cpp:394-407`),
/// which only `RHArtificialIntelligence::Position` consumes.
pub(crate) fn current_door_for_route_source(
    entity: &crate::element::Entity,
) -> Option<(crate::position_interface::DoorHandle, bool)> {
    entity
        .actor_data()
        .and_then(|actor| {
            actor.active_door_pass.as_ref().filter(|pass| {
                pass.triggers_fired == 0
                    && actor
                        .installed_order
                        .is_some_and(|order| order.order_type == pass.current_action)
            })
        })
        .map(|pass| (pass.door_index, pass.direct))
        .unwrap_or_else(|| {
            let position = entity.position_iface();
            position
                .get_door()
                .map(|door| (door, position.get_door_direction()))
        })
}

/// Compare the object identities returned for two authored positions.
///
/// Original's GoTo branch compares `RHSector*` values directly
/// (`RHartificialintelligence.cpp:2566`), so equal script-facing sector
/// numbers do not imply that the positions occupy the same motion sector.
#[cfg(test)]
pub(super) fn sector_hits_have_distinct_identity(
    source: crate::fast_find_grid::SectorHit,
    goal: crate::fast_find_grid::SectorHit,
    expected_sector: crate::position_interface::SectorHandle,
) -> bool {
    match (source, goal) {
        (
            crate::fast_find_grid::SectorHit::Found {
                sector_idx: source_idx,
                sector_number: source_number,
            },
            crate::fast_find_grid::SectorHit::Found {
                sector_idx: goal_idx,
                sector_number: goal_number,
            },
        ) => {
            let expected = crate::sector::SectorNumber::new(u16::from(expected_sector) as i16);
            source_number == expected && goal_number == expected && source_idx != goal_idx
        }
        _ => false,
    }
}

#[cfg(test)]
mod route_source_tests {
    use super::{current_door_for_route_source, sector_hits_have_distinct_identity};
    use crate::element::{
        ActiveDoorPass, ActorData, ActorPc, ElementData, Entity, HumanData, InstalledActorOrder,
        PcData,
    };
    use crate::gate::DoorIndex;
    use crate::order::OrderType;
    use crate::position_interface::DoorHandle;

    fn pc_with_door_pass(triggers_fired: u8) -> Entity {
        pc_with_door_pass_directions(triggers_fired, true, true)
    }

    fn pc_with_door_pass_directions(
        triggers_fired: u8,
        direct: bool,
        position_direct: bool,
    ) -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData::default(),
            actor: ActorData {
                active_door_pass: Some(ActiveDoorPass {
                    door_index: DoorIndex::new(53).expect("valid door index"),
                    direct,
                    position_direct,
                    steps: Default::default(),
                    triggers_fired,
                    current_action: OrderType::WalkingUpright,
                    current_reverse: false,
                    saved_action_state: None,
                }),
                installed_order: Some(InstalledActorOrder {
                    order_id: std::num::NonZeroU32::new(1).unwrap(),
                    order_type: OrderType::WalkingUpright,
                }),
                ..ActorData::default()
            },
            human: HumanData::default(),
            pc: PcData::default(),
        })
    }

    #[test]
    fn route_source_uses_active_door_before_pass_callback() {
        let pc = pc_with_door_pass(0);

        assert_eq!(
            current_door_for_route_source(&pc),
            Some((DoorHandle::new(53).expect("valid door index"), true))
        );
    }

    #[test]
    fn route_source_drops_active_door_after_pass_callback() {
        let pc = pc_with_door_pass(1);

        assert_eq!(current_door_for_route_source(&pc), None);
    }

    #[test]
    fn route_source_does_not_resurrect_postponed_door_under_unrelated_order() {
        let mut pc = pc_with_door_pass(0);
        pc.actor_data_mut().unwrap().installed_order = Some(InstalledActorOrder {
            order_id: std::num::NonZeroU32::new(2).unwrap(),
            order_type: OrderType::WaitingUpright,
        });

        assert_eq!(
            current_door_for_route_source(&pc),
            None,
            "Rust's dormant pass mirror must not replace Original's null PositionInterface door pointer"
        );
    }

    #[test]
    fn route_source_reports_live_traversal_direction_not_element_direction() {
        // `RHSequence::AppendMoveToSequence` reads `GetDoorDirection()`
        // (`original-code/RHsequence.cpp:369`), the position interface field
        // `SetDoor` writes from the live `GetSector() == pSectorIn` test at
        // launch. A v48-restored movement element can carry a different
        // serialized `mswDirection`; that value belongs to
        // `RHArtificialIntelligence::Position`, not to route sourcing.
        let pc = pc_with_door_pass_directions(0, true, false);

        assert_eq!(
            current_door_for_route_source(&pc),
            Some((DoorHandle::new(53).expect("valid door index"), true))
        );
    }

    #[test]
    fn route_source_uses_position_door_during_pass_callback_queue_window() {
        let mut pc = pc_with_door_pass(1);
        pc.position_iface_mut()
            .set_door(DoorHandle::new(17).expect("valid door index"), false);

        assert_eq!(
            current_door_for_route_source(&pc),
            Some((DoorHandle::new(17).expect("valid door index"), false))
        );
    }

    #[test]
    fn goto_compares_sector_object_identity_even_when_numbers_match() {
        use crate::fast_find_grid::{SectorHit, SectorIndex};
        use crate::sector::SectorNumber;

        let number = SectorNumber::new(18);
        let hit = |index| SectorHit::Found {
            sector_idx: SectorIndex::new(index).unwrap(),
            sector_number: number,
        };

        let expected = crate::position_interface::SectorHandle::new(18).unwrap();
        assert!(sector_hits_have_distinct_identity(
            hit(12),
            hit(37),
            expected
        ));
        assert!(!sector_hits_have_distinct_identity(
            hit(12),
            hit(12),
            expected
        ));
        assert!(!sector_hits_have_distinct_identity(
            hit(12),
            SectorHit::None,
            expected
        ));
        assert!(!sector_hits_have_distinct_identity(
            hit(12),
            SectorHit::Found {
                sector_idx: SectorIndex::new(37).unwrap(),
                sector_number: SectorNumber::new(19),
            },
            expected
        ));
    }
}

/// Radius for circular dispatch (one third of [`GROUP_LIMIT_MAX`]).
pub(in crate::engine) const CIRCULAR_DISPATCH_RADIUS: f32 = 60.0;

/// Maximum centroid-to-member distance for mercenary formation to apply.
/// When any member is farther than this from the centroid, fall back to
/// circular dispatch.
pub(in crate::engine) const GROUP_LIMIT_MAX: f32 = 180.0;

/// Rebuild Original's per-actor formation box at a candidate destination.
///
/// Ordinary sectors translate the live `GetMoveBoxMap()` by the displacement
/// from the actor to the candidate. Lift sectors instead ask for
/// `GetMoveBox(RHPOSTURE_UPRIGHT)` and translate that zero-centred box to the
/// candidate. Original's current `GetMoveBox(posture)` implementation returns
/// the primary move box for every posture, but keeping the two source forms
/// distinct preserves the actual call boundary and saved live-box state.
fn group_move_candidate_box(
    live_move_box_map: MapBBox,
    upright_move_box: crate::coordinates::MoveBox,
    actor_position: MapPoint,
    candidate: MapPoint,
    is_lift: bool,
) -> MapBBox {
    if is_lift {
        upright_move_box.translated(candidate)
    } else {
        live_move_box_map.translated(candidate - actor_position)
    }
}

/// Build a compact-group formation box in the same sequence of floating-point
/// operations as `RHEngine::PerformGroupMove`:
///
/// * ordinary: `GetMoveBoxMap() - vectorCenter + pointDestination`
/// * lift: `GetMoveBox(Upright) + actorPosition - vectorCenter + pointDestination`
///
/// These translations must not be algebraically collapsed. The intermediate
/// rounding is observable in the path goal recorded by the Original engine.
fn group_move_mercenary_box(
    live_move_box_map: MapBBox,
    upright_move_box: crate::coordinates::MoveBox,
    actor_position: MapPoint,
    center: MapPoint,
    click: MapPoint,
    is_lift: bool,
) -> MapBBox {
    let centered = if is_lift {
        upright_move_box
            .translated(actor_position)
            .translated(MapVec::new(-center.x, -center.y))
    } else {
        live_move_box_map.translated(MapVec::new(-center.x, -center.y))
    };
    centered.translated(MapVec::new(click.x, click.y))
}

fn group_move_sector_kinds(sector_type: crate::sector::SectorType) -> (bool, bool, bool) {
    (
        sector_type.is_lift(),
        sector_type.is_door(),
        sector_type.is_jump(),
    )
}

#[inline]
fn group_move_route_goal(
    recorded_goal: Option<(crate::sector::SectorNumber, u16)>,
    selected_sector: Option<crate::sector::SectorNumber>,
    selected_layer: u16,
) -> (Option<crate::sector::SectorNumber>, u16) {
    recorded_goal
        .map(|(sector, layer)| (Some(sector), layer))
        .unwrap_or((selected_sector, selected_layer))
}

/// Recover the exact Original route-goal pointer without resolving a public
/// number through the lossy number map. A recorded goal normally names the
/// selected motion sector directly; patch/jump overlays retain an explicit
/// `underlying_sector` edge to the authoritative route goal.
fn group_move_route_goal_index(
    recorded_goal: Option<(crate::sector::SectorNumber, u16)>,
    selected_sector: Option<crate::sector::SectorNumber>,
    selected_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    selected_layer: u16,
    selected_grid_sector: Option<&crate::fast_find_grid::GridSector>,
    level: &crate::fast_find_grid::LevelGrid,
) -> Option<crate::fast_find_grid::SectorIndex> {
    let Some((recorded_sector, recorded_layer)) = recorded_goal else {
        return selected_sector_index;
    };
    if selected_sector == Some(recorded_sector) && selected_layer == recorded_layer {
        return selected_sector_index;
    }
    selected_grid_sector
        .and_then(|sector| sector.underlying_sector)
        .filter(|&index| {
            level.sectors.get(usize::from(index)).is_some_and(|sector| {
                sector.sector_number == recorded_sector && sector.layer == recorded_layer
            })
        })
}

/// Prefer the exact sparse FastFindGrid slot retained by replay translation.
/// A public sector number is not unique in retained topology, so an explicit
/// slot must agree with the recorded public sector number rather than falling
/// back to a coincident spatial hit. Original passes the RHSector pointer and
/// the RHPosition goal level independently; a sector's topology layer is not
/// an identity component here.
fn resolve_group_move_route_goal_index(
    recorded_goal: Option<(crate::sector::SectorNumber, u16)>,
    exact_goal_index: Option<crate::fast_find_grid::SectorIndex>,
    selected_sector: Option<crate::sector::SectorNumber>,
    selected_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    selected_layer: u16,
    selected_grid_sector: Option<&crate::fast_find_grid::GridSector>,
    level: &crate::fast_find_grid::LevelGrid,
) -> Option<crate::fast_find_grid::SectorIndex> {
    if let Some(index) = exact_goal_index {
        let (recorded_sector, _recorded_layer) = recorded_goal.unwrap_or_else(|| {
            panic!("replay group-move exact goal index requires a recorded goal identity")
        });
        let sector = level.sectors.get(usize::from(index)).unwrap_or_else(|| {
            panic!("replay group-move exact goal index {index:?} is absent from retained topology")
        });
        assert_eq!(
            sector.sector_number, recorded_sector,
            "replay group-move exact goal index disagrees with its recorded public sector number"
        );
        return Some(index);
    }

    group_move_route_goal_index(
        recorded_goal,
        selected_sector,
        selected_sector_index,
        selected_layer,
        selected_grid_sector,
        level,
    )
}

#[inline]
fn group_move_door_selection(
    spatial_clicked_door_index: Option<u32>,
    spatial_is_door_click: bool,
    recorded_door_route: Option<bool>,
) -> (Option<u32>, bool, bool) {
    let (route_door_index, route_is_door) = match recorded_door_route {
        Some(false) => (None, false),
        Some(true) => (
            Some(spatial_clicked_door_index.unwrap_or_else(|| {
                panic!("recorded group move requires a door route but no Rust door was hit")
            })),
            true,
        ),
        None => (spatial_clicked_door_index, spatial_is_door_click),
    };

    // Schema-16's reconstructed route kind is authoritative for the selected
    // door identity too. Rust's spatial query can land on a coincident door
    // polygon even when Original selected the ordinary area underneath; using
    // that reconstructed hit would incorrectly skip FindAutorizedPosition.
    // Live commands have no override and continue to use the spatial result.
    let bypass_formation_authorization = recorded_door_route
        .map(|_| route_is_door)
        .unwrap_or(spatial_is_door_click);
    (
        route_door_index,
        route_is_door,
        bypass_formation_authorization,
    )
}

#[inline]
fn group_move_uses_simple_route(
    has_recorded_route_outcome: bool,
    is_door_click: bool,
    is_valid: bool,
    goal_sector: Option<crate::sector::SectorNumber>,
    goal_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    goal_layer: u16,
    source_sector: u16,
    source_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    source_layer: u16,
) -> bool {
    // A schema-16 route-construction event proves that Original entered
    // AppendMoveToSequence's cross-sector branch. Reconstructed source/goal
    // handles can nevertheless compare equal when overlapping Original
    // sectors collapse onto one Rust identity. The recorded outcome wins over
    // that apparent same-sector result, for both success and failure.
    if has_recorded_route_outcome {
        return false;
    }
    // PerformMove passes the patch-aware pSectorGoal to AppendMoveToSequence.
    // A coincident mpSelectedSector door only controls formation authorization;
    // it cannot turn an equal source/goal sector into a door traversal.
    let same_topology = match (goal_sector_index, source_sector_index) {
        (Some(goal), Some(source)) => goal == source,
        _ => goal_sector
            .is_some_and(|goal| u16::from(goal) == source_sector && goal_layer == source_layer),
    };
    same_topology || (!is_door_click && (!is_valid || goal_sector.is_none()))
}

/// Return an authoritative schema-16 route outcome when one was recorded.
/// `Some(None)` is deliberately distinct from `None`: the former means
/// Original already ran gate A* and observed failure, while the latter permits
/// the live engine to resolve a route itself.
fn recorded_group_move_route_result<T>(
    actor: EntityId,
    successful: Option<T>,
    failed_count: usize,
) -> Option<Option<T>> {
    assert!(
        failed_count <= 1,
        "recorded group move contains duplicate failed routes for {actor:?}"
    );
    assert!(
        failed_count == 0 || successful.is_none(),
        "recorded group move marks {actor:?} route as both successful and failed"
    );
    if failed_count != 0 {
        Some(None)
    } else {
        successful.map(Some)
    }
}

/// `PerformGroupMove` receives one resolved upright action from the click
/// dispatcher. It does not infer sword movement from an actor's opponent list;
/// `DetermineMovementAnimation` performs any live action-state adaptation when
/// the movement is instructed.
#[inline]
fn player_group_move_action(run: bool) -> OrderType {
    if run {
        OrderType::RunningUpright
    } else {
        OrderType::WalkingUpright
    }
}

/// Recover the complete live `RHSector*` identity sampled by Original's
/// `RHEngine::PerformGroupMove` before it calls `PerformMove`.
///
/// An adopted compatibility position can retain only the public sector
/// number. In a loaded exact grid, resolve that omitted identity from the
/// actor's current point/public number/layer using the same invariant-checked
/// boundary as every other Rust `Position(element)` snapshot. A wholly empty
/// fixture remains number-only; ambiguous or absent loaded topology is an
/// invariant failure rather than a lossy public-number guess.
#[inline]
fn group_move_source_sector(
    engine: &EngineInner,
    actor: EntityId,
    element: &crate::element::ElementData,
) -> crate::position_interface::SectorHandle {
    super::ai::ai_view_position_sector(engine, element).unwrap_or_else(|| {
        panic!("selected group-move actor {actor:?} has no source sector identity")
    })
}

#[inline]
fn group_move_route_source(
    engine: &EngineInner,
    actor: EntityId,
    entity: &crate::element::Entity,
    doors: &[crate::gate::Door],
) -> (MapPoint, crate::position_interface::SectorHandle, u16) {
    current_door_for_route_source(entity)
        .and_then(|(door_handle, door_direction)| {
            adapt_source_to_current_door_with_identity(doors, door_handle, door_direction)
        })
        .unwrap_or_else(|| {
            let element = entity.element_data();
            (
                element.position_map(),
                group_move_source_sector(engine, actor, element),
                element.layer(),
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn find_group_move_gate_path(
    doors: &[crate::gate::Door],
    owner: EntityId,
    source: MapPoint,
    source_sector: crate::position_interface::SectorHandle,
    goal: MapPoint,
    goal_sector: crate::sector::SectorNumber,
    goal_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    goal_layer: u16,
    auth: Option<&crate::gate::ActorAuthInfo>,
    building_is_authorized: &impl Fn(crate::sector::SectorNumber) -> bool,
    sector_lift_type: &impl Fn(crate::sector::SectorNumber) -> Option<crate::sector::LiftType>,
) -> Option<Vec<crate::gate::GatePathStep>> {
    let source_sector_index = source_sector.arena_index();
    let exact_graph = doors
        .iter()
        .any(|door| door.sector_out_index.is_some() || door.sector_in_index.is_some());
    if exact_graph {
        assert!(
            source_sector_index.is_some(),
            "TODO(parity): cross-sector group move for {owner:?} lacks exact source arena identity"
        );
        assert!(
            goal_sector_index.is_some(),
            "TODO(parity): authoritative group-move goal sector {} on layer {} lacks exact arena provenance",
            u16::from(goal_sector),
            goal_layer
        );
    }
    crate::gate::find_path_gates_with_sector_indices(
        doors,
        (source.x, source.y),
        u16::from(source_sector),
        source_sector_index,
        (goal.x, goal.y),
        u16::from(goal_sector),
        goal_sector_index,
        auth,
        false,
        building_is_authorized,
        sector_lift_type,
    )
}

#[allow(clippy::too_many_arguments)]
fn find_ai_move_gate_path(
    doors: &[crate::gate::Door],
    source: MapPoint,
    source_sector: crate::position_interface::SectorHandle,
    source_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    goal: MapPoint,
    goal_sector: crate::position_interface::SectorHandle,
    goal_sector_index: Option<crate::fast_find_grid::SectorIndex>,
    goal_door: Option<crate::gate::DoorIndex>,
    auth: Option<&crate::gate::ActorAuthInfo>,
    allow_leave_map: bool,
    building_is_authorized: &impl Fn(crate::sector::SectorNumber) -> bool,
    sector_lift_type: &impl Fn(crate::sector::SectorNumber) -> Option<crate::sector::LiftType>,
) -> Option<Vec<crate::gate::GatePathStep>> {
    if let Some(goal_door) = goal_door {
        crate::gate::find_path_into_door_with_sector_index(
            doors,
            (source.x, source.y),
            u16::from(source_sector),
            source_sector_index,
            goal_door,
            auth,
            allow_leave_map,
            building_is_authorized,
            sector_lift_type,
        )
    } else {
        crate::gate::find_path_gates_with_sector_indices(
            doors,
            (source.x, source.y),
            u16::from(source_sector),
            source_sector_index,
            (goal.x, goal.y),
            u16::from(goal_sector),
            goal_sector_index,
            auth,
            allow_leave_map,
            building_is_authorized,
            sector_lift_type,
        )
    }
}

/// Movement Execute arms which return without calling into `Sprite` still
/// produce an authoritative `mmotionState` in the Original actor. Rust uses
/// the sprite's transient motion latch to carry specialized Execute results to
/// the actor coordinator, so these non-sprite arms must publish their return
/// explicitly.
#[inline]
fn non_sprite_movement_motion(action: OrderType) -> Option<MotionState> {
    match action {
        OrderType::Freezing => Some(MotionState::InProgress),
        OrderType::PassingDoor => Some(MotionState::Terminated),
        _ => None,
    }
}

/// Compute per-character destination points using circular distribution.
///
/// The circular dispatch fallback when the group is too spread out for
/// the mercenary formation.
///
/// Characters are arranged in a circle around `click_point`. Each
/// unassigned character picks the nearest available slot; when multiple
/// characters want the same slot, the one farthest from the click gets it
/// (the "worst placed" heuristic). The loop repeats until all characters
/// are assigned.
pub(crate) fn circular_dispatch_destinations(
    pc_positions: &[MapPoint],
    click_point: MapPoint,
) -> Vec<MapPoint> {
    let n = pc_positions.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![click_point];
    }

    // Generate candidate positions in a circle around click_point.
    // Each candidate is `(0, -CIRCULAR_DISPATCH_RADIUS)` rotated by
    // `(i * TWO_PI / n)`.
    let candidates: Vec<MapPoint> = (0..n)
        .map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / n as f32;
            MapPoint::new(
                click_point.x + angle.sin() * CIRCULAR_DISPATCH_RADIUS,
                click_point.y - angle.cos() * CIRCULAR_DISPATCH_RADIUS,
            )
        })
        .collect();

    let mut result = vec![click_point; n];
    let mut assigned = vec![false; n];
    let mut candidate_taken = vec![false; candidates.len()];

    // Iterative assignment with conflict resolution.
    loop {
        // Each unassigned character picks its nearest untaken candidate.
        // Store (character_idx, sq_dist) per candidate.
        let mut claims: Vec<Vec<(usize, f32)>> = vec![Vec::new(); candidates.len()];

        for (ci, &pos) in pc_positions.iter().enumerate() {
            if assigned[ci] {
                continue;
            }
            let mut best_k = None;
            let mut best_d = f32::INFINITY;
            for (ki, &cand) in candidates.iter().enumerate() {
                if candidate_taken[ki] {
                    continue;
                }
                let dx = pos.x - cand.x;
                let dy = pos.y - cand.y;
                let d = dx * dx + dy * dy;
                if d < best_d {
                    best_d = d;
                    best_k = Some(ki);
                }
            }
            if let Some(ki) = best_k {
                claims[ki].push((ci, best_d));
            }
        }

        let mut any_assigned = false;
        for (ki, claimants) in claims.iter().enumerate() {
            match claimants.len() {
                0 => {}
                1 => {
                    let (ci, _) = claimants[0];
                    result[ci] = candidates[ki];
                    assigned[ci] = true;
                    candidate_taken[ki] = true;
                    any_assigned = true;
                }
                _ => {
                    // Multiple characters want this candidate.
                    // Give it to the "worst-placed" claimant — the one
                    // whose distance to the contested slot is largest
                    // (per-claimant distance to the slot, not distance
                    // to the click point).
                    let worst = claimants
                        .iter()
                        .max_by(|(_, da), (_, db)| {
                            da.partial_cmp(db).unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .unwrap()
                        .0;
                    result[worst] = candidates[ki];
                    assigned[worst] = true;
                    candidate_taken[ki] = true;
                    any_assigned = true;
                }
            }
        }

        if !any_assigned || assigned.iter().all(|&a| a) {
            break;
        }
    }

    // Any unassigned characters get the click point itself.
    result
}

/// Build the portion of a line-jump click sequence that follows arrival at
/// the selected source line.
///
/// Original routes the approach to that line through
/// `RHSequence::AppendMoveToLineToSequence`; the approach may therefore
/// contain an `AssertPosition` and a complete gate path.  Keeping only the
/// post-arrival elements here prevents callers from accidentally replacing
/// that route with one direct, potentially blocked movement segment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_line_jump_click_tail(
    owner: EntityId,
    action: OrderType,
    source_line_idx: crate::jump_line::JumpLineIndex,
    destination_line_idx: crate::jump_line::JumpLineIndex,
    click_point: MapPoint,
    click_layer: u16,
    speed_factor: f32,
) -> Vec<crate::sequence::SequenceElement> {
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, MoveFlags, SequenceElement, SequenceElementData};

    let mut jump = SequenceElement::new_generic(1, Command::JumpCmd, Some(owner));
    jump.set_property(Field::JumplineSource, FieldValue::LineId(source_line_idx));
    jump.set_property(
        Field::JumplineDestination,
        FieldValue::LineId(destination_line_idx),
    );

    let mut final_move = SequenceElement::new_movement(2, Command::Move, Some(owner), action);
    final_move.data = SequenceElementData::Movement {
        destination: click_point,
        layer: click_layer,
        sector: None,
        gate_id: None,
        line_id: None,
        element: None,
        // Original appends the post-jump click tail with no movement flags.
        // In particular it is a normal-priority move, so a later click may
        // interrupt it while the actor is still finishing this route.
        flags: MoveFlags::empty(),
        tolerance: 0.0,
        direction: 0,
        action,
        speed_factor,
        post_seek_sequence: None,
    };
    vec![jump, final_move]
}

/// Actor that owns the routed approach to a selected jump line.
///
/// `RHengine.cpp::PerformMove` substitutes the carrier only for
/// `AppendMoveToLineToSequence`; the explicit jump and post-jump movement
/// remain owned by the selected PC.
pub(in crate::engine) fn line_jump_approach_owner(
    engine: &EngineInner,
    selected_pc: EntityId,
) -> EntityId {
    let selected = engine.expect_entity(selected_pc, "line-jump selected PC");
    if selected.element_data().posture != crate::element::Posture::OnShoulders {
        return selected_pc;
    }
    let carrier = selected
        .human_data()
        .unwrap_or_else(|| panic!("OnShoulders line-jump owner {selected_pc:?} is not human"))
        .carrier
        .unwrap_or_else(|| {
            panic!("OnShoulders line-jump owner {selected_pc:?} has no retained carrier")
        });
    let carrier_entity = engine.expect_entity(carrier, "OnShoulders line-jump carrier");
    assert!(
        carrier_entity.is_pc(),
        "OnShoulders line-jump carrier {carrier:?} for {selected_pc:?} is not a PC"
    );
    carrier
}

#[derive(Clone, Copy, Default)]
struct FinalTol {
    tol: f32,
    directional: bool,
    target_is_actor: bool,
    /// Entity target resolved when the movement frame starts. Its
    /// position, sector, and current-row hotspot are sampled again at
    /// this actor's creation-order slot, after earlier actors have
    /// committed their movement.
    target_id: Option<EntityId>,
    use_point: bool,
    /// Shield seeks compare actor position to the movement
    /// element's computed shield destination, not to the
    /// protected PC's live position.
    shield_destination: Option<MapPoint>,
    /// Snapshot of `ActorData::last_seek_target_position` —
    /// the target position stamped at seek launch / refresh.
    /// Used by the final-order completion check to distinguish an
    /// arrival at the sampled target from an exhausted stale path.
    last_seek_target_position: MapPoint,
    /// Whether the actor has a `post_seek_sequence` attached.
    /// Lifts the `is_final_waypoint` gate on tolerance arrival
    /// for mid-path arrivals: the seek's same-sector +
    /// tolerance predicate runs every tick, not just at the
    /// final waypoint.  When the target wanders into range
    /// mid-route, the seek terminates early and the post-seek
    /// sequence fires.  Without a post-seek sequence to
    /// consume the arrival, the order_pop fall-through would
    /// drop intermediate waypoints and leave the actor
    /// stranded — so guard intermediate-tick arrival on this
    /// flag.
    has_post_seek: bool,
    /// Whether arrival calls `StartPostSeekSequence` rather than merely
    /// continuing to a later element in the same Rust sequence.
    launches_post_seek: bool,
}

/// Owner-scoped pre-pass snapshots for one `tick_entity_movement_owner`
/// call, captured before the mutable per-actor movement pass so
/// `tick_one_movement_actor` can borrow the entity table mutably.
struct MovementPrepass {
    combat_face_target: Option<MapPoint>,
    combat_face_target_is_ground: bool,
    speed_factor: f32,
    goal_target_info: Option<crate::position_interface::TargetInfo>,
    final_tolerance: FinalTol,
    point_seek_post_sector: Option<crate::position_interface::SectorHandle>,
    lift_translation: Option<LiftAnimContext>,
    door_pass_climb_direction: Option<i16>,
    decorative_building_trap_at_destination: bool,
}

/// Deferred side effects collected by `tick_one_movement_actor` while the
/// per-actor entity borrow is live, then drained in parity-critical order
/// by `tick_entity_movement_owner` after the movement pass.
#[derive(Default)]
struct MovementDeferred {
    post_completion_motion_override: Option<crate::sprite::MotionState>,
    sword_movement_starts: Vec<EntityId>,
    sword_movement_terminations: Vec<EntityId>,
    // Collect movement results that need sequence manager notification.
    // We can't call sequence_manager while iterating entities mutably.
    // Door-pass triggers to execute after the movement loop (need &mut self).
    door_triggers: Vec<(EntityId, crate::gate::DoorIndex, bool, u8)>,
    // Door-pass Transition orders to push onto the actor's current
    // sequence element after the loop closes (needs sequence_manager).
    transition_pushes: Vec<(crate::sequence::SequenceId, usize, crate::order::Order)>,
    // Pending `DoorPassStep::Select` hulk requests — processed after the
    // loop since they mutate both the carrier and its carried target.
    select_triggers: Vec<(EntityId, f32)>,
    completed_door_passes: Vec<(EntityId, crate::gate::DoorIndex, bool)>,
    // Rider entities whose running animation hit the charge
    // decision frames while carrying RIDER_CHARGE.
    galopp_event: bool,
    // Movement elements whose sprite motion returned the blocked-
    // abort signal and must be marked Impossible after the entity
    // borrow ends.
    blocked_impossible: Vec<(crate::sequence::SequenceId, usize)>,
    door_pass_transition_start_effects: Vec<EntityId>,
    door_pass_transition_done_effects: Vec<EntityId>,
    door_pass_transition_completion_effects: Vec<(EntityId, OrderType)>,
    /// Final door-pass goals retired by the terminal Execute arm. Original
    /// clears these through DoNextOrder/condolation only after
    /// Actor::CheckForLineCrossing has had a chance to rebuild the cached
    /// increment against the still-live destination.
    terminal_door_pass_goal_clears: Vec<EntityId>,
    post_seek_arrivals: Vec<(EntityId, crate::sequence::SequenceId, usize)>,
    /// Terminal state effect from a movement transition whose `PerformSeek`
    /// pre-motion tolerance arm launched a post-seek sequence. Original
    /// launches that sequence synchronously from inside `PerformSeek`; only
    /// after its callbacks return does the surrounding transition Execute arm
    /// apply its TERMINATED posture/action-state switch.
    post_seek_terminal_state_effects: Vec<(
        EntityId,
        crate::element::Posture,
        crate::element::ActionState,
    )>,
    /// Same ordering contract as `post_seek_terminal_state_effects`, for the
    /// Rust representation where the post-seek continuation is a following
    /// element of the same sequence and is exposed by the terminal order pop.
    sequence_seek_terminal_state_effects: Vec<(
        EntityId,
        crate::element::Posture,
        crate::element::ActionState,
    )>,
    // Elevation-line crossings detected during this tick. Dispatched
    // after the entity loop so `check_for_line_crossing` can borrow
    // `self` for the fast-grid query and obstacle swap.
    // Each entry is `(entity_id, old_pos, layer)` in projected map
    // coordinates; the segment endpoint is resolved from the actor's
    // live position when the checks are dispatched, because
    // CheckForLineCrossing runs after the whole Execute arm and some
    // arms reposition the actor in their completion branch. Geometry
    // queries convert at the call boundary.
    line_cross_checks: Vec<(EntityId, MapPoint, u16)>,
    // Original collects all non-elevation LINE_CROSS kinds together,
    // sorts once by travel distance, then checks patch/script/sound flags
    // on each line in that order.
    non_elevation_cross_checks: Vec<(EntityId, MapPoint, u16)>,
    // Final entity-seek orders whose live target no longer matches the
    // sampled endpoint. Original refreshes these only when the current
    // order itself terminates; merely exposing a stop-transition as the
    // next order is not a refresh boundary.
    transition_seek_refreshes: Vec<(EntityId, crate::sequence::SequenceId, usize)>,
    // Waypoint arrivals (both intermediate and final) — each
    // triggers one `do_next_order` call on the actor's Move
    // element to pop the walking order that represented that
    // waypoint.  Each waypoint is its own order on the actor's
    // movement order list, and the engine pops them as the actor
    // crosses them.  Collected here and processed after the entity
    // loop so the `do_next_order` call can borrow `self` mutably.
    order_pops: Vec<(crate::sequence::SequenceId, usize)>,
    // A resolved mouse orientation can run immediately before the last
    // tick of a generated stop transition.  The transition still turns
    // toward that externally supplied direction, but its outgoing Move
    // order owns the cached movement increment.  Keep enough context to
    // restore the external goal after the Move's terminal condolence has
    // retired that cache.
    terminal_pc_direction_goal_restores: Vec<(EntityId, i16, i16)>,

    // Water-splash titbit emissions queued from the walk branch.
    // Drained after the entity loop so `titbit_manager.add_titbit`
    // can borrow `&mut self` without colliding with the active
    // entity borrow.
    water_splash_emits: Vec<(EntityId, crate::coordinates::WorldPoint3D, u16)>,
    movement_state_effects: Vec<(
        EntityId,
        crate::element::Posture,
        crate::element::ActionState,
    )>,
    // PC movement actions actually dispatched this frame.  The original
    // RHElementActorPC performs action-specific side effects from inside
    // the matching Execute arm, so posture alone is not a substitute for
    // this per-frame execution record.
    executed_pc_movement_actions: Vec<(EntityId, OrderType)>,
    executed_sword_movement: bool,
}

/// Preserve Actor::Hourglass' post-Execute line-crossing segment when a
/// movement step reaches a seek arrival whose synchronous handoff returns
/// before the ordinary movement tail. The segment endpoint is deliberately
/// resolved later from the actor's live position, exactly like the common
/// tail; only the outer pre-Execute position is retained here.
fn queue_committed_arrival_crossing(
    deferred: &mut MovementDeferred,
    entity_id: EntityId,
    old_pos: MapPoint,
    layer: u16,
    arrived_after_committed_step: bool,
    eligible_for_crossing: bool,
) -> bool {
    if !arrived_after_committed_step || !eligible_for_crossing {
        return false;
    }
    deferred.line_cross_checks.push((entity_id, old_pos, layer));
    deferred
        .non_elevation_cross_checks
        .push((entity_id, old_pos, layer));
    true
}

fn clear_terminal_door_pass_goal(entity: &mut Entity) {
    entity
        .position_iface_mut()
        .set_map_goal(crate::coordinates::MapPoint::ZERO);
}

/// Argument plumbing shared by the two movement-Execute anti-collision
/// dispatches (the transition fast-climb arm and the ordinary walk arm).
/// A free function rather than a method: at both call sites the mover is
/// held as a live `&mut` borrow out of the entity table, so `self` cannot
/// be borrowed as a whole.
#[allow(clippy::too_many_arguments)]
fn apply_prepared_anti_collision_step(
    frame: u32,
    mover_snap: &super::anti_collision::ActorSnapshot,
    anti_snapshots: &EntitySlots<Option<super::anti_collision::ActorSnapshot>>,
    static_repulsive_points: &[crate::ai::RepulsivePoint],
    prepared: &LiveMobileGeometry,
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    state: &mut super::anti_collision::AntiCollisionState<'_>,
    nx: f32,
    ny: f32,
    speed: f32,
    anti_on: bool,
) -> (f32, f32) {
    super::anti_collision::with_goal_owner_anti_frame(frame, || {
        let trace = super::anti_collision::goal_owner_anti_debug_frame(mover_snap.id).is_some();
        let before = trace.then(|| {
            (
                state.pi.map_position(),
                state.pi.map_goal(),
                state.pi.is_deviated(),
                state.pi.blocked_count,
                state.pi.radius,
            )
        });
        let result = super::anti_collision::apply_anti_collision_step(
            mover_snap,
            anti_snapshots.as_slice(),
            static_repulsive_points,
            prepared
                .mobile_points_by_layer
                .get(&mover_snap.layer)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            prepared
                .mobile_lines_by_layer
                .get(&mover_snap.layer)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            prepared
                .mobile_polygons_by_layer
                .get(&mover_snap.layer)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            Some(fast_grid),
            Some(&mut *state),
            nx,
            ny,
            speed,
            anti_on,
        );
        if trace {
            eprintln!(
                "[GOAL_OWNER frame={frame} owner={:?} stage=anti_result requested_bits={:08x},{:08x},{:08x} result_bits={:08x},{:08x} before={before:?} after=({:?},{:?},{},{},{})]",
                mover_snap.id,
                nx.to_bits(),
                ny.to_bits(),
                speed.to_bits(),
                result.0.to_bits(),
                result.1.to_bits(),
                state.pi.map_position(),
                state.pi.map_goal(),
                state.pi.is_deviated(),
                state.pi.blocked_count,
                state.pi.radius,
            );
        }
        result
    })
}

impl EngineInner {
    /// Opt-in sequence/path ownership trace for parity frontiers where the
    /// queued path operands already agree but the selected movement command
    /// does not. Keep this on stderr and outside serialized state so enabling
    /// it cannot affect simulation or cache compatibility.
    pub(in crate::engine) fn trace_path_owner_lifecycle(
        &self,
        stage: &'static str,
        owner: EntityId,
        focus: Option<(crate::sequence::SequenceId, usize)>,
    ) {
        if std::env::var_os("PARITY_DEBUG_PATH_OWNER_LIFECYCLE").is_none() {
            return;
        }
        let parse_filter = |name: &str| {
            std::env::var(name).ok().map(|value| {
                value.parse::<u32>().unwrap_or_else(|error| {
                    panic!("invalid {name}={value:?} for path-owner lifecycle diagnostic: {error}")
                })
            })
        };
        let frame = self.control.frame_counter;
        if parse_filter("PARITY_DEBUG_PATH_OWNER_FRAME").is_some_and(|expected| expected != frame)
            || self.get_entity(owner).is_none()
        {
            return;
        }
        let creation_order = self.world.original_creation_order(owner);
        if parse_filter("PARITY_DEBUG_PATH_OWNER_CREATION_ORDER")
            .is_some_and(|expected| expected != creation_order)
        {
            return;
        }

        let manager = &self.orders.sequence_manager;
        let selected = manager.current_element_for_actor(owner);
        let current_order =
            manager
                .current_order_for_actor(owner)
                .map(|(sequence_id, element_index, order)| {
                    (
                        sequence_id,
                        element_index,
                        order.order_type,
                        order.order_id,
                        order.done,
                        order.target_x.to_bits(),
                        order.target_y.to_bits(),
                        order.tolerance.to_bits(),
                        order.move_flags,
                        order.antagonist,
                    )
                });
        let graph = manager
            .sequences_iter()
            .flat_map(|sequence| {
                sequence
                    .elements
                    .iter()
                    .enumerate()
                    .filter(move |(_, element)| element.owner == Some(owner))
                    .map(move |(element_index, element)| {
                        (
                            sequence.id,
                            element_index,
                            element.command,
                            element.state,
                            element.priority,
                            element.cross_postponed,
                            manager.is_registered_to_go(sequence.id, element_index),
                            element.current_order().map(|order| {
                                (
                                    order.order_type,
                                    order.order_id,
                                    order.done,
                                    order.target_x.to_bits(),
                                    order.target_y.to_bits(),
                                    order.tolerance.to_bits(),
                                    order.move_flags,
                                    order.antagonist,
                                )
                            }),
                        )
                    })
            })
            .collect::<Vec<_>>();
        let actor = self.get_entity(owner).and_then(|entity| {
            entity.actor_data().map(|actor| {
                (
                    actor.action_state,
                    actor
                        .installed_order
                        .as_ref()
                        .map(|order| (order.order_type, order.order_id)),
                )
            })
        });
        let position = self.get_entity(owner).map(|entity| {
            (
                entity.position_iface().map_position(),
                entity.position_iface().old_map_position(),
                entity.position_iface().map_goal(),
                entity
                    .position_iface()
                    .is_increment_map_computed()
                    .then(|| entity.position_iface().get_increment_map()),
                entity.position_iface().is_moving(),
                entity.position_iface().is_deviated(),
            )
        });
        let pending_paths = self
            .orders
            .pending_path_requests
            .parity_state(&self.world.fast_grid);
        eprintln!(
            "[PATH_OWNER frame={frame} co={creation_order} owner={} stage={stage} focus={focus:?} selected={selected:?} current_order={current_order:?} actor={actor:?} position={position:?} pending={pending_paths:?} graph={graph:?}]",
            owner.index(),
        );
    }

    pub(crate) fn parity_failed_path_requests(
        &self,
    ) -> Vec<crate::pathfinder::ParityFailedPathRequest> {
        self.orders
            .failed_path_requests
            .iter()
            .map(|failed| {
                let request = &failed.request;
                assert_eq!(
                    failed.owner, request.owner,
                    "failed-path timeout owner disagrees with retained request"
                );
                crate::pathfinder::ParityFailedPathRequest {
                    request: parity_path_request_state(&self.world.fast_grid, request),
                    sector: request.legacy_sector,
                    time: failed.first_fail_frame,
                }
            })
            .collect()
    }

    /// Rebuild Rust's derived active-movement latch after loading an Original
    /// save. Original keeps the executing movement in `mpSequenceElement`;
    /// Rust additionally caches its sequence identity for owner-local
    /// movement and anti-collision work.
    pub(crate) fn restore_loaded_active_movements(&mut self) {
        let active =
            self.orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| {
                    sequence.elements.iter().enumerate().filter_map(
                        move |(element_index, element)| {
                            (element.state == crate::sequence::SequenceState::InProgress
                                && element.data.is_movement())
                            .then_some((element.owner?, sequence.id, element_index))
                        },
                    )
                })
                .collect::<Vec<_>>();
        let mut owners = std::collections::BTreeSet::new();
        for (owner, sequence_id, element_index) in active {
            assert!(
                owners.insert(owner),
                "loaded actor {owner:?} owns multiple in-progress movement elements"
            );
            self.world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .unwrap_or_else(|| {
                    panic!("loaded movement owner {owner:?} is missing required actor state")
                })
                .active_movement = ActiveMovement::new(sequence_id, element_index);
        }
    }

    /// Consume one queued `RHElement` position update at the beginning of
    /// this actor's Hourglass, before order selection.
    ///
    /// Original gives the map-space queue priority when both flags are set;
    /// the world-space queue remains armed for the next actor frame. The
    /// resulting teleport still traverses Actor::CheckForLineCrossing.
    pub(super) fn apply_delayed_actor_position(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let (old_pos, new_pos, layer, posture, is_carried, is_pc, is_human) = {
            let Some(entity) = self.world.entities.get_mut(entity_id) else {
                panic!("delayed-position owner {entity_id:?} disappeared before Actor::Hourglass");
            };
            let posture = entity.element_data().posture;
            let is_carried = entity
                .human_data()
                .is_some_and(|human| human.carrier.is_some());
            let is_pc = entity.is_pc();
            let is_human = entity.is_human();
            let Some((old_pos, new_pos, layer)) =
                entity.element_data_mut().apply_next_delayed_position()
            else {
                return;
            };
            (
                old_pos, new_pos, layer, posture, is_carried, is_pc, is_human,
            )
        };

        if !actor_line_crossing_eligible(
            posture,
            is_carried,
            self.world.fast_grid.level.map_bbox.contains_point(new_pos),
        ) {
            return;
        }

        // Original queries one unified LINE_CROSS list here.  Its multi-line
        // arm runs the shared UpdateRoll/ComputeIncrementAll tail even when
        // every crossed line is non-elevation.  Keep the candidate count
        // intact across the elevation and callback dispatches; splitting the
        // queries first loses that observable `count > 1` branch.
        let crossing_indices = self
            .world
            .fast_grid
            .get_actor_crossing_line_indices(layer, old_pos, new_pos);
        let crossing_count = crossing_indices.len();
        let elevation_indices = crossing_indices
            .iter()
            .copied()
            .filter(|&line_index| {
                self.world.fast_grid.level.lines[usize::from(line_index)].is_elevation
            })
            .collect::<Vec<_>>();
        let callback_indices = crossing_indices
            .into_iter()
            .filter(|&line_index| {
                crossing_count == 1
                    || !self.world.fast_grid.level.lines[usize::from(line_index)].is_elevation
            })
            .collect::<Vec<_>>();
        let crossed_elevation = self.check_for_elevation_line_crossing_indices(
            assets,
            entity_id,
            old_pos,
            new_pos,
            layer,
            elevation_indices,
        );
        if crossed_elevation || crossing_count > 1 {
            if is_human {
                self.update_roll_after_crossing(assets, entity_id);
            }
            let compute_direction = self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
                .map(|(_, _, order)| order.compute_direction);
            if let Some(compute_direction) = compute_direction
                && let Some(entity) = self.world.entities.get_mut(entity_id)
            {
                entity
                    .position_iface_mut()
                    .compute_increment_all(compute_direction);
            }
        }
        let _ = is_pc;
        self.check_for_non_elevation_line_crossing_indices(
            sim,
            assets,
            entity_id,
            old_pos,
            new_pos,
            callback_indices,
        );
    }

    /// Match `RHSectorBuilding::IsAuthorized()` for gate pathfinding.
    ///
    /// The original initializes every building's maximum occupancy to
    /// `u16::MAX`; the occupant list remains live and is still consulted.
    pub(super) fn building_sector_is_authorized(
        &self,
        sector_number: crate::sector::SectorNumber,
    ) -> bool {
        let sector = self
            .grid_sector_by_number(sector_number)
            .unwrap_or_else(|| panic!("building door references missing sector {sector_number}"));
        let occupant_count = if let Some(building_index) = sector.building_index {
            self.script_domains
                .buildings
                .occupants
                .get(usize::from(building_index.get()))
                .unwrap_or_else(|| {
                    panic!(
                        "building sector {sector_number} references missing building {}",
                        building_index.get()
                    )
                })
                .len()
        } else {
            // TODO(original-parity): attach every door-authored building
            // sector to canonical BuildingState during level loading. A few
            // loaded sectors lack the attachment; count their live actors by
            // sector rather than fabricating an empty building.
            self.world
                .entities
                .actors()
                .filter(|(_, entity)| {
                    entity
                        .element_data()
                        .sector()
                        .is_some_and(|sector| u16::from(sector) == u16::from(sector_number))
                })
                .count()
        };
        occupant_count < usize::from(u16::MAX)
    }

    fn live_mobile_geometry(&self) -> LiveMobileGeometry {
        let mut prepared = LiveMobileGeometry {
            mobile_lines_by_layer: std::collections::BTreeMap::new(),
            mobile_points_by_layer: std::collections::BTreeMap::new(),
            mobile_polygons_by_layer: std::collections::BTreeMap::new(),
        };
        for mobile in &self.world.mobile_elements {
            if !mobile.active {
                continue;
            }
            prepared
                .mobile_lines_by_layer
                .entry(mobile.layer)
                .or_default()
                .extend(mobile.repulsive_lines());
            prepared
                .mobile_points_by_layer
                .entry(mobile.layer)
                .or_default()
                .extend(mobile.repulsive_points());
            prepared
                .mobile_polygons_by_layer
                .entry(mobile.layer)
                .or_default()
                .push(mobile.motion_polygon.clone());
        }
        prepared
    }

    #[cfg(test)]
    pub(super) fn first_live_mobile_polygon_point(
        &self,
        layer: u16,
    ) -> crate::coordinates::MapPoint {
        self.live_mobile_geometry()
            .mobile_polygons_by_layer
            .get(&layer)
            .and_then(|polygons| polygons.first())
            .and_then(|polygon| polygon.first())
            .copied()
            .unwrap_or_else(|| panic!("no live mobile polygon point on layer {layer}"))
    }

    /// Execute the Original's `RHNONANIMATION_RIDER_CHARGING` arm inside its
    /// rider's creation-ordered movement slot. Returns true only when the live
    /// selected movement order was exactly `RiderCharging` and consumed the
    /// slot; stale state is cleared for every other live-order shape.
    fn tick_rider_charge_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        rider_id: EntityId,
        frozen_all: bool,
        anti_context: Option<(
            &LiveMobileGeometry,
            &mut EntitySlots<Option<super::anti_collision::ActorSnapshot>>,
        )>,
    ) -> Option<RiderChargeExecution> {
        use crate::element::{ActionState, ActiveRiderCharge, Posture};
        use crate::weapons::SwordStrike;

        let provenance_frame = self.control.frame_counter;
        let selected = self
            .orders
            .sequence_manager
            .current_element_for_actor(rider_id);
        let live = selected.and_then(|(seq_id, elem_idx)| {
            let element = self.orders.sequence_manager.get_element(seq_id, elem_idx)?;
            if !element.data.is_movement() {
                return None;
            }
            Some((
                seq_id,
                elem_idx,
                element.current_order()?.clone(),
                element.next_order().cloned(),
                element.speed_factor(),
            ))
        });
        let is_live_charge = live
            .as_ref()
            .is_some_and(|(_, _, order, _, _)| order.order_type == OrderType::RiderCharging);
        if !is_live_charge {
            if let Some(entity) = self.world.entities.get_mut(rider_id)
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.active_rider_charge = None;
                actor.last_executed_rider_charge_order_id = None;
            }
            return None;
        }
        let (seq_id, elem_idx, order, next_order, speed_factor) = live.unwrap();

        let rider = self
            .world
            .entities
            .get(rider_id)
            .unwrap_or_else(|| panic!("live rider-charge owner {rider_id:?} disappeared"));
        let soldier = rider
            .soldier_data()
            .unwrap_or_else(|| panic!("RiderCharging owner {rider_id:?} is not a soldier"));
        assert!(
            soldier.rider,
            "RiderCharging owner {rider_id:?} is not a rider"
        );
        let weapon_profile_id =
            super::melee::get_hth_weapon_id_full(rider, &assets.profile_manager).unwrap_or_else(
                || panic!("rider {rider_id:?} has no hand-to-hand weapon profile id"),
            );
        assets
            .profile_manager
            .get_hth_weapon(weapon_profile_id)
            .unwrap_or_else(|| {
                panic!(
                    "rider {rider_id:?} references missing hand-to-hand weapon profile {weapon_profile_id}"
                )
            });
        let transition_frames = rider
            .sprite()
            .num_frames_for_anim(OrderType::TransitionCharging);
        assert!(
            rider.sprite().has_animation(OrderType::TransitionCharging) && transition_frames > 0,
            "rider {rider_id:?} is missing TransitionCharging animation"
        );

        // ExecuteRiderCharge samples these before Turn and PerformMotion on
        // every call. The same sample drives initialization and this frame's
        // narrow hit polygon.
        let (origin, sampled_layer, sampled_direction, forward, sidewards) = {
            let elem = rider.element_data();
            let direction = elem.direction();
            let [fx, fy] = crate::position_interface::sector_to_vector_iso(direction);
            let [sx, sy] = crate::position_interface::sector_to_vector_iso((direction + 4) & 15);
            (
                elem.position_map(),
                elem.layer(),
                direction,
                (fx, fy),
                (sx, sy),
            )
        };

        // RHElementActor's new-order identity is distinct from RHSprite's
        // processed-motion identity. FrozenAll executes this charge pass but
        // deliberately leaves sprite motion initialization pending.
        let needs_initialization = rider
            .actor_data()
            .expect("RiderCharging soldier must have actor data")
            .last_executed_rider_charge_order_id
            != Some(order.order_id);
        if needs_initialization {
            let initial_quad = [
                (
                    origin.x - 20.0 * forward.0 - 20.0 * sidewards.0,
                    origin.y - 20.0 * forward.1 - 20.0 * sidewards.1,
                ),
                (
                    origin.x + 180.0 * forward.0 - 20.0 * sidewards.0,
                    origin.y + 180.0 * forward.1 - 20.0 * sidewards.1,
                ),
                (
                    origin.x + 180.0 * forward.0 + 80.0 * sidewards.0,
                    origin.y + 180.0 * forward.1 + 80.0 * sidewards.1,
                ),
                (
                    origin.x - 20.0 * forward.0 + 80.0 * sidewards.0,
                    origin.y - 20.0 * forward.1 + 80.0 * sidewards.1,
                ),
            ];
            let obstacles = crate::sight_obstacle::ObstacleList {
                static_obstacles: assets.static_sight_obstacles.as_slice(),
                dynamic_obstacles: &self.world.dynamic_sight_obstacles,
                static_active: &self.world.static_sight_obstacle_active,
            };
            let mut pending_victims = Vec::new();
            for (victim_id, victim) in self.world.entities.humans() {
                let victim_id: EntityId = victim_id.into();
                if super::melee::is_possible_sword_strike_victim(
                    &self.world.entities,
                    rider_id,
                    victim,
                    victim_id,
                    &assets.profile_manager,
                    &self.world.fast_grid,
                    obstacles,
                ) && victim.element_data().layer() == sampled_layer
                    && rider_charge_point_in_quad(
                        victim.element_data().position_map(),
                        initial_quad,
                    )
                {
                    pending_victims.push(victim_id);
                }
            }
            let rider = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present during charge initialization");
            // ExecuteRiderCharge reuses RHElementActorHuman's serialized
            // mlistSwordStrikeVictims storage. A charge clears and refills
            // that list, removes each landed victim from it, but deliberately
            // leaves unhit candidates behind when the charge order ends. A
            // later lateral/circle strike can therefore inherit them. Keep
            // the active charge view and the human-owned serialized view in
            // lockstep instead of treating the charge candidates as private
            // transient state.
            rider
                .human_data_mut()
                .expect("RiderCharging soldier must have human data")
                .sword_sweep
                .victims = pending_victims.clone();
            let actor = rider
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data");
            actor.sweep_state = None;
            actor.active_rider_charge = Some(ActiveRiderCharge { pending_victims });
        }

        let goal = MapPoint::new(order.target_x, order.target_y);
        let next_destination_same_action = next_order
            .filter(|next| next.order_type == order.order_type)
            .map(|next| MapPoint::new(next.target_x, next.target_y));
        let motion_context = MotionOrderContext {
            order_id: order.order_id,
            destination: goal,
            reverse: order.reverse,
            tolerance: order.tolerance,
            directional_tolerance: false,
            compute_direction: order.compute_direction,
            next_destination_same_action,
            target_element: order.antagonist,
        };
        let (motion_state, actual_frame) = {
            let entity = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present before charge motion");
            let elem = entity.element_data_mut();
            // ExecuteRiderCharge calls Turn before PerformMotion. The first
            // Execute therefore turns toward the previously installed goal;
            // PerformMotion initializes this order and computes its new goal
            // only afterward.
            elem.sprite.position_iface.turn();
            let (mut state, distance) = if frozen_all {
                // FrozenAll short-circuits RHSprite::PerformMotion before it
                // changes row/frame/order state. ExecuteRiderCharge continues
                // around that call and uses the sprite's existing live frame.
                (MotionState::InProgress, 0.0)
            } else {
                elem.sprite.perform_motion(
                    sim,
                    Some(motion_context),
                    OrderType::TransitionCharging,
                    elem.direction() as u16,
                    FrameProgression::Default,
                    false,
                    MotionMethod::Run,
                    false,
                )
            };
            // PerformMotion initializes the new direction goal after the
            // caller's Turn, then applies the standard turning slowdown to
            // this frame's distance using that now-live direction/goal pair.
            let distance = scaled_motion_distance(
                distance,
                speed_factor,
                true,
                elem.sprite.position_iface.get_direction()
                    != elem.sprite.position_iface.get_direction_goal(),
            );
            if distance != 0.0 {
                let pre_position = elem.position_map();
                let increment = elem.sprite.position_iface.get_increment_map();
                let anti_on = elem.sprite.position_iface.is_anti_collision_on();
                let (dx_step, dy_step, recovered_from_deviation, rebuild_after_deviation) =
                    if let Some((prepared, anti_snapshots)) = anti_context.as_ref()
                        && anti_on
                        && let Some(mover_snapshot) = anti_snapshots
                            .get(rider_id)
                            .and_then(|slot| slot.as_ref())
                            .filter(|snapshot| snapshot.active)
                            .cloned()
                    {
                        let move_box = *elem.sprite.position_iface.get_move_box();
                        let half_diagonal = elem.sprite.position_iface.get_half_diagonal();
                        let was_deviated = elem.sprite.position_iface.is_deviated();
                        let mut anti_state = super::anti_collision::AntiCollisionState {
                            pi: &mut elem.sprite.position_iface,
                            move_box,
                            half_diagonal,
                            goal_map: goal,
                        };
                        let (dx_step, dy_step) = apply_prepared_anti_collision_step(
                            provenance_frame,
                            &mover_snapshot,
                            anti_snapshots,
                            &self.ai.global.repulsive_points,
                            prepared,
                            &self.world.fast_grid,
                            &mut anti_state,
                            increment.x,
                            increment.y,
                            distance,
                            anti_on,
                        );
                        (
                            dx_step,
                            dy_step,
                            was_deviated && !anti_state.pi.is_deviated(),
                            anti_state.pi.is_deviated() && anti_state.pi.blocked_count == 0,
                        )
                    } else {
                        (increment.x * distance, increment.y * distance, false, false)
                    };

                if elem.sprite.position_iface.is_blocked() {
                    // PerformMotion returns ABORTED before committing the
                    // requested step or refreshing its forecast.
                    state = MotionState::Aborted;
                } else {
                    if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
                        let raw = vector_to_sector_0_to_15(dx_step, dy_step);
                        elem.set_direction_goal(if order.reverse { raw ^ 8 } else { raw });
                    }
                    elem.set_position_map(MapPoint::new(
                        pre_position.x + dx_step,
                        pre_position.y + dy_step,
                    ));
                    if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
                        elem.sprite.position_iface.reset_increment_computed();
                        elem.sprite.position_iface.compute_increment_all(false);
                    } else if recovered_from_deviation {
                        elem.sprite.position_iface.reset_increment_computed();
                        elem.sprite.position_iface.compute_increment_all(true);
                    }
                    if elem
                        .sprite
                        .position_iface
                        .is_goal_reached(&self.world.fast_grid, None)
                    {
                        if !elem.sprite.position_iface.is_deviated()
                            && elem.sprite.position_iface.get_tolerance() == 0.0
                        {
                            elem.set_position_map(goal);
                        }
                        state = MotionState::Terminated;
                    }
                    let wait = elem
                        .sprite
                        .wait_time(elem.sprite.current_row, elem.sprite.current_frame);
                    elem.sprite
                        .position_iface
                        .update_forecasted_movement(distance, wait + 1);
                    elem.update_grid_cell();
                }

                if let Some((_, anti_snapshots)) = anti_context
                    && let Some(snapshot) = anti_snapshots
                        .get_mut(rider_id)
                        .and_then(|slot| slot.as_mut())
                {
                    sync_snapshot_after_committed_step(snapshot, pre_position, elem.position_map());
                }
            }
            elem.sprite.last_motion_state = Some(state);
            (state, elem.sprite.current_frame)
        };
        let last_frame = actual_frame == transition_frames - 1;
        if matches!(motion_state, MotionState::Start) {
            let entity = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present after charge motion");
            assert_eq!(
                entity.element_data().posture,
                Posture::Upright,
                "rider charge must start upright"
            );
            let actor = entity
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data");
            actor.action_state = ActionState::MovingFast;
            entity.element_data_mut().posture = Posture::Upright;
        }

        let back_length = (5.0 * f32::from(actual_frame)).min(50.0);
        let back = (-back_length * forward.0, -back_length * forward.1);
        let front = if last_frame { 15.0 } else { 0.0 };
        let hit_quad = [
            (origin.x + back.0, origin.y + back.1),
            (origin.x + front * forward.0, origin.y + front * forward.1),
            (
                origin.x + front * forward.0 + 60.0 * sidewards.0,
                origin.y + front * forward.1 + 60.0 * sidewards.1,
            ),
            (
                origin.x + back.0 + 60.0 * sidewards.0,
                origin.y + back.1 + 60.0 * sidewards.1,
            ),
        ];

        // Copy only the IDs to release the charge borrow. Each hit is removed
        // before launching its damage element, and queue_sword_damage
        // completes synchronously before the next candidate is inspected.
        let candidates = self.world.entities[rider_id]
            .as_ref()
            .expect("rider remained present before charge damage")
            .actor_data()
            .expect("RiderCharging soldier must have actor data")
            .active_rider_charge
            .as_ref()
            .expect("live RiderCharging order must retain active charge state")
            .pending_victims
            .clone();
        for victim_id in candidates {
            let Some(victim) = self.world.entities.get(victim_id) else {
                // Original pointers cannot become holes independently; Rust
                // entity removal can. Retain the pending ID so this is visible
                // state rather than silently fabricating a resolved hit.
                continue;
            };
            if victim.element_data().layer() != sampled_layer
                || !rider_charge_point_in_quad(victim.element_data().position_map(), hit_quad)
            {
                continue;
            }
            let rider = self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present while resolving charge hit");
            rider
                .human_data_mut()
                .expect("RiderCharging soldier must have human data")
                .sword_sweep
                .victims
                .retain(|pending| *pending != victim_id);
            rider
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data")
                .active_rider_charge
                .as_mut()
                .expect("active charge must remain installed through synchronous damage")
                .pending_victims
                .retain(|pending| *pending != victim_id);
            self.queue_sword_damage(
                sim,
                assets,
                victim_id,
                rider_id,
                SwordStrike::Charge,
                weapon_profile_id,
            );
        }

        let completion_order_id = if last_frame {
            // Rewrite only the same live order identity sampled above. Damage
            // can interrupt or replace it synchronously; never mutate a newer
            // order in that case.
            let still_same = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| element.current_order())
                .is_some_and(|current| {
                    current.order_type == OrderType::RiderCharging
                        && current.order_id == order.order_id
                });
            let rewritten_id = if still_same {
                let fresh_id = self.orders.allocate_order_id();
                let current = self
                    .orders
                    .sequence_manager
                    .get_element_mut(seq_id, elem_idx)
                    .and_then(|element| element.orders.front_mut())
                    .expect("validated rider charge order disappeared before rewrite");
                current.order_type = OrderType::RunningUpright;
                current.order_id = fresh_id;
                // Rider Execute mutates mpOrder->action and calls NewID on
                // the last charge frame; update the explicit pointer mirror
                // with that same in-place object mutation.
                self.world.entities[rider_id]
                    .as_mut()
                    .expect("rider disappeared before charge order publication")
                    .actor_data_mut()
                    .expect("RiderCharging soldier must have actor data")
                    .installed_order = Some(crate::element::InstalledActorOrder {
                    order_id: fresh_id,
                    order_type: OrderType::RunningUpright,
                });
                Some(fresh_id)
            } else {
                None
            };
            self.world.entities[rider_id]
                .as_mut()
                .expect("rider remained present at charge completion")
                .actor_data_mut()
                .expect("RiderCharging soldier must have actor data")
                .active_rider_charge = None;
            rewritten_id
        } else {
            self.orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| element.current_order())
                .filter(|current| {
                    current.order_type == OrderType::RiderCharging
                        && current.order_id == order.order_id
                })
                .map(|current| current.order_id)
        };

        // RHElementActor::Hourglass clears mbNewOrder after Execute even when
        // FrozenAll prevented RHSprite::PerformMotion from initializing. Stamp
        // only the actor-level identity. A synchronously installed fresh order
        // therefore still initializes on its next owner slot.
        self.world.entities[rider_id]
            .as_mut()
            .expect("rider remained present after charge execute")
            .actor_data_mut()
            .expect("RiderCharging soldier must have actor data")
            .last_executed_rider_charge_order_id = Some(order.order_id);

        tracing::trace!(
            ?rider_id,
            ?sampled_direction,
            actual_frame,
            last_frame,
            frozen_all,
            "executed rider charge in owner movement slot"
        );
        Some(RiderChargeExecution {
            completion_order_id,
        })
    }

    fn determine_lift_movement_animation(
        &self,
        owner: EntityId,
        posture_after: crate::element::Posture,
        action: OrderType,
        destination: MapPoint,
    ) -> OrderType {
        let Some(entity) = self.world.entities.get(owner) else {
            return action;
        };
        determine_lift_movement_animation_for(
            entity,
            &self.world.fast_grid,
            posture_after,
            action,
            destination,
        )
    }

    pub(crate) fn apply_sword_movement_start_initiative_transfer(&mut self, entity_id: EntityId) {
        let principal_id = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied());

        if let Some(entity) = self.world.entities.get_mut(entity_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_initiative = false;
        }

        let Some(principal_id) = principal_id else {
            return;
        };
        let is_mutual = self
            .expect_entity(principal_id, "sword-movement principal opponent")
            .human_data()
            .and_then(|h| h.opponents.first().copied())
            .map(|opp| opp == entity_id)
            .unwrap_or(false);
        if !is_mutual {
            return;
        }

        if let Some(entity) = self.world.entities.get_mut(principal_id)
            && let Some(human) = entity.human_data_mut()
        {
            human.smalltalk_initiative = true;
            human.received_smalltalk_initiative = true;
        }
    }

    pub(super) fn sword_movement_termination_warrants_provoke(
        &self,
        assets: &crate::engine::LevelAssets,
        entity_id: EntityId,
    ) -> bool {
        let principal_id = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .and_then(|h| h.opponents.first().copied());
        let Some(principal_id) = principal_id else {
            return false;
        };

        let is_mutual = self
            .expect_entity(principal_id, "sword-movement principal opponent")
            .human_data()
            .and_then(|h| h.opponents.first().copied())
            .map(|opp| opp == entity_id)
            .unwrap_or(false);
        if !is_mutual {
            return false;
        }

        let me = self.expect_entity(entity_id, "sword-movement provoke owner");
        let opponent = self.expect_entity(principal_id, "sword-movement principal opponent");
        let me_pos = me.element_data().position();
        let opponent_pos = opponent.element_data().position();
        let dx = me_pos.x - opponent_pos.x;
        let dy = me_pos.y - opponent_pos.y;
        let dz = me_pos.z - opponent_pos.z;
        let distance = (dx * dx + dy * dy + dz * dz).sqrt();

        let Some(me_weapon) =
            crate::engine::melee::get_hth_weapon_id_full(me, &assets.profile_manager)
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
        else {
            return false;
        };
        let Some(opponent_weapon) =
            crate::engine::melee::get_hth_weapon_id_full(opponent, &assets.profile_manager)
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
        else {
            return false;
        };

        let my_maximal = me_weapon.distance[crate::weapons::WeaponDistance::Maximal as usize];
        let my_uber = me_weapon.distance[crate::weapons::WeaponDistance::Uber as usize];
        let opponent_maximal =
            opponent_weapon.distance[crate::weapons::WeaponDistance::Maximal as usize];
        let opponent_uber = opponent_weapon.distance[crate::weapons::WeaponDistance::Uber as usize];
        tracing::trace!(
            ?entity_id,
            ?principal_id,
            distance,
            my_maximal,
            my_uber,
            opponent_maximal,
            opponent_uber,
            "checking sword-movement termination Provoke"
        );
        both_sword_ranges_contain_distance(
            distance,
            my_maximal,
            my_uber,
            opponent_maximal,
            opponent_uber,
        )
    }

    pub(super) fn launch_sword_movement_termination_provoke(&mut self, entity_id: EntityId) {
        self.launch_element(crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::Provoke,
            Some(entity_id),
        ));
    }

    /// Close the `StartPostSeekSequence` calls made from inside
    /// `RHElementActor::PerformSeek` before returning to the surrounding
    /// movement `Execute` arm.
    ///
    /// This ordering matters for sword movement. `PerformSeek` terminates the
    /// outgoing seek and registers its stored interaction first; only after it
    /// returns does `RHElementActorHuman::Execute` prune far opponents and
    /// potentially register `RHCOMMAND_PROVOKE` for the terminal movement.
    /// Keeping the registrations in that order lets the later Provoke queue
    /// behind an in-progress EnterSwordfight instead of being selected first
    /// and then interrupted by it.
    fn launch_perform_seek_arrivals(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        arrivals: Vec<(EntityId, crate::sequence::SequenceId, usize)>,
    ) -> Vec<EntityId> {
        let mut reentrant_order_advances = Vec::new();
        for (entity_id, seq_id, elem_idx) in arrivals {
            let launched =
                self.start_post_seek_sequence(sim, assets, entity_id, Some((seq_id, elem_idx)));
            if debug_post_seek_handoff_enabled() {
                eprintln!(
                    "[POST_SEEK frame={} owner={entity_id:?} stage=ordinary_launch launched={launched} current={:?}]",
                    self.control.frame_counter,
                    self.orders
                        .sequence_manager
                        .current_element_for_actor(entity_id),
                );
            }
            if launched {
                reentrant_order_advances.push(entity_id);
            }
        }
        // StartPostSeekSequence is synchronous inside PerformSeek. Settle the
        // launched interaction before the derived Human Execute tail resumes.
        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
            .unwrap_or_else(|error| {
                panic!("failed to drain synchronous post-seek work: {error:?}")
            });
        reentrant_order_advances
    }

    /// Mirror the Human Execute guard on the logical sword-movement
    /// non-animations. A stale sword move can still reach its owner slot after
    /// the actor's final opponent has gone away; unless the movement was
    /// explicitly forced, Original aborts that element and submits one
    /// QuitSwordfight command before it ever calls FaceOpponent/PerformMotion.
    fn abort_orphaned_sword_movement(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) -> bool {
        let should_abort = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .filter(|element| {
                element.owner == Some(owner)
                    && element.data.is_movement()
                    && !element.priority.is_non_interruptable()
            })
            .and_then(|element| {
                let order = element.current_order()?;
                if order.order_id != selected.order_id
                    || !matches!(
                        order.order_type,
                        OrderType::WalkingWithSword | OrderType::RunningWithSword
                    )
                {
                    return None;
                }
                let flags = match element.data {
                    crate::sequence::SequenceElementData::Movement { flags, .. } => flags,
                    _ => unreachable!("movement element changed data kind during sword guard"),
                };
                Some(!flags.contains(crate::sequence::MoveFlags::FORCE_SWORD_MOVEMENT))
            })
            .unwrap_or(false)
            && self
                .world
                .entities
                .get(owner)
                .and_then(|entity| entity.human_data())
                .is_some_and(|human| human.opponents.is_empty());
        if !should_abort {
            return false;
        }

        // Human::Execute calls the selected movement element's virtual
        // Stop(Injury) before registering QuitSwordfight. That exact-root
        // stop follows only the element's linked successor/postponed graph;
        // in particular, a FaceTo Turn queued by EventReachPoint is
        // interrupted and sends its condolence card on this stack. Do not use
        // Actor::Stop here: its pending-list scan would also stop unrelated
        // work, and retiring the movement before this boundary would release
        // the Turn to be inherited by QuitSwordfight.
        let selected_priority = {
            let resolver = Self::priority_resolver(&self.world.entities);
            let element = self
                .orders
                .sequence_manager
                .get_element_mut(selected.seq_id, selected.elem_idx)
                .expect("selected orphan sword movement disappeared before Stop");
            if element.priority == crate::sequence::SequencePriority::NotYetSet {
                let mut resolved = resolver(element);
                if resolved == crate::sequence::SequencePriority::None {
                    resolved = crate::sequence::SequencePriority::Normal;
                }
                element.priority = resolved;
            }
            element.priority
        };
        if selected_priority >= crate::sequence::SequencePriority::Injury {
            let owner_pos = self
                .get_entity(owner)
                .expect("orphan sword movement owner disappeared before Stop")
                .element_data()
                .position_map();
            {
                let resolver = Self::priority_resolver(&self.world.entities);
                let pathfinder = &mut self.world.pathfinder;
                self.orders.sequence_manager.stop_movement_from_root(
                    owner,
                    (selected.seq_id, selected.elem_idx),
                    owner_pos,
                    crate::sequence::SequencePriority::Injury,
                    &resolver,
                    &mut self.orders.next_order_id,
                    &mut |id| pathfinder.cancel_requests_for(id),
                );
            }
            // Movement::Stop delivers the selected movement's card from
            // StopMovement before its base SequenceElement::Stop walks the
            // linked successor/postponed graph.  That callback may re-enter
            // AI and mutate the graph, so it is a real owner boundary rather
            // than a batchable cleanup detail.
            self.dispatch_condolations_for_owner_boundary(sim, owner, assets);
            {
                let resolver = Self::priority_resolver(&self.world.entities);
                self.orders.sequence_manager.stop_owner_current_from_root(
                    owner,
                    Some((selected.seq_id, selected.elem_idx)),
                    crate::sequence::SequencePriority::Injury,
                    &resolver,
                );
            }
        }
        self.dispatch_condolations_for_owner_boundary(sim, owner, assets);
        // Human::Execute only registers this command here. Its ABORTED return
        // reaches Actor::Hourglass first; SequenceManager::Hourglass later
        // calls the ordinary Actor::Instruct path, which translates the
        // lowering order and overwrites mmotionState with IN_PROGRESS. Direct
        // prebuilt-order instruction at this Execute boundary left the later
        // ABORTED latch authoritative for the whole frame.
        self.launch_element(crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::QuitSwordfight,
            Some(owner),
        ));

        // Original announces EVENT_QUIT_SWORDFIGHT from this guard in the
        // same call, before the Execute arm returns ABORTED — so the soldier's
        // brain has already left its swordfight substate when any later phase
        // of this frame runs. Only soldiers have a receiver here.
        tracing::trace!(
            owner = owner.index(),
            frame = self.control.frame_counter,
            "orphaned sword movement aborted; sending EVENT_QUIT_SWORDFIGHT"
        );
        if matches!(self.world.entities.get(owner), Some(Entity::Soldier(_))) {
            self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                sim,
                owner,
                assets,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventQuitSwordfight),
            );
        }

        // Actor::Hourglass captures the entry movement before Execute. Only
        // after Human::Execute has returned ABORTED does it mark that captured
        // element Impossible. Keep this after the direct soldier callback so
        // neither QuitSwordfight nor the callback can inherit the stopped
        // Turn's cross-element link.
        self.orders
            .sequence_manager
            .element_impossible(selected.seq_id, selected.elem_idx);
        self.dispatch_condolations_for_owner_boundary(sim, owner, assets);
        true
    }

    /// Resolve the point `RHElementActorHuman::FaceOpponent`
    /// (RHelementactorhuman.cpp:7480) or `RHElementActorPC::FaceDangerPoint`
    /// would face for this owner, plus whether that point is compared against
    /// the actor's ground position rather than its map position.
    ///
    /// `None` reproduces FaceOpponent's non-soldier, non-swordfighting early
    /// return, which yields `WALKING_SWORD` without touching the facing.
    pub(super) fn combat_face_target_for_owner(&self, owner: EntityId) -> (Option<MapPoint>, bool) {
        let mut combat_face_target = None;
        let mut combat_face_target_is_ground = false;
        for (_actor_id, entity) in self
            .world
            .entities
            .actors()
            .filter(|(id, _)| EntityId::from(*id) == owner)
        {
            let actor = entity
                .actor_data()
                .expect("entities.actors() yielded non-actor entity");
            let is_shield_moving = matches!(
                actor.action_state,
                crate::element::ActionState::MovingShield
            );
            // Shield bearers face the stored danger point.
            // Sword fighters face their principal opponent.
            if is_shield_moving && let Some(pt) = actor.shield_face_point {
                combat_face_target = Some(pt);
                continue;
            }
            // Shield bearer with no danger point stored: face *away*
            // from the protected ally.  Encode this as a target equal
            // to `2 * self_pos - ally_pos` so the downstream
            // `vector_to_sector_0_to_15(target - self)` math aims the
            // shield-bearer away from the ally.
            if is_shield_moving
                && let Some(protected_id) = entity.pc_data().and_then(|pc| pc.shield_protected)
                && let Some(ally) = self.world.entities.get(protected_id)
            {
                let self_pos = entity.element_data().position_map();
                let ally_pos = ally.element_data().position_map();
                combat_face_target = Some(crate::coordinates::MapPoint {
                    x: 2.0 * self_pos.x - ally_pos.x,
                    y: 2.0 * self_pos.y - ally_pos.y,
                });
                continue;
            }
            // FaceOpponent dispatch for sword movement:
            //   swordfighting → principal opponent's ground position
            //   else if soldier → primary target's ground position
            //   else            → return WALKING_SWORD without facing change
            //
            // Build this even before `action_state` flips to MovingSword;
            // forced sword movement can still be represented only by the
            // movement element's FORCE_SWORD_MOVEMENT flag at this point.
            //
            // The non-soldier, non-swordfighting branch returns
            // `WALKING_SWORD` immediately, without constructing a facing
            // vector. Keep that distinct as `None`: using the actor's own
            // position as a sentinel is not equivalent because Position and
            // PositionGround can differ while cached projection state is
            // refreshed, turning a nominally-zero vector into a small real
            // angle and selecting a strafe row.
            let is_swordfighting = entity
                .human_data()
                .map(|human| !human.opponents.is_empty())
                .unwrap_or(false);
            let opp_id_opt: Option<EntityId> = if is_swordfighting {
                // Principal opponent = first in opponent list.
                entity
                    .human_data()
                    .and_then(|h| h.opponents.first())
                    .copied()
            } else if entity.is_soldier() {
                // GetPrimaryTarget — soldier's AI-picked priority target,
                // which can differ from opponents[0]. The stored handle is a
                // raw element slot and the occupant is any human, not just a
                // PC: soldiers routinely keep an enemy soldier as their
                // primary target once a swordfight has ended, and facing it
                // is what keeps the fighter turned toward the melee.
                entity
                    .ai_controller()
                    .map(|c| c.primary_target)
                    .filter(|slot| *slot != 0)
                    .and_then(|slot| self.world.entities.id_at_legacy_slot(slot))
            } else {
                None
            };

            if let Some(opp_id) = opp_id_opt
                && let Some(opp) = self.world.entities.get(opp_id)
            {
                let position = opp.element_data().position();
                combat_face_target =
                    Some(crate::coordinates::MapPoint::new(position.x, position.y));
                combat_face_target_is_ground = true;
            }
        }
        (combat_face_target, combat_face_target_is_ground)
    }

    /// Run the sword-/shield-walking Execute arm's facing prologue for a frame
    /// whose `PerformSeek` is about to take its moved-target `RefreshSeek`
    /// branch.
    ///
    /// `RHElementActorHuman::Execute` calls `FaceOpponent` at
    /// RHelementactorhuman.cpp:3662 and `RHElementActorPC::Execute` calls
    /// `FaceDangerPoint` at RHelementactorpc.cpp:5514, both *before* entering
    /// `RHElementActor::PerformSeek` (RHelementactorhuman.cpp:3667,
    /// RHelementactorpc.cpp:5541).  PerformSeek's moved-target branch
    /// (RHelementactor.cpp:7913) returns `RHMOTION_IN_PROGRESS` without ever
    /// reaching `PerformMotion`, so `FaceOpponent`'s
    /// `SetDirection( vDirection.GetSector0to15( ASPECT_RATIO ) )` and its
    /// following `Turn()` (RHelementactorhuman.cpp:7511-7512) still land on the
    /// RefreshSeek frame.  Rust evaluates RefreshSeek ahead of the movement
    /// Execute arm, so without this the goal keeps its previous value for one
    /// frame.
    pub(super) fn apply_pre_perform_seek_facing_prologue(&mut self, owner: EntityId) {
        let Some(entity) = self.world.entities.get(owner) else {
            return;
        };
        let Some(actor) = entity.actor_data() else {
            return;
        };
        if actor.execution_frozen {
            return;
        }
        let action_state = actor.action_state;
        let door_pass_anim: Option<OrderType> =
            actor.active_door_pass.as_ref().map(|dp| dp.current_action);
        let Some(seq_id) = actor.active_movement.sequence_id else {
            return;
        };
        let elem_idx = actor.active_movement.element_index;
        let Some(order_action) = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| element.orders.front())
            .map(|order| order.order_type)
        else {
            return;
        };
        let is_shield_motion = matches!(action_state, crate::element::ActionState::MovingShield);
        let is_sword_motion = is_sword_motion_context(action_state, door_pass_anim, order_action);
        let (combat_target, target_is_ground) = self.combat_face_target_for_owner(owner);
        // Human's arm always calls FaceOpponent; the PC shield arm always
        // calls FaceDangerPoint. Both return without writing when no facing
        // point exists, which is exactly `combat_target == None`.
        if !((is_shield_motion && combat_target.is_some()) || is_sword_motion) {
            return;
        }
        let Some(opp_pos) = combat_target else {
            return;
        };
        let entity = self
            .world
            .entities
            .get_mut(owner)
            .expect("facing-prologue owner disappeared between borrows");
        let elem = entity.element_data_mut();
        let face_origin = if target_is_ground {
            let position = elem.position();
            crate::coordinates::MapPoint::new(position.x, position.y)
        } else {
            elem.position_map()
        };
        let fdx = opp_pos.x - face_origin.x;
        let fdy = opp_pos.y - face_origin.y;
        elem.set_direction_goal(crate::position_interface::vector_to_sector_0_to_15_iso(
            fdx, fdy,
        ));
        let _ = elem.sprite.position_iface.turn();
    }

    pub(super) fn tick_entity_movement_owner(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        owner: EntityId,
        selected: Option<MovementOwnerSelection>,
    ) -> MovementOwnerMotion {
        let Some(selected) = selected else {
            return MovementOwnerMotion::default();
        };
        let selected_is_live = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .filter(|element| element.owner == Some(owner) && element.data.is_movement())
            .and_then(|element| element.current_order())
            .is_some_and(|order| order.order_id == selected.order_id);
        if !selected_is_live {
            return MovementOwnerMotion::default();
        }
        if let Some(entity) = self.world.entities.get(owner) {
            super::animation::direction_provenance_snapshot(
                entity.position_iface(),
                owner,
                self.control.frame_counter,
                "movement_entry",
            );
        }
        if self
            .world
            .entities
            .get(owner)
            .and_then(|entity| entity.actor_data())
            .is_some_and(|actor| actor.execution_frozen)
        {
            return MovementOwnerMotion::default();
        }
        let selected_command = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(seq_id, elem_idx)| {
                self.orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|element| element.command)
            });
        if matches!(
            selected_command,
            Some(crate::element::Command::WaitTimer | crate::element::Command::WaitFreeLift)
        ) {
            return MovementOwnerMotion::default();
        }
        if self.abort_orphaned_sword_movement(sim, assets, owner, selected) {
            // LaunchSequenceElement registers the replacement for the later
            // sequence-manager phase. It must not execute at this actor
            // boundary: Original exposes QuitSwordfight as the current
            // command for one frame before its lowering order starts.
            return MovementOwnerMotion::default();
        }
        // `IsFrozenAll()` is read ONLY inside `RHSprite`
        // (`original-code/RHsprite.cpp:739`, `:985`, `:1042`, `:1084`, `:1124`,
        // `:1430`) and in the NPC AI gates; `RHElementActor::Execute` itself is
        // never gated on it. Its `RHNONANIMATION_PASSING_DOOR` arm
        // (`original-code/RHelementactor.cpp:2786-2807`) reaches no Sprite
        // method at all: it calls `PassDoor()` / restores anti-collision,
        // forwards `MSG_STATURE`, and returns `RHMOTION_TERMINATED` whatever
        // the global freeze is. A `FreezeAll(true)` landing on the frame that
        // owns the door action point must therefore not defer it — doing so
        // held the pass open one extra frame and delayed every successor
        // (AI unlock, the route's own WaitTimer, the next path request).
        let selected_is_door_pass_action_point = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| element.current_order())
            .is_some_and(|order| {
                order.order_id == selected.order_id && order.order_type == OrderType::PassingDoor
            });
        if self.actors_frozen() && !selected_is_door_pass_action_point {
            // A globally frozen Sprite::PerformMotion returns IN_PROGRESS
            // before touching any row/frame state, and that is what the
            // movement Execute arm hands back to the actor. Latch it here:
            // otherwise the arm runs without a sprite call and the actor
            // keeps re-reporting whatever edge the last unfrozen frame left
            // behind, typically a stale START.
            if let Some(entity) = self.world.entities.get_mut(owner) {
                entity.element_data_mut().sprite.last_motion_state =
                    Some(crate::sprite::MotionState::InProgress);
            }
            let frozen_order =
                self.execute_globally_frozen_pre_motion_owner(sim, assets, owner, selected);
            // RunningUpright is exceptional among the ordinary movement
            // Execute arms: it calls SetStates(MOVING_FAST) unconditionally
            // after PerformMotion, not only for RHMOTION_START. Therefore the
            // IN_PROGRESS returned by globally frozen sprite motion still
            // changes a waiting actor to moving-fast.
            if frozen_order == OrderType::RunningUpright {
                let entity = self
                    .world
                    .entities
                    .get_mut(owner)
                    .unwrap_or_else(|| panic!("globally frozen runner {owner:?} disappeared"));
                entity.element_data_mut().posture = crate::element::Posture::Upright;
                entity
                    .actor_data_mut()
                    .expect("globally frozen runner is not an actor")
                    .action_state = crate::element::ActionState::MovingFast;
            }
            if frozen_order == OrderType::WalkingWithShield {
                let entity = self.world.entities.get_mut(owner).unwrap_or_else(|| {
                    panic!("globally frozen shield walker {owner:?} disappeared")
                });
                let (posture, action_state) = movement_execute_state_effect(
                    frozen_order,
                    crate::sprite::MotionState::InProgress,
                )
                .expect("WalkingWithShield must own an unconditional Execute state effect");
                entity.set_posture(posture);
                entity
                    .actor_data_mut()
                    .expect("globally frozen shield walker is not an actor")
                    .action_state = action_state;
                refresh_pc_walking_shield_after_execute(
                    entity,
                    &assets.profile_manager,
                    frozen_order,
                );
            }
            // FrozenAll suppresses Sprite::PerformMotion but not the Execute
            // work before it: climb Turn() above and both rider-specific
            // Soldier arms remain live. RiderCharging performs its polygon
            // work or RunningUpright samples that frozen frame and may Think.
            let charge_execution = self.tick_rider_charge_owner(sim, assets, owner, true, None);
            if charge_execution.is_none() && self.selected_galopp_decision_frame(owner, selected) {
                self.dispatch_galopp_loop_event(sim, assets, owner);
            }
            return MovementOwnerMotion::default();
        }

        // Sample mutable mobile geometry only now, at this actor's Original
        // entity slot. Preparing it once before the live owner walk freezes
        // every actor onto the same side of intervening mobile masters.
        let prepared = self.live_mobile_geometry();

        // Pre-pass: collect principal opponent positions for
        // combat-moving entities.  During sword/shield movement,
        // FaceOpponent / FaceDangerPoint overrides the entity's facing
        // direction toward their opponent instead of the movement
        // direction, and selects directional animations
        // (forward/backward/strafe) based on the angle between
        // movement and facing.
        let (combat_face_target, combat_face_target_is_ground) =
            self.combat_face_target_for_owner(owner);

        // Pre-pass: look up the current sequence-element speed factor
        // for every entity with an active movement
        // (`distance *= speed_factor` during the per-frame motion
        // update). Pre-computed here so the main loop can borrow
        // `self.world.entities` mutably while consulting
        // `self.orders.sequence_manager` for the factor.
        let mut speed_factor = 1.0;
        // `RHSprite::PerformMotion` passes its cached `mpTargetElement` into
        // `RHPositionInterface::IsGoalReached`.  The target radius matters
        // when anti-collision left the actor deviated and blocked: Original
        // accepts center separation below `self.radius + target.radius + 10`
        // even when the ordinary zero tolerance has not crossed the waypoint.
        // Snapshot it before the mutable movement pass for the same reason as
        // the speed factor and live-seek metadata below.
        let mut goal_target_info = None;
        // Per-entity final-waypoint tolerance snapshot for the arrival
        // check.  The seek-arrival predicate is:
        //
        //   target.sector == self.sector                     (same-sector)
        //   && dist_sq < seek_distance^2 * 1.1025            (5% margin)
        //
        // where `dist` is the vector from the actor to the target's
        // current position (or its current-row hotspot under
        // `USE_POINT`), with Y stretched by the inverse aspect ratio
        // when `DIRECTIONAL_TOLERANCE` is set.
        //
        // `target_is_actor` lets the main loop set the shield-follower
        // speed factor: when self is in `MovingShield` action state
        // and the seek target is an actor, the speed factor becomes
        // 1.0 / 1.5 / 2.0 depending on range.
        let mut final_tolerance = FinalTol::default();
        let mut point_seek_post_sector = None;
        for (_actor_id, entity) in self
            .world
            .entities
            .actors()
            .filter(|(id, _)| EntityId::from(*id) == owner)
        {
            let Some(actor) = entity.actor_data() else {
                continue;
            };
            let (seq_id, elem_idx) = (selected.seq_id, selected.elem_idx);
            if let Some(elem) = self.orders.sequence_manager.get_element(seq_id, elem_idx) {
                speed_factor = elem.speed_factor();
                goal_target_info = elem
                    .current_order()
                    .and_then(|order| order.antagonist)
                    .and_then(|target| self.world.entities.get(target))
                    .map(|target| crate::position_interface::TargetInfo {
                        radius: target.position_iface().get_radius(),
                    });
                if let crate::sequence::SequenceElementData::Movement {
                    flags,
                    tolerance: _,
                    element: target_elem,
                    destination,
                    ..
                } = &elem.data
                    && flags.contains(crate::sequence::MoveFlags::SEEK)
                {
                    // The per-tick seek-arrival predicate (and its
                    // FROZEN-wait sibling) is a SEEK-only
                    // mechanism.  Non-seek `GoNear`-style
                    // stop-distances are enforced earlier, by
                    // `insert_transition_end` adding the element's
                    // tolerance to the end-transition shift
                    // (`distance_remaining + tolerance`), which
                    // truncates the walking phase before order
                    // emission.  Gating here keeps the FinalTol
                    // snapshot meaningful only for true seeks, so
                    // the downstream tolerance-arrival check can
                    // rely on the creation-slot target observation /
                    // `shield_destination` being live. Gate-approach
                    // legs of an entity seek deliberately carry zero
                    // tolerance, but remain PerformSeek owners and must
                    // still age the shared refresh countdown.
                    let directional =
                        flags.contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE);
                    let use_point = flags.contains(crate::sequence::MoveFlags::USE_POINT);
                    let seek_shield = flags.contains(crate::sequence::MoveFlags::SEEK_SHIELD);
                    let (resolved_target_id, target_is_actor) =
                        match target_elem.and_then(|id| self.get_entity(id)) {
                            Some(t) => (*target_elem, t.actor_data().is_some()),
                            // SEEK without antagonist = seek-to-point
                            // mode.  Skip the dist-vs-tolerance
                            // check; arrival is detected by motion
                            // termination + same-sector match.
                            // Falls through to the standard
                            // `dist <= speed` final-waypoint
                            // arrival when there is no post-seek
                            // sequence. Leaving target_id None
                            // signals the consumer to skip the
                            // entity-target seek-distance check.
                            None => {
                                if actor.post_seek_sequence.is_some()
                                    && actor.continuation.seek_to_point
                                {
                                    point_seek_post_sector = match actor.continuation.seek_sector {
                                        Some(crate::actor_state::ActorSeekSector::Position(
                                            sector,
                                        )) => Some(sector),
                                        // Runtime point Seek stores a
                                        // Position sector. A legacy Door
                                        // pointer cannot equal the
                                        // actor's ordinary sector here,
                                        // matching Original's pointer
                                        // comparison in PerformSeek.
                                        Some(crate::actor_state::ActorSeekSector::Door(_))
                                        | None => None,
                                    };
                                }
                                (None, false)
                            }
                        };
                    // Skip the FinalTol snapshot entirely for
                    // seek-to-point + non-shield (target_id is
                    // None and there's no shield destination), so
                    // the seek-arrival predicate doesn't fire.
                    if resolved_target_id.is_some() || seek_shield {
                        // Original keeps an interaction following an
                        // entity Seek in `mpPostSeekSequence`. Gate-route
                        // construction in Rust currently represents that
                        // continuation as later elements of the same
                        // sequence. Treat either representation as a live
                        // post-seek handoff: PerformSeek returns before
                        // aging `mulWaitTime` when tolerance is reached.
                        let has_post_seek = actor.post_seek_sequence.is_some()
                            || self
                                .orders
                                .sequence_manager
                                .get_sequence(seq_id)
                                .is_some_and(|sequence| elem_idx + 1 < sequence.elements.len());
                        final_tolerance = FinalTol {
                            // `mfSeekDistance` remains the unadapted
                            // interaction radius.  RefreshSeek may halve
                            // the concrete movement element's tolerance
                            // while chasing a moving target, but
                            // PerformSeek never uses that adapted path
                            // tolerance for its live-target arrival test.
                            tol: actor.seek_distance,
                            directional,
                            target_is_actor,
                            target_id: resolved_target_id,
                            use_point,
                            shield_destination: seek_shield.then_some(*destination),
                            last_seek_target_position: actor.last_seek_target_position,
                            has_post_seek,
                            launches_post_seek: actor.post_seek_sequence.is_some(),
                        };
                    }
                }
            }
        }

        // Pre-pass: drive the per-tick `TurnDrunken()` turn for every
        // drunken soldier on an ordinary movement. `TurnDrunken()` picks
        // between `TurnSlow(2)` and `TurnVerySlow()` (delay 5) so the
        // soldier's facing lags behind the movement vector.  This
        // must run before the main loop because the per-tick turn
        // advances `position_iface` (a mutable borrow that would
        // conflict with `entity.element_data_mut()`).
        //
        // The soldier Execute branches call PerformSeek directly when
        // RHMOVE_SEEK is set; they do not call TurnDrunken first. PerformSeek
        // owns its normal Turn call, so adding this pre-pass there turns the
        // actor twice in one frame.
        let selected_uses_seek = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .is_some_and(|element| {
                matches!(
                    &element.data,
                    crate::sequence::SequenceElementData::Movement { flags, .. }
                        if flags.contains(crate::sequence::MoveFlags::SEEK)
                )
            });
        for (_, soldier) in self
            .world
            .entities
            .soldiers_mut()
            .filter(|(id, _)| EntityId::from(*id) == owner)
        {
            let is_drunk = soldier
                .npc
                .ai_brain
                .base()
                .map(|b| b.blood_alcohol > 0)
                .unwrap_or(false);
            if !is_drunk {
                continue;
            }
            // Compute the movement goal vector.  Skip entities without
            // an active movement path — idle drunk soldiers don't
            // wobble.  Goal is read from the actor's Move element's
            // current order (authoritative path source).
            let actor = &soldier.actor;
            let Some(_) = actor.active_movement.sequence_id else {
                continue;
            };
            let Some(order) = self
                .orders
                .sequence_manager
                .get_element(selected.seq_id, selected.elem_idx)
                .and_then(|element| element.current_order())
                .filter(|order| order.order_id == selected.order_id)
            else {
                continue;
            };
            if !should_apply_drunken_turn(selected_uses_seek, order.order_type) {
                continue;
            }
            turn_drunken(&mut soldier.element.sprite.position_iface);
        }

        // Pre-pass: per-entity current-sector lift translation, for
        // the lift branches of the movement-animation derivation.
        // When a moving actor is in a lift sector, the per-frame
        // walk/run animation is overridden by the lift's upwards /
        // downwards action mapping:
        //   * Upright posture: lift type rewrites the action; upwards
        //     and downwards animations are equal for upright, so we
        //     always use the upwards mapping.
        //   * OnLadder / OnWall: pick upwards vs downwards by
        //     dot-producting the ladder vector (`pt_low - pt_high`)
        //     with the movement vector — non-negative means moving
        //     down.  The high / low exit points are the in-side
        //     points of the lift's highest and lowest doors.
        //
        // Pre-computed here so the main loop can borrow `self.world.entities`
        // mutably without touching `self.world.fast_grid` or the door table.
        let mut lift_translation = None;
        let mut door_pass_climb_direction = None;
        let mut decorative_building_trap_at_destination = false;
        for (actor_id, entity) in self
            .world
            .entities
            .actors()
            .filter(|(id, _)| EntityId::from(*id) == owner)
        {
            let posture = entity.element_data().posture;
            let door_pass = entity
                .actor_data()
                .and_then(|actor| actor.active_door_pass.as_ref());
            let door_pass_action = door_pass.map(|dp| dp.current_action);
            let Some(sector) = entity.element_data().sector() else {
                continue;
            };
            let Some(gs) = grid_sector_for_position_handle(&self.world.fast_grid.level, sector)
            else {
                continue;
            };
            if let Some(action) = door_pass_action
                && let Some(expected) = climb_lift_type(action)
            {
                door_pass_climb_direction = entity
                    .actor_data()
                    .and_then(|actor| actor.active_door_pass.as_ref())
                    .and_then(|dp| {
                        let door = self
                            .script_domains
                            .interactables
                            .doors
                            .get(usize::from(dp.door_index))
                            .unwrap_or_else(|| {
                                panic!(
                                    "door-pass climb owner {actor_id:?} references missing door {}",
                                    dp.door_index
                                )
                            });
                        door_type_uses_lift_climb_direction(door.door_type)
                            .then(|| {
                                crate::position_interface::SectorHandle::new(u16::from(
                                    door.sector_in,
                                ))
                                .map(|handle| {
                                    door.sector_in_index
                                        .map_or(handle, |index| handle.with_arena_index(index))
                                })
                            })
                            .flatten()
                    })
                    .map(|sector_in| {
                        grid_sector_for_position_handle(&self.world.fast_grid.level, sector_in)
                        .unwrap_or_else(|| {
                            panic!(
                                "door-pass climb owner {actor_id:?} references missing lift sector {sector_in}"
                            )
                        })
                    })
                    .map(|sector| {
                        assert_eq!(
                            sector.lift_type,
                            Some(expected),
                            "door-pass climb owner {actor_id:?} action {action:?} requires {expected:?}, found {:?}",
                            sector.lift_type
                        );
                        sector.lift_direction
                    });
                if action == OrderType::ClimbingLadderDown
                    && door_pass.is_some_and(|pass| {
                        pass.current_reverse
                            && self
                                .script_domains
                                .interactables
                                .doors
                                .get(usize::from(pass.door_index))
                                .is_some_and(|door| {
                                    door.door_type == crate::gate::DoorType::BuildingTrap
                                })
                    })
                    && self
                        .orders
                        .sequence_manager
                        .get_element(selected.seq_id, selected.elem_idx)
                        .and_then(|element| element.current_order())
                        .filter(|order| order.order_id == selected.order_id)
                        .is_some_and(|order| {
                            entity.element_data().position_map()
                                == MapPoint::new(order.target_x, order.target_y)
                        })
                {
                    // TODO(parity): Original's decorative BuildingTrap row
                    // invalidly casts its RHSectorBuilding to RHSectorLift.
                    // The three shipped witnesses read direction zero. Keep
                    // that release-build compatibility value confined to the
                    // exact-target reverse row which immediately terminates.
                    door_pass_climb_direction = Some(0);
                    decorative_building_trap_at_destination = true;
                }
            }
            let Some(lt) = gs.lift_type else { continue };
            match posture {
                crate::element::Posture::Upright => {
                    lift_translation = Some(LiftAnimContext::Upright(lt));
                }
                crate::element::Posture::OnLadder | crate::element::Posture::OnWall
                    if matches!(
                        (posture, lt, door_pass_action),
                        (
                            crate::element::Posture::OnWall,
                            crate::sector::LiftType::Wall,
                            _
                        ) | (
                            crate::element::Posture::OnLadder,
                            crate::sector::LiftType::Ladder,
                            _
                        )
                    ) =>
                {
                    let (pt_low, pt_high) = lift_endpoint_points_for_sector(gs);
                    let ladder_dx = pt_low.x - pt_high.x;
                    let ladder_dy = pt_low.y - pt_high.y;
                    lift_translation = Some(LiftAnimContext::OnClimb {
                        lift_type: lt,
                        lift_direction: gs.lift_direction,
                        ladder_dx,
                        ladder_dy,
                    });
                }
                _ => {}
            }
            if lift_translation.is_none()
                && matches!(
                    (lt, door_pass_action),
                    (
                        crate::sector::LiftType::Wall,
                        Some(
                            OrderType::ClimbingWallUp
                                | OrderType::ClimbingWallDown
                                | OrderType::ClimbingWallUpFast
                                | OrderType::ClimbingWallDownFast
                        )
                    ) | (
                        crate::sector::LiftType::Ladder,
                        Some(
                            OrderType::ClimbingLadderUp
                                | OrderType::ClimbingLadderDown
                                | OrderType::ClimbingLadderUpFast
                                | OrderType::ClimbingLadderDownFast
                        )
                    )
                )
            {
                let (pt_low, pt_high) = lift_endpoint_points_for_sector(gs);
                lift_translation = Some(LiftAnimContext::OnClimb {
                    lift_type: lt,
                    lift_direction: gs.lift_direction,
                    ladder_dx: pt_low.x - pt_high.x,
                    ladder_dy: pt_low.y - pt_high.y,
                });
            }
        }

        // Pre-pass: snapshot every actor's position / layer / sector /
        // posture / repulsive-point contribution for the
        // anti-collision disturbing-actor lookup.  Captured once per
        // tick so the mutable main loop can read neighbour state
        // without a second borrow, matching the deterministic
        // start-of-tick view the replay system relies on.
        // Mutable — each entity's post-move position is written back
        // so later entities in the same tick see the serial
        // "already-moved" view: each actor's anti-collision lookup
        // reads live positions from earlier-processed actors.
        let mut anti_snapshots =
            super::anti_collision::snapshot_all(&self.world.entities, &assets.profile_manager);

        let prepass = MovementPrepass {
            combat_face_target,
            combat_face_target_is_ground,
            speed_factor,
            goal_target_info,
            final_tolerance,
            point_seek_post_sector,
            lift_translation,
            door_pass_climb_direction,
            decorative_building_trap_at_destination,
        };
        if let Some(entity) = self.world.entities.get(owner) {
            super::animation::direction_provenance_snapshot(
                entity.position_iface(),
                owner,
                self.control.frame_counter,
                "movement_after_prepass",
            );
        }
        let mut deferred = MovementDeferred::default();

        // Iterate a stable creation-order ID list instead of holding one
        // mutable iterator borrow across the whole pass. This lets each actor
        // sample its SEEK target directly from the entity table immediately
        // before its own movement. Mutations by an earlier-created actor are
        // therefore visible, while a later-created target still exposes its
        // pre-movement state, matching RHEngine's virtual Hourglass loop.
        let movement_actor_ids: Vec<_> = self
            .world
            .entities
            .actors()
            .filter(|(id, _)| EntityId::from(*id) == owner)
            .map(|(id, _)| id)
            .collect();
        for actor_id in movement_actor_ids {
            self.tick_one_movement_actor(
                sim,
                assets,
                owner,
                selected,
                actor_id,
                &prepass,
                &prepared,
                &mut anti_snapshots,
                &mut deferred,
            );
        }
        let MovementDeferred {
            post_completion_motion_override,
            sword_movement_starts,
            sword_movement_terminations,
            door_triggers,
            transition_pushes,
            select_triggers,
            completed_door_passes,
            galopp_event,
            blocked_impossible,
            door_pass_transition_start_effects,
            door_pass_transition_done_effects,
            door_pass_transition_completion_effects,
            terminal_door_pass_goal_clears,
            post_seek_arrivals,
            post_seek_terminal_state_effects,
            sequence_seek_terminal_state_effects,
            mut line_cross_checks,
            mut non_elevation_cross_checks,
            transition_seek_refreshes,
            mut order_pops,
            terminal_pc_direction_goal_restores,
            water_splash_emits,
            movement_state_effects,
            executed_pc_movement_actions,
            executed_sword_movement,
        } = deferred;
        if debug_post_seek_handoff_enabled() {
            let actor_seek = self.world.entities.get(owner).and_then(|entity| {
                let actor = entity.actor_data()?;
                Some((
                    actor.seek_target,
                    actor.post_seek_sequence.is_some(),
                    actor.active_door_pass.is_some(),
                ))
            });
            eprintln!(
                "[POST_SEEK frame={} owner={owner:?} stage=movement_deferred actors_frozen={} arrivals={post_seek_arrivals:?} order_pops={order_pops:?} actor_seek={actor_seek:?}]",
                self.control.frame_counter,
                self.actors_frozen(),
            );
        }

        // The PC WalkingWithCorpse override moves the carried actor inside
        // this Execute arm, immediately after the carrier's PerformMotion.
        // Later creation slots (including NPC RefreshDetection) therefore see
        // the body's new position in the same frame.
        for &(carrier_id, action) in &executed_pc_movement_actions {
            if action == OrderType::WalkingWithCorpse {
                crate::abilities::sync_walking_corpse_for_carrier(
                    &mut self.world.entities,
                    &assets.profile_manager,
                    carrier_id,
                );
            }
        }

        // PerformSeek calls StartPostSeekSequence before it returns its
        // motion state to the surrounding Human Execute arm. In particular,
        // an EnterSwordfight interaction is registered before that outer arm
        // evaluates and registers its terminal Provoke.
        let post_seek_reentrant_order_advances =
            self.launch_perform_seek_arrivals(sim, assets, post_seek_arrivals);

        // Human's sword-movement Execute arm prunes newly far opponents
        // immediately after PerformMotion/PerformSeek and before inspecting
        // the returned motion state (`RHelementactorhuman.cpp:3778-3844`).
        // In particular, this precedes the START initiative handoff: pruning
        // the old principal can promote a different reciprocal opponent, and
        // that promoted opponent is the one Original gives the initiative.
        if executed_sword_movement {
            self.quit_swordfight_with_far_opponents(sim, assets, owner);
        }

        let state_effect_frame = self.control.frame_counter;
        for (entity_id, posture, action_state) in movement_state_effects {
            if let Some(entity) = self.get_entity_mut(entity_id) {
                tracing::trace!(
                    target: "robin_engine::engine::movement::state_effect",
                    ?entity_id,
                    frame = state_effect_frame,
                    ?posture,
                    ?action_state,
                    "movement Execute state side effect"
                );
                entity.set_posture(posture);
                if let Some(actor) = entity.actor_data_mut() {
                    actor.action_state = action_state;
                }
            }
        }
        for entity_id in sword_movement_starts {
            self.apply_sword_movement_start_initiative_transfer(entity_id);
        }

        // Original evaluates the terminal sword-movement Provoke gate inside
        // Human::Execute, after the far-opponent removal above but before base
        // Actor::Hourglass runs line-crossing callbacks. Those callbacks may
        // project the actor onto a different elevation and move the live 3D
        // position across a weapon-range boundary. Snapshot the complete
        // mutual-opponent/range decision at that exact boundary.
        let sword_movement_provokes = sword_movement_terminations
            .into_iter()
            .filter(|&entity_id| {
                self.sword_movement_termination_warrants_provoke(assets, entity_id)
            })
            .collect::<Vec<_>>();
        // LaunchSequenceElement only registers this ordinary-priority work;
        // its later Go/Instruct still runs after terminal order advancement.
        // PerformSeek's synchronous post-seek registration has already run
        // above, matching the nested Original call order.
        for entity_id in sword_movement_provokes {
            self.launch_sword_movement_termination_provoke(entity_id);
        }
        for entity_id in door_pass_transition_start_effects {
            // TODO(parity): apply this START reposition and its crossing
            // callbacks inside the owner's actor slot, as Original does, so
            // later actor slots can observe the midpoint synchronously.
            self.apply_door_pass_transition_start_side_effects(assets, entity_id);
            // A stationary ladder-exit START returns before the ordinary
            // movement tail queues Actor::Hourglass's post-Execute crossing
            // check.  The START side effect above can nevertheless snap the
            // actor across a boundary to the door midpoint.  Original tests
            // that live post-Execute segment before interpreting START, so
            // recover it from NewMove's outer old-position latch here.  A
            // non-stationary START already queued its segment in the common
            // tail and must not dispatch the callbacks twice.
            if !line_cross_checks
                .iter()
                .any(|(queued, _, _)| *queued == entity_id)
            {
                let crossing = self.world.entities.get(entity_id).and_then(|entity| {
                    let old_pos = entity.position_iface().old_map_position();
                    let new_pos = entity.element_data().position_map();
                    let in_bounds = self.world.fast_grid.level.map_bbox.contains_point(new_pos);
                    let eligible = old_pos != new_pos
                        && actor_line_crossing_eligible(
                            entity.element_data().posture,
                            entity
                                .human_data()
                                .is_some_and(|human| human.carrier.is_some()),
                            in_bounds,
                        );
                    eligible.then_some((entity_id, old_pos, entity.element_data().layer()))
                });
                if let Some(crossing) = crossing {
                    line_cross_checks.push(crossing);
                    non_elevation_cross_checks.push(crossing);
                }
            }
        }
        for entity_id in door_pass_transition_done_effects {
            self.apply_door_pass_transition_done_side_effects(assets, entity_id);
        }
        for (entity_id, action) in door_pass_transition_completion_effects {
            self.apply_door_pass_transition_completion_side_effects(assets, entity_id, action);
        }

        // PerformSeek calls SetMovingActionState and RefreshSeek synchronously
        // from the actor's Execute arm. Actor::Hourglass observes the
        // replacement movement when it subsequently checks line crossings.
        // Keep this before crossing resolution/dispatch; delaying it until
        // afterwards lets LINE_SOUND/LINE_SCRIPT callbacks inspect the stale
        // seek and can enter the seeking AI state an extra time.
        // `PerformSeek` answers both of these branches with an explicit
        // `return RHMOTION_IN_PROGRESS` immediately after `RefreshSeek`
        // (`original-code/RHelementactor.cpp:7963-7970` for the stale final
        // waypoint, `:8002-8007` for the out-of-reach stop transition), so the
        // Execute result Actor::Hourglass latches is IN_PROGRESS regardless of
        // the state `RefreshSeek` left on the replaced element.
        let mut refreshed_seek_in_progress = false;
        for (owner, seq_id, elem_idx) in transition_seek_refreshes {
            // Re-read the seek element's flags / target / tolerance / action
            // because another staged Execute effect may have changed adjacent
            // elements. When it no longer looks like an entity-target seek,
            // skip silently.
            let snapshot = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| match &e.data {
                    crate::sequence::SequenceElementData::Movement {
                        flags,
                        element,
                        tolerance,
                        action,
                        ..
                    } => element.map(|t| (*flags, t, *tolerance, *action)),
                    _ => None,
                });
            if let Some((flags, target, tolerance, action)) = snapshot {
                let new_target_pos = self
                    .get_entity(target)
                    .map(|e| e.element_data().position_map())
                    .unwrap_or_default();
                if let Some(actor) = self
                    .get_entity_mut(owner)
                    .and_then(|entity| entity.actor_data_mut())
                {
                    let before = actor.action_state;
                    actor.action_state = actor.action_state.set_moving(false, false);
                    tracing::trace!(
                        target: "parity_post_process_path",
                        ?owner,
                        ?before,
                        after = ?actor.action_state,
                        "transition seek refresh arming moving state",
                    );
                }
                self.apply_seek_refresh(
                    sim,
                    assets,
                    owner,
                    seq_id,
                    elem_idx,
                    target,
                    action,
                    flags,
                    tolerance,
                    new_target_pos,
                );
                refreshed_seek_in_progress = true;
            }
        }

        // Resolve every queued crossing segment against the actor's live
        // position now that all Execute-arm completion branches have run.
        // CheckForLineCrossing samples GetPositionMap() at this point and
        // gathers the crossed lines once; the elevation swap and the
        // patch/script/sound tail then both work off that single segment,
        // so the endpoint must not be re-sampled between the two passes.
        // The in-bounds guard is the Original's GetBoxMap().IsInside early
        // return, likewise evaluated on the live position.
        let resolve_cross_checks = |engine: &Self, queued: Vec<(EntityId, MapPoint, u16)>| {
            queued
                .into_iter()
                .filter_map(|(entity_id, old_pos, layer)| {
                    let new_pos = engine.get_entity(entity_id)?.element_data().position_map();
                    engine
                        .world
                        .fast_grid
                        .level
                        .map_bbox
                        .contains_point(new_pos)
                        .then_some((entity_id, old_pos, new_pos, layer))
                })
                .collect::<Vec<_>>()
        };
        let line_cross_checks = resolve_cross_checks(self, line_cross_checks);
        let non_elevation_cross_checks = resolve_cross_checks(self, non_elevation_cross_checks);
        // Dispatch elevation-line crossings detected during the loop.
        // Runs as a post-pass after the per-actor movement update.
        // When a human actor crosses an elevation line, we also fire
        // `UpdateRoll` so any in-progress Rolling combat_anim can
        // re-aim its flight at the new obstacle's slope.
        for (entity_id, old_pos, new_pos, layer) in line_cross_checks {
            let crossed = self.check_for_line_crossing(assets, entity_id, old_pos, new_pos, layer);
            if crossed {
                let is_human = self
                    .expect_entity(entity_id, "line-crossing mover")
                    .is_human();
                if is_human {
                    self.update_roll_after_crossing(assets, entity_id);
                }
                let compute_direction = self
                    .orders
                    .sequence_manager
                    .in_progress_element_for_actor_matching(entity_id, |e| e.data.is_movement())
                    .and_then(|(seq_id, elem_idx)| {
                        self.orders
                            .sequence_manager
                            .get_element(seq_id, elem_idx)
                            .and_then(|e| e.current_order())
                    })
                    .map(|order| order.compute_direction);
                if let Some(compute_direction) = compute_direction
                    && let Some(entity) = self.get_entity_mut(entity_id)
                {
                    entity
                        .position_iface_mut()
                        .compute_increment_all(compute_direction);
                }
            }
        }

        for (entity_id, old_pos, new_pos, layer) in non_elevation_cross_checks {
            self.check_for_non_elevation_line_crossing(
                sim, assets, entity_id, old_pos, new_pos, layer,
            );
        }

        // RHElementActor::Hourglass dispatches CheckForLineCrossing before
        // its TERMINATED arm calls DoNextOrder. The latter can synchronously
        // retire the completed door pass and clear its movement goal. Keep
        // that ordering: an elevation crossing must recompute the increment
        // from the live destination, not from the cleared (0, 0) sentinel.
        for entity_id in terminal_door_pass_goal_clears {
            let entity = self.world.entities.get_mut(entity_id).unwrap_or_else(|| {
                panic!(
                    "terminal door-pass goal owner {entity_id:?} disappeared after line crossing"
                )
            });
            tracing::trace!(
                target: "parity_owner_handoff",
                owner = ?entity_id,
                goal = ?entity.position_iface().map_goal(),
                "door-pass stop transition clearing movement goal after line crossing"
            );
            clear_terminal_door_pass_goal(entity);
        }

        // These calls are inside the Human/PC sword movement Execute arms,
        // after PerformMotion and before base Actor completion/DoNextOrder.
        if executed_sword_movement && matches!(owner, EntityId::Pc(_)) {
            let pinch_abort = self.world.entities.get(owner).and_then(|entity| {
                entity.actor_data()?;
                // `RHElementActorPC::Execute` gates the override on the
                // live `mpSequenceElement`: it must exist AND must not
                // carry `RHPRIORITY_NON_INTERRUPTABLE`
                // (`RHelementactorpc.cpp:3667-3673`). A door pass is
                // exactly that priority
                // (`RHElementActor::DeterminePriority`,
                // `RHelementactor.cpp:5506-5507`), so a sword walk that
                // belongs to a PassDoor element never aborts — Hourglass'
                // ABORTED arm asserts the same invariant
                // (`RHelementactor.cpp:1206`). Without this gate the
                // aborted pop cancelled the door pass's own order advance
                // and the actor replayed the walk instead of reaching its
                // PASSING_DOOR action point.
                let selected_priority = self
                    .orders
                    .sequence_manager
                    .current_element_for_actor(owner)
                    .and_then(|(seq_id, elem_idx)| {
                        self.orders.sequence_manager.get_element(seq_id, elem_idx)
                    })
                    .map(|element| element.priority)?;
                if selected_priority == crate::sequence::SequencePriority::NonInterruptable {
                    return None;
                }
                if !entity.position_iface().is_moving_map()
                    || !crate::engine::melee::enemies_are_blocking_my_movement(
                        &self.world.entities,
                        owner,
                    )
                {
                    return None;
                }
                Some((selected.seq_id, selected.elem_idx))
            });
            if let Some((seq_id, elem_idx)) = pinch_abort {
                // RHElementActorPC::Execute overrides the nested Human
                // PerformMotion result with RHMOTION_ABORTED here.  The
                // base Actor::Hourglass therefore marks the entry-latched
                // element Impossible and does not run its TERMINATED
                // DoNextOrder arm, even when PerformMotion had already
                // reached the short step-back destination.
                cancel_aborted_order_pop(&mut order_pops, seq_id, elem_idx);
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
            }
        }

        // Execute pending door-pass triggers (PassingDoor steps).
        // These need &mut self for layer/sector changes and building callbacks.
        for (entity_id, door_index, direct, trigger_num) in door_triggers {
            self.execute_pass_door(sim, assets, entity_id, door_index, direct, trigger_num);
        }
        for (entity_id, door_index, direct) in completed_door_passes {
            tracing::debug!(
                entity = ?entity_id,
                door = %door_index,
                direct,
                "DoorPass: completed"
            );
            self.commit_completed_door_pass_position(assets, entity_id, door_index, direct);
            self.apply_completed_door_pass_lift_entry_state(entity_id, door_index, direct);
        }

        // Push queued door-pass Transition orders onto each actor's
        // current sequence element.  The current order list — the
        // transition order blocks subsequent orders until its sprite
        // animation completes.
        for (seq_id, elem_idx, order) in transition_pushes {
            let element = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
                .unwrap_or_else(|| {
                    panic!("door-pass successor has stale element handle ({seq_id:?}, {elem_idx})")
                });
            insert_door_pass_successor(element, order);
        }

        // Fire pending Select hulk flashes.
        for (entity_id, speed) in select_triggers {
            self.apply_select_hulk(entity_id, speed);
        }

        for (entity_id, posture, action_state) in post_seek_terminal_state_effects {
            let entity = self.get_entity_mut(entity_id).unwrap_or_else(|| {
                panic!(
                    "post-seek transition owner {entity_id:?} disappeared before its terminal state effect"
                )
            });
            entity.set_posture(posture);
            entity
                .actor_data_mut()
                .unwrap_or_else(|| {
                    panic!("post-seek transition owner {entity_id:?} is not an actor")
                })
                .action_state = action_state;
        }

        // These are derived Execute-arm tails in Original, so they close
        // after PerformMotion but before base Actor completion/DoNextOrder.
        self.tick_shouldered_carry_ceiling(assets, &executed_pc_movement_actions);
        if galopp_event {
            self.dispatch_galopp_loop_event(sim, assets, owner);
        }
        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
            .unwrap_or_else(|error| {
                panic!(
                    "movement owner {owner:?} failed to drain synchronous Execute-arm callback work: {error:?}"
                )
            });

        // Actor::Hourglass remembers the entry-selected element only for its
        // ABORTED arm.  A successful PerformSeek interaction is different:
        // StartPostSeekSequence terminates the seek and synchronously selects
        // the interaction's first owned element, then the surrounding
        // Execute returns TERMINATED and Hourglass calls DoNextOrder through
        // the actor's *live* mpSequenceElement.  Consequently a newly queued
        // MoveWaiting can have its Freezing order consumed here while its
        // path request remains in RHPathFinder's queue.  The ordinary
        // captured order pop below intentionally rejects replacement
        // selections, so reproduce this re-entrant live-pointer seam
        // explicitly for post-seek launches only.
        for entity_id in post_seek_reentrant_order_advances {
            self.advance_live_order_after_terminal_handoff(entity_id);
        }

        // Drain collected waypoint pops against each actor's Move
        // element.  One pop per waypoint-arrival (both intermediate
        // and final).  When the final pop empties the order queue,
        // `do_next_order` internally calls `element_terminated` +
        // `ensure_wait_element` to transition the sequence element to
        // Terminated on queue exhaustion.  When an end-transition
        // order was spliced in by `post_process_path`, the final
        // walking pop leaves the end-transition as the new current
        // and the animation driver plays it; its own `do_next_order`
        // on completion then terminates the element.
        for (seq_id, elem_idx) in order_pops {
            self.pop_selected_movement_order(seq_id, elem_idx);
        }
        for (entity_id, external_direction, movement_direction) in
            terminal_pc_direction_goal_restores
        {
            let entity = self.world.entities.get_mut(entity_id).unwrap_or_else(|| {
                panic!(
                    "terminal PC direction-goal owner {entity_id:?} disappeared during movement completion"
                )
            });
            // A synchronously instructed successor may deliberately have
            // installed a third direction.  Only replace the outgoing Move's
            // trajectory direction; never overwrite such successor work.
            if i16::from(entity.position_iface().get_direction_goal()) == movement_direction {
                entity
                    .element_data_mut()
                    .set_direction_goal(external_direction);
            }
        }
        // Drain water-splash titbit emissions queued from the walk
        // branch.  Emits a water particle at the actor's 3D position
        // with no element supplier.
        for (_eid, position, layer) in water_splash_emits {
            self.feedback.titbit_manager.add_titbit(
                position,
                layer,
                crate::titbit::TitbitKind::Water,
                crate::titbit::ElementHandle::INVALID,
                0,
                crate::titbit::ElementHandle::INVALID,
                false,
                crate::titbit::INVALID_ID,
                true, // display_titbits_enabled — config plumbing not threaded through this site yet
                None,
                None,
            );
        }
        for (seq_id, elem_idx) in blocked_impossible {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
        }
        let selected_terminal = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .is_some_and(|element| {
                matches!(
                    element.state,
                    crate::sequence::SequenceState::Terminated
                        | crate::sequence::SequenceState::Impossible
                        | crate::sequence::SequenceState::Interrupted
                )
            });
        if selected_terminal {
            // Termination may synchronously instruct a cross-postponed
            // movement successor, but Actor::Hourglass has already made
            // and executed its one entry-latched order choice for this
            // owner slot. The successor becomes observable immediately and
            // executes at the actor's next Hourglass; never recurse into a
            // second Execute in the same slot.
            let _ = self.dispatch_condolations_for_owner_boundary(sim, owner, assets);
        }

        self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
            .unwrap_or_else(|error| {
                panic!(
                    "movement owner {owner:?} failed to drain synchronous callback work: {error:?}"
                )
            });

        for (entity_id, posture, action_state) in sequence_seek_terminal_state_effects {
            let entity = self.get_entity_mut(entity_id).unwrap_or_else(|| {
                panic!(
                    "sequence-seek transition owner {entity_id:?} disappeared before its terminal state effect"
                )
            });
            entity.set_posture(posture);
            entity
                .actor_data_mut()
                .unwrap_or_else(|| {
                    panic!("sequence-seek transition owner {entity_id:?} is not an actor")
                })
                .action_state = action_state;
        }

        MovementOwnerMotion {
            initial: refreshed_seek_in_progress.then_some(crate::sprite::MotionState::InProgress),
            post_completion_override: post_completion_motion_override,
        }
    }

    /// Movement Execute body for the single movement owner. The caller's
    /// actor-id collection filters the entity table down to `actor_id ==
    /// owner`, so this runs at most once per `tick_entity_movement_owner`
    /// call; every early `return` is a per-actor "done" exit.
    #[allow(clippy::too_many_arguments)]
    fn tick_one_movement_actor(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
        owner: EntityId,
        selected: MovementOwnerSelection,
        actor_id: crate::entity_id::ActorId,
        prepass: &MovementPrepass,
        prepared: &LiveMobileGeometry,
        anti_snapshots: &mut EntitySlots<Option<super::anti_collision::ActorSnapshot>>,
        deferred: &mut MovementDeferred,
    ) {
        let mut speed_factor = prepass.speed_factor;
        let entity_id = actor_id.into();
        let rider_entry_compute_direction = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| element.current_order())
            .filter(|order| order.order_id == selected.order_id)
            .map(|order| order.compute_direction);
        if let Some(charge_execution) = self.tick_rider_charge_owner(
            sim,
            assets,
            entity_id,
            false,
            Some((prepared, anti_snapshots)),
        ) {
            let charge_motion = self
                .world
                .entities
                .get(entity_id)
                .and_then(|entity| entity.element_data().sprite.last_motion_state);
            self.dispatch_actor_post_execute_line_crossing(
                sim,
                assets,
                entity_id,
                rider_entry_compute_direction,
            );
            if charge_motion == Some(MotionState::Terminated) {
                // Actor::Hourglass advances only after line-crossing
                // callbacks. ExecuteRiderCharge may legitimately call NewID
                // on its same `pOrder`; compare against that post-Execute
                // identity, while still refusing to consume a callback
                // replacement installed after Execute returned.
                let entry_still_current = self
                    .orders
                    .sequence_manager
                    .get_element(selected.seq_id, selected.elem_idx)
                    .and_then(|element| element.current_order())
                    .is_some_and(|order| {
                        Some(order.order_id) == charge_execution.completion_order_id
                    });
                if entry_still_current {
                    self.do_next_order(selected.seq_id, selected.elem_idx);
                }
                if let Some(actor) = self
                    .world
                    .entities
                    .get_mut(entity_id)
                    .and_then(Entity::actor_data_mut)
                {
                    actor.active_rider_charge = None;
                }
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    entity.element_data_mut().sprite.last_motion_state = charge_motion;
                }
            } else if charge_motion == Some(MotionState::Aborted) {
                deferred
                    .blocked_impossible
                    .push((selected.seq_id, selected.elem_idx));
                if let Some(entity) = self.world.entities.get_mut(entity_id) {
                    let actor = entity
                        .actor_data_mut()
                        .expect("RiderCharging owner must remain an actor");
                    actor.clear_path();
                    actor.active_movement.clear();
                    actor.active_rider_charge = None;
                    entity.position_iface_mut().reset_box_blocked();
                }
            }
            return;
        }
        let ft = prepass.final_tolerance;
        let live_seek_target = ft.target_id.and_then(|target_id| {
            self.world.entities.get(target_id).map(|target| {
                let target_data = target.element_data();
                let target_position = target_data.position_map();
                let use_point = if ft.use_point {
                    target
                        .cxx_current_point_map()
                        .filter(|point| *point != target_position)
                } else {
                    None
                };
                (target_position, target_data.sector(), use_point)
            })
        });
        let live_seek_target_ground = ft
            .target_id
            .and_then(|target_id| self.world.entities.get(target_id))
            .map(|target| target.ground_position());
        // PerformSeek owns mpSeekTarget on the actor independently of the
        // copied movement element (`RHelementactor.cpp`). A terminating
        // transition can therefore have no FinalTol target while the
        // actor still owns the entity interaction. Keep this snapshot
        // separate: genuine point seeks have no actor-owned target.
        let actor_seek_flags = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .and_then(|element| match &element.data {
                crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "selected PerformSeek owner {entity_id:?} lost movement flags for sequence {:?} element {}",
                    selected.seq_id, selected.elem_idx
                )
            });
        let live_actor_seek_target = self.world.entities.get(entity_id).and_then(|entity| {
            let actor = entity.actor_data()?;
            let target = self.world.entities.get(actor.seek_target?)?;
            let target_position = target.element_data().position_map();
            let sampled_target = actor_seek_flags
                .contains(crate::sequence::MoveFlags::USE_POINT)
                .then(|| target.cxx_current_point_map())
                .flatten()
                .filter(|point| *point != target_position)
                .unwrap_or(target_position);
            let delta = sampled_target - entity.element_data().position_map();
            let stretched_y =
                if actor_seek_flags.contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE) {
                    delta.y * 1.743_446_8
                } else {
                    delta.y
                };
            let target_unchanged_or_in_tolerance = target_position
                == actor.last_seek_target_position
                || delta.x * delta.x + stretched_y * stretched_y
                    < actor.seek_distance * actor.seek_distance * 1.1025;
            Some((
                target_position,
                target.ground_position(),
                target.element_data().sector(),
                target_unchanged_or_in_tolerance,
            ))
        });
        let seek_tolerance_reached = |position: MapPoint, self_sector| {
            if ft.tol <= 0.0 {
                return false;
            }
            let target_sector = live_seek_target.and_then(|(_, sector, _)| sector);
            if target_sector.is_some() && self_sector != target_sector {
                false
            } else {
                let target_center = ft
                    .shield_destination
                    .or(live_seek_target.map(|(position, _, _)| position))
                    .expect("SEEK FinalTol must have shield_destination or a live target position");
                let target = live_seek_target
                    .and_then(|(_, _, point)| point)
                    .unwrap_or(target_center);
                let (dx_use, dy_use) = (target.x - position.x, target.y - position.y);
                let dy_effective = if ft.directional {
                    const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
                    dy_use * INVERSE_ASPECT_RATIO
                } else {
                    dy_use
                };
                let dist_sq = dx_use * dx_use + dy_effective * dy_effective;
                dist_sq < ft.tol * ft.tol * 1.1025
            }
        };
        let provenance_frame = self.control.frame_counter;
        let diagnostic_creation_order =
            crate::sprite::sprite_row_diagnostic_creation_order(provenance_frame, || {
                self.world.original_creation_order(entity_id)
            });
        let sprite_row_diagnostic = diagnostic_creation_order.is_some();
        let selected_command = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .map(|element| element.command)
            .expect("selected movement element disappeared before Execute");
        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("movement actor ID collected from entity table must remain present");
        super::animation::direction_provenance_snapshot(
            entity.position_iface(),
            entity_id,
            provenance_frame,
            "movement_execute_entry",
        );
        let is_pc = entity.is_pc();
        let is_drunken_soldier = entity.is_soldier()
            && entity
                .npc_data()
                .and_then(|npc| npc.ai_brain.base())
                .is_some_and(|base| base.blood_alcohol > 0);
        let human_is_carried = entity
            .human_data()
            .is_some_and(|human| human.carrier.is_some());
        // Check swordfight status before mutable borrows — needed at
        // movement completion to preserve WaitingSword (idle state
        // is derived from the action state machine, not hardcoded
        // Waiting).
        let is_swordfighting = entity
            .human_data()
            .map(|h| !h.opponents.is_empty())
            .unwrap_or(false);

        // Extract movement data from actor (scoped borrow).
        //
        // The walk goal is read from the current order's
        // destination on the actor's active Move element —
        // accessed via `SequenceManager::current_order_for_actor`.
        // `path_waypoints` is kept as a mirror for legacy bolt-ons
        // (drunken wobble, abilities, debug overlays) but is no
        // longer the authoritative path source in the hot loop.
        let (
            goal,
            action_state,
            order_id,
            door_pass_anim,
            is_final_waypoint,
            order_action,
            move_seq_id,
            move_elem_idx,
            active_move_flags,
            order_tolerance,
            mut order_compute_direction,
            order_reverse,
            order_antagonist,
            transition_distance_continuation,
            deferred_movement_state_start,
            next_destination_same_action,
            legacy_serialized_order_chain,
        ) = {
            let actor = match entity.actor_data_mut() {
                Some(a) => a,
                None => return,
            };
            let has_moving_state = actor.action_state.is_moving()
                || actor.action_state == crate::element::ActionState::MovingSword
                || actor.action_state == crate::element::ActionState::MovingFastSword
                || actor.action_state == crate::element::ActionState::MovingShield;
            // Read goal from the current **movement** element's
            // front order on the Move / PassDoor / Seek element.
            //
            // We explicitly filter by element data type instead
            // of using `current_order_for_actor` directly: another
            // element type (`Turn`, `Generic` animation, …) may
            // have become InProgress concurrently — e.g. a Turn
            // launched at `SequencePriority::Turn` while the Move
            // is still in flight.  Its front order has no
            // destination (`Turning` orders are (0,0)), so using
            // it as a goal would make the actor walk toward the
            // map origin.  Hold a pointer to the *movement*
            // element specifically by picking the InProgress
            // element whose data is a `Movement`.
            let move_elem = self
                .orders
                .sequence_manager
                .get_element(selected.seq_id, selected.elem_idx)
                .filter(|element| {
                    element.owner == Some(entity_id)
                        && element.state == crate::sequence::SequenceState::InProgress
                        && element.data.is_movement()
                        && element
                            .current_order()
                            .is_some_and(|order| order.order_id == selected.order_id)
                })
                .map(|_| (selected.seq_id, selected.elem_idx));
            let Some((seq_id, elem_idx)) = move_elem else {
                if !has_moving_state {
                    return;
                }
                // No active Move element (element terminated or
                // was never active) — drop out of the moving
                // state back to Waiting.
                let restore_anti_collision = {
                    let restore_anti_collision = actor.active_door_pass.is_some();
                    if restore_anti_collision {
                        tracing::warn!(
                            entity = ?entity_id,
                            "DoorPass: clearing stale active pass after movement element disappeared"
                        );
                        actor.active_door_pass = None;
                    }
                    actor.action_state = if is_swordfighting || actor.action_state.is_sword() {
                        crate::element::ActionState::WaitingSword
                    } else {
                        crate::element::ActionState::Waiting
                    };
                    actor.active_movement.clear();
                    restore_anti_collision
                };
                if restore_anti_collision {
                    entity.position_iface_mut().set_anti_collision_on(true);
                }
                return;
            };
            if !has_moving_state
                && self
                    .orders
                    .sequence_manager
                    .current_element_for_actor(actor_id)
                    != Some((seq_id, elem_idx))
            {
                // A parallel movement element can remain in progress
                // while a higher-priority non-movement element owns the
                // actor. Only bootstrap a non-moving actor when this Move
                // is its selected current element.
                return;
            }
            let Some(order) = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| e.current_order())
            else {
                return;
            };
            let goal = MapPoint::new(order.target_x, order.target_y);
            let order_id = Some(order.order_id);
            let order_action = order.order_type;
            let order_tolerance = order.tolerance;
            let order_compute_direction = order.compute_direction;
            let order_reverse = order.reverse;
            let order_antagonist = order.antagonist;
            let transition_distance_continuation = order.transition_distance_continuation;
            let deferred_movement_state_start = order.deferred_movement_state_start;
            let next_destination_same_action = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| e.next_order())
                .filter(|next| next.order_type == order_action)
                .map(|next| MapPoint::new(next.target_x, next.target_y));
            let active_move_flags = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| match &e.data {
                    crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                    _ => None,
                })
                .unwrap_or(crate::sequence::MoveFlags::empty());
            let legacy_serialized_order_chain = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| element.legacy_v48.is_some());

            // A materialized walk/run successor can sit behind a
            // MakeFast/MakeSlow transition in the sequence-manager queue.
            // When it becomes current, Original's single order list makes
            // that concrete action authoritative; retire the split
            // door-pass transition mirror at the same owner boundary.
            if let Some(pass) = actor.active_door_pass.as_mut() {
                synchronize_selected_door_pass_walk_action(&mut pass.current_action, order_action);
            }

            // Selecting a door-pass Walk successor is not the same as
            // executing it.  Restore the movement state only when that
            // concrete order reaches its owner slot; PassingDoor and
            // transition completion retain their preceding state for the
            // remainder of the tick in Original.
            if order_uses_distance_motion(order_action)
                && actor.active_door_pass.as_ref().is_some_and(|pass| {
                    pass.current_action == order_action && pass.saved_action_state.is_some()
                })
            {
                let saved = actor
                    .active_door_pass
                    .as_mut()
                    .expect("checked active door pass")
                    .saved_action_state
                    .take()
                    .expect("checked saved door-pass action state");
                actor.action_state = saved;
            }

            // Is this the literal last order in the queue?  The
            // Movement element's `tolerance` applies to the final
            // arrival (tolerance applies only on the last order),
            // so we must only allow `tolerance_arrival`
            // to short-circuit when *no* orders remain behind the
            // current one — including end-transition orders spliced
            // in by `insert_transition_end`, which still carry the
            // actual destination as their target.  A prior version
            // of this check counted "last walk-style order", which
            // made the penultimate walking order inserted by
            // `insert_transition_end` look final and triggered an
            // instant tolerance arrival the moment the start
            // transition popped — the actor teleported past the
            // walking phase, played the stop transition in place
            // and never covered any ground.
            let is_final_waypoint = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .map(|e| e.orders.len() <= 1)
                .unwrap_or(true);
            // Use the animation from the active door-pass Walk step.
            let door_pass_anim: Option<OrderType> =
                actor.active_door_pass.as_ref().map(|dp| dp.current_action);

            (
                goal,
                actor.action_state,
                order_id,
                door_pass_anim,
                is_final_waypoint,
                order_action,
                seq_id,
                elem_idx,
                active_move_flags,
                order_tolerance,
                order_compute_direction,
                order_reverse,
                order_antagonist,
                transition_distance_continuation,
                deferred_movement_state_start,
                next_destination_same_action,
                legacy_serialized_order_chain,
            )
        };
        let terminal_pc_external_direction_goal = if is_pc
            && is_final_waypoint
            && matches!(
                order_action,
                OrderType::TransitionWalkingUprightWaitingUpright
                    | OrderType::TransitionRunningUprightWaitingUpright
                    | OrderType::TransitionWalkingCrouchedWaitingCrouched
            )
            && order_compute_direction
            // A new movement order owns the goal unconditionally:
            // Original PerformMotion initializes it with
            // ComputeIncrementAll before any terminal cleanup can
            // observe an external orientation.  Only an already-running
            // order can have been reoriented between Execute calls.
            && order_id.is_some_and(|order_id| {
                entity.element_data().sprite.last_processed_order_id == order_id.get()
            }) {
            let pi = entity.position_iface();
            if !pi.is_increment_all_computed() {
                None
            } else {
                let increment = pi.get_increment();
                let mut movement_direction = vector_to_sector_0_to_15(increment.x, increment.y);
                if order_reverse {
                    movement_direction ^= 8;
                }
                let live_direction_goal = i16::from(pi.get_direction_goal());
                (live_direction_goal != movement_direction)
                    .then_some((live_direction_goal, movement_direction))
            }
        } else {
            None
        };

        if order_action == OrderType::Freezing {
            // `MOVE_WAITING` carries a temporary FREEZING order while
            // the pathfinder owns the request.  The original
            // RHElementActor::Execute arm returns IN_PROGRESS without
            // touching the sprite; this token has no destination-backed
            // motion state to initialize or validate.
            entity.element_data_mut().sprite.last_motion_state =
                non_sprite_movement_motion(order_action);
            return;
        }

        if order_action == OrderType::PassingDoor {
            // Actor::Execute returns TERMINATED directly after the door
            // callback; no Sprite method runs for this action point.
            entity.element_data_mut().sprite.last_motion_state =
                non_sprite_movement_motion(order_action);
            let eid = entity_id;
            if entity
                .actor_data()
                .expect("door-pass action point owner is not an actor")
                .active_door_pass
                .is_none()
            {
                assert!(
                    legacy_serialized_order_chain,
                    "runtime door-pass action point {order_action:?} for {eid:?} lost its active pass"
                );
                // Original saves the complete translated order queue,
                // PositionInterface door pointer/direction, and the
                // actor's direct flag. It has no separate ActiveDoorPass.
                // Execute that authoritative queue directly: the first
                // PassingDoor consumes the saved door and changes sector;
                // a later one sees NULL and merely restores
                // anti-collision.
                let door = entity.position_iface().get_door();
                if let Some(door) = door {
                    let direct = entity.position_iface().get_door_direction();
                    deferred.door_triggers.push((eid, door, direct, 0));
                    entity.position_iface_mut().clear_door();
                } else {
                    entity.position_iface_mut().set_anti_collision_on(true);
                }
                self.orders.messenger.send(crate::messenger::Message::new(
                    crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
                ));
                deferred.order_pops.push((move_seq_id, move_elem_idx));
                return;
            }
            let actor = entity
                .actor_data_mut()
                .expect("door-pass action point owner is not an actor");
            let dp = actor.active_door_pass.as_mut().unwrap_or_else(|| {
                panic!("door-pass action point {order_action:?} for {eid:?} has no active pass")
            });
            let trigger_num = dp.triggers_fired;
            dp.triggers_fired += 1;
            deferred
                .door_triggers
                .push((eid, dp.door_index, dp.direct, trigger_num));

            // A TillLastFrame continuation can force the action-point prefix
            // into the concrete queue. In that case generic DoNextOrder must
            // select the already-queued successor; advancing the lazy tail as
            // well would skip ahead of the copied continuation.
            if !is_final_waypoint {
                self.orders.messenger.send(crate::messenger::Message::new(
                    crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
                ));
                deferred.order_pops.push((move_seq_id, move_elem_idx));
                return;
            }

            let advance = Self::advance_door_pass(
                actor,
                eid,
                goal,
                &mut deferred.door_triggers,
                &mut deferred.select_triggers,
                &mut self.orders.next_order_id,
            );
            match advance {
                DoorPassAdvance::Continue {
                    destination,
                    action,
                    reverse,
                    compute_direction,
                    tolerance,
                } => {
                    let order_id = crate::order::alloc_order_id(&mut self.orders.next_order_id);
                    let mut order =
                        crate::order::Order::new(action, destination.x, destination.y, order_id);
                    order.reverse = reverse;
                    order.compute_direction = compute_direction;
                    order.tolerance = tolerance;
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, order));
                }
                DoorPassAdvance::Paused { transition_order } => {
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, transition_order));
                }
                DoorPassAdvance::ActionPoint { order } => {
                    deferred
                        .transition_pushes
                        .push((move_seq_id, move_elem_idx, order));
                }
                DoorPassAdvance::Done { completed } => {
                    if let Some((door_index, direct)) = completed {
                        deferred
                            .completed_door_passes
                            .push((eid, door_index, direct));
                    }
                    actor.clear_path();
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                }
                DoorPassAdvance::NoActive => {
                    panic!(
                        "door-pass action point {order_action:?} for {eid:?} lost its active pass"
                    );
                }
            }
            self.orders.messenger.send(crate::messenger::Message::new(
                crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
            ));
            deferred.order_pops.push((move_seq_id, move_elem_idx));
            return;
        }

        let tolerance_arrival = seek_tolerance_reached(
            entity.element_data().position_map(),
            entity.element_data().sector(),
        );
        // Original ages this countdown only inside `PerformSeek`, and
        // dispatches there by the current animation arm rather than by
        // RHMOVE_SEEK alone. Cross-sector wall/ladder orders therefore
        // retain the flag but freeze this counter while their Execute
        // arms call PerformMotion directly. Keep the actual decrement
        // ahead of transition and zero-motion early returns; a successful
        // pre-motion post-seek arrival is the one path that returns before
        // it.
        let perform_seek_calls = perform_seek_calls_per_execute(order_action);
        if ft.target_id.is_some()
            && active_move_flags.contains(crate::sequence::MoveFlags::SEEK)
            && perform_seek_calls != 0
            && !(tolerance_arrival && ft.has_post_seek)
        {
            let actor = entity.actor_data_mut().expect("movement owner is actor");
            let wait_before = actor.seek_refresh_wait;
            for _ in 0..perform_seek_calls {
                actor.seek_refresh_wait = age_seek_refresh_wait(actor.seek_refresh_wait);
            }
            // Original performs this aging directly on the overloaded
            // `mulWaitTime` member. Keep the Rust ordinary-wait copy in
            // sync while seek owns that legacy scalar so every possible
            // exit (post-seek interaction, a following Move, interruption
            // or cancellation) retains the last wrapped value.
            actor.wait_time = actor.seek_refresh_wait;
            tracing::trace!(
                entity = ?entity_id,
                wait_before,
                wait_after = actor.seek_refresh_wait,
                perform_seek_calls,
                tolerance_arrival,
                has_post_seek = ft.has_post_seek,
                "entity-target PerformSeek aged refresh countdown"
            );
        }

        let soldier_attentive = matches!(entity, crate::element::Entity::Soldier(_))
            && entity.enemy_ai().is_some_and(|enemy| enemy.attentive);
        let execute_order_initialising = entity
            .actor_data()
            .expect("movement owner lost actor initialization state")
            .execute_order_initialising;
        if execute_order_initialising && is_authored_climb_action(order_action) {
            // Every climb Execute arm calls SetDirection(lift direction)
            // and clears the selected order's bComputeDirection during
            // initialization. Without the clear, PerformMotion
            // immediately replaces that lift-facing goal with the
            // destination vector. This is observable when a save resumes
            // with mbNewOrder set on an already-running climb.
            let order = self
                .orders
                .sequence_manager
                .get_element_mut(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.front_mut())
                .filter(|order| Some(order.order_id) == order_id)
                .unwrap_or_else(|| {
                    panic!(
                        "initializing climb owner {entity_id:?} lost selected order {order_id:?}"
                    )
                });
            order.compute_direction = false;
            order_compute_direction = false;
        }
        let elem = entity.element_data_mut();
        let dx = goal.x - elem.position_map().x;
        let dy = goal.y - elem.position_map().y;
        let dist = (dx * dx + dy * dy).sqrt();
        // Combat movement: face opponent, select directional
        // animation.  `compute_direction=false` (don't auto-face
        // movement direction), face toward opponent, pick
        // forward/backward/strafe animation based on angle between
        // movement vector and facing vector.
        let combat_target = prepass.combat_face_target;
        let is_sword_motion = is_sword_motion_context(action_state, door_pass_anim, order_action);
        let executes_sword_movement = executes_sword_movement_action(door_pass_anim, order_action);
        let is_shield_motion = matches!(action_state, crate::element::ActionState::MovingShield);
        let is_combat = (is_shield_motion && combat_target.is_some()) || is_sword_motion;
        if is_combat {
            // Face opponent instead of movement direction.  Use
            // `set_direction_goal` + per-frame `turn()` rather
            // than instantly snapping facing, so the facing
            // rotates one step per frame toward the opponent.
            if let Some(opp_pos) = combat_target {
                let face_origin = if prepass.combat_face_target_is_ground {
                    let position = elem.position();
                    crate::coordinates::MapPoint::new(position.x, position.y)
                } else {
                    elem.position_map()
                };
                let fdx = opp_pos.x - face_origin.x;
                let fdy = opp_pos.y - face_origin.y;
                tracing::trace!(
                    entity = ?entity_id,
                    frame = self.control.frame_counter,
                    origin_x = face_origin.x,
                    origin_y = face_origin.y,
                    target_x = opp_pos.x,
                    target_y = opp_pos.y,
                    sector = crate::position_interface::vector_to_sector_0_to_15_iso(fdx, fdy),
                    "combat facing target"
                );
                super::animation::direction_provenance_snapshot(
                    &elem.sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "writer:combat_face_goal:before",
                );
                elem.set_direction_goal(crate::position_interface::vector_to_sector_0_to_15_iso(
                    fdx, fdy,
                ));
                super::animation::direction_provenance_snapshot(
                    &elem.sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "writer:combat_face_goal:after",
                );
            }
        }
        // Ordinary movement does not recompute facing from the remaining
        // map-space goal every tick. Original ComputeIncrementAll stamps
        // the goal once from its normalized 3D increment (including the
        // live ground plane), then returns early while that increment
        // remains valid. PerformMotion below owns that initialization;
        // anti-collision and path-boundary code explicitly invalidate and
        // rebuild it when the trajectory actually changes.

        // Choose animation based on action state and movement angle.
        let anim = if let Some(dp_anim) =
            door_pass_sprite_animation_override(order_action, door_pass_anim)
                .filter(|anim| !is_sword_movement_nonanimation(*anim))
        {
            // PassDoor supplies the current translated movement step, but
            // Soldier::Execute still dispatches that logical action
            // through its attentive-animation override. In particular,
            // an attentive WalkingUpright door step plays
            // WalkingAlerted and therefore uses its distinct frame
            // distances.
            super::animation::soldier_movement_animation(dp_anim, soldier_attentive, action_state)
        } else if is_combat {
            if is_sword_motion && combat_target.is_none() {
                // Plain WALKING_SWORD when a non-soldier is forced
                // through sword movement without an active
                // opponent.  The `WalkingWithSword` /
                // `RunningWithSword` values are non-animations and
                // must never be sent directly to the per-frame
                // motion update.
                if elem.sprite.has_animation(OrderType::WalkingSword) {
                    OrderType::WalkingSword
                } else {
                    order_action
                }
            } else {
                // Compute angle between movement direction and
                // facing direction, normalised to [0, 2π).
                // UNIT = π/4 (45°).  8-sector mapping:
                //   [0, π/4) or [7π/4, 2π) → forward
                //   [π/4, 3π/4)             → strafe right
                //   [3π/4, 5π/4)            → backward
                //   [5π/4, 7π/4)            → strafe left
                // The facing vector is the one FaceOpponent measures
                // against, so keep it as a vector: reducing it to an angle
                // first would lose the degenerate cases the Original
                // resolves through its determinant test.
                let facing = if let Some(opp_pos) = combat_target {
                    let face_origin = if prepass.combat_face_target_is_ground {
                        let position = elem.position();
                        crate::coordinates::MapPoint::new(position.x, position.y)
                    } else {
                        elem.position_map()
                    };
                    let fdx = opp_pos.x - face_origin.x;
                    let fdy = opp_pos.y - face_origin.y;
                    // Preserve FaceOpponent's literal vector, including
                    // the zero vector produced by co-located fighters.
                    // SBGeoVector2D::Angle resolves dot == det == 0 to PI,
                    // selecting the backwards-sword animation. Replacing
                    // it with the current heading selects a strafe row.
                    (fdx, fdy)
                } else {
                    let heading = (elem.direction() as f32) * std::f32::consts::PI / 8.0;
                    (heading.cos(), heading.sin())
                };
                let angle = combat_movement_angle((dx, dy), facing);
                // MovingSword and MovingFastSword both use the
                // directional walking/strafing sword animations — the
                // `fast` flag is ignored when selecting the animation.
                // Running in combat is implemented by playing the walking
                // animation under `MotionMethod::Fast`.
                let sword_anim = combat_directional_animation(action_state, angle);
                tracing::trace!(
                    target: "parity_face_opponent",
                    ?entity_id,
                    goal_x = goal.x,
                    goal_y = goal.y,
                    here_x = elem.position_map().x,
                    here_y = elem.position_map().y,
                    ?combat_target,
                    ground_origin = prepass.combat_face_target_is_ground,
                    facing_x = facing.0,
                    facing_y = facing.1,
                    angle,
                    ?sword_anim,
                    "FaceOpponent combat row selection",
                );
                if elem.sprite.has_animation(sword_anim) {
                    sword_anim
                } else {
                    order_action
                }
            }
        } else {
            // Animation comes from the current order's type —
            // dispatch is on `order.action`.  Order types get
            // rewritten by `MakeFast` / `MakeSlow` / `MakeUpright`
            // / `MakeCrouched`, so reading the order directly is
            // how a mid-movement speed change propagates to the
            // sprite.  Falls back to an action_state-derived base
            // only when the order type isn't a movement animation
            // (shouldn't happen for a Move element but is
            // defensive).
            let base = literal_lift_sprite_action(order_action).unwrap_or(match order_action {
                OrderType::WalkingUpright
                | OrderType::WalkingCrouched
                | OrderType::WalkingAlerted
                | OrderType::RunningUpright
                | OrderType::TransitionWalkingUprightRunningUpright
                | OrderType::TransitionRunningUprightWalkingUpright
                | OrderType::TransitionWaitingUprightWalkingUpright
                | OrderType::TransitionWalkingUprightWaitingUpright
                | OrderType::TransitionWaitingUprightRunningUpright
                | OrderType::TransitionRunningUprightWaitingUpright
                | OrderType::TransitionWalkingCrouchedWalkingUpright
                | OrderType::TransitionWalkingUprightWalkingCrouched
                | OrderType::TransitionWalkingCrouchedRunningUpright
                | OrderType::TransitionRunningUprightWalkingCrouched
                | OrderType::TransitionWaitingCrouchedWalkingCrouched
                | OrderType::TransitionWalkingCrouchedWaitingCrouched
                | OrderType::TransitionWaitingUprightSpecial
                | OrderType::TransitionSpecialWaitingUpright
                | OrderType::TransitionWaitingUprightBoredWaitingUpright
                | OrderType::TransitionWaitingUprightWaitingUprightBored
                | OrderType::TransitionCrouchingUp
                | OrderType::TransitionCrouchingDown
                | OrderType::TransitionSittingWaitingUpright
                | OrderType::TransitionLeaningOutWaitingAlerted
                | OrderType::LoweringShield
                | OrderType::WalkingStairs
                | OrderType::RunningStairs
                | OrderType::ClimbingWallUp
                | OrderType::ClimbingWallDown
                | OrderType::ClimbingWallUpFast
                | OrderType::ClimbingWallDownFast
                | OrderType::ClimbingLadderUp
                | OrderType::ClimbingLadderDown
                | OrderType::ClimbingLadderUpAlerted
                | OrderType::ClimbingLadderDownAlerted
                | OrderType::ClimbingLadderUpFast
                | OrderType::ClimbingLadderDownFast
                | OrderType::WalkingWithCorpse
                | OrderType::WalkingCarryingOnShoulders => order_action,
                _ => match action_state {
                    crate::element::ActionState::MovingFast => OrderType::RunningUpright,
                    _ => OrderType::WalkingUpright,
                },
            });
            // DetermineMovementAnimation translates the movement
            // element's primary distance-producing action while it is
            // instructed. PostProcessPath runs afterwards and may insert
            // explicit start/end transition orders; Execute dispatches
            // those transition actions literally even when the actor is
            // standing in a live lift sector. Applying the lift map to a
            // transition here would, for example, turn a walk-to-run
            // transition on stairs back into WalkingStairs.
            //
            // For ordinary distance motion, upright posture takes the
            // upwards mapping unconditionally; on-ladder / on-wall
            // posture chooses upwards vs downwards by dot-producting the
            // ladder vector (`pt_low - pt_high`) with the movement
            // vector. Snapshotted in `lift_translation` so we don't have
            // to re-borrow the grid or door table mid-loop.
            let base =
                if !order_uses_distance_motion(order_action) || is_authored_climb_action(base) {
                    // DetermineMovementAnimation rewrites the movement
                    // element once when it is instructed. Every path order
                    // retains that authored climb direction, even if a later
                    // waypoint briefly bends the other way.
                    base
                } else {
                    match prepass.lift_translation {
                        Some(LiftAnimContext::Upright(lt)) => lt.translate_upright_action(base),
                        Some(LiftAnimContext::OnClimb {
                            lift_type,
                            lift_direction: _,
                            ladder_dx,
                            ladder_dy,
                        }) => {
                            let going_down = ladder_dx * dx + ladder_dy * dy >= 0.0;
                            lift_type.translate_climb_action(base, going_down)
                        }
                        None => base,
                    }
                };
            // RHElementActorSoldier::Execute receives the action after
            // DetermineMovementAnimation has translated it for the lift,
            // then substitutes the attentive sprite animation. The order
            // matters for stairs: translating WalkingStairsAlerted again
            // would collapse it back to ordinary WalkingStairs.
            super::animation::soldier_movement_animation(base, soldier_attentive, action_state)
        };
        // Advance sprite animation and get per-frame distance.
        // PerformMotion sets `row = conversion[anim] + direction`,
        // increments the frame, then reads `GetDistance(row,
        // frame)` only when `frame_count == 0` (the first tick of
        // a new animation frame).  Between frames the distance is
        // 0, so entities move in discrete steps synced to the
        // animation.
        //
        // Motion methods:
        //   Walk / Run: normal frame distance * speed_factor
        //   Fast: double frame rate + double distance (only used
        //     for RUNNING_WITH_SWORD in combat, NOT for normal
        //     running)
        // Normal running uses Run, which is identical to Walk in
        // distance calculation — only the animation differs.  The
        // running animation's per-frame distances in the sprite
        // data are already larger than walking distances.
        //
        // The per-frame sprite distance is scaled by the active
        // sequence element's speed factor.  PC-issued moves use
        // 1.0; shield-following and the AI patrol/approach paths
        // set variable factors.
        //
        // Shield-follower speed adjust: when a PC in MovingShield
        // action state is seeking an actor target (the shield
        // holder), the sequence element's speed factor is
        // rewritten per tick to close gaps quickly and slow down
        // when near.
        //   dist² < 25  → 1.0
        //   dist² < 100 → 1.5
        //   else        → 2.0
        // We override the captured value so `current_frame_distance
        // * speed_factor` sees the adjusted value this tick.  The
        // captured value is reread from the element next tick.
        {
            let ft = prepass.final_tolerance;
            if ft.tol > 0.0
                && ft.target_is_actor
                && matches!(action_state, crate::element::ActionState::MovingShield)
            {
                let (sdx, sdy) = ft
                    .shield_destination
                    .or(live_seek_target.map(|(position, _, _)| position))
                    .map(|p| (p.x - elem.position_map().x, p.y - elem.position_map().y))
                    .unwrap_or((dx, dy));
                let dist_sq = sdx * sdx + sdy * sdy;
                speed_factor = if dist_sq < 25.0 {
                    1.0
                } else if dist_sq < 100.0 {
                    1.5
                } else {
                    2.0
                };
            }
        }
        let speed_factor = speed_factor;
        // Dispatch by order action: transition-animation orders
        // route to `MotionMethod::TillLastFrame`, while walking /
        // running orders route to `MotionMethod::Walk` (or
        // `MotionMethod::Fast` for RUNNING_WITH_SWORD).  The
        // TillLastFrame branch advances the order on animation
        // loop (`Terminated`) rather than on position arrival,
        // which is the right semantics for zero-distance pose
        // changes whose destination is already the actor's current
        // position.
        // Distance-producing movement animations use Walk/Fast.
        // Everything else (transitions, posture-changes, misc)
        // dispatched via tick_move maps to TillLastFrame.
        let is_movement_anim = order_uses_distance_motion(order_action);
        let is_transition_anim = !is_movement_anim;
        // Execute's transition arms have two distinct C++ call paths.
        // Ordinary transitions call PerformMotion directly and retain its
        // default factor of 1. Seek transitions call PerformSeek, which
        // forwards the movement element's speed factor to PerformMotion.
        let apply_speed_factor =
            !is_transition_anim || active_move_flags.contains(crate::sequence::MoveFlags::SEEK);
        // RHElementActorHuman::Execute selects FAST solely from the
        // current logical movement token. The actor can still be in
        // MOVING_FAST_SWORD when a newly selected WALKING_WITH_SWORD
        // order starts; carrying that old state into the method choice
        // would execute the walking order twice before its START side
        // effect changes the state to MOVING_SWORD.
        let fast_sword_motion = order_action == OrderType::RunningWithSword
            || door_pass_anim == Some(OrderType::RunningWithSword);
        // Fast stairs/ladder/wall actions are non-animation dispatch
        // tokens: the Original executes the ordinary sprite motion
        // twice. Lift
        // translation above may therefore turn an already-authored fast
        // token into its ordinary sprite action; retain the dispatch
        // semantics from the sequence order itself.
        let fast_climb_motion = is_fast_climb_action(order_action) || is_fast_climb_action(anim);
        let fast_climb_stops_after_first_termination =
            fast_climb_stops_after_first_termination(order_action)
                || fast_climb_stops_after_first_termination(anim);
        let motion_method = if is_transition_anim {
            MotionMethod::TillLastFrame
        } else if fast_sword_motion {
            MotionMethod::Fast
        } else {
            MotionMethod::Walk
        };
        if let Some(LiftAnimContext::OnClimb {
            lift_type,
            lift_direction,
            ..
        }) = prepass.lift_translation
            && initialising_climb_uses_lift_direction(anim, lift_type, execute_order_initialising)
        {
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_climb_lift_goal:before",
            );
            elem.set_direction_goal(lift_direction);
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_climb_lift_goal:after",
            );
        }
        if let Some(posture) = door_pass_eager_posture(
            anim,
            door_pass_anim.is_some(),
            execute_order_initialising,
            prepass.decorative_building_trap_at_destination,
        ) {
            elem.posture = posture;
        }
        if execute_order_initialising && let Some(climb_dir) = prepass.door_pass_climb_direction {
            let dir = if matches!(
                (anim, elem.posture),
                (
                    OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel,
                    crate::element::Posture::Flying
                )
            ) {
                (climb_dir + 8) & 15
            } else {
                climb_dir
            };
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_door_climb_goal:before",
            );
            elem.set_direction_goal(dir);
            super::animation::direction_provenance_snapshot(
                &elem.sprite.position_iface,
                entity_id,
                provenance_frame,
                "writer:initial_door_climb_goal:after",
            );
        }

        let motion_order = order_id.map(|order_id| MotionOrderContext {
            order_id,
            destination: goal,
            reverse: order_reverse,
            tolerance: order_tolerance,
            directional_tolerance: active_move_flags
                .contains(crate::sequence::MoveFlags::DIRECTIONAL_TOLERANCE),
            compute_direction: order_compute_direction,
            next_destination_same_action,
            target_element: order_antagonist,
        });

        if let Some(motion_order) = motion_order
            && let Some(mismatch) = elem.sprite.motion_order_state_mismatch(motion_order)
        {
            panic!(
                "movement order state invariant failed for entity {entity_id:?}, order {order_action:?}, id {}: {mismatch:?}",
                motion_order.order_id
            );
        }

        // Fast stairs/ladder/wall Execute is two literal
        // Turn/PerformMotion pairs in Original. The second pair is
        // skipped when the first
        // motion terminates; folding it into MotionMethod::Fast would
        // over-rotate on that terminal tick and cannot expose the first
        // call's termination barrier.
        // Original short-circuits a newly initialized non-transition
        // motion only for exact `pointDestination2D == GetPositionMap()`.
        // A near-target continuation must still run PerformMotion so its
        // ordinary arrival path snaps and retires it in this owner slot.
        let dest_already_at_pos =
            motion_method != MotionMethod::TillLastFrame && elem.position_map() == goal;
        let sprite = &mut elem.sprite;
        // Human::FaceOpponent / FaceDangerPoint calls Turn before
        // PerformSeek. When the seek continues, PerformSeek calls Turn a
        // second time immediately before PerformMotion; when tolerance
        // has already been reached, it returns after only this first
        // turn. A non-soldier without a live opponent returns from
        // FaceOpponent before setting a direction or turning.
        if is_combat
            && combat_target.is_some()
            && active_move_flags.contains(crate::sequence::MoveFlags::SEEK)
        {
            super::animation::direction_provenance_snapshot(
                &sprite.position_iface,
                entity_id,
                provenance_frame,
                "turn:combat_face:before",
            );
            let _ = sprite.position_iface.turn();
            super::animation::direction_provenance_snapshot(
                &sprite.position_iface,
                entity_id,
                provenance_frame,
                "turn:combat_face:after",
            );
        }
        // Human's sword-movement Execute arm
        // (`RHelementactorhuman.cpp:3631`) is the one movement arm that has no
        // `Turn()` of its own: its non-SEEK branch goes straight to
        // `mpSprite->PerformMotion` (`RHelementactorhuman.cpp:3660`), and its
        // only pre-motion rotation is the one inside `FaceOpponent`
        // (`RHelementactorhuman.cpp:7513`). `FaceOpponent` returns at
        // `RHelementactorhuman.cpp:7505` — before `SetDirection` and before
        // `Turn` — when a non-soldier is no longer swordfighting, which is
        // exactly the `combat_target.is_none()` case resolved above. A
        // non-interruptible order such as PASS_DOOR keeps that arm selected
        // after `QuitSwordfightWithFarOpponents` has emptied the opponent
        // list, so the Original then rotates the actor not at all while its
        // direction goal stays where the door route last left it. Every other
        // arm reaching the block below does turn: `Actor::Execute`'s ordinary
        // movement arms call `Turn()` explicitly in their non-SEEK branch
        // (`RHelementactor.cpp:2687`), `PerformSeek` calls it in both of its
        // own branches (`RHelementactor.cpp:7805`, `:7925`), and
        // `FaceDangerPoint` always turns (`RHelementactorpc.cpp:8914`).
        let sword_arm_without_face_turn = executes_sword_movement
            && combat_target.is_none()
            && !active_move_flags.contains(crate::sequence::MoveFlags::SEEK);
        // Entity-target PerformSeek returns from its successful
        // pre-motion tolerance branch without calling PerformMotion.
        // Besides avoiding displacement, this preserves the prior sprite
        // action and suppresses START-owned side effects such as combat
        // initiative transfer. When StartPostSeekSequence succeeds the
        // wrapper returns TERMINATED, however; the surrounding Execute
        // arm must still observe that result so a pending movement-end
        // transition applies its terminal posture/action-state effect
        // before the interaction is instructed.
        let (mut motion_state, mut frame_dist_raw) = if tolerance_arrival {
            (
                if ft.has_post_seek {
                    MotionState::Terminated
                } else {
                    MotionState::InProgress
                },
                0.0,
            )
        } else {
            // Entity-target PerformSeek tests its successful tolerance
            // branch before the ordinary Turn/PerformMotion block. Do
            // not advance anti-vibration turning on a terminal tolerance
            // sample whose post-seek sequence is taking over.
            if !sword_arm_without_face_turn
                && should_apply_plain_movement_turn(
                    is_drunken_soldier,
                    active_move_flags,
                    order_action,
                )
            {
                super::animation::direction_provenance_snapshot(
                    &sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "turn:perform_seek:before",
                );
                let _ = sprite.position_iface.turn();
                super::animation::direction_provenance_snapshot(
                    &sprite.position_iface,
                    entity_id,
                    provenance_frame,
                    "turn:perform_seek:after",
                );
            }
            let diagnostic_pre = sprite_row_diagnostic.then(|| sprite.sprite_row_diagnostic_pre());
            let played_direction = u16::from(sprite.position_iface.get_direction().as_u8());
            let result = sprite.perform_motion(
                sim,
                motion_order,
                sprite_motion_order_for_nonanimation(anim),
                played_direction,
                FrameProgression::Default,
                false,
                motion_method,
                dest_already_at_pos,
            );
            if let Some(pre) = diagnostic_pre {
                sprite.emit_sprite_row_diagnostic(
                    "perform_motion",
                    provenance_frame,
                    diagnostic_creation_order.expect("enabled diagnostic has owner"),
                    entity_id.index(),
                    order_action,
                    sprite_motion_order_for_nonanimation(anim),
                    played_direction,
                    FrameProgression::Default,
                    pre,
                    result.0,
                );
            }
            super::animation::direction_provenance_snapshot(
                &sprite.position_iface,
                entity_id,
                provenance_frame,
                "perform_motion:return",
            );
            // A generated walking- or running-start transition can begin
            // exactly where an anti-collision deviation ended (destination
            // == current map position). The shipped Linux game drops the
            // deviation latch on that zero-distance START tick, so every
            // following in-place `Turn()` rotates immediately in *both*
            // directions — a counter-clockwise first-call rotation from a +2
            // count (Savegame_010 replay-014 frame 1030) rules out the
            // previous count-priming model from task #545. Savegame_032
            // replay-010 additionally proves the running-start case: the
            // visible turn history establishes a -2 count immediately before
            // the aligned start, then its next clockwise shield turn rotates
            // on the first call. The available C++ source does not expose the
            // latch update responsible for this save-observable detail; see
            // `clear_deviated_for_aligned_transition_start` for the trace
            // evidence bounding it to exactly this startup initialization.
            // The matching walking-to-waiting exit deliberately preserves
            // the latch (Savegame_023 replay-027, Soldier 136, frame 25195).
            if should_clear_deviated_for_aligned_transition_start(
                is_pc,
                execute_order_initialising,
                is_transition_anim,
                order_action,
                sprite.position_iface.is_deviated(),
                sprite.position_iface.map_position(),
                goal,
            ) {
                sprite
                    .position_iface
                    .clear_deviated_for_aligned_transition_start();
            }
            result
        };
        if tolerance_arrival {
            // This PerformSeek branch returns before calling any Sprite
            // method. Preserve the wrapper's authoritative Execute result
            // for Actor::Hourglass just as the non-sprite movement arms
            // above do. Leaving the prior sprite DONE latched causes the
            // successful StartPostSeekSequence termination to be hidden as
            // IN_PROGRESS by the generic entity-seek projection.
            sprite.last_motion_state = Some(motion_state);
        }
        let first_frame_dist_raw = frame_dist_raw;
        let first_direction_differs_from_goal =
            sprite.position_iface.get_direction() != sprite.position_iface.get_direction_goal();
        let fast_motion_outer_pre = sprite.position_iface.map_position();
        let mut first_fast_commit = None;
        let mut second_fast_operands = None;
        // Fast ladder/wall Execute contains two literal PerformMotion
        // calls, but returns immediately when the first one reaches the
        // order goal. RunningStairs has the same two-call loop without
        // that early return, so its terminal tick still advances the
        // sprite in the second call.
        // Project that first call through the same anti-collision query
        // used by the committed movement below.  Deferring all position
        // work until after both sprite calls otherwise advances the
        // animation counter once too often on a terminal first call; the
        // next climb order can then move one simulation frame early.
        let first_fast_call_terminates = if !tolerance_arrival
            && fast_climb_stops_after_first_termination
            && motion_state != MotionState::Terminated
        {
            let first_speed = scaled_motion_distance(
                first_frame_dist_raw,
                speed_factor,
                apply_speed_factor,
                first_direction_differs_from_goal,
            );
            projected_step_reaches_goal(
                &sprite.position_iface,
                anti_snapshots.get(actor_id).and_then(|slot| slot.as_ref()),
                anti_snapshots.as_slice(),
                &self.ai.global.repulsive_points,
                prepared,
                &self.world.fast_grid,
                goal,
                prepass.goal_target_info,
                first_speed,
            )
        } else {
            false
        };
        let mut second_frame_dist_raw = None;
        if !tolerance_arrival
            && fast_climb_motion
            && motion_state != MotionState::Terminated
            && !first_fast_call_terminates
        {
            let first_speed = scaled_motion_distance(
                first_frame_dist_raw,
                speed_factor,
                apply_speed_factor,
                first_direction_differs_from_goal,
            );
            if first_speed != 0.0 {
                let first_pre = sprite.position_iface.map_position();
                let first_increment = sprite.position_iface.get_increment_map();
                let anti_on = sprite.position_iface.is_anti_collision_on();
                let (first_dx, first_dy, recovered, rebuild) = if anti_on
                    && let Some(mover_snapshot) = anti_snapshots
                        .get(actor_id)
                        .and_then(|slot| slot.as_ref())
                        .filter(|snapshot| snapshot.active)
                        .cloned()
                {
                    let move_box = *sprite.position_iface.get_move_box();
                    let half_diagonal = sprite.position_iface.get_half_diagonal();
                    let was_deviated = sprite.position_iface.is_deviated();
                    let mut state = super::anti_collision::AntiCollisionState {
                        pi: &mut sprite.position_iface,
                        move_box,
                        half_diagonal,
                        goal_map: goal,
                    };
                    let (dx, dy) = apply_prepared_anti_collision_step(
                        provenance_frame,
                        &mover_snapshot,
                        anti_snapshots,
                        &self.ai.global.repulsive_points,
                        prepared,
                        &self.world.fast_grid,
                        &mut state,
                        first_increment.x,
                        first_increment.y,
                        first_speed,
                        true,
                    );
                    (
                        dx,
                        dy,
                        was_deviated && !state.pi.is_deviated(),
                        state.pi.is_deviated() && state.pi.blocked_count == 0,
                    )
                } else {
                    (
                        first_increment.x * first_speed,
                        first_increment.y * first_speed,
                        false,
                        false,
                    )
                };
                let first_raw_post = MapPoint::new(first_pre.x + first_dx, first_pre.y + first_dy);
                sprite.position_iface.set_map_position(first_raw_post);
                if rebuild && (first_dx != 0.0 || first_dy != 0.0) {
                    let raw = vector_to_sector_0_to_15(first_dx, first_dy);
                    sprite.position_iface.set_direction(
                        crate::position_interface::Direction::from_raw(i32::from(
                            if order_reverse { raw ^ 8 } else { raw },
                        )),
                    );
                    sprite.position_iface.reset_increment_computed();
                    sprite.position_iface.compute_increment_all(false);
                } else if recovered {
                    sprite.position_iface.reset_increment_computed();
                    sprite.position_iface.compute_increment_all(true);
                }
                // RHNONANIMATION_RUNNING_STAIRS is the one double-motion
                // Execute arm which deliberately continues after its first
                // PerformMotion returns TERMINATED.  That first call still
                // owns the complete ordinary arrival branch: IsGoalReached,
                // followed by the zero-tolerance goal snap.  The second
                // Turn/PerformMotion therefore observes the snapped position,
                // rather than both raw displacements being committed before a
                // single aggregate arrival check.
                let first_post = if order_action == OrderType::RunningStairs
                    && sprite
                        .position_iface
                        .is_goal_reached(&self.world.fast_grid, prepass.goal_target_info)
                    && order_tolerance == 0.0
                    && !sprite.position_iface.is_deviated()
                {
                    sprite.position_iface.set_map_position(goal);
                    goal
                } else {
                    first_raw_post
                };
                if let Some(snapshot) = anti_snapshots
                    .get_mut(actor_id)
                    .and_then(|slot| slot.as_mut())
                {
                    sync_snapshot_after_committed_step(snapshot, first_pre, first_post);
                }
                // Fast wall/ladder Execute arms contain two literal
                // PerformMotion calls. Original refreshes the forecast at
                // the end of each nonzero call, immediately after its
                // position commit. Keep that first write here: when the
                // second sprite frame has zero distance the stationary tail
                // returns before the aggregate commit below, and the first
                // call's forecast must remain observable.
                refresh_motion_forecast(sprite, first_speed, None);
                first_fast_commit = Some((first_pre, first_increment, first_speed, first_post));
            }
            let _ = sprite.position_iface.turn();
            let (second_state, second_distance) = sprite.perform_motion(
                sim,
                motion_order,
                sprite_motion_order_for_nonanimation(anim),
                u16::from(sprite.position_iface.get_direction().as_u8()),
                FrameProgression::Default,
                false,
                MotionMethod::Walk,
                dest_already_at_pos,
            );
            motion_state = second_state;
            second_frame_dist_raw = Some(second_distance);
            frame_dist_raw += second_distance;
            second_fast_operands = Some((
                sprite.position_iface.map_position(),
                sprite.position_iface.get_increment_map(),
            ));
        }
        // PerformMotion refreshes RHPositionInterface::mpTargetElement
        // when a new order is initialized. Anti-collision follows that
        // call in the same actor slot in Original, so the mover snapshot
        // must observe the newly installed order's antagonist now rather
        // than the target retained from the preceding order at the
        // top-of-tick snapshot boundary.
        if let Some(snapshot) = anti_snapshots
            .get_mut(actor_id)
            .and_then(|slot| slot.as_mut())
        {
            snapshot.target_element = sprite.position_iface.target_element();
        }
        deferred.executed_sword_movement = is_sword_motion;
        if is_pc {
            deferred
                .executed_pc_movement_actions
                .push((entity_id, order_action));
        }
        if door_pass_anim.is_some()
            && matches!(motion_state, MotionState::Start)
            && matches!(
                anim,
                OrderType::TransitionClimbingLadderUpWaitingCrouched
                    | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
            )
        {
            deferred.door_pass_transition_start_effects.push(entity_id);
        }
        if door_pass_anim.is_some()
            && matches!(motion_state, MotionState::Done)
            && matches!(
                anim,
                OrderType::TransitionWaitingUprightClimbingWallUp
                    | OrderType::TransitionClimbingWallUpWaitingCrouched
                    | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                    | OrderType::TransitionWaitingCrouchedClimbingWallDown
                    | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                    | OrderType::TransitionClimbingWallDownWaitingUpright
                    | OrderType::TransitionClimbingLadderUpWaitingCrouched
                    | OrderType::TransitionClimbingLadderUpWaitingUprightAlerted
            )
        {
            deferred.door_pass_transition_done_effects.push(entity_id);
        }
        if active_move_flags.contains(crate::sequence::MoveFlags::RIDER_CHARGE)
            && anim == OrderType::RunningUpright
        {
            let frame_count = sprite.num_frames_for_anim(OrderType::RunningUpright);
            let cur = sprite.current_frame;
            if is_galopp_decision_frame(cur, frame_count) {
                assert_eq!(
                    entity_id, owner,
                    "owner-local rider Execute collected a gallop callback for another actor"
                );
                deferred.galopp_event = true;
            }
        }
        // `PerformMotion` applies the sequence speed factor before its
        // turn slowdown and 0.7-unit minimum. The order is observable:
        // a slow patrol member with raw distance 2 and factor ~0.58 is
        // clamped to exactly 0.7 after the 0.6 multiplier, rather than
        // scaling an already-clamped 0.7 back below the minimum.
        //
        // PerformMotion initializes a new order's direction goal after
        // the caller's Turn() above. The slowdown test in Original
        // happens later and reads the now-live direction/goal pair, so it
        // applies even though the pre-initialization Turn was a no-op.
        let direction_differs_from_goal =
            sprite.position_iface.get_direction() != sprite.position_iface.get_direction_goal();
        // Direct transition Execute arms call `PerformMotion(...,
        // RHMOTIONMETHOD_TILL_LAST_FRAME)` without a speed factor. Seek
        // transitions instead route through PerformSeek and do pass it.
        let (speed, split_motion_speeds) = if let Some(second_distance) = second_frame_dist_raw {
            // The fast stairs/ladder/wall arms contain two literal
            // PerformMotion calls. Each call applies its own turning
            // slowdown using the direction reached by the immediately
            // preceding Turn(), so a first call that is still rotating
            // must not inherit the second call's newly aligned state.
            let first_speed = scaled_motion_distance(
                first_frame_dist_raw,
                speed_factor,
                apply_speed_factor,
                first_direction_differs_from_goal,
            );
            let second_speed = scaled_motion_distance(
                second_distance,
                speed_factor,
                apply_speed_factor,
                direction_differs_from_goal,
            );
            (
                if first_fast_commit.is_some() {
                    second_speed
                } else {
                    first_speed + second_speed
                },
                Some((first_speed, second_speed)),
            )
        } else {
            (
                scaled_motion_distance(
                    frame_dist_raw,
                    speed_factor,
                    apply_speed_factor,
                    direction_differs_from_goal,
                ),
                None,
            )
        };
        let mut discarded_lazy_door_followers = false;
        // PerformMotion applies the distance before returning its motion
        // state. A fresh walking order that reaches its goal on that same
        // invocation returns TERMINATED, not START, so the walking
        // Execute arm does not enter the Moving action state. Our
        // position update is staged below; fold that imminent arrival
        // into the state-effect result now.
        let entity_target_seek =
            active_move_flags.contains(crate::sequence::MoveFlags::SEEK) && ft.target_id.is_some();
        // The ordinary (non-TillLastFrame) arrival branch runs only when
        // the sprite actually advanced the actor, and it asks the position
        // interface rather than comparing straight-line distances. A
        // walker that sidesteps a neighbour covers more ground than
        // remains to its goal and still ends the frame short of it.
        let reaches_goal_this_step = !is_transition_anim
            && projected_step_reaches_goal(
                &sprite.position_iface,
                anti_snapshots.get(actor_id).and_then(|slot| slot.as_ref()),
                anti_snapshots.as_slice(),
                &self.ai.global.repulsive_points,
                prepared,
                &self.world.fast_grid,
                goal,
                prepass.goal_target_info,
                speed,
            );
        let state_effect_motion = movement_execute_visible_motion(
            order_action,
            motion_state,
            reaches_goal_this_step,
            entity_target_seek,
        );
        deferred.post_completion_motion_override = committed_arrival_post_completion_override(
            motion_state,
            state_effect_motion,
            reaches_goal_this_step,
        );
        let deferred_movement_state_start_due = if deferred_movement_state_start {
            let current_order = self
                .orders
                .sequence_manager
                .get_element_mut(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.front_mut())
                .unwrap_or_else(|| {
                    panic!(
                        "deferred movement-state successor for {entity_id:?} disappeared during execution"
                    )
                });
            assert_eq!(
                Some(current_order.order_id),
                order_id,
                "deferred movement-state successor changed identity during execution"
            );
            assert!(
                take_deferred_movement_state_start(
                    &mut current_order.deferred_movement_state_start
                ),
                "deferred movement-state successor marker was already consumed"
            );
            true
        } else {
            false
        };
        // The initiative handoff belongs to the Human Execute START arm,
        // so it observes entity-target PerformSeek's wrapper result just
        // like posture/action-state changes do. A raw sprite START hidden
        // as IN_PROGRESS by PerformSeek must not transfer initiative.
        if matches!(state_effect_motion, MotionState::Start) && executes_sword_movement {
            deferred.sword_movement_starts.push(entity_id);
        }
        tracing::trace!(
            entity = ?entity_id,
            frame = self.control.frame_counter,
            ?order_action,
            ?motion_state,
            ?state_effect_motion,
            action_state = ?action_state,
            sprite_frame = sprite.current_frame,
            sprite_counter = sprite.frame_count,
            sprite_num_frames = sprite.num_frames_for_row(sprite.current_row),
            sprite_wait = sprite.wait_time(sprite.current_row, sprite.current_frame),
            frame_distance_raw = frame_dist_raw,
            speed_factor,
            effective_distance = speed,
            remaining_distance = dist,
            reaches_goal_this_step,
            order_tolerance,
            deviated = sprite.position_iface.is_deviated(),
            anti_collision = sprite.position_iface.is_anti_collision_on(),
            goal_x = goal.x,
            goal_y = goal.y,
            increment_x = sprite.position_iface.get_increment_map().x,
            increment_y = sprite.position_iface.get_increment_map().y,
            "movement Execute result"
        );
        // Transition motion can still change from InProgress to
        // Terminated in the TILL_LAST_FRAME arrival handling below.
        // Original applies the Execute switch's state side effect after
        // PerformMotion returns that final result, before Proceed rewrites
        // the diagnostic motion for a successor order.
        let transition_distance_first_execute_due = if transition_distance_continuation {
            let element = self
                .orders
                .sequence_manager
                .get_element_mut(move_seq_id, move_elem_idx)
                .unwrap_or_else(|| {
                    panic!(
                        "transition-distance continuation for {entity_id:?} disappeared during its first execution"
                    )
                });
            let current_order = element.orders.front_mut().unwrap_or_else(|| {
                panic!(
                    "transition-distance continuation for {entity_id:?} lost its current order during its first execution"
                )
            });
            assert_eq!(
                Some(current_order.order_id),
                order_id,
                "transition-distance continuation changed identity during its first execution"
            );
            take_transition_distance_first_execute(
                &mut current_order.transition_distance_continuation,
            )
        } else {
            false
        };
        let suppress_transition_continuation_start = transition_distance_first_execute_due
            && matches!(state_effect_motion, MotionState::Start);
        if !is_transition_anim
            && !suppress_transition_continuation_start
            // A deferred PC successor deliberately postpones this
            // START-only state effect until after order completion has
            // decided whether the authored walking order survived.  The
            // guarded handoff below owns that one-shot side effect.
            && !deferred_movement_state_start_due
            // PerformMotion commits the physical step before returning.
            // A fresh order can therefore reach its goal and return
            // TERMINATED instead of exposing START to Execute. Defer all
            // START-only effects until the committed step has decided
            // whether this exact order survives.
            && !matches!(state_effect_motion, MotionState::Start)
            && let Some((posture, action_state)) =
                movement_execute_state_effect(order_action, state_effect_motion)
        {
            deferred
                .movement_state_effects
                .push((entity_id, posture, action_state));
        }
        if is_transition_anim
            && tolerance_arrival
            && let Some((posture, action_state)) =
                movement_execute_state_effect(order_action, state_effect_motion)
        {
            if ft.launches_post_seek {
                // StartPostSeekSequence runs synchronously inside
                // PerformSeek. Its interaction callbacks must therefore see
                // the pre-transition action state. The surrounding transition
                // Execute switch applies TERMINATED only after that recursive
                // work returns.
                deferred
                    .post_seek_terminal_state_effects
                    .push((entity_id, posture, action_state));
            } else if ft.has_post_seek {
                deferred.sequence_seek_terminal_state_effects.push((
                    entity_id,
                    posture,
                    action_state,
                ));
            } else {
                deferred
                    .movement_state_effects
                    .push((entity_id, posture, action_state));
            }
        }

        if door_pass_anim.is_some()
            && matches!(
                anim,
                OrderType::ClimbingWallUp
                    | OrderType::ClimbingWallDown
                    | OrderType::ClimbingWallUpFast
                    | OrderType::ClimbingWallDownFast
                    | OrderType::TransitionWaitingUprightClimbingWallUp
                    | OrderType::TransitionClimbingWallUpWaitingCrouched
                    | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                    | OrderType::TransitionWaitingCrouchedClimbingWallDown
                    | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                    | OrderType::TransitionClimbingWallDownWaitingUpright
            )
        {
            let goal_dir = elem.sprite.position_iface.get_direction_goal().as_u8();
            tracing::debug!(
                entity = ?entity_id,
                ?anim,
                posture = ?elem.posture,
                action_state = ?action_state,
                dir = elem.direction(),
                goal_dir,
                row = elem.sprite.current_row,
                frame = elem.sprite.current_frame,
                pos_x = elem.position_map().x,
                pos_y = elem.position_map().y,
                pos3_y = elem.position().y,
                pos3_z = elem.position().z,
                speed,
                dist,
                "DoorPass movement state"
            );
        }

        // TillLastFrame branch: transition animations advance via
        // the animation-loop `Terminated` edge, not via position
        // arrival.  Still update position by the sprite's
        // per-frame distance along the vector toward the order's
        // target — end-of-run transitions carry ~26 units of
        // distance and must actually move the actor to reach the
        // goal (without this advance, soldiers stop at the
        // running-phase endpoint and never close the final ~26u
        // gap, leaving them outside sword_range forever and unable
        // to trigger begin_swordfight). C++ routes every nonzero
        // transition distance through UpdatePositionAntiCollision, so
        // transition displacement must also participate in elevation,
        // patch, and sound boundary crossing.
        //
        // C++ seeds `PositionInterface` at the start of every new
        // sprite motion order and moves transition animations via
        // `UpdatePositionMap(fDistance)`, so this branch uses the
        // same precomputed map increment instead of a separate
        // dx/dy step.
        // Entity-target PerformSeek checks its live tolerance before it
        // dispatches the current sprite order.  An already-in-range seek
        // therefore bypasses transition execution and enters the shared
        // post-seek/frozen arrival tail below.
        if is_transition_anim && !tolerance_arrival {
            let transition_has_map_target = goal.x != 0.0 || goal.y != 0.0;
            if !transition_has_map_target && !is_in_place_movement_transition(order_action) {
                panic!(
                    "movement transition {:?} for entity {:?} has zero map target; refusing to treat (0,0) as an implicit destination",
                    order_action, entity_id
                );
            }
            // A movement transition can legitimately target the actor's
            // exact current point (for example the generated
            // Waiting→Walking pose at the end of a combat sequence).
            // PerformMotion still advances that animation, but the zero
            // goal vector contributes no map displacement.  In
            // particular, do not feed a stale pre-order increment into
            // anti-collision: ComputeIncrementAll deliberately preserves
            // the stored vector when the new vector is zero.
            let transition_has_distance =
                transition_has_map_target && speed > 0.0 && dist > f32::EPSILON;
            let transition_recomputes_exact_position = motion_recomputes_exact_position(
                is_transition_anim,
                transition_has_map_target,
                speed,
                dist,
            );
            let transition_crossing_start = transition_has_distance.then(|| {
                let old_pos = entity.element_data().position_map();
                let layer = entity.element_data().layer();
                let eligible = actor_line_crossing_eligible(
                    entity.element_data().posture,
                    human_is_carried,
                    self.world.fast_grid.level.map_bbox.contains_point(old_pos),
                );
                (old_pos, layer, eligible)
            });
            if transition_has_distance {
                // Match GetIncrementMap(): PerformMotion seeded this
                // normalized vector when the order began and reuses it
                // unchanged until anti-collision explicitly rebuilds it.
                let increment = entity.position_iface().get_increment_map();
                let nx = increment.x;
                let ny = increment.y;
                let anti_on = entity.position_iface().is_anti_collision_on();
                // The fast stairs/ladder/wall tokens invoke PerformMotion
                // twice. With anti-collision disabled, Original stores the
                // first position update before applying the second one.
                // Combining both distances and rounding only the final
                // sum moves large map coordinates by an ULP and can
                // amplify into a visible elevation error on steep planes.
                let split_motion_target =
                    split_motion_speeds
                        .filter(|_| !anti_on)
                        .map(|(first_speed, second_speed)| {
                            let mut target = entity.element_data().position_map();
                            target.x += nx * first_speed;
                            target.y += ny * first_speed;
                            target.x += nx * second_speed;
                            target.y += ny * second_speed;
                            target
                        });
                let goal_map = crate::coordinates::MapPoint::new(goal.x, goal.y);
                let (move_box, half_diagonal) = {
                    let pi = entity.position_iface();
                    (*pi.get_move_box(), pi.get_half_diagonal())
                };
                let (dx_step, dy_step, deviated, recovered_from_deviation) =
                    if let Some(mover_snap) = anti_snapshots
                        .get(actor_id)
                        .and_then(|slot| slot.as_ref())
                        .filter(|snapshot| snapshot.active)
                    {
                        let pi = entity.position_iface_mut();
                        let was_deviated = pi.is_deviated();
                        let mut state = super::anti_collision::AntiCollisionState {
                            pi,
                            move_box,
                            half_diagonal,
                            goal_map,
                        };
                        let (dx_step, dy_step) = apply_prepared_anti_collision_step(
                            provenance_frame,
                            mover_snap,
                            anti_snapshots,
                            &self.ai.global.repulsive_points,
                            prepared,
                            &self.world.fast_grid,
                            &mut state,
                            nx,
                            ny,
                            speed,
                            anti_on,
                        );
                        (
                            dx_step,
                            dy_step,
                            // Only a committed deviation (blocked counter
                            // reset) faces along its step and rebuilds the
                            // increment here; a break-through barge keeps
                            // the facing and cached increment the
                            // anti-collision step left behind.
                            state.pi.is_deviated() && state.pi.blocked_count == 0,
                            was_deviated && !state.pi.is_deviated(),
                        )
                    } else {
                        (nx * speed, ny * speed, false, false)
                    };
                let elem = entity.element_data_mut();
                if deviated && (dx_step != 0.0 || dy_step != 0.0) {
                    let raw = vector_to_sector_0_to_15(dx_step, dy_step);
                    elem.set_direction_goal(if order_reverse { raw ^ 8 } else { raw });
                }
                let position = split_motion_target.unwrap_or_else(|| {
                    let mut position = elem.position_map();
                    position.x += dx_step;
                    position.y += dy_step;
                    position
                });
                elem.set_position_map(position);
                if deviated && (dx_step != 0.0 || dy_step != 0.0) {
                    elem.sprite.position_iface.reset_increment_computed();
                    elem.sprite.position_iface.compute_increment_all(false);
                } else if recovered_from_deviation {
                    // Original rebuilds the trajectory even when this
                    // animation frame contributes no movement.
                    elem.sprite.position_iface.reset_increment_computed();
                    elem.sprite.position_iface.compute_increment_all(true);
                }
                elem.update_grid_cell();
            } else if transition_recomputes_exact_position {
                // PerformMotion gates its position update on animation
                // distance, not on the length of the normalized map
                // increment. With an exact-position transition target a
                // nonzero sprite-frame distance therefore still reaches
                // UpdatePositionAntiCollision with a zero increment. That
                // call is observable even though it cannot displace the
                // actor: its empty-candidate recovery drops a preceding
                // deviation latch before ComputePositionAll. Skipping the
                // call left a stopping soldier in TurnAntiVibration on the
                // following frame (Linux Savegame_036 replay-015, Soldier
                // 144), delaying the visible counter-clockwise turn.
                let recovered_from_deviation = if entity.position_iface().is_anti_collision_on()
                    && let Some(mover_snap) = anti_snapshots
                        .get(actor_id)
                        .and_then(|slot| slot.as_ref())
                        .filter(|snapshot| snapshot.active)
                {
                    let goal_map = crate::coordinates::MapPoint::new(goal.x, goal.y);
                    let (move_box, half_diagonal) = {
                        let pi = entity.position_iface();
                        (*pi.get_move_box(), pi.get_half_diagonal())
                    };
                    let pi = entity.position_iface_mut();
                    let was_deviated = pi.is_deviated();
                    let mut state = super::anti_collision::AntiCollisionState {
                        pi,
                        move_box,
                        half_diagonal,
                        goal_map,
                    };
                    let step = apply_prepared_anti_collision_step(
                        provenance_frame,
                        mover_snap,
                        anti_snapshots,
                        &self.ai.global.repulsive_points,
                        prepared,
                        &self.world.fast_grid,
                        &mut state,
                        0.0,
                        0.0,
                        speed,
                        true,
                    );
                    debug_assert_eq!(step, (0.0, 0.0));
                    was_deviated && !state.pi.is_deviated()
                } else {
                    false
                };
                let position = entity.element_data().position_map();
                let elem = entity.element_data_mut();
                elem.set_position_map(position);
                if recovered_from_deviation {
                    elem.sprite.position_iface.reset_increment_computed();
                    elem.sprite.position_iface.compute_increment_all(true);
                }
                elem.update_grid_cell();
                // The same nonzero-animation-distance block ends with
                // UpdateForecastedMovement even though the cached
                // increment is zero at the goal. This clears a preceding
                // running forecast before projectile leading samples it.
                refresh_motion_forecast(entity.sprite_mut(), speed, split_motion_speeds);
            }
            if transition_has_distance {
                // Original's shared PerformMotion path refreshes target
                // leading after every committed transition displacement,
                // before IsGoalReached can clear the live increment. A
                // missing refresh here made arrows aim at the target's
                // current point during start/stop transitions.
                refresh_motion_forecast(entity.sprite_mut(), speed, split_motion_speeds);
            }
            // TILL_LAST_FRAME still performs the ordinary arrival check
            // after every nonzero transition step. Reaching the target
            // zeros both increments and snaps an undeviated zero-tolerance
            // actor, but the transition keeps playing until its animation
            // loops unless the next order uses the same animation.
            let transition_goal_reached = entity
                .position_iface()
                .is_goal_reached(&self.world.fast_grid, prepass.goal_target_info);
            let transition_increment_nonzero = {
                let increment = entity.position_iface().get_increment_map();
                increment.x != 0.0 || increment.y != 0.0
            };
            if transition_goal_reached && speed != 0.0 && transition_increment_nonzero {
                let should_snap = !entity.position_iface().is_deviated() && order_tolerance == 0.0;
                entity.position_iface_mut().zero_all_increments();
                tracing::trace!(
                    ?entity_id,
                    ?anim,
                    ?goal,
                    should_snap,
                    from = ?entity.element_data().position_map(),
                    "transition goal reached"
                );
                if should_snap {
                    entity.element_data_mut().set_position_map(goal);
                }
                if next_destination_same_action.is_some() {
                    motion_state = MotionState::Terminated;
                }
            }
            // Actor::Hourglass runs CheckForLineCrossing after Execute
            // returns, so the segment endpoint is resolved from the live
            // position at dispatch time. A TillLastFrame step may
            // overshoot and snap back to its goal; the discarded
            // overshoot must not trigger a boundary.
            if let Some((old_pos, layer, eligible)) = transition_crossing_start
                && eligible
            {
                deferred.line_cross_checks.push((entity_id, old_pos, layer));
                deferred
                    .non_elevation_cross_checks
                    .push((entity_id, old_pos, layer));
            }
            let transition_effect_motion = movement_execute_visible_motion(
                order_action,
                motion_state,
                false,
                entity_target_seek,
            );
            if let Some((posture, next_action_state)) =
                movement_execute_state_effect(order_action, transition_effect_motion)
            {
                // A speed-transition completion establishes the live
                // walking/running state itself. Do not let an older state
                // saved by a preceding door transition overwrite it when
                // the generated continuation order executes next tick.
                if next_action_state.is_moving()
                    && let Some(pass) = entity
                        .actor_data_mut()
                        .and_then(|actor| actor.active_door_pass.as_mut())
                {
                    pass.saved_action_state = None;
                }
                deferred
                    .movement_state_effects
                    .push((entity_id, posture, next_action_state));
            }
            let door_transition_state_effect_due = matches!(motion_state, MotionState::Terminated)
                || matches!(motion_state, MotionState::Done)
                    && matches!(
                        anim,
                        OrderType::TransitionClimbingLadderDownWaitingUpright
                            | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
                    );
            if pass_door_transition_completion_has_owner(
                selected_command,
                door_pass_anim.is_some()
                    || (legacy_serialized_order_chain
                        && selected_command == crate::element::Command::PassDoor),
                order_action,
                is_pc,
            ) && door_transition_state_effect_due
                && matches!(
                    anim,
                    OrderType::TransitionWaitingUprightClimbingWallUp
                        | OrderType::TransitionWaitingCrouchedClimbingLadderDown
                        | OrderType::TransitionWaitingUprightClimbingLadderDownAlerted
                        | OrderType::TransitionClimbingLadderDownWaitingUpright
                        | OrderType::TransitionClimbingLadderDownWaitingUprightAlerted
                        | OrderType::TransitionClimbingWallUpWaitingCrouched
                        | OrderType::TransitionClimbingWallUpWaitingCrouchedCrenel
                        | OrderType::TransitionWaitingCrouchedClimbingWallDown
                        | OrderType::TransitionWaitingCrouchedClimbingWallDownCrenel
                        | OrderType::TransitionClimbingWallDownWaitingUpright
                )
            {
                deferred
                    .door_pass_transition_completion_effects
                    .push((entity_id, order_action));
            }
            if matches!(motion_state, MotionState::Terminated) {
                if let Some((external_direction, movement_direction)) =
                    terminal_pc_external_direction_goal
                {
                    deferred.terminal_pc_direction_goal_restores.push((
                        entity_id,
                        external_direction,
                        movement_direction,
                    ));
                }
                // TillLastFrame can exhaust its animation before its
                // distance target is reached (notably the short
                // Waiting→Walking startup transition). The Original does
                // not discard that remaining distance: it copies the
                // current order at the first following animation change,
                // changes the copy to that next animation, then retires
                // the exhausted transition. This keeps the copied order's
                // old target as a one-tick continuation.
                if !transition_goal_reached {
                    // Original's movement element already contains the
                    // whole PassDoor route. Rust keeps the untranslated
                    // tail on ActiveDoorPass, so the next distinct
                    // destination animation may live there rather than in
                    // `element.orders`.
                    let lazy_next_animation = entity
                        .actor_data()
                        .and_then(|actor| actor.active_door_pass.as_ref())
                        .and_then(|pass| {
                            pass.steps.iter().find_map(|step| match step {
                                crate::element::DoorPassStep::Walk {
                                    destination,
                                    action,
                                    ..
                                } if *destination != MapPoint::ZERO && *action != order_action => {
                                    Some(*action)
                                }
                                _ => None,
                            })
                        });
                    let next_order_id = &mut self.orders.next_order_id;
                    let concrete_door_prefix = if lazy_next_animation.is_some() {
                        entity
                            .actor_data_mut()
                            .and_then(|actor| actor.active_door_pass.as_mut())
                            .map(|pass| materialize_door_action_point_prefix(pass, next_order_id))
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let mut continuation_door_action = None;
                    let mut discard_lazy_door_followers = false;
                    if let Some(element) = self
                        .orders
                        .sequence_manager
                        .get_element_mut(move_seq_id, move_elem_idx)
                    {
                        for order in concrete_door_prefix {
                            element.push_order(order);
                        }
                        let current_action = element
                            .orders
                            .front()
                            .expect("terminated movement transition lost its current order")
                            .order_type;
                        let next_animation = element
                            .orders
                            .iter()
                            .enumerate()
                            .skip(1)
                            .find(|(_, order)| {
                                order.order_type != current_action
                                    && (order.target_x != 0.0 || order.target_y != 0.0)
                            })
                            .map(|(index, order)| (index, order.order_type));
                        let next_animation = next_animation.or_else(|| {
                            lazy_next_animation.map(|animation| (element.orders.len(), animation))
                        });
                        if let Some((insertion, animation)) = next_animation {
                            let mut continuation = element.orders.front().unwrap().clone();
                            continuation.order_type = animation;
                            // PerformMotion(TILL_LAST_FRAME) can exhaust
                            // the transition animation before reaching its
                            // distance target. Original inserts this
                            // changed-animation copy from inside that same
                            // PerformMotion call. Defer the copy's START
                            // state effect until we know whether the copy
                            // actually survives its first Execute.
                            continuation.transition_distance_continuation = true;
                            continuation.reseed_id(crate::order::alloc_order_id(next_order_id));
                            continuation_door_action = Some((animation, continuation.reverse));
                            element.insert_order(insertion, continuation);
                            if should_defer_pc_movement_state_start(is_pc, entity_target_seek)
                                && let Some(authored_successor) =
                                    element.orders.get_mut(insertion + 1)
                            {
                                assert_eq!(
                                    authored_successor.order_type, animation,
                                    "transition-distance continuation must precede the authored order whose animation it continues"
                                );
                                authored_successor.deferred_movement_state_start = true;
                            }
                        } else {
                            element.orders.truncate(1);
                            discard_lazy_door_followers = true;
                        }
                    }
                    if discard_lazy_door_followers {
                        discard_lazy_door_pass_following_orders(
                            entity
                                .actor_data_mut()
                                .and_then(|actor| actor.active_door_pass.as_mut()),
                        );
                        discarded_lazy_door_followers = true;
                    }
                    // Original stores the complete translated door route
                    // in the movement element, so changing to this copied
                    // successor changes the one authoritative current
                    // action. Rust keeps the untranslated route tail in a
                    // parallel ActiveDoorPass. Keep its animation mirror
                    // in lockstep with the concrete continuation order:
                    // lift handling and the next Execute slot both consult
                    // it before dispatching sprite motion.
                    if let Some((animation, reverse)) = continuation_door_action
                        && let Some(pass) = entity
                            .actor_data_mut()
                            .and_then(|actor| actor.active_door_pass.as_mut())
                    {
                        pass.current_action = animation;
                        pass.current_reverse = reverse;
                    }
                }
                let eid = entity_id;
                // PerformSeek wraps the transition animation too. When
                // the last stop transition terminates, Original checks
                // the live target before retiring the movement: an
                // unchanged same-sector target completes the seek and
                // starts its actor-owned post-seek interaction.
                //
                // The ordinary walking-arrival path below performs this
                // same check, but transition animations return through
                // this earlier branch and must close the handoff here.
                // PerformMotion(TILL_LAST_FRAME) may have deleted every
                // same-animation follower above after looping short of the
                // current destination. Original PerformSeek asks
                // GetNextOrder() only after that synchronous cleanup, so
                // the just-truncated current order is now the final
                // waypoint even when it was not final at Execute entry.
                let is_final_waypoint_after_transition_cleanup = self
                    .orders
                    .sequence_manager
                    .get_element(move_seq_id, move_elem_idx)
                    .is_none_or(|element| element.orders.len() <= 1);
                let movement_is_last_sequence_element = self
                    .orders
                    .sequence_manager
                    .get_sequence(move_seq_id)
                    .map(|sequence| move_elem_idx + 1 >= sequence.elements.len())
                    .unwrap_or(false);
                let final_entity_seek_arrival = if is_final_waypoint_after_transition_cleanup
                    && movement_is_last_sequence_element
                    && ft.target_id.is_some()
                {
                    live_seek_target.map(|(target_position, target_sector, _)| {
                        let same_sector = target_sector.is_some()
                            && target_sector == entity.element_data().sector();
                        let target_unchanged = target_position == ft.last_seek_target_position;
                        same_sector && (target_unchanged || tolerance_arrival)
                    })
                } else {
                    None
                };
                if final_entity_seek_arrival == Some(false) {
                    // PerformSeek reports this frame as still in progress
                    // once it decides to refresh, so the Execute arm never
                    // reaches the switch that would retire the actor into
                    // its waiting state. Drop the effect the terminating
                    // transition queued a moment ago; leaving it applied
                    // strands the actor at a standstill, and the refresh
                    // then reads that as a walk rather than the run it was
                    // already doing.
                    deferred
                        .movement_state_effects
                        .retain(|(id, _, _)| *id != eid);
                    deferred
                        .transition_seek_refreshes
                        .push((eid, move_seq_id, move_elem_idx));
                    return;
                }
                // PerformMotion(TILL_LAST_FRAME) can mutate the order list
                // before returning TERMINATED: when a startup transition
                // loops short of its destination, it inserts a copied order
                // using the next distinct animation. PerformSeek then reads
                // that live successor, just like it does after an ordinary
                // walking waypoint, and rejects an out-of-reach stop
                // transition when the seek target moved in the meantime
                // (RHelementactor.cpp:7974-8007, RHsprite.cpp:1849-1925).
                if !is_final_waypoint_after_transition_cleanup
                    && let Some((target_position, _, target_point)) = live_seek_target
                    && target_position != ft.last_seek_target_position
                    && let Some(next_action) = self
                        .orders
                        .sequence_manager
                        .get_element(move_seq_id, move_elem_idx)
                        .and_then(|element| element.orders.get(1))
                        .map(|order| order.order_type)
                    && matches!(
                        next_action,
                        OrderType::TransitionRunningUprightWaitingUpright
                            | OrderType::TransitionWalkingUprightWaitingUpright
                            | OrderType::TransitionWalkingCrouchedWaitingCrouched
                    )
                {
                    let aim = target_point.unwrap_or(target_position);
                    let here = entity.element_data().position_map();
                    let dx = aim.x - here.x;
                    let dy = if ft.directional {
                        const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
                        (aim.y - here.y) * INVERSE_ASPECT_RATIO
                    } else {
                        aim.y - here.y
                    };
                    let reach = (f32::from(entity.sprite().distance_for_animation(next_action))
                        + ft.tol)
                        * 1.05;
                    if dx * dx + dy * dy > reach * reach {
                        deferred
                            .movement_state_effects
                            .retain(|(id, _, _)| *id != eid);
                        deferred
                            .transition_seek_refreshes
                            .push((eid, move_seq_id, move_elem_idx));
                        tracing::trace!(
                            ?eid,
                            ?next_action,
                            reach,
                            "tick_move: looped transition exposed stale stop; refreshing seek",
                        );
                        return;
                    }
                }
                // A Hit can be attached to a Seek whose authored stop
                // transition uses up the last few map units before the
                // interaction.  Original terminates that transition at
                // this boundary, then the HITTING init guard rejects an
                // antagonist still farther than 40 map units away.  The
                // rejected post-seek never becomes the actor's visible
                // command at the frame boundary (Nescafe save controls:
                // 55.8 and 41.4 units respectively).  Rust previously
                // instructed the Hit during this same movement drain,
                // exposing one spurious HitCmd frame before its ordinary
                // next-Execute validity guard rejected it.
                let terminal_interaction = entity
                    .is_pc()
                    .then(|| {
                        actor_post_seek_interaction(entity.actor_data().expect("actor-only branch"))
                    })
                    .flatten();
                let terminal_interaction_out_of_range = final_entity_seek_arrival == Some(true)
                    // HITTING's rejected initialization is collapsed at this
                    // boundary by the controls above. TYING is different:
                    // Original publishes the newly instructed Tying order as
                    // IN_PROGRESS first, and its Execute-time position check
                    // cannot run until the actor's following Hourglass.
                    && terminal_interaction == Some(ActorPostSeekInteraction::Hit)
                    && live_seek_target
                        .map(|(target_position, _, _)| {
                            let here = entity.element_data().position_map();
                            interaction_exceeds_init_range(here, target_position)
                        })
                        .unwrap_or(false);
                if terminal_interaction_out_of_range {
                    // HITTING initialization turns toward its antagonist
                    // before the validity check which aborts it
                    // (`RHelementactorhuman.cpp:4462-4472`).
                    let target_ground = live_seek_target_ground
                        .expect("terminal entity seek retained its target ground position");
                    let here_ground = entity.ground_position();
                    let facing = vector_to_sector_0_to_15(
                        target_ground.x - here_ground.x,
                        target_ground.y - here_ground.y,
                    );
                    entity.element_data_mut().set_direction_goal(facing);

                    // PC-only: these replay controls use the PC Hit arm,
                    // whose invalid interaction has no NPC Think/AI
                    // continuation. Keep NPC post-seek lifecycle on the
                    // ordinary sequence-manager path.
                    let actor = entity.actor_data_mut().expect("actor-only branch");
                    // StartPostSeekSequence clears the seek ownership and
                    // folds its overloaded wait scalar before HITTING is
                    // instructed; the later ABORTED result does not
                    // restore any of it. Mirror that pre-abort teardown.
                    actor.wait_time = actor.seek_refresh_wait;
                    actor.seek_target = None;
                    actor.post_seek_sequence = None;
                    actor.clear_path();
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    deferred.order_pops.push((move_seq_id, move_elem_idx));
                    return;
                }
                if final_entity_seek_arrival == Some(true) {
                    let actor = entity.actor_data_mut().expect("actor-only branch");
                    if actor.post_seek_sequence.is_some() && actor.active_door_pass.is_none() {
                        deferred
                            .post_seek_arrivals
                            .push((eid, move_seq_id, move_elem_idx));
                        actor.clear_path();
                        actor.active_movement.clear();
                        actor.active_door_pass = None;
                    } else {
                        // No action consumes the arrival yet. Match
                        // PerformSeek's frozen refresh arm rather than
                        // exhausting the final transition order.
                        actor.seek_refresh_wait = 0;
                    }
                    return;
                }
                // Point-target Seek reaches this early transition arm
                // after its authored stop transition terminates. Original
                // only starts the post-seek when mpSeekSector still equals
                // the actor's sector; a player Stop can terminate the
                // transition short of that sector.
                // Retiring it through the ordinary order-pop path first
                // creates a fallback Wait and leaves the post-seek action
                // stranded on ActorData for one frame (or forever).
                let final_point_post_seek_arrival = is_final_waypoint_after_transition_cleanup
                    && ft.target_id.is_none()
                    && prepass
                        .point_seek_post_sector
                        .map(|seek_sector| entity.element_data().sector() == Some(seek_sector))
                        .unwrap_or(false)
                    && entity
                        .actor_data()
                        .is_some_and(|actor| actor.post_seek_sequence.is_some())
                    && entity
                        .actor_data()
                        .is_some_and(|actor| actor.active_door_pass.is_none());
                // A copied terminal stop transition can lose the movement
                // element's target while Actor::PerformSeek still owns the
                // entity in mpSeekTarget. That remains entity-seek mode, not
                // point-seek mode: on the last order Original starts the
                // post-seek when the target is still in our sector and either
                // has not moved since RefreshSeek or is inside tolerance.
                let final_actor_entity_post_seek_arrival = is_final_waypoint
                    && movement_is_last_sequence_element
                    && ft.target_id.is_none()
                    && actor_seek_flags.contains(crate::sequence::MoveFlags::SEEK)
                    && live_actor_seek_target.is_some_and(
                        |(_, _, target_sector, target_unchanged_or_in_tolerance)| {
                            target_sector.is_some()
                                && target_sector == entity.element_data().sector()
                                && target_unchanged_or_in_tolerance
                        },
                    )
                    && entity
                        .actor_data()
                        .is_some_and(|actor| actor.post_seek_sequence.is_some())
                    && entity
                        .actor_data()
                        .is_some_and(|actor| actor.active_door_pass.is_none());
                let final_actor_owned_post_seek_arrival =
                    final_point_post_seek_arrival || final_actor_entity_post_seek_arrival;
                let actor_owned_interaction = entity
                    .is_pc()
                    .then(|| {
                        actor_post_seek_interaction(entity.actor_data().expect("actor-only branch"))
                    })
                    .flatten();
                let actor_owned_interaction_out_of_range = final_actor_owned_post_seek_arrival
                    && actor_owned_interaction == Some(ActorPostSeekInteraction::Hit)
                    && live_actor_seek_target
                        .map(|(target_position, _, _, _)| {
                            interaction_exceeds_init_range(
                                entity.element_data().position_map(),
                                target_position,
                            )
                        })
                        .unwrap_or(false);
                if actor_owned_interaction_out_of_range {
                    // A copied terminal transition can lose its movement
                    // element target while PerformSeek's mpSeekTarget
                    // remains actor-owned. HITTING still turns before its
                    // validity abort.
                    let (_, target_ground, _, _) = live_actor_seek_target
                        .expect("out-of-range actor-owned Hit retained a live target");
                    let here_ground = entity.ground_position();
                    let facing = vector_to_sector_0_to_15(
                        target_ground.x - here_ground.x,
                        target_ground.y - here_ground.y,
                    );
                    entity.element_data_mut().set_direction_goal(facing);

                    let actor = entity.actor_data_mut().expect("actor-only branch");
                    actor.wait_time = actor.seek_refresh_wait;
                    actor.seek_target = None;
                    actor.post_seek_sequence = None;
                    actor.clear_path();
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    deferred.order_pops.push((move_seq_id, move_elem_idx));
                    return;
                }
                let actor = entity.actor_data_mut().expect("actor-only branch");
                if final_actor_owned_post_seek_arrival {
                    deferred
                        .post_seek_arrivals
                        .push((eid, move_seq_id, move_elem_idx));
                    actor.clear_path();
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    return;
                }
                // Pop via the element we actually dispatched (`move_seq_id` /
                // `move_elem_idx`), not `actor.active_movement.sequence_id`
                // — the latter can be stale/None when the Move element was
                // launched by the AI without setting active_movement
                // (soldier chase paths).
                deferred.order_pops.push((move_seq_id, move_elem_idx));
                // Last order of the Move element just completed — flip
                // back to Waiting and clear the active movement.
                // Matches the `DoorPassAdvance::Done` arm below but for
                // the transition-terminated path.
                if is_final_waypoint {
                    let mut clear_completed_movement_goal = false;
                    let advance = if actor.active_door_pass.is_some() {
                        Self::advance_door_pass(
                            actor,
                            eid,
                            goal,
                            &mut deferred.door_triggers,
                            &mut deferred.select_triggers,
                            &mut self.orders.next_order_id,
                        )
                    } else {
                        DoorPassAdvance::Done { completed: None }
                    };
                    match advance {
                        DoorPassAdvance::Continue {
                            destination,
                            action,
                            reverse,
                            compute_direction,
                            tolerance,
                        } => {
                            let order_id =
                                crate::order::alloc_order_id(&mut self.orders.next_order_id);
                            let mut order = crate::order::Order::new(
                                action,
                                destination.x,
                                destination.y,
                                order_id,
                            );
                            order.reverse = reverse;
                            order.compute_direction = compute_direction;
                            order.tolerance = tolerance;
                            deferred
                                .transition_pushes
                                .push((move_seq_id, move_elem_idx, order));
                        }
                        DoorPassAdvance::Paused { transition_order } => {
                            deferred.transition_pushes.push((
                                move_seq_id,
                                move_elem_idx,
                                transition_order,
                            ));
                        }
                        DoorPassAdvance::ActionPoint { order } => {
                            deferred
                                .transition_pushes
                                .push((move_seq_id, move_elem_idx, order));
                        }
                        DoorPassAdvance::Done { completed } => {
                            if let Some((door_index, direct)) = completed_door_pass_to_commit(
                                discarded_lazy_door_followers,
                                completed,
                            ) {
                                deferred
                                    .completed_door_passes
                                    .push((eid, door_index, direct));
                            }
                            actor.clear_path();
                            actor.action_state =
                                if is_swordfighting || actor.action_state.is_sword() {
                                    crate::element::ActionState::WaitingSword
                                } else {
                                    crate::element::ActionState::Waiting
                                };
                            actor.active_movement.clear();
                            actor.active_door_pass = None;
                            clear_completed_movement_goal = true;
                        }
                        DoorPassAdvance::NoActive => {
                            tracing::warn!(
                                entity = ?eid,
                                "DoorPass: transition-terminated movement lost active pass"
                            );
                        }
                    }
                    if clear_completed_movement_goal {
                        deferred.terminal_door_pass_goal_clears.push(eid);
                    }
                }
            }
            return;
        }

        // Zero-distance animation ticks are still real PerformSeek /
        // PerformMotion calls. The pre-motion tolerance branch and an
        // ordinary order whose destination already equals the actor's
        // position both complete without sprite displacement. In
        // particular, a freshly initialized exact-position walk returns
        // TERMINATED in Original on that first Execute. Only defer a
        // genuinely stationary motion that has not reached its goal.
        if stationary_motion_waits(speed, tolerance_arrival, dist) {
            if let Some((posture, next_action_state)) =
                movement_execute_state_effect(order_action, state_effect_motion)
            {
                deferred
                    .movement_state_effects
                    .push((entity_id, posture, next_action_state));
            }
            return;
        }

        tracing::trace!(
            "tick_move: entity={:?} pos=({:.0},{:.0}) goal=({:.0},{:.0}) speed={speed:.1} action={:?} state={:?}",
            entity_id,
            elem.position_map().x,
            elem.position_map().y,
            goal.x,
            goal.y,
            anim,
            action_state,
        );

        // Snapshot the pre-move position + layer + posture so we
        // can run the elevation-line-crossing check after the
        // position is updated. Original excludes flying actors and
        // humans with a live carrier; wall/ladder climbers and actors
        // carrying somebody else remain eligible.
        let old_pos = elem.position_map();
        // Actor::Hourglass snapshots mpPosition before Execute and uses that
        // outer position for CheckForLineCrossing after Execute returns.
        // RunningStairs performs two literal motion commits inside Execute;
        // by this point `old_pos` is already the post-first-commit position.
        // Retain the outer snapshot so a bond crossed only by the first
        // substep is not lost.
        let crossing_old_pos = if first_fast_commit.is_some() {
            fast_motion_outer_pre
        } else {
            old_pos
        };
        let entity_layer = elem.layer();
        let entity_posture = elem.posture;
        let eligible_for_crossing = actor_line_crossing_eligible(
            entity_posture,
            human_is_carried,
            self.world
                .fast_grid
                .level
                .map_bbox
                .contains_point(crossing_old_pos),
        );

        // Seek-arrival predicate:
        //
        //   - dist_sq = squared distance (target - pos), with Y
        //     stretched by the inverse aspect ratio (≈1.7434)
        //     when DIRECTIONAL_TOLERANCE is set (used for net
        //     pickup).
        //   - Arrive iff target.sector == self.sector AND
        //     dist_sq < tolerance² × 1.1025 (the "5% tolerance"
        //     margin baked into the squared comparison).
        //
        // The check runs every tick (not just the last waypoint),
        // so a moving target that wanders into range mid-route
        // ends the seek immediately and the post-seek sequence
        // fires.  The pre-pass only populates `FinalTol` for
        // SEEK-flagged movements with a resolvable target (entity
        // or shield destination), so `ft.tol > 0` is the
        // live-seek gate; non-seek elements skip this branch
        // entirely and fall through to the standard `dist <=
        // speed` arrival.  USE_POINT samples the target's current
        // hotspot; SEEK_SHIELD uses the movement element
        // destination.
        let ft = prepass.final_tolerance;
        let mut point_seek_post_arrival = is_final_waypoint
            && dist <= speed
            && prepass
                .point_seek_post_sector
                .map(|seek_sector| elem.sector() == Some(seek_sector))
                .unwrap_or(false);
        // FROZEN stand-still wait.  When the seek arrival
        // predicate fires at an intermediate waypoint and there
        // is no `post_seek_sequence` to consume the arrival, the
        // actor freezes its sprite frame in place near the target
        // until either the target moves out of tolerance
        // (next-tick `tick_refresh_seeks` detects drift and
        // rebuilds the path) or a post-seek is later attached.
        // We honour this by simply skipping the per-tick movement
        // step (no order pop, no position update, no sprite
        // advance) so the actor's position + orders persist for
        // the next tick to re-evaluate.
        //
        // This branch only fires for entity-target seeks without
        // a queued post-seek interaction (e.g. AI follow seeks
        // built outside `apply_interaction_with_seek`).  The
        // common PC interaction path always carries a post-seek
        // and routes through the `start_post_seek` branch below
        // instead.
        let frozen_seek_wait = tolerance_arrival && !is_final_waypoint && !ft.has_post_seek;
        if frozen_seek_wait {
            tracing::trace!(
                entity = ?entity_id,
                "tick_move: FROZEN seek wait (target in range, no post-seek, mid-path)",
            );
            refresh_pc_walking_shield_after_execute(entity, &assets.profile_manager, order_action);
            return;
        }

        // Original entity-target `PerformSeek` samples its live-target
        // tolerance before `PerformMotion`. If already in range it takes
        // the frozen/post-seek arm without committing a movement step.
        // If this frame's step merely crosses into range, that new
        // distance is not sampled until the next actor Hourglass.
        //
        // Ordinary waypoint arrival is different: PerformMotion commits
        // through UpdatePositionAntiCollision and then asks
        // IsGoalReached. Re-enter the shared tail after that commit while
        // retaining (not recomputing) the pre-motion seek predicate.
        let mut post_step_arrival = dist <= f32::EPSILON || tolerance_arrival;
        let mut arrived_after_committed_step = false;
        let mut arrival_crossing_queued = false;
        'arrival: loop {
            if post_step_arrival {
                // Original PerformMotion/PerformSeek returns TERMINATED
                // after committing the step which reaches the goal. Rust
                // stages geometry after the sprite call, so its raw
                // motion state can still be DONE here. Queue the Human
                // Execute termination callback at the authoritative
                // arrival boundary; it owns the range-based Provoke
                // launched after sword movement.
                // Reached waypoint — snap to it and advance. Original's
                // ordinary PerformMotion snap lives inside `if (bMoving)`;
                // its TillLastFrame equivalent requires nonzero distance
                // and increment. If an order starts at its exact goal,
                // consume it without needlessly recomputing map -> 3D.
                if should_snap_arrival(
                    arrived_after_committed_step,
                    tolerance_arrival,
                    order_tolerance,
                    entity.position_iface().is_deviated(),
                ) {
                    entity
                        .element_data_mut()
                        .set_position_map(crate::coordinates::MapPoint {
                            x: goal.x,
                            y: goal.y,
                        });
                }
                let eid = entity_id;
                arrival_crossing_queued |= queue_committed_arrival_crossing(
                    deferred,
                    eid,
                    crossing_old_pos,
                    entity_layer,
                    arrived_after_committed_step,
                    eligible_for_crossing,
                );

                // A final concrete waypoint is only the position at
                // which the target was observed when this Seek was
                // built.  When the walking order terminates, Original
                // PerformSeek validates that stale waypoint against the
                // live target before it may hand off to the post-seek
                // action:
                //
                //   same sector
                //   && (target has not moved || live target is in range)
                //
                // If that check fails, RefreshSeek replaces the movement
                // immediately and the exhausted old order must not reach
                // generic `do_next_order` (which would launch the Hit /
                // interaction tail unconditionally).
                let movement_is_last_sequence_element = self
                    .orders
                    .sequence_manager
                    .get_sequence(move_seq_id)
                    .map(|sequence| move_elem_idx + 1 >= sequence.elements.len())
                    .unwrap_or(false);
                let final_entity_seek_arrival = if is_final_waypoint
                    && movement_is_last_sequence_element
                    && ft.target_id.is_some()
                {
                    live_seek_target.map(|(target_position, target_sector, _)| {
                        let same_sector = target_sector.is_some()
                            && target_sector == entity.element_data().sector();
                        let target_unchanged = target_position == ft.last_seek_target_position;
                        same_sector && (target_unchanged || tolerance_arrival)
                    })
                } else {
                    None
                };
                if final_entity_seek_arrival == Some(false) {
                    deferred
                        .transition_seek_refreshes
                        .push((eid, move_seq_id, move_elem_idx));
                    tracing::trace!(
                        ?eid,
                        "tick_move: final seek waypoint is stale; refreshing against live target",
                    );
                    refresh_pc_walking_shield_after_execute(
                        entity,
                        &assets.profile_manager,
                        order_action,
                    );
                    return;
                }

                // The sibling case, where a stop transition is still
                // queued behind the movement order that just terminated.
                // A transition covers its own animation distance, so it
                // may only take over when the live target sits within
                // that travel plus the seek distance. A target that has
                // drifted beyond it refreshes the seek instead, and the
                // stale transition never plays.
                if !is_final_waypoint
                    && let Some((target_position, _, target_point)) = live_seek_target
                    && target_position != ft.last_seek_target_position
                    && let Some(next_action) = self
                        .orders
                        .sequence_manager
                        .get_element(move_seq_id, move_elem_idx)
                        .and_then(|element| element.orders.get(1))
                        .map(|order| order.order_type)
                    && matches!(
                        next_action,
                        OrderType::TransitionRunningUprightWaitingUpright
                            | OrderType::TransitionWalkingUprightWaitingUpright
                            | OrderType::TransitionWalkingCrouchedWaitingCrouched
                    )
                {
                    let aim = target_point.unwrap_or(target_position);
                    let here = entity.element_data().position_map();
                    let dx = aim.x - here.x;
                    let dy = if ft.directional {
                        const INVERSE_ASPECT_RATIO: f32 = 1.743_446_8;
                        (aim.y - here.y) * INVERSE_ASPECT_RATIO
                    } else {
                        aim.y - here.y
                    };
                    let reach = (f32::from(entity.sprite().distance_for_animation(next_action))
                        + ft.tol)
                        * 1.05;
                    if dx * dx + dy * dy > reach * reach {
                        // PerformMotion already committed this frame's
                        // step before PerformSeek decided to refresh.
                        // Actor::Hourglass still runs
                        // CheckForLineCrossing after Execute returns, so
                        // preserve the segment even though the refreshed
                        // seek replaces the current movement before the
                        // crossing callback.
                        if eligible_for_crossing {
                            deferred
                                .line_cross_checks
                                .push((eid, crossing_old_pos, entity_layer));
                            deferred.non_elevation_cross_checks.push((
                                eid,
                                crossing_old_pos,
                                entity_layer,
                            ));
                        }
                        deferred
                            .transition_seek_refreshes
                            .push((eid, move_seq_id, move_elem_idx));
                        tracing::trace!(
                            ?eid,
                            ?next_action,
                            reach,
                            "tick_move: seek target out of stop-transition reach; refreshing",
                        );
                        refresh_pc_walking_shield_after_execute(
                            entity,
                            &assets.profile_manager,
                            order_action,
                        );
                        return;
                    }
                }

                let actor = entity.actor_data_mut().unwrap();
                // The post-seek sequence fires whenever the seek
                // arrival predicate is true and a post-seek sequence
                // is attached — no final-waypoint gate.  The
                // `tolerance_arrival` guard above already enforces the
                // post-seek requirement for intermediate waypoints, so
                // reaching this point with both flags set is the
                // "terminate the seek and launch the post-seek" path.
                let start_post_seek = (tolerance_arrival
                    || point_seek_post_arrival
                    || final_entity_seek_arrival == Some(true))
                    && actor.post_seek_sequence.is_some();
                let start_post_seek = if start_post_seek && actor.active_door_pass.is_some() {
                    tracing::warn!(
                        entity = ?eid,
                        "DoorPass: suppressing post-seek teardown during active pass"
                    );
                    false
                } else {
                    start_post_seek
                };

                if is_sword_motion
                    && perform_seek_exposes_motion_termination(
                        start_post_seek,
                        final_entity_seek_arrival,
                    )
                {
                    deferred.sword_movement_terminations.push(entity_id);
                }

                // Waypoint reached — queue a `do_next_order` pop on
                // the actor's Move element.
                if start_post_seek {
                    deferred
                        .post_seek_arrivals
                        .push((eid, move_seq_id, move_elem_idx));
                } else {
                    deferred.order_pops.push((move_seq_id, move_elem_idx));
                }

                if start_post_seek {
                    // StartPostSeekSequence makes PerformSeek return
                    // TERMINATED, so Human::Execute observes the sword
                    // movement completion before Actor::Hourglass advances
                    // the selected element.
                    actor.clear_path();
                    // Original StartPostSeekSequence terminates the seek
                    // and launches the interaction without rewriting the
                    // actor state. The interaction's generated transition
                    // owns any later Moving→Waiting change.
                    actor.active_movement.clear();
                    actor.active_door_pass = None;
                    if is_sword_motion && let Some(human) = entity.human_data_mut() {
                        human.last_motion_was_step_back_in_combat = active_move_flags
                            .contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT);
                    }
                    refresh_pc_walking_shield_after_execute(
                        entity,
                        &assets.profile_manager,
                        order_action,
                    );
                    return;
                }

                // With no post-seek tail, the successful final
                // entity-target arrival remains inside PerformSeek.  It
                // arms an immediate refresh check and returns InProgress
                // instead of consuming the final order.
                if final_entity_seek_arrival == Some(true) {
                    actor.seek_refresh_wait = 0;
                    refresh_pc_walking_shield_after_execute(
                        entity,
                        &assets.profile_manager,
                        order_action,
                    );
                    return;
                }

                if is_final_waypoint {
                    // All waypoints for current walk step consumed.
                    // Check if we have more door-pass steps.
                    let advance = if actor.active_door_pass.is_some() {
                        Self::advance_door_pass(
                            actor,
                            eid,
                            goal,
                            &mut deferred.door_triggers,
                            &mut deferred.select_triggers,
                            &mut self.orders.next_order_id,
                        )
                    } else {
                        DoorPassAdvance::Done { completed: None }
                    };

                    match advance {
                        DoorPassAdvance::Continue {
                            destination,
                            action,
                            reverse,
                            compute_direction,
                            tolerance,
                        } => {
                            // Push a walking order for the new Walk
                            // step onto the actor's current sequence
                            // element, to be installed after the
                            // entity loop closes (same deferred
                            // mechanism as Transition steps).
                            let order_id =
                                crate::order::alloc_order_id(&mut self.orders.next_order_id);
                            let mut order = crate::order::Order::new(
                                action,
                                destination.x,
                                destination.y,
                                order_id,
                            );
                            order.reverse = reverse;
                            order.compute_direction = compute_direction;
                            order.tolerance = tolerance;
                            deferred
                                .transition_pushes
                                .push((move_seq_id, move_elem_idx, order));
                        }
                        DoorPassAdvance::Paused { transition_order } => {
                            // Transition animation queued — push the
                            // order onto the actor's current sequence
                            // element after the loop closes.
                            deferred.transition_pushes.push((
                                move_seq_id,
                                move_elem_idx,
                                transition_order,
                            ));
                        }
                        DoorPassAdvance::ActionPoint { order } => {
                            deferred
                                .transition_pushes
                                .push((move_seq_id, move_elem_idx, order));
                        }
                        DoorPassAdvance::Done { completed } => {
                            if let Some((door_index, direct)) = completed {
                                deferred
                                    .completed_door_passes
                                    .push((eid, door_index, direct));
                            }
                            // Final waypoint's do_next_order pop was
                            // already collected above when
                            // `path_waypoint_index` advanced past the
                            // end of the list; that pop will either
                            // drain the Move element entirely
                            // (triggering `element_terminated` +
                            // `ensure_wait_element` internally) or
                            // leave an end-transition order as the
                            // new current, which the animation driver
                            // will play next tick.
                            actor.clear_path();
                            // Keep the movement action state until an
                            // optional end transition actually finishes.
                            // RHElementActor's walking Execute arm leaves
                            // MOVING unchanged on RHMOTION_TERMINATED; the
                            // transition-to-waiting arm performs the state
                            // change itself. The two PC carry-walk Execute
                            // overrides are exceptions: both explicitly
                            // restore WAITING on RHMOTION_TERMINATED even
                            // when the Move has NO_TRANSITIONS.
                            if matches!(
                                order_action,
                                OrderType::WalkingWithCorpse
                                    | OrderType::WalkingCarryingOnShoulders
                            ) {
                                actor.action_state = crate::element::ActionState::Waiting;
                            }
                            actor.active_movement.clear();
                            actor.active_door_pass = None;
                            if is_sword_motion && let Some(human) = entity.human_data_mut() {
                                human.last_motion_was_step_back_in_combat = active_move_flags
                                    .contains(crate::sequence::MoveFlags::STEP_BACK_IN_COMBAT);
                            }
                        }
                        DoorPassAdvance::NoActive => {
                            tracing::warn!(
                                entity = ?eid,
                                "DoorPass: final waypoint reached but active pass was already gone"
                            );
                        }
                    }
                }
                break 'arrival;
            } else {
                // Move toward waypoint.
                //
                // Actor-vs-actor anti-collision: deviate around other
                // actors' repulsive zones before committing the step.
                // Runs between the motion advance and the position
                // commit, gated on the mover's `anti_collision_on`
                // flag — the flag stays `true` by default so this is
                // active for every normal walk.
                // `PerformMotion` advances with PositionInterface's
                // cached `GetIncrementMap()`. It does not renormalize the
                // remaining goal vector each frame; that cache is rebuilt
                // only when a new order starts or anti-collision changes
                // deviation state. Recomputing here introduced tiny drift
                // into patrol-chief history and eventually flipped exact
                // transition-arrival dot products.
                let cached_increment = entity.position_iface().get_increment_map();
                let nx = cached_increment.x;
                let ny = cached_increment.y;
                let anti_on = entity.position_iface().is_anti_collision_on();
                let movement_diag_pre_position = if first_fast_commit.is_some() {
                    fast_motion_outer_pre
                } else {
                    entity.element_data().position_map()
                };
                let movement_diag_old_position = entity.position_iface().old_map_position();
                let movement_diag_deviated_before = entity.position_iface().is_deviated();
                let movement_diag_blocked_count_before = entity.position_iface().blocked_count;
                // Preserve the two storage roundings of Original's
                // double-PerformMotion fast-climb dispatch. See the
                // transition branch above for why the summed distance is
                // insufficient even when both calls use one increment.
                let split_motion_target = split_motion_speeds
                    .filter(|_| !anti_on && first_fast_commit.is_none())
                    .map(|(first_speed, second_speed)| {
                        let mut target = entity.element_data().position_map();
                        target.x += nx * first_speed;
                        target.y += ny * first_speed;
                        target.x += nx * second_speed;
                        target.y += ny * second_speed;
                        target
                    });
                // Pull transient anti-collision context from position_iface
                // (move box, half-diagonal) + the current path goal.  The
                // persistent state (deviated / blocked_count / box_blocked /
                // radius) lives on the actor's PI directly now.
                let (dx_step, dy_step, recovered_from_deviation, rebuild_after_deviation) =
                    if anti_on
                        && let Some(mover_snap) = anti_snapshots
                            .get(actor_id)
                            .and_then(|slot| slot.as_ref())
                            .filter(|snapshot| snapshot.active)
                    {
                        let goal_map = crate::coordinates::MapPoint::new(goal.x, goal.y);
                        let (move_box, half_diagonal) = {
                            let pi = entity.position_iface();
                            (*pi.get_move_box(), pi.get_half_diagonal())
                        };
                        let pi = entity.position_iface_mut();
                        let was_deviated = pi.is_deviated();
                        let mut state = super::anti_collision::AntiCollisionState {
                            pi,
                            move_box,
                            half_diagonal,
                            goal_map,
                        };
                        let (dx_step, dy_step) = apply_prepared_anti_collision_step(
                            provenance_frame,
                            mover_snap,
                            anti_snapshots,
                            &self.ai.global.repulsive_points,
                            prepared,
                            &self.world.fast_grid,
                            &mut state,
                            nx,
                            ny,
                            speed,
                            anti_on,
                        );
                        (
                            dx_step,
                            dy_step,
                            was_deviated && !state.pi.is_deviated(),
                            // A successfully committed deviation expands the
                            // blocked box, resets the counter, and Original
                            // rebuilds the cached increment. Its
                            // blocked-count break-through path instead uses
                            // MoveMap and deliberately retains the old cache.
                            state.pi.is_deviated() && state.pi.blocked_count == 0,
                        )
                    } else {
                        (nx * speed, ny * speed, false, false)
                    };
                let new_pos_x;
                let new_pos_y;
                {
                    let elem = entity.element_data_mut();
                    if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
                        // A committed deviation faces along the step it
                        // just took, then invalidates and reconstructs the
                        // cached increment from the new position to the
                        // original goal (the rebuild deliberately retains
                        // this direction rather than recomputing it).  The
                        // break-through barge sets its own facing inside
                        // the anti-collision step, so it is excluded here.
                        let raw = vector_to_sector_0_to_15(dx_step, dy_step);
                        elem.set_direction_goal(if order_reverse { raw ^ 8 } else { raw });
                    }
                    let pm = split_motion_target.unwrap_or_else(|| {
                        let mut pm = elem.position_map();
                        pm.x += dx_step;
                        pm.y += dy_step;
                        pm
                    });
                    elem.set_position_map(pm);
                    if rebuild_after_deviation && (dx_step != 0.0 || dy_step != 0.0) {
                        elem.sprite.position_iface.reset_increment_computed();
                        elem.sprite.position_iface.compute_increment_all(false);
                    } else if recovered_from_deviation {
                        // Original's no-new-deviation recovery branch commits
                        // the (possibly zero-length) step, clears
                        // `IsDeviated`, and rebuilds the increment with
                        // direction computation enabled.
                        elem.sprite.position_iface.reset_increment_computed();
                        elem.sprite.position_iface.compute_increment_all(true);
                    }
                    new_pos_x = pm.x;
                    new_pos_y = pm.y;
                }

                // Refresh the movement forecast used to lead moving
                // targets (arrow / stone / apple aiming).  This sits at
                // the same point as the position commit: after the
                // anti-collision step, using the effective distance and
                // the wait time of the frame the sprite has just
                // reached.  A blocked step aborts before reaching it.
                //
                // The fast climb arms commit two motion calls in one
                // tick; only the later one's distance survives in the
                // forecast, so prefer the second speed when it moved.
                refresh_motion_forecast(entity.sprite_mut(), speed, split_motion_speeds);

                // Water splash titbit emission.  Every walk tick
                // where `speed > 2` and the actor's cached material
                // is water, the sprite's splatter counter ticks up;
                // on `>= 2` a water particle is added at the actor's
                // 3D position and the counter resets.  Cosmetic but
                // observable — actors crossing a stream kick up
                // splash titbits.
                {
                    let elem = entity.element_data_mut();
                    if speed > 2.0 && elem.material() == crate::element::GameMaterial::Water {
                        if elem.sprite.splitch_count >= 2 {
                            elem.sprite.splitch_count = 0;
                            let pos = elem.position();
                            let layer = elem.layer();
                            deferred.water_splash_emits.push((
                                entity_id,
                                crate::coordinates::WorldPoint3D {
                                    x: pos.x,
                                    y: pos.y,
                                    z: pos.z,
                                },
                                layer,
                            ));
                        } else {
                            elem.sprite.splitch_count = elem.sprite.splitch_count.saturating_add(1);
                        }
                    }
                }

                // When the blocked counter trips, the motion aborts
                // and the backing sequence element is marked
                // Impossible.
                let movement_aborted = entity.position_iface().is_blocked();
                if movement_aborted {
                    let actor = entity.actor_data_mut().expect("actor-only branch");
                    if let Some(seq_id) = actor.active_movement.sequence_id {
                        deferred
                            .blocked_impossible
                            .push((seq_id, actor.active_movement.element_index));
                    }
                    let restore_anti_collision = {
                        let restore_anti_collision = actor.active_door_pass.is_some();
                        if restore_anti_collision {
                            tracing::warn!(
                                entity = ?entity_id,
                                "DoorPass: movement blocked; clearing active pass with aborted movement"
                            );
                            actor.active_door_pass = None;
                        }
                        actor.clear_path();
                        // The movement Execute switches have no ABORTED
                        // state arm. Actor::Hourglass marks the captured
                        // element Impossible, but the actor keeps whatever
                        // live state Execute established before returning.
                        // In particular a walking actor remains Moving;
                        // RunningUpright's unconditional Execute effect is
                        // applied below and still publishes MovingFast.
                        actor.active_movement.clear();
                        restore_anti_collision
                    };
                    if restore_anti_collision {
                        entity.position_iface_mut().set_anti_collision_on(true);
                    }
                    entity.position_iface_mut().reset_box_blocked();
                }

                // Sync the just-moved position back into the snapshot
                // so later actors in this tick see the serial
                // "already-moved" position of this one.  Without this
                // two actors heading for the same cell both see each
                // other at the *old* position and can still overlap.
                if let Some(snap) = anti_snapshots
                    .get_mut(actor_id)
                    .and_then(|slot| slot.as_mut())
                {
                    let new_pos = MapPoint::new(new_pos_x, new_pos_y);
                    super::anti_collision::sync_snapshot_after_move(
                        snap,
                        new_pos,
                        MapVec::new(dx_step, dy_step),
                    );
                }

                if movement_aborted {
                    break 'arrival;
                }

                // `UpdatePositionAntiCollision` has now committed the
                // ordinary frame and rebuilt the increment when deviation
                // changed.  This is the exact point where Original calls
                // `RHPositionInterface::IsGoalReached`.
                let movement_goal_reached = entity
                    .position_iface()
                    .is_goal_reached(&self.world.fast_grid, prepass.goal_target_info);
                let movement_diag_raw_post = entity.element_data().position_map();
                // PerformMotion snaps an undeviated zero-tolerance arrival
                // after IsGoalReached. Include that authoritative visible
                // result in the diagnostic while retaining the raw
                // anti-collision commit separately.
                let movement_diag_post = if movement_goal_reached
                    && order_tolerance == 0.0
                    && !entity.position_iface().is_deviated()
                {
                    goal
                } else {
                    movement_diag_raw_post
                };
                let movement_diag_split_calls =
                    if crate::movement_diagnostics::parity_movement_capture_active() {
                        split_motion_speeds.map_or_else(Vec::new, |(_, second_speed)| {
                            let mut calls = Vec::with_capacity(2);
                            if let Some((first_pre, first_increment, first_speed, first_post)) =
                                first_fast_commit
                            {
                                calls.push(crate::movement_diagnostics::ParityMovementCall {
                                    frame_distance_raw: first_frame_dist_raw.into(),
                                    effective_distance: first_speed.into(),
                                    pre_position: first_pre.into(),
                                    requested_delta: MapVec::new(
                                        first_increment.x * first_speed,
                                        first_increment.y * first_speed,
                                    )
                                    .into(),
                                    post_position: first_post.into(),
                                });
                            }
                            let (second_pre, second_increment) = second_fast_operands
                                .expect("split motion requires captured second-call operands");
                            calls.push(crate::movement_diagnostics::ParityMovementCall {
                                frame_distance_raw: second_frame_dist_raw
                                    .expect("split speeds require a second motion distance")
                                    .into(),
                                effective_distance: second_speed.into(),
                                pre_position: second_pre.into(),
                                requested_delta: MapVec::new(
                                    second_increment.x * second_speed,
                                    second_increment.y * second_speed,
                                )
                                .into(),
                                post_position: movement_diag_raw_post.into(),
                            });
                            calls
                        })
                    } else {
                        Vec::new()
                    };
                crate::movement_diagnostics::record_parity_movement_step(
                    crate::movement_diagnostics::ParityMovementStep {
                        entity: entity_id,
                        order_action: format!("{order_action:?}"),
                        animation: format!("{anim:?}"),
                        motion_method: format!("{motion_method:?}"),
                        pre_position: movement_diag_pre_position.into(),
                        old_position: movement_diag_old_position.into(),
                        goal: goal.into(),
                        cached_increment: cached_increment.into(),
                        frame_distance_raw: frame_dist_raw.into(),
                        speed_factor: speed_factor.into(),
                        speed_factor_applied: apply_speed_factor,
                        direction_differs_from_goal,
                        effective_distance: speed.into(),
                        anti_collision_on: anti_on,
                        deviated_before: movement_diag_deviated_before,
                        blocked_count_before: movement_diag_blocked_count_before,
                        requested_delta: crate::coordinates::MapVec::new(nx * speed, ny * speed)
                            .into(),
                        raw_committed_delta: (movement_diag_raw_post - movement_diag_pre_position)
                            .into(),
                        committed_delta: (movement_diag_post - movement_diag_pre_position).into(),
                        post_position: movement_diag_post.into(),
                        deviated_after: entity.position_iface().is_deviated(),
                        blocked_count_after: entity.position_iface().blocked_count,
                        goal_reached_after_commit: movement_goal_reached,
                        split_calls: movement_diag_split_calls,
                    },
                );
                point_seek_post_arrival = is_final_waypoint
                    && movement_goal_reached
                    && prepass
                        .point_seek_post_sector
                        .map(|seek_sector| entity.element_data().sector() == Some(seek_sector))
                        .unwrap_or(false);
                post_step_arrival = movement_goal_reached || tolerance_arrival;
                if post_step_arrival {
                    arrived_after_committed_step = true;
                    continue 'arrival;
                }
                break 'arrival;
            }
        }

        // RHElementActorPC::Execute updates the retained shield after
        // every WALKING_WITH_SHIELD PerformSeek/PerformMotion call,
        // including a tolerance-arrival frame with no displacement.
        refresh_pc_walking_shield_after_execute(entity, &assets.profile_manager, order_action);

        // Queue an elevation-line-cross check for this tick. The
        // actual fast-grid query + obstacle swap runs after the
        // loop, since `check_for_line_crossing` needs `&mut self`.
        //
        // Also queue a patch-line-cross check for PC actors —
        // LINE_PATCH handling is gated to PCs only.
        let new_pos = entity.element_data().position_map();
        let new_position_in_bounds = self.world.fast_grid.level.map_bbox.contains_point(new_pos);
        tracing::trace!(
            target: "robin_engine::elevation_crossing",
            ?entity_id,
            eligible_for_crossing,
            new_position_in_bounds,
            posture = ?entity_posture,
            human_is_carried,
            layer = entity_layer,
            old_x = crossing_old_pos.x,
            old_y = crossing_old_pos.y,
            new_x = new_pos.x,
            new_y = new_pos.y,
            "considered queuing elevation crossing"
        );
        if eligible_for_crossing && !arrival_crossing_queued {
            deferred
                .line_cross_checks
                .push((entity_id, crossing_old_pos, entity_layer));
            deferred
                .non_elevation_cross_checks
                .push((entity_id, crossing_old_pos, entity_layer));
        }
        // Order pops are drained after all actors so the current order is
        // still physically at the front here. Treat an already-queued
        // pop as a completed Execute when deciding whether a deferred
        // START survives this actor slot.
        let current_order_will_advance = deferred
            .order_pops
            .iter()
            .any(|&(seq_id, elem_idx)| seq_id == move_seq_id && elem_idx == move_elem_idx);
        // Ordinary walking START effects have the same survival rule as
        // generated transition-distance and deferred PC successors.
        // Original PerformMotion moves first and only then returns its
        // final motion state to Execute; when anti-collision deviation
        // lands inside the goal predicate on that first call, Execute
        // observes TERMINATED and must not briefly enter Moving.
        if matches!(state_effect_motion, MotionState::Start)
            && !deferred_movement_state_start_due
            && !transition_distance_first_execute_due
            && !current_order_will_advance
            && self
                .orders
                .sequence_manager
                .get_element(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.front())
                .is_some_and(|order| Some(order.order_id) == order_id)
            && let Some((posture, next_action_state)) =
                movement_execute_state_effect(order_action, MotionState::Start)
        {
            deferred
                .movement_state_effects
                .push((entity_id, posture, next_action_state));
        }
        // The authored successor owns the deferred movement START only
        // if it remains current after this Execute. A very short
        // successor can complete and hand off to its stop transition in
        // the same call; Original retains Waiting in that case. The
        // Execute switch still only reacts to the motion state it is
        // handed, so a successor whose START the seek wrapper swallowed
        // owns no state effect to postpone.
        if deferred_movement_state_start_due
            && matches!(state_effect_motion, MotionState::Start)
            && !current_order_will_advance
            && self
                .orders
                .sequence_manager
                .get_element(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.front())
                .is_some_and(|order| Some(order.order_id) == order_id)
            && let Some((posture, next_action_state)) =
                movement_execute_state_effect(order_action, MotionState::Start)
        {
            if executes_sword_movement {
                deferred.sword_movement_starts.push(entity_id);
            }
            deferred
                .movement_state_effects
                .push((entity_id, posture, next_action_state));
        }
        // A generated transition-distance copy reports START when first
        // booked, but its movement state is authoritative only if that
        // copied order remains current after the Execute. A short copy
        // may satisfy its arrival predicate and hand off in the same
        // call; Original retains the transition's Waiting state for that
        // frame. This survival rule applies to PCs too; their separate
        // deferred-successor marker covers the later authored order.
        if transition_distance_first_execute_due
            && matches!(state_effect_motion, MotionState::Start)
            && !current_order_will_advance
            && self
                .orders
                .sequence_manager
                .get_element(move_seq_id, move_elem_idx)
                .and_then(|element| element.orders.front())
                .is_some_and(|order| Some(order.order_id) == order_id)
            && let Some((posture, next_action_state)) =
                movement_execute_state_effect(order_action, MotionState::Start)
        {
            if executes_sword_movement {
                deferred.sword_movement_starts.push(entity_id);
            }
            deferred
                .movement_state_effects
                .push((entity_id, posture, next_action_state));
        }
    }

    fn selected_galopp_decision_frame(
        &self,
        owner: EntityId,
        selected: MovementOwnerSelection,
    ) -> bool {
        let element = self
            .orders
            .sequence_manager
            .get_element(selected.seq_id, selected.elem_idx)
            .unwrap_or_else(|| {
                panic!("selected gallop movement element disappeared for {owner:?}")
            });
        let order = element
            .current_order()
            .unwrap_or_else(|| panic!("selected gallop movement order disappeared for {owner:?}"));
        if element.owner != Some(owner)
            || order.order_id != selected.order_id
            || order.order_type != OrderType::RunningUpright
        {
            return false;
        }
        let flags = match element.data {
            crate::sequence::SequenceElementData::Movement { flags, .. } => flags,
            _ => panic!("selected gallop owner {owner:?} no longer has a movement element"),
        };
        if !flags.contains(crate::sequence::MoveFlags::RIDER_CHARGE) {
            return false;
        }
        let sprite = self
            .world
            .entities
            .get(owner)
            .unwrap_or_else(|| panic!("selected gallop owner {owner:?} disappeared"))
            .sprite();
        is_galopp_decision_frame(
            sprite.current_frame,
            sprite.num_frames_for_anim(OrderType::RunningUpright),
        )
    }

    /// Test-only compatibility wrapper. Production movement is owned by the
    /// live legacy-slot Actor coordinator and never batches callback results.
    #[cfg(test)]
    pub(super) fn tick_entity_movement(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &crate::engine::LevelAssets,
    ) {
        let owners: Vec<EntityId> = self
            .world
            .entities
            .actors()
            .map(|(id, _)| id.into())
            .collect();
        for owner in owners {
            let selected = self
                .orders
                .sequence_manager
                .current_order_for_actor(owner)
                .and_then(|(seq_id, elem_idx, order)| {
                    self.orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .filter(|element| element.data.is_movement())
                        .map(|_| MovementOwnerSelection {
                            seq_id,
                            elem_idx,
                            order_id: order.order_id,
                        })
                });
            let _ = self.tick_entity_movement_owner(sim, assets, owner, selected);
        }
    }

    /// Advance through door-pass steps after a walk step completes.
    ///
    /// Pops one translated motion/door sub-order. `PassingDoor` action
    /// points are returned as real orders instead of being drained in the
    /// predecessor's completion slot: Original `Hourglass` executes one
    /// current order and only then advances to its successor. `Select`
    /// retains its generic-animation callback plumbing for now.
    ///
    /// See [`DoorPassAdvance`] for return semantics.
    pub(super) fn advance_door_pass(
        actor: &mut crate::element::ActorData,
        entity_id: EntityId,
        transition_destination: MapPoint,
        _door_triggers: &mut Vec<(EntityId, crate::gate::DoorIndex, bool, u8)>,
        _select_triggers: &mut Vec<(EntityId, f32)>,
        next_order_id: &mut u32,
    ) -> DoorPassAdvance {
        let dp = match actor.active_door_pass.as_mut() {
            Some(dp) => dp,
            None => {
                tracing::warn!(
                    entity = ?entity_id,
                    "DoorPass: advance requested without active pass"
                );
                return DoorPassAdvance::NoActive;
            }
        };
        let step = match dp.steps.pop_front() {
            Some(s) => s,
            None => {
                let completed = Some((dp.door_index, dp.direct));
                actor.active_door_pass = None;
                return DoorPassAdvance::Done { completed };
            }
        };

        match step {
            crate::element::DoorPassStep::PassingDoor => {
                let order_id = crate::order::alloc_order_id(next_order_id);
                let order = crate::order::Order::new(OrderType::PassingDoor, 0.0, 0.0, order_id);
                DoorPassAdvance::ActionPoint { order }
            }
            crate::element::DoorPassStep::Select { speed } => {
                // Original translates SELECT into a real non-animation order:
                // it is promoted after the preceding walk, executes the Human
                // hulk side effect in its own actor slot, then resumes the
                // remaining door chain. Skipping it advances PASSING_DOOR and
                // its topology swap by one frame.
                let order_id = crate::order::alloc_order_id(next_order_id);
                let mut order = crate::order::Order::new(OrderType::Select, 0.0, 0.0, order_id);
                order.compute_direction = true;
                order.tolerance = speed;
                order.completion = crate::order::OrderCompletion::ResumeDoorPass;
                DoorPassAdvance::ActionPoint { order }
            }
            crate::element::DoorPassStep::Transition { action, reverse } => {
                // The transition order sits at the front of the
                // order queue and blocks subsequent orders until
                // its sprite animation completes.  We build the
                // transition order here and hand it back to the
                // caller, who pushes it onto the actor's current
                // sequence element.  `ResumeDoorPass` completion
                // re-enters this function when the animation
                // finishes.
                //
                // Save the walking action state for the post-transition
                // walk.  Merely materializing the successor must not change
                // it yet: Original does not execute the transition until its
                // own Hourglass slot starts on the following tick.
                let saved = actor.action_state;
                actor.clear_path();
                if let Some(dp) = actor.active_door_pass.as_mut() {
                    dp.saved_action_state = Some(saved);
                    dp.current_action = action;
                    dp.current_reverse = reverse;
                }
                let order_id = crate::order::alloc_order_id(next_order_id);
                let mut order = crate::order::Order::new(
                    action,
                    transition_destination.x,
                    transition_destination.y,
                    order_id,
                );
                order.reverse = reverse;
                order.compute_direction = false;
                order.completion = crate::order::OrderCompletion::ResumeDoorPass;
                tracing::debug!(
                    entity = ?entity_id,
                    ?action,
                    reverse,
                    "DoorPass: pausing for Transition animation"
                );
                DoorPassAdvance::Paused {
                    transition_order: order,
                }
            }
            crate::element::DoorPassStep::Walk {
                destination,
                action,
                reverse,
                compute_direction,
                tolerance,
            } => {
                // The walk animation itself comes from `current_action`
                // (read by tick_entity_movement via `door_pass_anim`).  Keep
                // the saved pre-transition state until this new order is
                // actually dispatched on the following owner tick.
                if let Some(dp) = actor.active_door_pass.as_mut() {
                    dp.current_action = action;
                    dp.current_reverse = reverse;
                }
                // Hand the Walk destination back to the caller —
                // advance_door_pass doesn't have sequence_manager
                // access, so it can't push the walking order
                // directly onto the PassDoor element.  The caller
                // (tick_entity_movement's post-loop door-pass
                // dispatch) does the order push.
                DoorPassAdvance::Continue {
                    destination,
                    action,
                    reverse,
                    compute_direction,
                    tolerance,
                }
            }
        }
    }

    /// Runtime detector for Shape 1 contract violations — logs a warning
    /// (and fires a `debug_assert!`) when a movement intent is drained
    /// while the actor is still in a "waiting" substate that relies on
    /// an exit event the halt-teardown will suppress.
    ///
    /// The Shape 1 wrappers (`EnemyAi::go_to` et al.) force callers to
    /// commit a new substate before queuing a movement — if the current
    /// substate is still in the wedge-prone set at drain time, either a
    /// new caller bypassed the wrapper via `ai.base.go_to(...)` or an
    /// external code path queued the intent on the actor's behalf
    /// without a corresponding `set_state`.  In either case the halt
    /// below will swallow the exit event and leave the AI stranded.
    fn check_shape1_contract(&self, entity_id: EntityId) {
        let Some(entity) = self.world.entities.get(entity_id) else {
            return;
        };
        let Some(ai) = entity.ai_controller() else {
            return;
        };
        use crate::ai::Substate;
        let wedge_prone = matches!(
            ai.current_substate,
            Substate::AttackingSwordfightParade
                | Substate::AttackingReactiontime
                | Substate::AttackingReactiontimeTurning
                | Substate::AttackingReactiontimeBending
        );
        if wedge_prone {
            tracing::warn!(
                entity = entity_id.index(),
                substate = ?ai.current_substate,
                "Shape 1 violation: movement intent drained while actor is in a \
                 wedge-prone substate — halt-teardown will swallow the exit event. \
                 Likely cause: a caller bypassed EnemyAi::go_to / ai.base.go_to, or \
                 queued a movement intent without calling set_state first."
            );
            debug_assert!(
                !wedge_prone,
                "Shape 1 violation at entity {} in substate {:?}",
                entity_id.index(),
                ai.current_substate
            );
        }
    }

    /// Each tick, AI controllers may produce movement/action orders.
    /// This method drains them and submits corresponding path requests.
    ///
    /// `AiController::pending_halt` (set by `stop_all` / `FaceTo`) is
    /// drained inside [`Self::launch_pending_orders_for_npc`] so the
    /// halt happens on the same call stack as the new element launch,
    /// `StopAll` / `FaceTo` halt the actor inline.
    pub(super) fn process_pending_ai_orders(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for npc_id in npc_ids {
            self.launch_pending_orders_for_npc(sim, assets, npc_id);
        }
    }

    /// Per-NPC half of [`Self::process_pending_ai_orders`] — drains one
    /// NPC's `pending_orders` queue and launches the corresponding
    /// movement / turn / generic sequences.  Called both from the
    /// top-of-tick global pass and from the per-NPC synchronous drain
    /// in [`EngineInner::dispatch_think_with_drain`] so `Face` / `GoTo`
    /// etc. take effect inside the same call stack as the handler that
    /// issued them — `Face` / `GoTo` launch the sequence inline.
    pub(super) fn launch_pending_orders_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        self.launch_pending_orders_for_npc_mode(sim, assets, entity_id, false);
    }

    pub(super) fn launch_pending_orders_for_npc_mode(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        defer_turn_instruction: bool,
    ) {
        self.launch_pending_orders_for_npc_mode_after_halt(
            sim,
            assets,
            entity_id,
            defer_turn_instruction,
            false,
        );
    }

    pub(super) fn launch_pending_orders_for_npc_mode_after_halt(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
        _defer_turn_instruction: bool,
        halt_already_applied: bool,
    ) {
        let debug_decision_path = crate::ai_enemy::decision_path_debug_enabled()
            && crate::ai_enemy::decision_path_debug_matches_raw(
                self.control.frame_counter,
                entity_id.index(),
            );
        // `StopAll` halts the actor inline before subsequent work,
        // and `FaceTo` / `GoTo` do the same on their own.  The Halt
        // is deferred to this drain (via `pending_halt`) so it runs
        // on the same tick as the pending-order launch.  Honor the
        // flag here — before launching new orders — so the
        // `Stop(Preference)` cascade interrupts any in-progress
        // sequence element (e.g. a yellow-? Turn mid-`bored-exit`)
        // and the new element launched below starts from a clean
        // slate.  `halt_actor` brackets the stop with
        // `inside_halt_method=true` so condolations queued by the
        // interrupt are tagged `from_halt` and don't fire
        // `Think(EventDone)`.
        let (has_pending_orders, take_halt) = {
            let Some(entity) = self.world.entities.get_mut(entity_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            let halt = ai.outbox.actor.halt;
            ai.outbox.actor.halt = false;
            (ai.has_pending_orders(), halt)
        };
        if take_halt {
            self.halt_actor(entity_id);
        }
        let had_explicit_halt = halt_already_applied || take_halt;
        if !has_pending_orders {
            return;
        }
        let intents: Vec<crate::order::AiOrderIntent> = {
            let Some(entity) = self.world.entities.get_mut(entity_id) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            ai.take_pending_orders()
        };
        if debug_decision_path {
            let (couldnt_reachpoint, already_on_point, owner_work) = self
                .world
                .entities
                .get(entity_id)
                .and_then(Entity::ai_controller)
                .map(|ai| {
                    (
                        ai.couldnt_reachpoint,
                        ai.already_on_point,
                        format!("{:?}", ai.outbox.reentrant.owner_work),
                    )
                })
                .unwrap_or_else(|| panic!("diagnostic owner {} lost AI", entity_id.index()));
            eprintln!(
                "AIDECISION frame={} owner={} stage=drain_intents count={} take_halt={} halt_already_applied={} couldnt={} already={} owner_work={}",
                self.control.frame_counter,
                entity_id.index(),
                intents.len(),
                take_halt,
                halt_already_applied,
                couldnt_reachpoint,
                already_on_point,
                owner_work,
            );
        }
        // Once an authorized GoTo has deferred the live path-waiter's tail
        // Halt, later GoTo calls in the same authored batch observe the
        // post-Halt state. The request drain preserves FIFO, so the marked
        // first replacement is constructed and cancelled before those later
        // moves are built.
        let mut path_waiter_tail_deferred = false;
        for intent in intents {
            let is_movement = matches!(
                intent.order_type,
                OrderType::WalkingUpright
                    | OrderType::RunningUpright
                    | OrderType::WalkingCrouched
                    | OrderType::WalkingAlerted
                    | OrderType::RiderCharging
            );
            if is_movement && !self.tactical_allows_combat_movement(entity_id) {
                tracing::trace!(
                    ?entity_id,
                    order = ?intent.order_type,
                    "allied stance suppressed AI-authored combat movement"
                );
                self.resolve_ai_engine_completion_verdict(entity_id);
                continue;
            }
            match intent.order_type {
                OrderType::WalkingUpright
                | OrderType::RunningUpright
                | OrderType::WalkingCrouched
                | OrderType::WalkingAlerted
                | OrderType::RiderCharging => {
                    let was_computing_path = !path_waiter_tail_deferred
                        && self
                            .orders
                            .sequence_manager
                            .current_element_for_actor(entity_id)
                            .and_then(|(sequence_id, element_index)| {
                                self.orders
                                    .sequence_manager
                                    .get_element(sequence_id, element_index)
                            })
                            .is_some_and(|element| {
                                element.command == crate::element::Command::MoveWaiting
                            });
                    if was_computing_path {
                        let ai = self
                            .world
                            .entities
                            .get_mut(entity_id)
                            .and_then(Entity::ai_controller_mut)
                            .unwrap_or_else(|| {
                                panic!(
                                    "movement owner {} lost AI while preserving path-waiter provenance",
                                    entity_id.index()
                                )
                            });
                        if ai.outbox.reentrant.reconsider_approach_completion_pending {
                            ai.outbox.reentrant.reconsider_approach_replaced_path_waiter = true;
                        }
                    }
                    // `find_accessible` / `ask_obstacle` pre-flight
                    // gates.  Run them before the halt so a failure
                    // leaves the outgoing sequence in place rather
                    // than tearing it down only to abandon the new
                    // move.
                    let mut intent = intent;
                    if !self.preflight_ai_goto(entity_id, &mut intent) {
                        self.resolve_ai_engine_completion_verdict(entity_id);
                        continue;
                    }
                    // A generated locomotion transition continues to own its
                    // previously published waypoint while a replacement Move
                    // becomes mpSequenceElement. Preserve that cached goal
                    // until the replacement installs a concrete waypoint.
                    // A concrete MoveOk walk/run does not have this lifetime:
                    // Original clears its goal at replacement arbitration and
                    // leaves zero visible until the new movement executes.
                    //
                    // An explicit StopAll before this GoTo has already
                    // performed the selected-element cleanup and must not
                    // resurrect the old destination.
                    let retained_movement_goal = (!had_explicit_halt)
                        .then(|| {
                            self.orders
                                .sequence_manager
                                .current_element_for_actor(entity_id)
                                .and_then(|(seq, idx)| {
                                    self.orders.sequence_manager.get_element(seq, idx)
                                })
                                .is_some_and(|element| {
                                    element.data.is_movement()
                                        && element.current_order().is_some_and(|order| {
                                            movement_transition_retains_goal(order.order_type)
                                        })
                                })
                        })
                        .filter(|selected_is_movement| *selected_is_movement)
                        .and_then(|_| {
                            self.world
                                .entities
                                .get(entity_id)
                                .map(|entity| entity.position_iface().map_goal())
                        });
                    intent.retained_movement_goal = retained_movement_goal;
                    // The Original source spells this pre-launch gate as
                    // `uwFlags & GOTO_NOHALT == 0`. C/C++ precedence parses
                    // it as `uwFlags & (GOTO_NOHALT == 0)`, which is always
                    // zero: ordinary GoTo never halts here. Keep explicit
                    // StopAll/Halt effects at their own call sites instead of
                    // "fixing" the legacy bug.
                    let route_rejected_before_launch = was_computing_path
                        && !self.ai_move_gate_route_is_authorized(entity_id, &intent);
                    if route_rejected_before_launch {
                        self.set_ai_couldnt_reachpoint(entity_id);
                        self.resolve_ai_engine_completion_verdict(entity_id);
                    } else {
                        // `launch_ai_move` only stages the intent; the actual
                        // AppendMoveToSequence construction happens in the
                        // pending-request drain below. Preserve the effective
                        // GoTo tail until that drain so an existing
                        // MOVE_WAITING does not erase the replacement before
                        // its gate sequence (and building-exit random waits)
                        // has been constructed. Original constructs and
                        // launches first, then observes IsComputingPath and
                        // Halts (`RHartificialintelligence.cpp:2538-2620`).
                        // A synchronous continuation may already carry the
                        // outgoing path-waiter's GoTo tail after that waiter
                        // has been halted. Preserve that authored provenance;
                        // the current manager command can only add the same
                        // requirement, never revoke it.
                        intent.halt_after_launch_for_path_waiter |= was_computing_path;
                        self.launch_ai_move(entity_id, &intent);
                        path_waiter_tail_deferred |= was_computing_path;
                    }
                    if debug_decision_path {
                        let ai = self
                            .world
                            .entities
                            .get(entity_id)
                            .and_then(Entity::ai_controller)
                            .unwrap_or_else(|| {
                                panic!("diagnostic owner {} lost AI", entity_id.index())
                            });
                        eprintln!(
                            "AIDECISION frame={} owner={} stage=launch_move_done order={:?} target=({:08x},{:08x}) move_flags={} tolerance_bits={:08x} no_halt={} reverse={} couldnt={} already={} owner_work={:?}",
                            self.control.frame_counter,
                            entity_id.index(),
                            intent.order_type,
                            intent.target_x.to_bits(),
                            intent.target_y.to_bits(),
                            intent.move_flags,
                            intent.tolerance.to_bits(),
                            intent.no_halt,
                            intent.reverse,
                            ai.couldnt_reachpoint,
                            ai.already_on_point,
                            ai.outbox.reentrant.owner_work,
                        );
                    }

                    // GoTo has a separate, effective tail check after
                    // launching its sequence: an actor whose old movement is
                    // still waiting on the pathfinder is halted. The halt
                    // also cancels the just-registered replacement, matching
                    // StopNotYetLaunchedSequenceElements.
                    if was_computing_path {
                        self.check_shape1_contract(entity_id);
                        if route_rejected_before_launch {
                            // No replacement sequence exists on this branch,
                            // so the tail can halt the outgoing waiter now.
                            // Authorized routes carry the halt marker through
                            // `do_launch_ai_move` and are halted immediately
                            // after construction by the request drain.
                            self.halt_actor(entity_id);
                        }
                    }
                }
                OrderType::Turning => {
                    let turn_command = if intent.fast_turn {
                        crate::element::Command::TurnFast
                    } else {
                        crate::element::Command::Turn
                    };
                    // FaceTo(point/vector) resolves its sector before it
                    // enters FaceTo(UWORD), which then Halts and registers
                    // TURN. Preserve that authored sector across both Halt
                    // and the deferred manager boundary instead of
                    // recomputing it from a potentially newer actor position.
                    let direction = intent.explicit_direction.or_else(|| {
                        self.world.entities.get(entity_id).map(|entity| {
                            let position = entity.element_data().position_map();
                            crate::position_interface::vector_to_sector_0_to_15_iso(
                                intent.target_x - position.x,
                                intent.target_y - position.y,
                            )
                        })
                    });
                    // A SetState callback can synchronously register an
                    // attentive-mode transition, then a re-entrant
                    // EventReachPoint can register FaceTo before the manager
                    // hourglass gets to either element. Original arbitration
                    // postpones the Turn without translating it, so its
                    // direction goal remains untouched until the attentive
                    // transition finishes.
                    let selected_is_movement = self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(entity_id)
                        .and_then(|(seq, idx)| self.orders.sequence_manager.get_element(seq, idx))
                        .is_some_and(|element| element.data.is_movement());
                    if selected_is_movement || !intent.no_halt {
                        self.halt_actor(entity_id);
                    }
                    // FaceTo's Halt has now synchronously performed any
                    // selected element's Actor condolence.  Whatever map
                    // goal remains after that callback is the value the
                    // ensuing Turn observes: RHSprite::PerformAction does
                    // not overwrite PositionGoalMap for an animation
                    // order.  A halt that only rewrites a live movement into
                    // its stop transition leaves the element selected and the
                    // cached destination intact; a halt that interrupts the
                    // element outright clears it.  Sampling after the halt
                    // reproduces both without having to predict which
                    // happened.  Carry the result on the deferred element so
                    // Rust's staged Turn initialization does not mistake the
                    // animation order's zero destination for a movement goal.
                    // This also covers FaceTo launched from an
                    // EventReachPoint callback after an empty SEEK was
                    // rewritten to MOVE and terminated without becoming the
                    // actor's selected element.
                    let retained_goal = self
                        .world
                        .entities
                        .get(entity_id)
                        .map(|entity| entity.position_iface().map_goal());
                    self.launch_turn_sequence_deferred_no_transitions(
                        entity_id,
                        turn_command,
                        direction,
                        intent.target_x,
                        intent.target_y,
                        retained_goal,
                    );
                }
                _ => {
                    // Other order types go on their own single-order
                    // sequence for the animation driver to pick up.
                    let order = intent.stamp(self.orders.allocate_order_id());
                    self.launch_single_order_sequence_stamped(
                        sim,
                        assets,
                        entity_id,
                        crate::element::Command::Generic,
                        order,
                    );
                }
            }
            if !is_movement {
                self.resolve_ai_engine_completion_verdict(entity_id);
            }
        }
    }

    /// Prepare a Move / Seek sequence element for dispatch.
    ///
    /// Direct moves populate their orders immediately. A*-requiring moves
    /// snapshot a [`PendingPathRequest`], transition to `MoveWaiting`, and
    /// complete later through [`EngineInner::process_next_path_request`].
    pub(crate) fn try_dispatch_move_path(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        _assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        dest: MapPoint,
        mut move_action: OrderType,
    ) -> MovePathOutcome {
        // Swap walking/running into the sword variant when the actor
        // is already in a sword action state — but only under two
        // gates:
        //   1. The post-transition posture is Upright — the swap is
        //      skipped for non-upright post-transition postures (e.g.
        //      CarryingCorpse, HelpingToClimb, ...).
        //   2. The action-state-after-transition is a sword state.
        // Read both from the SequenceElement rather than the live
        // entity state so a Move queued with a post-transition sword
        // state (e.g. launched from a posture/action transition that
        // hasn't applied yet) uses the intended post-transition
        // values.
        //
        // WalkingWithSword / RunningWithSword are logical non-animation
        // dispatch tokens. The Human Execute override resolves them through
        // FaceOpponent to a concrete forward/backward/strafe sword row.
        let (posture_after, action_after) = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| (e.posture_after_transition, e.action_state_after_transition))
            .unwrap_or_default();
        let elem_flags = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| match &e.data {
                crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .unwrap_or(crate::sequence::MoveFlags::empty());
        let is_fast = elem_flags.contains(crate::sequence::MoveFlags::FAST);
        // Human::DetermineMovementAnimation chooses the sword token only
        // from the action state stamped by Actor::Instruct. The FORCE flag
        // does not participate in translation; Human::Execute reads it later
        // solely to keep an already-translated sword move alive after the
        // opponent list becomes empty. This distinction matters when an
        // upright Move|FORCE is postponed behind QuitSwordfight: on resume
        // the freshly stamped Waiting state must produce an ordinary walk.
        let sword_movement_context =
            posture_after == crate::element::Posture::Upright && action_after.is_sword();
        if sword_movement_context {
            move_action = sword_movement_dispatch_action(move_action);
        }
        // PC shield-action arm: a shield-wielding PC with an Upright
        // stamped posture rewrites the movement element's stored `action`:
        //   WALKING_UPRIGHT / WALKING_WITH_CORPSE → WALKING_WITH_SHIELD
        //   WALKING_WITH_SHIELD                     → already set, no-op
        //   RUNNING_UPRIGHT                         → no
        //                                             running-with-shield
        //                                             anim, leave the
        //                                             upright variant
        //   default                                 → warn (would
        //                                             assert in dev).
        // This derived override is gated on PC and on the stamped state. It
        // authors the logical shield token unconditionally; FaceDangerPoint
        // resolves the concrete sprite row later. Actors which delegate to
        // the base implementation get the separate live-state rewrite below.
        let owner_entity =
            self.world.entities.get(owner).unwrap_or_else(|| {
                panic!("movement owner {owner:?} disappeared during translation")
            });
        let owner_is_pc = owner_entity.is_pc();
        let live_action_state = owner_entity
            .actor_data()
            .expect("movement owner must retain actor data during translation")
            .action_state;
        let owner_sector = owner_entity.element_data().sector();
        let owner_is_on_lift = self.sector_is_lift(owner_sector);
        let pc_stamped_shield_context = owner_is_pc
            && posture_after == crate::element::Posture::Upright
            && action_after.is_shield();
        if pc_stamped_shield_context {
            let want = match move_action {
                OrderType::WalkingUpright | OrderType::WalkingWithCorpse => {
                    Some(OrderType::WalkingWithShield)
                }
                OrderType::WalkingWithShield => None,
                OrderType::RunningUpright => None,
                _ => {
                    tracing::warn!(
                        ?owner,
                        ?move_action,
                        "DetermineMovementAnimation: shield action_state with \
                         unrecognised movement action",
                    );
                    None
                }
            };
            if let Some(want) = want {
                move_action = want;
            }
        }
        // The PC and Human overrides delegate to Actor's base implementation
        // unless their stamped shield/sword arms above consume the request.
        // Base DetermineMovementAnimation switches on the actor's *live*
        // action state.  This is how a soldier already moving with a shield
        // rewrites a newly instructed upright path even though the sequence
        // retained an older non-shield action-state stamp.
        if !sword_movement_context
            && !pc_stamped_shield_context
            && posture_after == crate::element::Posture::Upright
            && !owner_is_on_lift
            && live_action_state.is_shield()
        {
            move_action = OrderType::WalkingWithShield;
        }
        // Posture forces: non-Upright postures rewrite the action
        // regardless of the action-state inner switch.
        // CARRYING_CORPSE and CROUCHED are pure rewrites;
        // CARRYING_ON_SHOULDERS additionally sets `MoveFlags::REVERSED`
        // on the element flags.  The corpse-lost guard
        // (`WalkingWithCorpse → WalkingUpright`) closes the case where
        // a postponed Move retained `WalkingWithCorpse` after the
        // corpse target was lost — apply that under the Upright arm.
        let mut want_reverse_flag = false;
        match posture_after {
            crate::element::Posture::CarryingCorpse => {
                if let Some(entity) = self.world.entities.get(owner)
                    && entity.sprite().has_animation(OrderType::WalkingWithCorpse)
                {
                    move_action = OrderType::WalkingWithCorpse;
                }
            }
            crate::element::Posture::Crouched => {
                if let Some(entity) = self.world.entities.get(owner)
                    && entity.sprite().has_animation(OrderType::WalkingCrouched)
                {
                    move_action = OrderType::WalkingCrouched;
                }
            }
            crate::element::Posture::CarryingOnShoulders => {
                if let Some(entity) = self.world.entities.get(owner)
                    && entity
                        .sprite()
                        .has_animation(OrderType::WalkingCarryingOnShoulders)
                {
                    move_action = OrderType::WalkingCarryingOnShoulders;
                }
                want_reverse_flag = true;
            }
            crate::element::Posture::Upright => {
                // Inner action-state switch (non-lift Upright): for
                // action states in {Waiting, Bored, Moving,
                // MovingFast, *Bow*, Sleeping, Listening}, normalise
                // STAIRS / CLIMBING_* / CARRYING_ON_SHOULDERS /
                // CROUCHED inbound actions to WalkingUpright or
                // RunningUpright per `is_fast`.  WALKING_STAIRS always
                // normalises to WALKING_UPRIGHT regardless of speed.
                // A PC can resume a movement whose authored action still
                // carries a sword token after QuitSwordfight lowered the
                // weapon. Original's base DetermineMovementAnimation treats
                // that combination as an ordinary upright walk/run; NPCs
                // retain the token.
                let inner_arm = matches!(
                    action_after,
                    crate::element::ActionState::Waiting
                        | crate::element::ActionState::Bored
                        | crate::element::ActionState::Moving
                        | crate::element::ActionState::MovingFast
                        | crate::element::ActionState::Sleeping
                        | crate::element::ActionState::Listening
                ) || action_after.is_bow();
                if !owner_is_on_lift && inner_arm {
                    let walk_or_run = if is_fast {
                        OrderType::RunningUpright
                    } else {
                        OrderType::WalkingUpright
                    };
                    move_action = match move_action {
                        // Pass-through.
                        OrderType::WalkingUpright
                        | OrderType::RunningUpright
                        | OrderType::RiderCharging => move_action,
                        // Stairs always → walking upright.
                        OrderType::WalkingStairs => OrderType::WalkingUpright,
                        OrderType::WalkingWithSword if owner_is_pc => OrderType::WalkingUpright,
                        OrderType::RunningWithSword if owner_is_pc => OrderType::RunningUpright,
                        // Climbing / carry-on-shoulders → walk/run upright.
                        OrderType::ClimbingWallUp
                        | OrderType::ClimbingWallDown
                        | OrderType::ClimbingLadderUp
                        | OrderType::ClimbingLadderDown
                        | OrderType::ClimbingLadderUpFast
                        | OrderType::ClimbingLadderDownFast
                        | OrderType::ClimbingWallUpFast
                        | OrderType::ClimbingWallDownFast
                        | OrderType::WalkingCarryingOnShoulders => walk_or_run,
                        // Crouched → walk/run upright.
                        OrderType::WalkingCrouched => walk_or_run,
                        // Default arm: leave `move_action` as-is for
                        // any non-listed type.
                        other => other,
                    };
                }
                // Corpse-lost guard.
                if move_action == OrderType::WalkingWithCorpse {
                    move_action = OrderType::WalkingUpright;
                }
            }
            _ => {}
        }
        // RHElementActorHuman::DetermineMovementAnimation handles upright
        // sword states in the derived override and deliberately does not call
        // RHElementActor's base implementation.  The logical sword token is
        // therefore authoritative even in a lift sector (the Human Execute
        // override chooses the concrete combat row later).  This matters when
        // a postponed combat approach resumes on stairs: RUNNING_WITH_SWORD
        // must not collapse to the lift's ordinary WalkingStairs row.  Base
        // lift translation still applies to non-sword and authored climb
        // movement.
        if !sword_movement_context && !pc_stamped_shield_context {
            // The sword / shield / corpse movement tokens are only ever
            // assigned to an element whose post-transition posture is
            // Upright, so a movement that reaches a wall or ladder carries
            // the plain walk or run action and the lift sector answers a run
            // with the fast climb. Rust can still arrive here holding a
            // carried-over variant token; normalise it to the speed the
            // element is actually moving at before the lift translates it.
            let lift_input = if matches!(
                posture_after,
                crate::element::Posture::OnWall | crate::element::Posture::OnLadder
            ) {
                climb_lift_translation_input(move_action, is_fast)
            } else {
                move_action
            };
            move_action =
                self.determine_lift_movement_animation(owner, posture_after, lift_input, dest);
        }
        // Write the rewritten action back onto the movement sequence
        // element so downstream consumers (refresh-seek, post-process,
        // NPC AI re-reads) see it.  Apply both the action rewrite and
        // the CARRYING_ON_SHOULDERS REVERSED-flag mutation here.
        if let Some(elem) = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            && let crate::sequence::SequenceElementData::Movement { flags, action, .. } =
                &mut elem.data
        {
            *action = move_action;
            if want_reverse_flag {
                *flags |= crate::sequence::MoveFlags::REVERSED;
            }
            if elem.posture_after_transition == crate::element::Posture::Undefined
                && let Some(entity) = self.world.entities.get(owner)
            {
                elem.posture_after_transition = entity.element_data().posture;
            }
        }

        // Read entity position / layer / sector / pathfinder index +
        // current move box + half diagonal (half diagonal drives the
        // thick-reachability pre-check below).
        let (mut source, entity_layer, entity_sector, pf_idx, move_box_map, half_diagonal) = {
            let entity = match self.world.entities.get(owner) {
                Some(e) => e,
                _ => return MovePathOutcome::ActorGone,
            };
            let elem = entity.element_data();
            let pi = entity.position_iface();
            let pf_idx = u16::from(pi.get_pathfinder_index().unwrap_or_else(|| {
                panic!("movement owner {owner:?} has no configured pathfinder index")
            }));
            (
                elem.position_map(),
                elem.layer(),
                elem.sector().map(u16::from).unwrap_or(0),
                pf_idx,
                *pi.get_move_box_map(),
                pi.get_half_diagonal(),
            )
        };

        // A PC disguised as an anonymous archer is pinned to its shooting
        // spot for the duration of the contest: the move is refused outright
        // and the hero complains instead of walking away.
        if owner_is_pc
            && self.world.entities.get(owner).is_some_and(|e| {
                e.element_data().posture == crate::element::Posture::AnonymousArcher
            })
        {
            tracing::debug!(
                actor = ?owner,
                "try_dispatch_move_path: anonymous archer may not move",
            );
            self.hero_speaking(
                _assets,
                owner,
                crate::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
            );
            return MovePathOutcome::Refused;
        }

        // Before queuing a path request, if the move is flagged
        // MAP / STRAIGHT, or the source→dest segment is
        // thick-reachable, skip the pathfinder entirely and emit a
        // single direct order.  The pathfinder is never invoked when
        // a straight line suffices.
        //
        // Without this pre-check, short clicks that are directly
        // walkable still hit A*, which can route the actor through
        // source-adjacent graph nodes (extra waypoints around
        // `PassAroundLastNode`) and produce the "keeps moving old
        // direction briefly" click-walk regression.
        let move_flags = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|e| match &e.data {
                crate::sequence::SequenceElementData::Movement { flags, .. } => Some(*flags),
                _ => None,
            })
            .unwrap_or(crate::sequence::MoveFlags::empty());
        let is_pass_door = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(|e| e.command == crate::element::Command::PassDoor);
        let straight_ok = if movement_flags_force_direct_dispatch(move_flags) {
            true
        } else {
            self.world
                .fast_grid
                .is_reachable_thick(source, dest, entity_layer, half_diagonal)
        };

        // Before submitting a path request, check whether the actor's
        // move box is in an authorized position.  This mirrors legacy implementation
        // `RHPathFinder::AddPathRequest`; direct MAP / STRAIGHT /
        // thick-reachable moves do not enter the pathfinder and do
        // not run this extraction gate.
        //
        // If extraction is needed, call `find_authorized_position` to
        // mutate the box to a nearby valid spot, set
        // `use_first_point = true`, and snap the request source to the
        // recovered box centre.  When extraction fails, stop the actor
        // and `Wait` it.
        //
        // Without this snap the downstream strict source-authorization
        // check rejects every candidate and the actor is permanently
        // stuck — A* can't seed.  An earlier fallback only handles
        // the inverse case (source authorized but corridor too thin);
        // this handles "actor must always stay on an authorized
        // position" by pre-snapping the request source.
        let mut use_first_point = false;
        let source_authorized = straight_ok
            || self
                .world
                .fast_grid
                .is_position_authorized(&move_box_map, entity_layer);
        if path_request_needs_source_extraction(straight_ok, source_authorized) {
            let mut box_element = move_box_map;
            if !self
                .world
                .fast_grid
                .find_authorized_position(&mut box_element, entity_layer)
            {
                // Extraction failed; stop the actor and bail.  Route
                // through `stop_owner` (which clears active sequences
                // and pending path requests for this owner) and
                // launch a `Wait` sequence element at `Wait` priority.
                //
                // `RHPathFinder::AddPathRequest` (`RHpathfinder.cpp:464-465`)
                // calls `pRequest->pActor->Stop()` with no argument, so the
                // stop priority is the declared default
                // `RHPRIORITY_NORMAL` (`RHelementactor.h:273`) — NOT
                // `RHPRIORITY_WAIT`.  The distinction decides whether the
                // incoming Normal-priority Move element is stopped at all:
                // `RHSequenceElement::Stop` only acts when
                // `mPriority >= priorityOfStop`
                // (`RHsequenceelement.cpp:528`), and `RHPRIORITY_NORMAL`(8)
                // is stronger than `RHPRIORITY_WAIT`(9)
                // (`RHsequenceelement.h:38-51`).  With `Wait` the Move
                // survived, kept the actor's selection, and left the sprite's
                // PositionGoalMap installed for one extra frame.
                tracing::warn!(
                    actor = ?owner,
                    src_x = source.x,
                    src_y = source.y,
                    layer = entity_layer,
                    "try_dispatch_move_path: actor cannot be extracted from obstacle (Stop + Wait)",
                );
                self.stop_owner(owner, crate::sequence::SequencePriority::Normal);
                let mut wait_elem = crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::Wait,
                    Some(owner),
                );
                wait_elem.priority = crate::sequence::SequencePriority::Wait;
                let mut seq = crate::sequence::Sequence::new();
                seq.append_element(wait_elem);
                self.launch_sequence(seq);
                return MovePathOutcome::Failed;
            }
            let center = box_element.center();
            tracing::info!(
                actor = ?owner,
                old_src_x = source.x,
                old_src_y = source.y,
                new_src_x = center.x,
                new_src_y = center.y,
                "try_dispatch_move_path: extracted source from obstacle (use_first_point=true)",
            );
            source = MapPoint::new(center.x, center.y);
            use_first_point = true;
        }

        let request = PendingPathRequest {
            restored_from_v48: false,
            owner,
            seq_id,
            elem_idx,
            source,
            dest,
            layer: entity_layer,
            sector: entity_sector,
            // Original leaves `uwSector` uninitialized and never reads it.
            // Rust initializes the otherwise dormant serialized member.
            legacy_sector: 0,
            half_diagonal_idx: pf_idx,
            use_first_point,
            move_action,
            speed: if owner_is_pc {
                crate::pathfinder::PathFinderSpeed::Fast
            } else {
                crate::pathfinder::PathFinderSpeed::Medium
            },
            reverse: elem_flags.contains(crate::sequence::MoveFlags::REVERSED),
            tolerance: self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| match &element.data {
                    crate::sequence::SequenceElementData::Movement { tolerance, .. } => {
                        Some(*tolerance)
                    }
                    _ => None,
                })
                .unwrap_or(0.0),
            antagonist: self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|element| match &element.data {
                    crate::sequence::SequenceElementData::Movement { element, .. }
                        if !elem_flags.contains(crate::sequence::MoveFlags::SEEK)
                            || !elem_flags.contains(crate::sequence::MoveFlags::USE_POINT) =>
                    {
                        Some(*element)
                    }
                    crate::sequence::SequenceElementData::Movement { .. } => Some(None),
                    _ => None,
                })
                .flatten(),
            is_pass_door,
            elem_flags,
            sword_movement_context,
            is_fast,
        };

        // `RHElementActor::InstructOwner` completes direct / straight moves
        // immediately, but converts only A*-requiring moves to MOVE_WAITING
        // and queues an `RHpathRequest`.
        if !straight_ok {
            let mut retained_movement_goal = None;
            if let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
            {
                retained_movement_goal = elem.retained_movement_goal;
                elem.command = crate::element::Command::MoveWaiting;
                elem.push_order(crate::order::Order::new(
                    OrderType::Freezing,
                    source.x,
                    source.y,
                    crate::order::alloc_order_id(&mut self.orders.next_order_id),
                ));
            }
            self.orders
                .sequence_manager
                .element_in_progress(seq_id, elem_idx);
            if let Some(goal) = retained_movement_goal
                && let Some(entity) = self.world.entities.get_mut(owner)
                && entity.position_iface().map_goal() == MapPoint::ZERO
            {
                // A pending replacement owns the actor now, but has no
                // concrete waypoint with which to initialize the sprite.
                // Restore the outgoing movement's cached goal only when
                // eager Rust cleanup already erased it. The replacement can
                // be queued before the outgoing actor slot and instructed
                // afterward; in that interval the live movement may advance
                // to another waypoint. Original leaves that newer sprite
                // goal untouched because the interrupted element is no
                // longer selected.
                entity.position_iface_mut().set_map_goal(goal);
            }
            let parity_request = crate::pathfinder::parity_path_capture_is_active()
                .then(|| parity_path_request_state(&self.world.fast_grid, &request));
            self.trace_path_owner_lifecycle("before_path_enqueue", owner, Some((seq_id, elem_idx)));
            self.orders.pending_path_requests.enqueue(request);
            self.trace_path_owner_lifecycle("after_path_enqueue", owner, Some((seq_id, elem_idx)));
            if let Some(request) = parity_request {
                crate::pathfinder::record_parity_path_event(
                    crate::pathfinder::ParityPathEvent::Queued(request),
                );
            }
            return MovePathOutcome::Pending;
        }

        self.finish_move_path(sim, request, vec![source, dest])
    }

    pub(super) fn finish_move_path(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        request: PendingPathRequest,
        mut waypoints: Vec<MapPoint>,
    ) -> MovePathOutcome {
        let PendingPathRequest {
            restored_from_v48,
            owner,
            seq_id,
            elem_idx,
            source,
            dest: _,
            layer: entity_layer,
            sector: _,
            legacy_sector: _,
            half_diagonal_idx: _,
            use_first_point,
            move_action,
            speed: _,
            reverse,
            tolerance,
            antagonist,
            is_pass_door,
            elem_flags,
            sword_movement_context,
            is_fast: _,
        } = request;

        let selected_pre_path_tail = self
            .orders
            .sequence_manager
            .current_order_for_actor(owner)
            .and_then(|(selected_seq, selected_idx, current)| {
                let element = self.orders.sequence_manager.get_element(seq_id, elem_idx)?;
                let tail_index = element.orders.len().checked_sub(1)?;
                let tail = element.orders.get(tail_index)?;
                let installed_matches = self
                    .world
                    .entities
                    .get(owner)
                    .and_then(Entity::actor_data)
                    .and_then(|actor| actor.installed_order)
                    .is_some_and(|installed| installed.order_id == current.order_id);
                (selected_seq == seq_id
                    && selected_idx == elem_idx
                    && current.order_id == tail.order_id
                    && installed_matches)
                    .then_some(tail_index)
            });

        // ProcessPathRequests materializes movement orders beginning at
        // `uwFirstPathIndex`, so an unrequested raw source never becomes an
        // order seen by either actor or soldier PostProcessPath.
        let raw_waypoint_count =
            prepare_path_waypoints_for_postprocess(&mut waypoints, use_first_point);

        // Drunken-soldier path deviation applies later, after actor
        // PostProcessPath has inserted its transition orders. The Original
        // soldier override skips those transitions and inserts midpoint
        // copies only before upright walking/running orders.
        let is_movement_anim = matches!(
            move_action,
            OrderType::WalkingUpright | OrderType::RunningUpright
        );

        tracing::trace!(
            actor = ?owner,
            ?seq_id,
            elem_idx,
            wp = waypoints.len(),
            ?move_action,
            ?elem_flags,
            sword_movement_context,
            "try_dispatch_move_path: dispatched {} waypoints to actor",
            waypoints.len(),
        );

        // Build one walking/running order per waypoint.  The final
        // order carries the element's tolerance + antagonist, and
        // every order carries the element's reverse flag.
        //
        // The `antagonist`: when SEEK+USE_POINT the target element is
        // *not* carried on the move (the seek is to a hotspot, not to
        // the antagonist itself); otherwise the movement element's
        // `element` (antagonist) rides along on the final order so
        // downstream consumers (touch-on-Done etc.) can resolve it.
        // ProcessPathRequests only stamps tolerance and antagonist when the
        // raw C++ path contains more than one point. Decide this before
        // removing a leading source waypoint: raw [source, goal] emits one
        // order with metadata, while a direct raw [goal] order keeps the
        // RHOrder constructor defaults.
        let (final_order_tolerance, final_order_antagonist) =
            original_final_path_metadata(raw_waypoint_count, tolerance, antagonist);

        // `use_first_point` handling: the emission loop starts at
        // index 0 if set, otherwise 1.
        //
        // * `use_first_point == false` — the normal case where the
        //   source was already authorized.  `path[0]` IS the actor's
        //   current position (the pathfinder returns
        //   `[source, ..., goal]` for graph paths), so skip it to
        //   avoid a zero-length first order.  Direct paths return
        //   just `[goal]` (len == 1) and the skip doesn't apply.
        //
        // * `use_first_point == true` — set above when the source
        //   had to be extracted from an obstacle.  `path[0]` is the
        //   snapped source, NOT the actor's current position; keep
        //   it as the first waypoint so the actor walks back to safe
        //   ground before continuing.  (For direct paths this is a
        //   no-op: `[goal]` stays a single waypoint and the actor
        //   walks straight to goal — anti-collision handles the small
        //   obstacle clip on that first leg.)
        let mut rewritten_installed_order = None;
        {
            let next_order_id = &mut self.orders.next_order_id;
            if let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
            {
                // Fresh Rust movement elements retain their generated
                // transition prefix through `num_transition_orders`. A
                // restored Original MOVE_WAITING instead owns the exact
                // serialized pre-path queue, whose last waiting order must be
                // reused in place when the saved request completes.
                // ProcessPathRequests marks a resolved movement element as
                // MOVE_OK before installing its path orders. The command is
                // observable by actor execution and condolation logic; it is
                // not merely a pathfinder implementation detail.
                if !is_pass_door {
                    elem.command = crate::element::Command::MoveOk;
                }
                crate::movement::build_orders_from_path(
                    elem,
                    &waypoints,
                    move_action,
                    final_order_tolerance,
                    reverse,
                    final_order_antagonist,
                    next_order_id,
                    restored_from_v48,
                );
                rewritten_installed_order = selected_pre_path_tail
                    .and_then(|tail_index| elem.orders.get(tail_index))
                    .map(|order| crate::element::InstalledActorOrder {
                        order_id: order.order_id,
                        order_type: order.order_type,
                    });
            }
        }

        if let Some(installed_order) = rewritten_installed_order {
            // ProcessPathRequests reuses the selected movement's final
            // pre-path RHOrder, calls NewID, and changes its action in place.
            // Keep the explicit mpOrder mirror on that rewritten object; a
            // later PostProcessPath may insert other orders ahead of it but
            // does not repoint mpOrder until the next Actor::Hourglass.
            self.world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .expect("resolved path owner lost actor data")
                .installed_order = Some(installed_order);
        }

        // Splice startup / end transitions into the order queue
        // based on the actor's posture + action state.
        self.post_process_path(seq_id, elem_idx);

        if is_movement_anim && !is_pass_door {
            let (blood_alcohol, half_diagonal, move_box) = self
                .world
                .entities
                .get(owner)
                .and_then(|entity| {
                    let blood_alcohol = entity
                        .npc_data()
                        .and_then(|npc| npc.ai_brain.base())?
                        .blood_alcohol;
                    let position = entity.position_iface();
                    Some((
                        blood_alcohol,
                        position.get_half_diagonal(),
                        *position.get_move_box(),
                    ))
                })
                .unwrap_or_default();
            if blood_alcohol > 0 {
                let grid = self.world.fast_grid.clone();
                let next_order_id = &mut self.orders.next_order_id;
                if let Some(element) = self
                    .orders
                    .sequence_manager
                    .get_element_mut(seq_id, elem_idx)
                {
                    crate::engine::tick::apply_drunken_order_deviation(
                        sim,
                        element,
                        source,
                        blood_alcohol,
                        move_action == OrderType::RunningUpright,
                        entity_layer,
                        &move_box,
                        half_diagonal,
                        &grid,
                        next_order_id,
                    );
                }
            }
        }

        // Install the derived Rust movement latch, but do not change the
        // actor's action state here. Original Translate/PostProcessPath only
        // builds the order queue; the later actor Execute slot changes state
        // when PerformMotion returns START (for every movement family, not
        // only sword movement). This distinction is observable when the
        // sequence manager instructs a Move after the actor loop: that frame
        // must retain the pre-movement action state.
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.active_movement = ActiveMovement::new(seq_id, elem_idx);
            // The outer SEEK translation or RefreshSeek owns the target
            // snapshot and TIME_SEEK_REFRESH assignment. AppendMoveToSequence
            // creates one or more concrete MOVE|SEEK elements without
            // resampling either value, so a route through a door retains the
            // original target reference and accumulated countdown.
            // Mirror the original actor lifecycle flag once the movement
            // element promotes to InProgress.
            actor.sequence_element_started = true;
        }

        // Transition element to InProgress.
        self.orders
            .sequence_manager
            .element_in_progress(seq_id, elem_idx);

        MovePathOutcome::Success
    }
}

/// Append the PC arrival bark after all movement at the next command level.
///
/// Original `PerformMove` passes `uwCount` by reference through
/// `AppendMoveToSequence`; every appended movement consumes the current value
/// and increments it before `SPEAK_HERO_REACH_DESTINATION` is constructed
/// (`original-code/RHengine.cpp:10046-10052`,
/// `original-code/RHsequence.cpp:657-661`). Keeping the bark parallel with a
/// pathfinding Move makes its immediate termination complete the whole level,
/// killing the new `MoveWaiting` and cancelling its queued request.
fn append_arrival_speech(sequence: &mut crate::sequence::Sequence, owner: EntityId) {
    let level = sequence
        .last()
        .unwrap_or_else(|| panic!("arrival speech requires a preceding movement element"))
        .command_level
        .saturating_add(1);
    sequence.append_element(crate::sequence::SequenceElement::new(
        level,
        crate::element::Command::SpeakHeroReachDestination,
        Some(owner),
    ));
}

#[cfg(test)]
mod arrival_speech_topology_tests {
    use super::*;
    use crate::element::Command;
    use crate::order::OrderType;
    use crate::sequence::{Sequence, SequenceElement};

    #[test]
    fn same_sector_arrival_speech_follows_move_instead_of_running_in_parallel() {
        let owner = EntityId::Pc(crate::entity_id::PcId(7));
        let mut sequence = Sequence::new();
        sequence.append_element(SequenceElement::new_movement(
            1,
            Command::Move,
            Some(owner),
            OrderType::WalkingUpright,
        ));

        append_arrival_speech(&mut sequence, owner);

        assert_eq!(sequence.elements.len(), 2);
        assert_eq!(sequence.elements[0].command_level, 1);
        assert_eq!(
            sequence.elements[1].command,
            Command::SpeakHeroReachDestination
        );
        assert_eq!(
            sequence.elements[1].command_level, 2,
            "arrival speech must wait for the pathfinding Move to finish"
        );
    }
}

impl EngineInner {
    fn trace_selected_movement_order_pop(
        &self,
        stage: &'static str,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        result: &'static str,
    ) {
        let frame = self.control.frame_counter;
        if !movement_pop_goal_owner_debug_matches(frame, owner) {
            return;
        }

        let manager = &self.orders.sequence_manager;
        let captured = manager.get_element(seq_id, elem_idx).map(|element| {
            (
                element.command,
                element.state,
                element.data.is_movement(),
                element.orders.len(),
                element.current_order().map(|order| {
                    (
                        order.order_type,
                        order.order_id,
                        order.done,
                        order.target_x.to_bits(),
                        order.target_y.to_bits(),
                    )
                }),
            )
        });
        let live_selected = manager.current_element_for_actor(owner);
        let live_order =
            manager
                .current_order_for_actor(owner)
                .map(|(live_seq, live_idx, order)| {
                    (
                        live_seq,
                        live_idx,
                        order.order_type,
                        order.order_id,
                        order.done,
                        order.target_x.to_bits(),
                        order.target_y.to_bits(),
                    )
                });
        let translating = manager.goal_owner_debug_translating();
        let entity = self.world.entities.get(owner).unwrap_or_else(|| {
            panic!("movement-pop diagnostic owner {owner:?} disappeared at frame {frame}")
        });
        let actor = entity.actor_data().unwrap_or_else(|| {
            panic!("movement-pop diagnostic owner {owner:?} is not an actor at frame {frame}")
        });
        let position = entity.position_iface();
        eprintln!(
            "[GOAL_OWNER frame={frame} owner={owner:?} stage=movement_pop_{stage} result={result} captured_seq={seq_id:?} captured_elem={elem_idx} captured={captured:?} live_selected={live_selected:?} live_order={live_order:?} translating={translating:?} active_movement={:?} installed_order={:?} action_state={:?} goal={:?} position={:?} moving={} moving_map={}]",
            actor.active_movement,
            actor.installed_order,
            actor.action_state,
            position.map_goal(),
            position.map_position(),
            position.is_moving(),
            position.is_moving_map(),
        );
    }

    fn pop_selected_movement_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let selection = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .and_then(|element| {
                let owner = element.owner?;
                let selected = (element.state == crate::sequence::SequenceState::InProgress
                    && element.data.is_movement()
                    && self
                        .orders
                        .sequence_manager
                        .current_element_for_actor(owner)
                        == Some((seq_id, elem_idx)))
                .then_some((owner, element.orders.len() == 1));
                Some((owner, selected))
            });
        let diagnostic_owner = selection.map(|(owner, _)| owner);
        if let Some(owner) = diagnostic_owner {
            self.trace_selected_movement_order_pop("entry", owner, seq_id, elem_idx, "pending");
        }
        let selected = selection.and_then(|(_, selected)| selected);
        let Some((owner, final_order_will_exhaust)) = selected else {
            if let Some(owner) = diagnostic_owner {
                let manager = &self.orders.sequence_manager;
                let result = match manager.get_element(seq_id, elem_idx) {
                    None => "rejected_missing_element",
                    Some(element) if element.owner.is_none() => "rejected_missing_owner",
                    Some(element)
                        if element.state != crate::sequence::SequenceState::InProgress =>
                    {
                        "rejected_not_in_progress"
                    }
                    Some(element) if !element.data.is_movement() => "rejected_not_movement",
                    Some(_) => "rejected_live_selection_mismatch",
                };
                self.trace_selected_movement_order_pop("return", owner, seq_id, elem_idx, result);
            }
            // The pop was collected before a synchronous callback selected a
            // replacement. It no longer owns either the actor order or its
            // sprite goal, so applying it now would mutate the replacement.
            return;
        };

        if final_order_will_exhaust {
            // `RHElementActor::DoNextOrder` exhausts the selected Move, and
            // its synchronous `SendCondolationCard` clears PositionGoalMap
            // before a postponed replacement is instructed. Rust's
            // `do_next_order` drives that same synchronous promotion.
            // Invalidate the replacement's queue-time snapshot first, or a
            // promoted MoveWaiting can restore the outgoing goal during the
            // callback and hide the selected-element clear until its first
            // Execute.
            self.orders
                .sequence_manager
                .clear_retained_movement_goals_for_actor(owner);
        }
        self.do_next_order(seq_id, elem_idx);
        self.trace_selected_movement_order_pop("return", owner, seq_id, elem_idx, "accepted");
    }

    pub(in crate::engine) fn advance_live_order_after_terminal_handoff(&mut self, owner: EntityId) {
        if let Some((seq_id, elem_idx)) = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
        {
            let exhausts_pending_move = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .is_some_and(|element| {
                    element.command == crate::element::Command::MoveWaiting
                        && element.orders.len() == 1
                });
            if exhausts_pending_move {
                // The live DoNextOrder below exhausts the re-entrantly
                // selected RHSequenceElementMovement. Its Original
                // SetState(TERMINATED) teardown calls CancelPathRequest, so
                // the retained logical queue head must still complete one
                // frame later with valid=false and an empty raw path
                // (RHpathfinder.cpp:538-598, 712-738; FindPathNodes exits on
                // mbIgnoreNextPath at 3130-3150).
                self.world.pathfinder.cancel_requests_for(owner);
                self.orders.pending_path_requests.cancel_for_owner(owner);
                self.orders
                    .failed_path_requests
                    .retain(|request| request.owner != owner);
            }
            if debug_post_seek_handoff_enabled() {
                let command_and_orders = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .map(|element| (element.command, element.orders.len()));
                eprintln!(
                    "[POST_SEEK frame={} owner={owner:?} stage=live_advance target={:?} command_and_orders={command_and_orders:?} exhausts_pending_move={exhausts_pending_move}]",
                    self.control.frame_counter,
                    (seq_id, elem_idx),
                );
            }
            self.do_next_order(seq_id, elem_idx);
        } else if debug_post_seek_handoff_enabled() {
            eprintln!(
                "[POST_SEEK frame={} owner={owner:?} stage=live_advance_no_current]",
                self.control.frame_counter,
            );
        }
    }

    pub(in crate::engine) fn live_pending_freezing_order(&self, owner: EntityId) -> bool {
        self.orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(seq_id, elem_idx)| {
                self.orders.sequence_manager.get_element(seq_id, elem_idx)
            })
            .is_some_and(|element| {
                element.command == crate::element::Command::MoveWaiting
                    && element.orders.len() == 1
                    && element
                        .current_order()
                        .is_some_and(|order| order.order_type == OrderType::Freezing)
            })
    }

    pub(in crate::engine) fn live_move_has_completed_parallel_element(
        &self,
        owner: EntityId,
    ) -> bool {
        self.orders
            .sequence_manager
            .current_element_for_actor(owner)
            .and_then(|(sequence_id, element_index)| {
                let sequence = self.orders.sequence_manager.get_sequence(sequence_id)?;
                let command_level = sequence.elements.get(element_index)?.command_level;
                Some(
                    sequence
                        .elements
                        .iter()
                        .enumerate()
                        .any(|(index, element)| {
                            index != element_index
                                && element.command_level == command_level
                                && element.state == crate::sequence::SequenceState::Terminated
                        }),
                )
            })
            .unwrap_or(false)
    }

    pub(in crate::engine) fn recent_terminal_move_has_completed_sibling(
        &self,
        owner: EntityId,
    ) -> bool {
        // A Stop-rewritten Move can finish a multi-level arrival sequence
        // immediately before its postponed replacement is instructed. The
        // latest completed movement sequence mirrors the entry-latched
        // `pSequenceElement`; a terminated non-movement sibling proves that
        // Ready ran a continuation on that same terminal stack.
        let live_sequence = self
            .orders
            .sequence_manager
            .current_element_for_actor(owner)
            .map(|(sequence_id, _)| sequence_id);
        self.orders
            .sequence_manager
            .sequences_iter()
            .filter(|sequence| Some(sequence.id) != live_sequence)
            .filter(|sequence| {
                sequence.elements.iter().any(|element| {
                    element.owner == Some(owner)
                        && element.data.is_movement()
                        && element.state == crate::sequence::SequenceState::Terminated
                })
            })
            .max_by_key(|sequence| sequence.id)
            .is_some_and(|sequence| {
                sequence.elements.iter().any(|element| {
                    !element.data.is_movement()
                        && element.state == crate::sequence::SequenceState::Terminated
                })
            })
    }
}

/// `RHElementActorSoldier::Execute` calls `TurnDrunken` before
/// `RHSprite::PerformMotion`. On a fresh order this therefore observes the
/// retained direction goal from the previous motion; PerformMotion installs
/// the new order's goal afterwards through `ComputeIncrementAll`.
fn turn_drunken(pi: &mut crate::position_interface::PositionInterface) {
    let current = u16::from(pi.get_direction());
    let goal = u16::from(pi.get_direction_goal());
    if crate::engine::soldier_helpers::turn_drunken_is_very_slow(current, goal) {
        pi.turn_very_slow();
    } else {
        pi.turn_slow(2);
    }
}

fn should_apply_drunken_turn(
    selected_uses_seek: bool,
    order_action: crate::order::OrderType,
) -> bool {
    !selected_uses_seek && order_action == crate::order::OrderType::WalkingUpright
}

fn should_apply_plain_movement_turn(
    is_drunken_soldier: bool,
    flags: crate::sequence::MoveFlags,
    order_action: crate::order::OrderType,
) -> bool {
    !is_drunken_soldier
        || flags.contains(crate::sequence::MoveFlags::SEEK)
        || order_action != crate::order::OrderType::WalkingUpright
}

#[cfg(test)]
mod drunken_turn_timing_tests {
    use super::{should_apply_drunken_turn, should_apply_plain_movement_turn, turn_drunken};
    use crate::position_interface::{Direction, PositionInterface};

    #[test]
    fn fresh_order_turn_uses_retained_goal_before_motion_initialization() {
        let mut position = PositionInterface::new();
        position.set_direction_instantly(Direction::from_raw(8));

        // The next movement order points north (sector 0), but Original does
        // not install that direction goal until after this Execute prologue.
        turn_drunken(&mut position);
        assert_eq!(position.get_direction(), Direction::from_raw(8));
        assert_eq!(position.get_direction_goal(), Direction::from_raw(8));

        // Mirror the later PerformMotion/ComputeIncrementAll goal update: the
        // sprite keeps the old facing for this frame while future drunken
        // turns now see the new target.
        position.set_direction(Direction::from_raw(0));
        assert_eq!(position.get_direction(), Direction::from_raw(8));
        assert_eq!(position.get_direction_goal(), Direction::from_raw(0));
    }

    #[test]
    fn seek_movement_does_not_add_a_drunken_turn_before_perform_seek() {
        assert!(!should_apply_drunken_turn(
            true,
            crate::order::OrderType::WalkingUpright
        ));
        assert!(should_apply_drunken_turn(
            false,
            crate::order::OrderType::WalkingUpright
        ));
    }

    #[test]
    fn drunken_and_seek_turn_branches_match_original_execute_matrix() {
        for (name, drunk, seek, action, expect_drunken, expect_plain) in [
            (
                "drunk ordinary walk",
                true,
                false,
                crate::order::OrderType::WalkingUpright,
                true,
                false,
            ),
            (
                "drunk seek walk",
                true,
                true,
                crate::order::OrderType::WalkingUpright,
                false,
                true,
            ),
            (
                "drunk startup transition",
                true,
                false,
                crate::order::OrderType::TransitionWaitingUprightWalkingUpright,
                false,
                true,
            ),
            (
                "sober ordinary walk",
                false,
                false,
                crate::order::OrderType::WalkingUpright,
                false,
                true,
            ),
            (
                "sober seek walk",
                false,
                true,
                crate::order::OrderType::WalkingUpright,
                false,
                true,
            ),
        ] {
            assert_eq!(
                drunk && should_apply_drunken_turn(seek, action),
                expect_drunken,
                "{name}: TurnDrunken branch"
            );
            let flags = if seek {
                crate::sequence::MoveFlags::SEEK
            } else {
                crate::sequence::MoveFlags::empty()
            };
            assert_eq!(
                should_apply_plain_movement_turn(drunk, flags, action),
                expect_plain,
                "{name}: plain Turn branch"
            );
        }
    }
}

#[cfg(test)]
#[path = "movement/tests/orphaned_sword_movement.rs"]
mod orphaned_sword_movement_tests;

#[cfg(test)]
#[path = "movement/tests/movement_transition_state.rs"]
mod movement_transition_state_tests;

#[cfg(test)]
#[path = "movement/tests/arrival_snap.rs"]
mod arrival_snap_tests;

#[cfg(test)]
#[path = "movement/tests/aligned_transition_deviation.rs"]
mod aligned_transition_deviation_tests;

#[cfg(test)]
#[path = "movement/tests/path_request_timing.rs"]
mod path_request_timing_tests;

#[cfg(test)]
#[path = "movement/tests/line_jump.rs"]
mod line_jump_tests;
