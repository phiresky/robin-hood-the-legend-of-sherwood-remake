//! Strict Original v48 path-request and mutable graph-state adoption.
//!
//! Original load order decodes failed requests before sequence-pointer fixup,
//! fixes every manager-owned sequence pointer, then restores RHPathFinder's
//! pending FIFO and area states. This module preflights the same complete
//! reference graph against the already-converted sequence plan and applies all
//! path-owned state atomically.

use std::collections::HashSet;

use thiserror::Error;

use crate::{
    coordinates::MapPoint,
    element::{Command, EntityId, Posture},
    engine::{
        EngineInner, FailedPathRequest, LevelAssets, PendingPathRequest, PendingPathRequestQueue,
    },
    order::OrderType,
    pathfinder::PathFinderSpeed,
    sequence::{MoveFlags, SequenceElement, SequenceElementData, SequenceState},
};

use super::{
    LegacySaveAbiProfile,
    adopt::LegacyEntityFixups,
    adopt_sequences::{LegacySequenceAdoptError, LegacySequenceAdoptionPlan},
    payload_base::{LegacyElementRef, LegacyPoint2, LegacySequenceElementRef},
    post_simple::LegacyFailedPathRequests,
    post_tail::{LegacyPathRequest, LegacyPathfinderState},
};

#[derive(Debug, Error)]
pub enum LegacyPathAdoptError {
    #[error("path adoption only supports Linux i386 v48, not {profile:?}")]
    UnsupportedAbi { profile: LegacySaveAbiProfile },
    #[error(transparent)]
    Sequence(#[from] LegacySequenceAdoptError),
    #[error("saved path reference {field} cannot be resolved: {detail}")]
    EntityReference { field: &'static str, detail: String },
    #[error("saved path request {queue}[{index}] has null {field}")]
    NullReference {
        queue: &'static str,
        index: usize,
        field: &'static str,
    },
    #[error(
        "saved path request {queue}[{index}] field {field} has value {value}; expected {expected}"
    )]
    InvalidRequest {
        queue: &'static str,
        index: usize,
        field: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error(
        "saved path request {queue}[{index}] actor {actor:?} does not match sequence-element owner {owner:?}"
    )]
    OwnerMismatch {
        queue: &'static str,
        index: usize,
        actor: EntityId,
        owner: Option<EntityId>,
    },
    #[error("saved pending path FIFO contains actor {actor:?} more than once")]
    DuplicatePendingActor { actor: EntityId },
    #[error(
        "saved pathfinder state shape at layer {layer:?}: saved {saved}, initialized graph {graph}, runtime {runtime}"
    )]
    StateShape {
        layer: Option<usize>,
        saved: usize,
        graph: usize,
        runtime: usize,
    },
    #[error(
        "initialized path graph motion obstacle references grid line {line}, but runtime grid has only {line_count} lines"
    )]
    MissingGridLine { line: usize, line_count: usize },
    #[error(
        "saved pathfinder has do_not_ignore_next_path=true; the v48 writer always emits false after excluding an ignored head"
    )]
    IgnoredHeadNotRepresentable,
}

/// Fully preflighted path-owned state. Applying this value cannot fail and
/// cannot expose a mix of old queues with newly restored graph state.
#[derive(Debug)]
pub(crate) struct LegacyPathAdoptionPlan {
    failed: Vec<FailedPathRequest>,
    pending: PendingPathRequestQueue,
    pathfinder_states: Vec<Vec<u32>>,
    line_active: Vec<bool>,
}

impl LegacyPathAdoptionPlan {
    pub(crate) fn apply(self, engine: &mut EngineInner) {
        engine.orders.failed_path_requests = self.failed;
        engine.orders.pending_path_requests = self.pending;
        engine.world.pathfinder.states = self.pathfinder_states;
        engine.world.fast_grid.line_active = self.line_active;
    }
}

/// Preflight both engine-owned failed requests and RHPathFinder-owned pending
/// requests against the exact sequence conversion that will be installed.
pub(crate) fn preflight_v48_paths(
    engine: &EngineInner,
    assets: &LevelAssets,
    failed: &LegacyFailedPathRequests,
    pathfinder: &LegacyPathfinderState,
    sequences: &LegacySequenceAdoptionPlan,
    entities: &LegacyEntityFixups,
) -> Result<LegacyPathAdoptionPlan, LegacyPathAdoptError> {
    if pathfinder.do_not_ignore_next_path {
        return Err(LegacyPathAdoptError::IgnoredHeadNotRepresentable);
    }

    let mut converted_failed = Vec::with_capacity(failed.requests.len());
    for (index, saved) in failed.requests.iter().enumerate() {
        let request = convert_request(
            engine,
            assets,
            sequences,
            entities,
            "failed",
            index,
            SavedRequest {
                action: saved.action,
                reverse: saved.reverse,
                use_first_point: saved.use_first_point,
                tolerance: saved.tolerance,
                speed: saved.speed,
                area: saved.area,
                half_diagonal_index: saved.half_diagonal_index,
                layer: saved.layer,
                legacy_sector: saved.sector,
                goal: saved.goal,
                source: saved.source,
                actor: saved.actor,
                antagonist: saved.antagonist,
                sequence_element: saved.sequence_element,
            },
        )?;
        converted_failed.push(FailedPathRequest::from_pending(request, saved.time));
    }

    let mut pending_actors = HashSet::new();
    let mut converted_pending = Vec::with_capacity(pathfinder.requests.len());
    for (index, saved) in pathfinder.requests.iter().enumerate() {
        let request = convert_request(
            engine,
            assets,
            sequences,
            entities,
            "pending",
            index,
            SavedRequest::from_pending(saved),
        )?;
        if !pending_actors.insert(request.owner) {
            return Err(LegacyPathAdoptError::DuplicatePendingActor {
                actor: request.owner,
            });
        }
        converted_pending.push(request);
    }

    let (pathfinder_states, line_active) =
        preflight_graph_states(engine, assets, &pathfinder.layer_area_states)?;

    Ok(LegacyPathAdoptionPlan {
        failed: converted_failed,
        pending: PendingPathRequestQueue::restore_v48_waiting(converted_pending),
        pathfinder_states,
        line_active,
    })
}

#[derive(Clone, Copy)]
struct SavedRequest {
    action: i32,
    reverse: bool,
    use_first_point: bool,
    tolerance: f32,
    speed: u8,
    area: u16,
    half_diagonal_index: u16,
    layer: u16,
    legacy_sector: u16,
    goal: LegacyPoint2,
    source: LegacyPoint2,
    actor: LegacyElementRef,
    antagonist: LegacyElementRef,
    sequence_element: LegacySequenceElementRef,
}

impl SavedRequest {
    fn from_pending(saved: &LegacyPathRequest) -> Self {
        Self {
            action: saved.action,
            reverse: saved.reverse,
            use_first_point: saved.use_first_point,
            tolerance: saved.tolerance,
            speed: saved.speed,
            area: saved.area,
            half_diagonal_index: saved.half_diagonal_index,
            layer: saved.layer,
            // This member is serialized even though Original never uses it.
            legacy_sector: saved.sector,
            goal: saved.goal,
            source: saved.source,
            actor: saved.actor,
            antagonist: saved.antagonist,
            sequence_element: saved.sequence_element,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn convert_request(
    engine: &EngineInner,
    assets: &LevelAssets,
    sequences: &LegacySequenceAdoptionPlan,
    entities: &LegacyEntityFixups,
    queue: &'static str,
    index: usize,
    saved: SavedRequest,
) -> Result<PendingPathRequest, LegacyPathAdoptError> {
    let actor = resolve_required_entity(entities, queue, index, "actor", saved.actor)?;
    let antagonist = resolve_optional_entity(entities, "antagonist", saved.antagonist)?;
    if engine
        .world
        .entities
        .get(actor)
        .and_then(|entity| entity.actor_data())
        .is_none()
    {
        return Err(invalid(
            queue,
            index,
            "actor",
            format!("{actor:?}"),
            "a live actor entity",
        ));
    }
    if antagonist.is_some_and(|id| engine.world.entities.get(id).is_none()) {
        return Err(invalid(
            queue,
            index,
            "antagonist",
            format!("{antagonist:?}"),
            "null or a live entity",
        ));
    }

    let Some(sequence_element_id) = saved.sequence_element.0 else {
        return Err(LegacyPathAdoptError::NullReference {
            queue,
            index,
            field: "sequence_element",
        });
    };
    let (element_ref, element) = sequences
        .resolve_element("path_request.sequence_element", saved.sequence_element)?
        .expect("non-null sequence-element reference resolves to Some");
    validate_movement_element(queue, index, actor, sequence_element_id, element)?;

    let (flags, posture, action_state) = match &element.data {
        SequenceElementData::Movement { flags, .. } => (
            *flags,
            element.posture_after_transition,
            element.action_state_after_transition,
        ),
        _ => unreachable!("validate_movement_element accepted non-movement element"),
    };

    let move_action = OrderType::try_from(u32::try_from(saved.action).map_err(|_| {
        invalid(
            queue,
            index,
            "action",
            saved.action,
            "a non-negative RHanimation",
        )
    })?)
    .map_err(|_| invalid(queue, index, "action", saved.action, "a known RHanimation"))?;
    let speed = match saved.speed {
        0 => PathFinderSpeed::Fast,
        1 => PathFinderSpeed::Medium,
        2 => PathFinderSpeed::Slow,
        3 => PathFinderSpeed::VerySlow,
        value => {
            return Err(invalid(
                queue,
                index,
                "speed",
                value,
                "PATHFINDERSPEED_FAST..=PATHFINDERSPEED_VERY_SLOW (0..=3)",
            ));
        }
    };

    validate_finite(queue, index, "tolerance", saved.tolerance)?;
    if saved.tolerance < 0.0 {
        return Err(invalid(
            queue,
            index,
            "tolerance",
            saved.tolerance,
            "a finite non-negative distance",
        ));
    }
    validate_point(queue, index, "source", saved.source)?;
    validate_point(queue, index, "goal", saved.goal)?;

    let layer = usize::from(saved.layer);
    let graph_layer = assets.pathfinder_graph.states.get(layer).ok_or_else(|| {
        invalid(
            queue,
            index,
            "layer",
            saved.layer,
            "an initialized path-graph layer",
        )
    })?;
    let area = assets
        .pathfinder_graph
        .try_convert_sector(saved.area)
        .ok_or_else(|| {
            invalid(
                queue,
                index,
                "area",
                saved.area,
                "an Original sector present in the graph conversion table",
            )
        })?;
    if usize::from(area) >= graph_layer.len() {
        return Err(invalid(
            queue,
            index,
            "area",
            saved.area,
            "a sector mapping inside the saved layer",
        ));
    }
    if usize::from(saved.half_diagonal_index)
        >= assets.pathfinder_graph.static_data.half_diagonals.len()
    {
        return Err(invalid(
            queue,
            index,
            "half_diagonal_index",
            saved.half_diagonal_index,
            "an initialized pathfinder move-box index",
        ));
    }

    // STEP_BACK_IN_COMBAT does not force sword state: Original may rewrite
    // that surviving movement to upright while lowering the weapon.
    let force_sword = flags.contains(MoveFlags::FORCE_SWORD_MOVEMENT);
    let sword_movement_context =
        (posture == Posture::Upright && action_state.is_sword()) || force_sword;

    Ok(PendingPathRequest {
        owner: actor,
        seq_id: element_ref.sequence_id,
        elem_idx: element_ref.element_index,
        source: MapPoint::new(saved.source.x, saved.source.y),
        dest: MapPoint::new(saved.goal.x, saved.goal.y),
        layer: saved.layer,
        sector: saved.area,
        legacy_sector: saved.legacy_sector,
        half_diagonal_idx: saved.half_diagonal_index,
        use_first_point: saved.use_first_point,
        move_action,
        speed,
        reverse: saved.reverse,
        tolerance: saved.tolerance,
        antagonist,
        is_pass_door: false,
        elem_flags: flags,
        sword_movement_context,
        is_fast: flags.contains(MoveFlags::FAST),
    })
}

fn validate_movement_element(
    queue: &'static str,
    index: usize,
    actor: EntityId,
    sequence_element_id: u32,
    element: &SequenceElement,
) -> Result<(), LegacyPathAdoptError> {
    if element.owner != Some(actor) {
        return Err(LegacyPathAdoptError::OwnerMismatch {
            queue,
            index,
            actor,
            owner: element.owner,
        });
    }
    if !matches!(element.data, SequenceElementData::Movement { .. }) {
        return Err(invalid(
            queue,
            index,
            "sequence_element",
            sequence_element_id,
            "a movement sequence element",
        ));
    }
    if element.command != Command::MoveWaiting {
        return Err(invalid(
            queue,
            index,
            "sequence_element.command",
            format!("{:?}", element.command),
            "MoveWaiting",
        ));
    }
    if element.state != SequenceState::InProgress {
        return Err(invalid(
            queue,
            index,
            "sequence_element.state",
            format!("{:?}", element.state),
            "InProgress",
        ));
    }
    Ok(())
}

fn resolve_required_entity(
    entities: &LegacyEntityFixups,
    queue: &'static str,
    index: usize,
    field: &'static str,
    reference: LegacyElementRef,
) -> Result<EntityId, LegacyPathAdoptError> {
    resolve_optional_entity(entities, field, reference)?.ok_or(
        LegacyPathAdoptError::NullReference {
            queue,
            index,
            field,
        },
    )
}

fn resolve_optional_entity(
    entities: &LegacyEntityFixups,
    field: &'static str,
    reference: LegacyElementRef,
) -> Result<Option<EntityId>, LegacyPathAdoptError> {
    entities
        .resolve_element(reference)
        .map_err(|error| LegacyPathAdoptError::EntityReference {
            field,
            detail: error.to_string(),
        })
}

fn preflight_graph_states(
    engine: &EngineInner,
    assets: &LevelAssets,
    saved: &[Vec<u32>],
) -> Result<(Vec<Vec<u32>>, Vec<bool>), LegacyPathAdoptError> {
    let graph = assets.pathfinder_graph.as_ref();
    if saved.len() != graph.states.len() || saved.len() != engine.world.pathfinder.states.len() {
        return Err(LegacyPathAdoptError::StateShape {
            layer: None,
            saved: saved.len(),
            graph: graph.states.len(),
            runtime: engine.world.pathfinder.states.len(),
        });
    }
    for (layer, saved_areas) in saved.iter().enumerate() {
        let graph_areas = graph.states[layer].len();
        let runtime_areas = engine.world.pathfinder.states[layer].len();
        if saved_areas.len() != graph_areas || saved_areas.len() != runtime_areas {
            return Err(LegacyPathAdoptError::StateShape {
                layer: Some(layer),
                saved: saved_areas.len(),
                graph: graph_areas,
                runtime: runtime_areas,
            });
        }
    }
    if graph.static_data.move_layers.len() != saved.len() {
        return Err(LegacyPathAdoptError::StateShape {
            layer: None,
            saved: saved.len(),
            graph: graph.static_data.move_layers.len(),
            runtime: engine.world.pathfinder.states.len(),
        });
    }

    let mut line_active = engine.world.fast_grid.line_active.clone();
    for (layer, states) in saved.iter().enumerate() {
        let move_areas = &graph.static_data.move_layers[layer];
        if move_areas.len() != states.len() {
            return Err(LegacyPathAdoptError::StateShape {
                layer: Some(layer),
                saved: states.len(),
                graph: move_areas.len(),
                runtime: engine.world.pathfinder.states[layer].len(),
            });
        }
        for (area, state) in move_areas.iter().zip(states) {
            for obstacle in &area.motion_obstacles {
                let active = (obstacle.state_id & *state) == obstacle.state_id;
                for &line in &obstacle.grid_line_indices {
                    let index = usize::from(line);
                    let Some(slot) = line_active.get_mut(index) else {
                        return Err(LegacyPathAdoptError::MissingGridLine {
                            line: index,
                            line_count: line_active.len(),
                        });
                    };
                    *slot = active;
                }
            }
        }
    }

    Ok((saved.to_vec(), line_active))
}

fn validate_point(
    queue: &'static str,
    index: usize,
    field: &'static str,
    point: LegacyPoint2,
) -> Result<(), LegacyPathAdoptError> {
    validate_finite(queue, index, field, point.x)?;
    validate_finite(queue, index, field, point.y)
}

fn validate_finite(
    queue: &'static str,
    index: usize,
    field: &'static str,
    value: f32,
) -> Result<(), LegacyPathAdoptError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(invalid(queue, index, field, value, "a finite f32"))
    }
}

fn invalid(
    queue: &'static str,
    index: usize,
    field: &'static str,
    value: impl std::fmt::Display,
    expected: &'static str,
) -> LegacyPathAdoptError {
    LegacyPathAdoptError::InvalidRequest {
        queue,
        index,
        field,
        value: value.to_string(),
        expected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coordinates::MapBBox,
        entity_id::{EntityIdKind, SoldierId},
        fast_find_grid::LineIndex,
        pathfinder::{MotionArea, MotionObstacle, PathGraph},
        sequence::SequenceId,
    };

    fn request(owner: EntityId, legacy_sector: u16) -> PendingPathRequest {
        PendingPathRequest {
            owner,
            seq_id: SequenceId(7),
            elem_idx: 3,
            source: MapPoint::new(1.0, 2.0),
            dest: MapPoint::new(3.0, 4.0),
            layer: 0,
            sector: 12,
            legacy_sector,
            half_diagonal_idx: 0,
            use_first_point: true,
            move_action: OrderType::WalkingUpright,
            speed: PathFinderSpeed::Medium,
            reverse: true,
            tolerance: 6.5,
            antagonist: None,
            is_pass_door: false,
            elem_flags: MoveFlags::REVERSED,
            sword_movement_context: false,
            is_fast: false,
        }
    }

    #[test]
    fn restored_pending_fifo_and_failed_timestamp_keep_exact_request_payload() {
        let first = EntityId::new(4, EntityIdKind::Soldier);
        let second = EntityId::Soldier(SoldierId(9));
        let first_request = request(first, 0x1234);
        let failed = FailedPathRequest::from_pending(first_request.clone(), 0x8765_4321);
        assert_eq!(failed.first_fail_frame, 0x8765_4321);
        assert_eq!(
            failed
                .authoritative_request
                .as_ref()
                .expect("exact failed payload")
                .legacy_sector,
            0x1234
        );

        let queue = PendingPathRequestQueue::restore_v48_waiting(vec![
            first_request,
            request(second, 0xabcd),
        ]);
        assert_eq!(queue.v48_waiting()[0].owner, first);
        assert_eq!(queue.v48_waiting()[0].legacy_sector, 0x1234);
        assert_eq!(queue.v48_waiting()[1].owner, second);
        assert_eq!(queue.v48_waiting()[1].legacy_sector, 0xabcd);
        assert!(!queue.has_in_flight());
    }

    #[test]
    fn graph_state_preflight_synchronizes_motion_lines_without_mutating_engine() {
        let mut engine = EngineInner::new();
        engine.world.pathfinder.states = vec![vec![0x5555_5555]];
        engine.world.fast_grid.line_active = vec![false, true];

        let mut graph = PathGraph::new();
        graph.states = vec![vec![0]];
        graph.layers = vec![vec![vec![Vec::new(), Vec::new()]]];
        graph.alternative_layers = graph.layers.clone();
        graph.static_mut().move_layers = vec![vec![MotionArea {
            skeleton: Vec::new(),
            polygon: Vec::new(),
            motion_obstacles: vec![
                MotionObstacle {
                    state_id: 1,
                    active: false,
                    bounding_box: MapBBox::default(),
                    polygon: Vec::new(),
                    grid_line_indices: vec![LineIndex::new(0).unwrap()],
                },
                MotionObstacle {
                    state_id: 2,
                    active: true,
                    bounding_box: MapBBox::default(),
                    polygon: Vec::new(),
                    grid_line_indices: vec![LineIndex::new(1).unwrap()],
                },
            ],
        }]];
        let mut assets = LevelAssets::new();
        assets.pathfinder_graph = std::sync::Arc::new(graph);

        let (states, lines) =
            preflight_graph_states(&engine, &assets, &[vec![1]]).expect("valid graph state");
        assert_eq!(states, vec![vec![1]]);
        assert_eq!(lines, vec![true, false]);
        assert_eq!(engine.world.pathfinder.states, vec![vec![0x5555_5555]]);
        assert_eq!(engine.world.fast_grid.line_active, vec![false, true]);
    }

    #[test]
    fn graph_state_preflight_rejects_shape_before_mutation() {
        let mut engine = EngineInner::new();
        engine.world.pathfinder.states = vec![vec![1]];
        let mut graph = PathGraph::new();
        graph.states = vec![vec![0]];
        graph.static_mut().move_layers = vec![vec![MotionArea {
            skeleton: Vec::new(),
            polygon: Vec::new(),
            motion_obstacles: Vec::new(),
        }]];
        let mut assets = LevelAssets::new();
        assets.pathfinder_graph = std::sync::Arc::new(graph);

        assert!(matches!(
            preflight_graph_states(&engine, &assets, &[vec![1, 2]]),
            Err(LegacyPathAdoptError::StateShape { layer: Some(0), .. })
        ));
        assert_eq!(engine.world.pathfinder.states, vec![vec![1]]);
    }
}
