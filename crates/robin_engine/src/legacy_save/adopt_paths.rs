//! Strict Original v48 path-request and mutable graph-state adoption.
//!
//! Original-game load order decodes failed requests before sequence-reference fixup,
//! fixes every manager-owned sequence reference, then restores pathfinder
//! pending FIFO and area states. This module preflights the same complete
//! reference graph against the already-converted sequence plan and applies all
//! path-owned state atomically.

use std::collections::HashSet;

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
    adopt::LegacyEntityFixups,
    adopt_common::{AdoptErrorKind, AdoptSite, LegacyAdoptError},
    adopt_sequences::LegacySequenceAdoptionPlan,
    payload_base::{LegacyElementRef, LegacyPoint2, LegacySequenceElementRef},
    post_simple::LegacyFailedPathRequests,
    post_tail::{LegacyPathRequest, LegacyPathfinderState},
};

fn request_site(queue: &'static str, index: usize) -> AdoptSite {
    AdoptSite::owned(format!("saved path request {queue}[{index}]"))
}

const MOTION_OBSTACLE: AdoptSite = AdoptSite::new("initialized path graph motion obstacle");

/// Fully preflighted path-owned state. Applying this value cannot fail and
/// cannot expose a mix of old queues with newly restored graph state.
#[derive(Debug)]
pub(crate) struct LegacyPathAdoptionPlan {
    failed: Vec<FailedPathRequest>,
    pending: PendingPathRequestQueue,
    pathfinder_states: Vec<Vec<u32>>,
    line_updates: Vec<(usize, bool)>,
    sector_updates: Vec<(usize, bool)>,
}

impl LegacyPathAdoptionPlan {
    pub(crate) fn apply(self, engine: &mut EngineInner) {
        engine
            .orders
            .install_legacy_path_schedule(self.pending, self.failed);
        engine.world.pathfinder.states = self.pathfinder_states;
        for (index, active) in self.line_updates {
            engine.world.fast_grid_mut().line_active[index] = active;
        }
        for (index, active) in self.sector_updates {
            engine.world.fast_grid_mut().sector_active[index] = active;
        }
    }
}

/// Preflight both engine-owned failed requests and pathfinder-owned pending
/// requests against the exact sequence conversion that will be installed.
pub(crate) fn preflight_v48_paths(
    engine: &EngineInner,
    assets: &LevelAssets,
    failed: &LegacyFailedPathRequests,
    pathfinder: &LegacyPathfinderState,
    sequences: &LegacySequenceAdoptionPlan,
    entities: &LegacyEntityFixups,
) -> Result<LegacyPathAdoptionPlan, LegacyAdoptError> {
    if pathfinder.do_not_ignore_next_path {
        return Err(AdoptErrorKind::IgnoredHeadNotRepresentable.into());
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
            return Err(AdoptErrorKind::DuplicatePendingActor {
                actor: request.owner,
            }
            .into());
        }
        converted_pending.push(request);
    }

    let (pathfinder_states, line_updates, sector_updates) =
        preflight_graph_states(engine, assets, &pathfinder.layer_area_states)?;

    Ok(LegacyPathAdoptionPlan {
        failed: converted_failed,
        pending: PendingPathRequestQueue::restore_v48_waiting(converted_pending),
        pathfinder_states,
        line_updates,
        sector_updates,
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
            // This value is serialized even though the original game never uses it.
            legacy_sector: saved.sector,
            goal: saved.goal,
            source: saved.source,
            actor: saved.actor,
            antagonist: saved.antagonist,
            sequence_element: saved.sequence_element,
        }
    }
}

fn convert_request(
    engine: &EngineInner,
    assets: &LevelAssets,
    sequences: &LegacySequenceAdoptionPlan,
    entities: &LegacyEntityFixups,
    queue: &'static str,
    index: usize,
    saved: SavedRequest,
) -> Result<PendingPathRequest, LegacyAdoptError> {
    let site = request_site(queue, index);
    let actor = resolve_required_entity(entities, &site, "actor", saved.actor)?;
    let antagonist = resolve_optional_entity(entities, "antagonist", saved.antagonist)?;
    if engine
        .world
        .entities
        .get(actor)
        .and_then(|entity| entity.actor_data())
        .is_none()
    {
        return Err(site.invalid("actor", format!("{actor:?}"), "a live actor entity"));
    }
    if antagonist.is_some_and(|id| engine.world.entities.get(id).is_none()) {
        return Err(site.invalid(
            "antagonist",
            format!("{antagonist:?}"),
            "null or a live entity",
        ));
    }

    let Some(sequence_element_id) = saved.sequence_element.0 else {
        return Err(site.field_error("sequence_element", AdoptErrorKind::NullReference));
    };
    let (element_ref, element) = sequences
        .resolve_element("path_request.sequence_element", saved.sequence_element)?
        .expect("non-null sequence-element reference resolves to Some");
    validate_movement_element(&site, actor, sequence_element_id, element)?;

    let (flags, posture, action_state) = match &element.data {
        SequenceElementData::Movement { flags, .. } => (
            *flags,
            element.posture_after_transition,
            element.action_state_after_transition,
        ),
        _ => unreachable!("validate_movement_element accepted non-movement element"),
    };

    let move_action = OrderType::try_from(
        u32::try_from(saved.action)
            .map_err(|_| site.invalid("action", saved.action, "a non-negative animation"))?,
    )
    .map_err(|_| site.invalid("action", saved.action, "a known animation"))?;
    let speed = match saved.speed {
        0 => PathFinderSpeed::Fast,
        1 => PathFinderSpeed::Medium,
        2 => PathFinderSpeed::Slow,
        3 => PathFinderSpeed::VerySlow,
        value => {
            return Err(site.invalid(
                "speed",
                value,
                "PATHFINDERSPEED_FAST..=PATHFINDERSPEED_VERY_SLOW (0..=3)",
            ));
        }
    };

    site.finite("tolerance", saved.tolerance)?;
    if saved.tolerance < 0.0 {
        return Err(site.invalid(
            "tolerance",
            saved.tolerance,
            "a finite non-negative distance",
        ));
    }
    site.finite_point("source", saved.source)?;
    site.finite_point("goal", saved.goal)?;

    let layer = usize::from(saved.layer);
    let graph_layer = assets
        .navigation
        .pathfinder_graph
        .states
        .get(layer)
        .ok_or_else(|| site.invalid("layer", saved.layer, "an initialized path-graph layer"))?;
    let area = assets
        .navigation
        .pathfinder_graph
        .try_convert_sector(saved.area)
        .ok_or_else(|| {
            site.invalid(
                "area",
                saved.area,
                "an Original sector present in the graph conversion table",
            )
        })?;
    if usize::from(area) >= graph_layer.len() {
        return Err(site.invalid(
            "area",
            saved.area,
            "a sector mapping inside the saved layer",
        ));
    }
    if usize::from(saved.half_diagonal_index)
        >= assets
            .navigation
            .pathfinder_graph
            .static_data
            .half_diagonals
            .len()
    {
        return Err(site.invalid(
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
        restored_from_v48: true,
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
    site: &AdoptSite,
    actor: EntityId,
    sequence_element_id: u32,
    element: &SequenceElement,
) -> Result<(), LegacyAdoptError> {
    if element.owner != Some(actor) {
        return Err(site.error(AdoptErrorKind::PathOwnerMismatch {
            actor,
            owner: element.owner,
        }));
    }
    if !matches!(element.data, SequenceElementData::Movement { .. }) {
        return Err(site.invalid(
            "sequence_element",
            sequence_element_id,
            "a movement sequence element",
        ));
    }
    if element.command != Command::MoveWaiting {
        return Err(site.invalid(
            "sequence_element.command",
            format!("{:?}", element.command),
            "MoveWaiting",
        ));
    }
    if element.state != SequenceState::InProgress {
        return Err(site.invalid(
            "sequence_element.state",
            format!("{:?}", element.state),
            "InProgress",
        ));
    }
    Ok(())
}

fn resolve_required_entity(
    entities: &LegacyEntityFixups,
    site: &AdoptSite,
    field: &'static str,
    reference: LegacyElementRef,
) -> Result<EntityId, LegacyAdoptError> {
    resolve_optional_entity(entities, field, reference)?
        .ok_or_else(|| site.field_error(field, AdoptErrorKind::NullReference))
}

fn resolve_optional_entity(
    entities: &LegacyEntityFixups,
    field: &'static str,
    reference: LegacyElementRef,
) -> Result<Option<EntityId>, LegacyAdoptError> {
    entities
        .resolve_element(reference)
        .map_err(|error| error.context(format!("saved path reference {field} cannot be resolved")))
}

fn preflight_graph_states(
    engine: &EngineInner,
    assets: &LevelAssets,
    saved: &[Vec<u32>],
) -> Result<(Vec<Vec<u32>>, Vec<(usize, bool)>, Vec<(usize, bool)>), LegacyAdoptError> {
    let graph = assets.navigation.pathfinder_graph.as_ref();
    if saved.len() != graph.states.len() || saved.len() != engine.world.pathfinder.states.len() {
        return Err(AdoptErrorKind::PathStateShape {
            layer: None,
            saved: saved.len(),
            graph: graph.states.len(),
            runtime: engine.world.pathfinder.states.len(),
        }
        .into());
    }
    for (layer, saved_areas) in saved.iter().enumerate() {
        let graph_areas = graph.states[layer].len();
        let runtime_areas = engine.world.pathfinder.states[layer].len();
        if saved_areas.len() != graph_areas || saved_areas.len() != runtime_areas {
            return Err(AdoptErrorKind::PathStateShape {
                layer: Some(layer),
                saved: saved_areas.len(),
                graph: graph_areas,
                runtime: runtime_areas,
            }
            .into());
        }
    }
    if graph.static_data.move_layers.len() != saved.len() {
        return Err(AdoptErrorKind::PathStateShape {
            layer: None,
            saved: saved.len(),
            graph: graph.static_data.move_layers.len(),
            runtime: engine.world.pathfinder.states.len(),
        }
        .into());
    }

    let mut line_updates = Vec::new();
    let mut sector_updates = Vec::new();
    for (layer, states) in saved.iter().enumerate() {
        let move_areas = &graph.static_data.move_layers[layer];
        if move_areas.len() != states.len() {
            return Err(AdoptErrorKind::PathStateShape {
                layer: Some(layer),
                saved: states.len(),
                graph: move_areas.len(),
                runtime: engine.world.pathfinder.states[layer].len(),
            }
            .into());
        }
        for (area, state) in move_areas.iter().zip(states) {
            for obstacle in &area.motion_obstacles {
                let active = (obstacle.state_id & *state) == obstacle.state_id;
                let sector = obstacle.grid_sector_index.ok_or_else(|| {
                    MOTION_OBSTACLE.error(AdoptErrorKind::Missing {
                        what: "fast-grid sector binding",
                    })
                })?;
                let index =
                    usize::try_from(sector.get()).expect("u32 sector index does not fit usize");
                let sector_count = engine.world.fast_grid.sector_active.len();
                if index >= sector_count {
                    return Err(MOTION_OBSTACLE.out_of_range(
                        "grid_sector_index",
                        "grid sector",
                        index,
                        sector_count,
                    ));
                }
                sector_updates.push((index, active));
                for &line in &obstacle.grid_line_indices {
                    let index = usize::from(line);
                    if index >= engine.world.fast_grid.line_active.len() {
                        return Err(MOTION_OBSTACLE.out_of_range(
                            "grid_line_indices",
                            "grid line",
                            index,
                            engine.world.fast_grid.line_active.len(),
                        ));
                    }
                    line_updates.push((index, active));
                }
            }
        }
    }

    Ok((saved.to_vec(), line_updates, sector_updates))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coordinates::MapBBox,
        entity_id::{EntityIdKind, SoldierId},
        fast_find_grid::{LineIndex, SectorIndex},
        pathfinder::{MotionArea, MotionObstacle, PathGraph},
        sequence::SequenceId,
    };

    fn request(owner: EntityId, legacy_sector: u16) -> PendingPathRequest {
        PendingPathRequest {
            restored_from_v48: true,
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
        assert_eq!(failed.request.legacy_sector, 0x1234);

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
    fn graph_state_preflight_synchronizes_motion_grid_without_mutating_engine() {
        let mut engine = EngineInner::new();
        engine.world.pathfinder.states = vec![vec![0x5555_5555]];
        engine.world.fast_grid_mut().line_active = vec![false, true, false];
        engine.world.fast_grid_mut().sector_active = vec![false, true, false];

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
                    grid_sector_index: SectorIndex::new(0),
                    grid_line_indices: vec![LineIndex::new(0).unwrap()],
                },
                MotionObstacle {
                    state_id: 2,
                    active: true,
                    bounding_box: MapBBox::default(),
                    polygon: Vec::new(),
                    grid_sector_index: SectorIndex::new(1),
                    grid_line_indices: vec![LineIndex::new(1).unwrap()],
                },
            ],
        }]];
        let mut assets = LevelAssets::new();
        assets.navigation.pathfinder_graph = std::sync::Arc::new(graph);

        let (states, line_updates, sector_updates) =
            preflight_graph_states(&engine, &assets, &[vec![1]]).expect("valid graph state");
        assert_eq!(states, vec![vec![1]]);
        assert_eq!(line_updates, vec![(0, true), (1, false)]);
        assert_eq!(sector_updates, vec![(0, true), (1, false)]);
        assert_eq!(engine.world.pathfinder.states, vec![vec![0x5555_5555]]);
        assert_eq!(engine.world.fast_grid.line_active, vec![false, true, false]);
        assert_eq!(
            engine.world.fast_grid.sector_active,
            vec![false, true, false]
        );

        // Full legacy adoption applies the independently preflighted grid plan
        // before this path plan. Preserve its unrelated patch/door flags and
        // overwrite only the motion-obstacle slots represented above.
        engine.world.fast_grid_mut().line_active = vec![false, false, true];
        engine.world.fast_grid_mut().sector_active = vec![false, false, true];
        LegacyPathAdoptionPlan {
            failed: Vec::new(),
            pending: PendingPathRequestQueue::default(),
            pathfinder_states: states,
            line_updates,
            sector_updates,
        }
        .apply(&mut engine);
        assert_eq!(engine.world.fast_grid.line_active, vec![true, false, true]);
        assert_eq!(
            engine.world.fast_grid.sector_active,
            vec![true, false, true]
        );
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
        assets.navigation.pathfinder_graph = std::sync::Arc::new(graph);

        assert!(matches!(
            preflight_graph_states(&engine, &assets, &[vec![1, 2]]),
            Err(LegacyAdoptError {
                kind: AdoptErrorKind::PathStateShape { layer: Some(0), .. },
                ..
            })
        ));
        assert_eq!(engine.world.pathfinder.states, vec![vec![1]]);
    }
}
