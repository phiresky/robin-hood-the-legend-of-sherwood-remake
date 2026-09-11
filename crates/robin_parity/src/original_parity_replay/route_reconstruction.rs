//! Translate recorded route metadata into runtime commands.
use super::{
    BTreeSet, Engine, EntityId, EntityMap, LegacyGridSectorAsset, MapPoint, SectorNumber,
    TraceCommand, TraceEntityId, TraceJsonTree, TraceJsonValue, TraceRouteConstructionEvent,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReplayDropAleResolution {
    pub(super) goal: (SectorNumber, u16),
    pub(super) goal_sector_index: Option<robin_engine::fast_find_grid::SectorIndex>,
    pub(super) recorded_gate_path: Option<robin_engine::gate::RecordedGatePath>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ReplayGroupMoveResolution {
    pub(super) door_route: bool,
    /// A patch sector is a real Original motion-area identity but has no
    /// standalone Rust position-sector number. For a successful recorded
    /// gate route, its terminal gate exit is the equivalent Rust graph goal.
    pub(super) unmapped_goal_search_sector: Option<u16>,
    /// Successful movement-sequence gate paths are already
    /// authoritative at this boundary. Replaying them avoids a second A*
    /// search choosing a different valid path and changing the emitted
    /// building waits (including their RNG draws).
    pub(super) recorded_gate_routes: Vec<(TraceEntityId, Vec<(u32, bool)>)>,
    /// Authoritative failed movement-sequence searches, keyed by actor.
    /// The empty gate list is an observed failure outcome, not permission to
    /// run Rust's A* again against reconstructed topology.
    pub(super) recorded_failed_gate_routes: Vec<TraceEntityId>,
}

pub(super) fn required_route_construction_ordinal(event: &TraceRouteConstructionEvent) -> u64 {
    match event
        .draft_diagnostics
        .get("ordinal")
        .map(TraceJsonValue::tree)
    {
        Some(TraceJsonTree::Unsigned(ordinal)) => ordinal,
        other => panic!("schema-16 route event lacks an unsigned ordinal: {other:?}"),
    }
}

/// Restore additive route diagnostics omitted by early schema-16 captures.
///
/// Original appends each event immediately after assigning the incrementing
/// per-frame counter, so vector position is the
/// authoritative missing ordinal. The archived pre-failure-diagnostics
/// recorder emitted route events only after successful construction; the
/// later route-attempt/failure reporting added explicit failure events and
/// the `result` field together. Therefore
/// an omitted result in this legacy generation is authoritatively success.
/// This is called only for the legacy header generation; current captures
/// must continue to carry both fields explicitly.
pub(super) fn restore_legacy_route_construction_diagnostics(
    events: &mut [TraceRouteConstructionEvent],
) {
    for (ordinal, event) in events.iter_mut().enumerate() {
        let ordinal = u64::try_from(ordinal).expect("route event count exceeds u64");
        match event
            .draft_diagnostics
            .get("ordinal")
            .map(TraceJsonValue::tree)
        {
            None => {
                event.draft_diagnostics.insert(
                    "ordinal".to_owned(),
                    TraceJsonValue::from(TraceJsonTree::Unsigned(ordinal)),
                );
            }
            Some(TraceJsonTree::Unsigned(recorded)) => assert_eq!(
                recorded, ordinal,
                "legacy schema-16 route ordinal disagrees with Original append order"
            ),
            other => panic!("legacy schema-16 route event has an invalid ordinal: {other:?}"),
        }
        event
            .draft_diagnostics
            .entry("result".to_owned())
            .or_insert_with(|| TraceJsonValue::from(TraceJsonTree::String("success".to_owned())));
    }
}

/// Recover whether movement-sequence construction selected its internal
/// gate-path or door-entry branch for a current-schema group move.
/// Both branches record route kind `move`; `move_to_door` belongs to the
/// separate door-movement API and is not this discriminator.
/// The retained Original sparse sector topology preserves that distinction:
/// a door goal names its exact gate, while an ordinary patch remains ordinary
/// even when its route happens to end across that same overlay door.
// TODO(parity-schema): record the selected group-move route constructor on
// TraceCommand::GroupMove so future traces do not need this event join.
pub(super) fn resolve_current_group_move_route(
    command: &TraceCommand,
    route_events: &[TraceRouteConstructionEvent],
    consumed_route_ordinals: &mut BTreeSet<u64>,
    entity_map: &EntityMap,
    retained_sector_kinds: &[LegacyGridSectorAsset],
) -> Option<ReplayGroupMoveResolution> {
    let TraceCommand::GroupMove {
        actors,
        goal_sector,
        ..
    } = command
    else {
        return None;
    };
    let goal_sector = u16::try_from(*goal_sector)
        .unwrap_or_else(|_| panic!("schema-16 group-move goal sector is negative: {goal_sector}"));
    let goal_kind = retained_sector_kinds
        .get(usize::from(goal_sector))
        .unwrap_or_else(|| {
            panic!(
                "schema-16 group-move goal sector {goal_sector} is absent from retained Original topology"
            )
        });
    let goal_door = match goal_kind {
        LegacyGridSectorAsset::Door { gate_index } => Some(entity_map.translate_gate(*gate_index)),
        LegacyGridSectorAsset::NullOrOrdinary
        | LegacyGridSectorAsset::Building
        | LegacyGridSectorAsset::Lift => None,
    };

    let mut matching = route_events
        .iter()
        .filter_map(|event| {
            let ordinal = required_route_construction_ordinal(event);
            if consumed_route_ordinals.contains(&ordinal)
                || !actors.contains(&event.actor)
                || event.goal_sector != goal_sector
                || event.kind != "move"
            {
                return None;
            }
            Some((ordinal, event))
        })
        .collect::<Vec<_>>();
    // Same-sector moves do not construct a gate route, but retained Original
    // topology still authoritatively identifies whether group movement's
    // selected goal was a door. Do not fall back to Rust's overlapping-polygon
    // hit in that case.
    if matching.is_empty() {
        return Some(ReplayGroupMoveResolution {
            door_route: goal_door.is_some(),
            unmapped_goal_search_sector: None,
            recorded_gate_routes: Vec::new(),
            recorded_failed_gate_routes: Vec::new(),
        });
    }
    matching.sort_unstable_by_key(|(ordinal, _)| *ordinal);
    // Group movement executes movement once for each selected actor, and
    // that reaches at most one movement-sequence route construction
    // in the original game. A
    // frame can nevertheless contain several group-move commands with the
    // same actor and goal sector. Their route events share the only identities
    // schema 16 recorded for this join, so greedily taking every match assigns
    // later commands' routes to the first command. Consume the earliest
    // unclaimed route per actor and leave later ordinals for the following
    // command in frame order.
    let mut actors_with_route = BTreeSet::new();
    matching.retain(|(_, event)| actors_with_route.insert(event.actor));
    if let Some(goal_door) = goal_door {
        assert!(
            matching.iter().all(|(_, event)| {
                !matches!(
                    event.draft_diagnostics.get("result").map(TraceJsonValue::tree),
                    Some(TraceJsonTree::String(result)) if result == "success"
                ) || event
                    .gates
                    .last()
                    .is_some_and(|gate| entity_map.translate_gate(gate.gate_id) == goal_door)
            }),
            "successful door-target group move did not terminate at retained goal door {goal_door}: {matching:?}"
        );
    }
    let door_route = goal_door.is_some();
    let mut terminal_exit_sectors = matching
        .iter()
        .filter_map(|(_, event)| {
            event.gates.last().map(|gate| {
                if gate.direct {
                    gate.sector_in
                } else {
                    gate.sector_out
                }
            })
        })
        .collect::<BTreeSet<_>>();
    assert!(
        terminal_exit_sectors.len() <= 1,
        "one group move produced routes ending in different sectors: {matching:?}"
    );
    let unmapped_goal_search_sector = terminal_exit_sectors.pop_first();
    let recorded_gate_routes = matching
        .iter()
        .filter(|(_, event)| {
            matches!(
                event.draft_diagnostics.get("result").map(TraceJsonValue::tree),
                Some(TraceJsonTree::String(result)) if result == "success"
            ) && !event.gates.is_empty()
        })
        .map(|(_, event)| {
            (
                event.actor,
                event
                    .gates
                    .iter()
                    .map(|gate| (gate.gate_id, gate.direct))
                    .collect(),
            )
        })
        .collect();
    let recorded_failed_gate_routes = matching
        .iter()
        .filter(|(_, event)| {
            matches!(
                event.draft_diagnostics.get("result").map(TraceJsonValue::tree),
                Some(TraceJsonTree::String(result)) if result == "failure"
            )
        })
        .map(|(_, event)| {
            assert!(
                event.gates.is_empty(),
                "failed schema-16 group-move route unexpectedly retained gates: {event:?}"
            );
            event.actor
        })
        .collect::<Vec<_>>();
    assert_eq!(
        recorded_failed_gate_routes
            .iter()
            .collect::<BTreeSet<_>>()
            .len(),
        recorded_failed_gate_routes.len(),
        "one schema-16 group move recorded multiple failed routes for the same actor"
    );
    for (ordinal, _) in matching {
        assert!(
            consumed_route_ordinals.insert(ordinal),
            "schema-16 route ordinal {ordinal} matched twice"
        );
    }
    Some(ReplayGroupMoveResolution {
        door_route,
        unmapped_goal_search_sector,
        recorded_gate_routes,
        recorded_failed_gate_routes,
    })
}

/// Recover the goal that Original retained before authorizing a DropAle
/// destination. The current schema does not yet carry that goal on
/// `drop_ale_at`, but a
/// cross-sector Seek records it in the same frame's route-construction stream.
/// Match by route ordinal plus stable actor/point identity; projected point
/// containment is intentionally not consulted because overlapping floors can
/// select a different layer. Same-sector seeks publish no route event, so the
/// caller also supplies a geometrically verified same-sector identity as a
/// fallback. Legacy schema-16 artifacts can omit the route stream entirely;
/// in that case a target outside the actor's exact sector must fall back to
/// ordinary live resolution rather than being mislabeled as same-sector.
// TODO(parity-schema): record DropAle's pre-authorization goal sector/layer on
// the command itself so replay does not have to infer same-sector destinations.
pub(super) fn resolve_current_drop_ale(
    command: &TraceCommand,
    route_events: &[TraceRouteConstructionEvent],
    consumed_route_ordinals: &mut BTreeSet<u64>,
    entity_map: &EntityMap,
    same_sector_goal: Option<ReplayDropAleResolution>,
    qa_recording: bool,
) -> Option<ReplayDropAleResolution> {
    let TraceCommand::DropAleAt { actor, target, .. } = command else {
        return None;
    };

    let actor = entity_map.translate(*actor);
    let matching_events = route_events
        .iter()
        .filter_map(|event| {
            let ordinal = required_route_construction_ordinal(event);
            if consumed_route_ordinals.contains(&ordinal)
                || event.kind != "move"
                || entity_map.translate(event.actor) != actor
                || event.goal.x.bits != target.x.bits
                || event.goal.y.bits != target.y.bits
                || event.source_sector == event.goal_sector
            {
                return None;
            }
            Some((ordinal, event))
        })
        .collect::<Vec<_>>();
    assert!(
        matching_events.len() <= 1,
        "schema-16 DropAle command matched {} exact route events",
        matching_events.len()
    );
    let Some((ordinal, event)) = matching_events.into_iter().next() else {
        if qa_recording {
            // The original game stores the selected target-sector identity in the quick action's
            // Seek without launching it, so there is intentionally no route
            // event from which schema 16 can recover that pointer. The actor
            // sector is not a valid substitute: the selected target may be a
            // different (or duplicate-number) sector.
            //
            // TODO(parity-schema): record DropAle's selected target sector,
            // exact arena identity, and layer directly on the command.
            panic!(
                "schema-16 DropAle recorded as a quick action has no authoritative target-sector identity"
            );
        }
        // A cross-sector DropAle must have an authoritative route event. Do
        // not disguise a mismatched/corrupt event as a same-sector command.
        let has_actor_route = route_events.iter().any(|event| {
            event.kind == "move"
                && entity_map.translate(event.actor) == actor
                && event.source_sector != event.goal_sector
        });
        return (!has_actor_route).then_some(same_sector_goal).flatten();
    };
    assert!(
        consumed_route_ordinals.insert(ordinal),
        "schema-16 route ordinal {ordinal} matched twice"
    );

    let (goal_sector, goal_sector_index) =
        entity_map.translate_required_drop_ale_goal_sector(event.goal_sector);
    let recorded_gate_path = recorded_gate_path_from_event(event, entity_map);
    Some(ReplayDropAleResolution {
        goal: (goal_sector, event.goal_level),
        goal_sector_index: Some(goal_sector_index),
        recorded_gate_path: Some(recorded_gate_path),
    })
}

pub(super) fn recorded_gate_path_from_event(
    event: &TraceRouteConstructionEvent,
    entity_map: &EntityMap,
) -> robin_engine::gate::RecordedGatePath {
    let (source_sector, source_sector_index) =
        entity_map.translate_required_drop_ale_goal_sector(event.source_sector);
    let outcome = match event
        .draft_diagnostics
        .get("result")
        .map(TraceJsonValue::tree)
    {
        Some(TraceJsonTree::String(result)) if result == "success" => {
            assert!(
                !event.gates.is_empty(),
                "successful cross-sector DropAle route has no gates: {event:?}"
            );
            robin_engine::gate::RecordedGateOutcome::Success(
                event
                    .gates
                    .iter()
                    .map(|gate| robin_engine::gate::GatePathStep {
                        door_index: robin_engine::gate::DoorIndex::from(
                            entity_map.translate_gate(gate.gate_id),
                        ),
                        direct: gate.direct,
                    })
                    .collect(),
            )
        }
        Some(TraceJsonTree::String(result)) if result == "failure" => {
            assert!(
                event.gates.is_empty(),
                "failed cross-sector DropAle route retained gates: {event:?}"
            );
            robin_engine::gate::RecordedGateOutcome::Failure
        }
        other => panic!("schema-16 DropAle route has invalid result: {other:?}"),
    };
    robin_engine::gate::RecordedGatePath {
        source_sector,
        source_sector_index: Some(source_sector_index),
        source_layer: event.source_level,
        outcome,
    }
}

pub(super) fn collect_current_delayed_drop_ale_routes(
    route_events: &[TraceRouteConstructionEvent],
    consumed_drop_ale_route_ordinals: &mut BTreeSet<u64>,
    consumed_group_move_route_ordinals: &BTreeSet<u64>,
    entity_map: &EntityMap,
    engine: &mut Engine,
) -> Vec<robin_engine::engine::RecordedDropAleRoute> {
    let replay_setup = engine.parity_replay_setup();
    collect_current_delayed_drop_ale_routes_matching(
        route_events,
        consumed_drop_ale_route_ordinals,
        consumed_group_move_route_ordinals,
        entity_map,
        |actor, destination| replay_setup.has_pending_recorded_drop_ale_route(actor, destination),
    )
}

pub(super) fn collect_current_delayed_drop_ale_routes_matching(
    route_events: &[TraceRouteConstructionEvent],
    consumed_drop_ale_route_ordinals: &mut BTreeSet<u64>,
    consumed_group_move_route_ordinals: &BTreeSet<u64>,
    entity_map: &EntityMap,
    mut has_pending_replay_seek: impl FnMut(EntityId, robin_engine::coordinates::MapPoint) -> bool,
) -> Vec<robin_engine::engine::RecordedDropAleRoute> {
    assert!(
        consumed_drop_ale_route_ordinals.is_disjoint(consumed_group_move_route_ordinals),
        "schema-16 route ordinal was consumed independently by DropAle and group-move joins"
    );
    let mut seen_route_ordinals = BTreeSet::new();
    let mut routes = Vec::new();
    for event in route_events {
        let ordinal = required_route_construction_ordinal(event);
        if event.kind != "move" || event.source_sector == event.goal_sector {
            continue;
        }
        assert!(
            seen_route_ordinals.insert(ordinal),
            "schema-16 route stream contains duplicate ordinal {ordinal}"
        );
        if consumed_drop_ale_route_ordinals.contains(&ordinal)
            || consumed_group_move_route_ordinals.contains(&ordinal)
        {
            continue;
        }
        let actor = entity_map.translate(event.actor);
        let destination = event.goal.into();
        // A route-construction event is a shared diagnostic stream: ordinary
        // movement and group moves can leave cross-sector entries here too.
        // Only a staged point-Seek proves that this event belongs to a delayed
        // DropAle command. Older recordings cannot carry a separate provenance
        // bit, so treating every otherwise-unclaimed move route as DropAle
        // both rejects valid traces and is not justified by Original's flow.
        if !has_pending_replay_seek(actor, destination) {
            continue;
        }
        claim_delayed_drop_ale_route_ordinal(
            ordinal,
            consumed_drop_ale_route_ordinals,
            consumed_group_move_route_ordinals,
        );
        let (goal_sector, goal_sector_index) =
            entity_map.translate_required_drop_ale_goal_sector(event.goal_sector);
        routes.push(robin_engine::engine::RecordedDropAleRoute {
            actor,
            destination,
            goal_sector,
            goal_sector_index,
            goal_layer: event.goal_level,
            recorded_gate_path: recorded_gate_path_from_event(event, entity_map),
        });
    }
    routes
}

pub(super) fn claim_delayed_drop_ale_route_ordinal(
    ordinal: u64,
    consumed_drop_ale_route_ordinals: &mut BTreeSet<u64>,
    consumed_group_move_route_ordinals: &BTreeSet<u64>,
) {
    assert!(
        !consumed_group_move_route_ordinals.contains(&ordinal),
        "schema-16 route ordinal {ordinal} was already consumed by the group-move join"
    );
    assert!(
        consumed_drop_ale_route_ordinals.insert(ordinal),
        "schema-16 delayed DropAle route ordinal {ordinal} matched twice"
    );
}

pub(super) fn legacy_drop_ale_target_is_same_exact_sector(
    actor: robin_engine::position_interface::SectorHandle,
    target: robin_engine::position_interface::SectorHandle,
) -> bool {
    target.number() == actor.number()
        && target.arena_index().is_some()
        && target.arena_index() == actor.arena_index()
}

pub(super) fn current_drop_ale_same_sector_goal(
    command: &TraceCommand,
    entity_map: &EntityMap,
    engine: &Engine,
) -> Option<ReplayDropAleResolution> {
    let TraceCommand::DropAleAt { actor, target, .. } = command else {
        return None;
    };
    let actor = entity_map.translate(*actor);
    let entity = engine
        .get_entity(actor)
        .unwrap_or_else(|| panic!("schema-16 DropAle actor {actor:?} is missing"));
    let element = entity.element_data();
    let sector = element
        .sector()
        .unwrap_or_else(|| panic!("schema-16 DropAle actor {actor:?} has no current sector"));
    let target_point: MapPoint = (*target).into();
    let grid = engine.fast_grid();
    let hit = grid.get_sector_screen(target_point, element.position_map());
    let target_sector = hit.sector_idx.and_then(|index| {
        let selected = grid
            .level
            .sectors
            .get(usize::from(index))
            .unwrap_or_else(|| panic!("schema-16 DropAle target sector {index} is missing"));
        if selected.sector_type.is_patch() || selected.sector_type.is_jump() {
            let underlying = selected.underlying_sector.unwrap_or_else(|| {
                panic!("schema-16 DropAle target overlay {index} has no underlying sector")
            });
            let underlying_sector = grid
                .level
                .sectors
                .get(usize::from(underlying))
                .unwrap_or_else(|| {
                    panic!(
                        "schema-16 DropAle target overlay {index} references missing sector {underlying}"
                    )
                });
            u16::try_from(underlying_sector.sector_number.get())
                .ok()
                .and_then(robin_engine::position_interface::SectorHandle::new)
                .map(|sector| sector.with_arena_index(underlying))
        } else {
            hit.sector_handle()
        }
    });
    let target_is_same_exact_sector = target_sector
        .is_some_and(|target| legacy_drop_ale_target_is_same_exact_sector(sector, target));
    if !target_is_same_exact_sector {
        // The old trace did not attest a gate path. Let DropAle's normal live
        // target/route resolver reconstruct it from the already-authorized
        // target point instead of inventing the actor sector as its goal.
        return None;
    }
    let public = i16::try_from(sector.get()).unwrap_or_else(|_| {
        panic!(
            "schema-16 DropAle actor {actor:?} sector {} exceeds its signed identity domain",
            sector.get()
        )
    });
    Some(ReplayDropAleResolution {
        goal: (SectorNumber::new(public), element.layer()),
        // Same-sector DropAle has no route event with which to recover a
        // stronger identity. Preserve the actor's current representation so
        // the seek compares equal whether it is exact or number-only.
        goal_sector_index: sector.arena_index(),
        // Same-sector DropAle never searches gate paths.
        recorded_gate_path: None,
    })
}
