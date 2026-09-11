//! Diagnostic text and rolling dumps; reporting does not decide equivalence.
/// Stable reporting shape; diagnostic sentences remain available to humans.
pub(super) fn structured_divergences(
    first_by_field: &BTreeMap<String, (u64, String)>,
) -> Vec<crate::result::FieldDivergence> {
    first_by_field
        .iter()
        .map(
            |(field, (frame, description))| crate::result::FieldDivergence {
                field: field.clone(),
                frame: *frame,
                description: description.clone(),
            },
        )
        .collect()
}
use super::{
    BTreeMap, BTreeSet, BufWriter, Engine, EntityId, EntityMap, File, PathBuf, Serialize,
    TRACE_SCHEMA_VERSION, TraceElement, TraceEntityId, TraceEntityKind, TraceFlightStep,
    TraceFrame, TraceHeader, TraceMovementStep, TracePathEvent, TraceRngBatch,
    TraceVisibilityQuery, VecDeque,
};
use std::io::Write as _;

pub(super) fn print_current_trace_events<T: Serialize>(label: &str, events: &[T]) {
    if !events.is_empty() {
        eprintln!(
            "  Original schema-{TRACE_SCHEMA_VERSION} {label} this frame: {}",
            serde_json::to_string(events).expect("serialize current-schema event diagnostics")
        );
    }
}

pub(super) fn print_current_trace_actor_diagnostics(elements: &[TraceElement]) {
    let diagnostics = elements
        .iter()
        .filter_map(|element| {
            let actor = element.actor.as_ref()?;
            let sequence = actor.sequence_element.as_ref();
            Some(serde_json::json!({
                "entity": element.entity_id,
                "creation_order": element.creation_order,
                "position_interface": actor.position_interface,
                "following": sequence.and_then(|value| value.following.as_ref()),
                "postponed": sequence.and_then(|value| value.postponed.as_ref()),
                "current_order": sequence.and_then(|value| value.current_order.as_ref()),
                "movement_payload": sequence.and_then(|value| value.movement_payload.as_ref()),
            }))
        })
        .take(40)
        .collect::<Vec<_>>();
    if !diagnostics.is_empty() {
        eprintln!(
            "  Original schema-{TRACE_SCHEMA_VERSION} actor diagnostics (up to 40): {}",
            serde_json::to_string(&diagnostics)
                .expect("serialize current-schema actor diagnostics")
        );
    }
}

pub(super) struct DumpOptions {
    pub(super) path: PathBuf,
    pub(super) from_frame: u64,
    pub(super) through_frame: u64,
    pub(super) entities: Vec<TraceEntityId>,
}

pub(super) const AUTOMATIC_DUMP_PRIOR_FRAMES: usize = 32;

pub(super) struct RollingDumpFrame {
    pub(super) engine: Engine,
    pub(super) frame_before: u64,
    pub(super) frame_after: u64,
    pub(super) selected_pcs: Vec<TraceEntityId>,
    pub(super) rng_draws: TraceRngBatch,
    pub(super) resolved_commands: serde_json::Value,
    pub(super) original_path_events: Vec<TracePathEvent>,
    pub(super) rust_path_events: Vec<robin_engine::pathfinder::ParityPathEvent>,
    pub(super) original_visibility_queries: Vec<TraceVisibilityQuery>,
    pub(super) rust_visibility_queries: Vec<robin_engine::sight_obstacle::ParityVisibilityQuery>,
    pub(super) original_movement_steps: Vec<TraceMovementStep>,
    pub(super) rust_movement_steps: Vec<robin_engine::movement_diagnostics::ParityMovementStep>,
    pub(super) original_flight_steps: Vec<TraceFlightStep>,
    pub(super) rust_flight_steps: Vec<robin_engine::movement_diagnostics::ParityFlightStep>,
    pub(super) rust_move_box_extractions:
        Vec<robin_engine::movement_diagnostics::ParityMoveBoxExtraction>,
    pub(super) rng_start: usize,
    pub(super) expected_rng_end: usize,
    pub(super) actual_rng_end: usize,
    pub(super) rust_rng_sites: Vec<robin_engine::sim_rng::RngSite>,
    pub(super) rust_rng_diagnostics: robin_engine::sim_rng::OriginalRngDiagnostics,
    pub(super) differences: Vec<String>,
}

impl DumpOptions {
    pub(super) fn includes(&self, frame: u64) -> bool {
        (self.from_frame..=self.through_frame).contains(&frame)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write_engine_dump_frame(
    writer: &mut BufWriter<File>,
    options: &DumpOptions,
    engine: &Engine,
    entity_map: &EntityMap,
    frame: &TraceFrame,
    resolved_commands: serde_json::Value,
    rng_start: usize,
    expected_rng_end: usize,
    actual_rng_end: usize,
    rust_rng_sites: &[robin_engine::sim_rng::RngSite],
    rust_rng_diagnostics: &robin_engine::sim_rng::OriginalRngDiagnostics,
    rust_path_events: &[robin_engine::pathfinder::ParityPathEvent],
    rust_visibility_queries: &[robin_engine::sight_obstacle::ParityVisibilityQuery],
    original_movement_steps: &[TraceMovementStep],
    rust_movement_steps: &[robin_engine::movement_diagnostics::ParityMovementStep],
    original_flight_steps: &[TraceFlightStep],
    rust_flight_steps: &[robin_engine::movement_diagnostics::ParityFlightStep],
    rust_move_box_extractions: &[robin_engine::movement_diagnostics::ParityMoveBoxExtraction],
    differences: &[String],
) {
    let diagnostic_engine = engine.diagnostic_snapshot_without_original_rng_replay();
    let original_entities = options
        .entities
        .iter()
        .map(|entity_id| {
            frame
                .elements
                .iter()
                .find(|element| element.entity_id == *entity_id)
                .unwrap_or_else(|| {
                    panic!(
                        "manual parity dump requested missing Original entity {entity_id:?} at frame {}",
                        frame.frame_after
                    )
                })
        })
        .collect::<Vec<_>>();
    write_engine_dump_snapshot_frame(
        writer,
        options,
        &diagnostic_engine,
        entity_map,
        frame.frame_before,
        frame.frame_after,
        &frame.selected_pcs,
        &frame.rng_draws,
        &frame.path_events,
        rust_path_events,
        &frame.visibility_queries,
        rust_visibility_queries,
        original_movement_steps,
        rust_movement_steps,
        original_flight_steps,
        rust_flight_steps,
        rust_move_box_extractions,
        resolved_commands,
        rng_start,
        expected_rng_end,
        actual_rng_end,
        rust_rng_sites,
        rust_rng_diagnostics,
        differences,
        Some(&original_entities),
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write_engine_dump_snapshot_frame(
    writer: &mut BufWriter<File>,
    options: &DumpOptions,
    diagnostic_engine: &Engine,
    entity_map: &EntityMap,
    frame_before: u64,
    frame_after: u64,
    selected_pcs: &[TraceEntityId],
    rng_draws: &TraceRngBatch,
    original_path_events: &[TracePathEvent],
    rust_path_events: &[robin_engine::pathfinder::ParityPathEvent],
    original_visibility_queries: &[TraceVisibilityQuery],
    rust_visibility_queries: &[robin_engine::sight_obstacle::ParityVisibilityQuery],
    original_movement_steps: &[TraceMovementStep],
    rust_movement_steps: &[robin_engine::movement_diagnostics::ParityMovementStep],
    original_flight_steps: &[TraceFlightStep],
    rust_flight_steps: &[robin_engine::movement_diagnostics::ParityFlightStep],
    rust_move_box_extractions: &[robin_engine::movement_diagnostics::ParityMoveBoxExtraction],
    resolved_commands: serde_json::Value,
    rng_start: usize,
    expected_rng_end: usize,
    actual_rng_end: usize,
    rust_rng_sites: &[robin_engine::sim_rng::RngSite],
    rust_rng_diagnostics: &robin_engine::sim_rng::OriginalRngDiagnostics,
    differences: &[String],
    original_entities: Option<&[&TraceElement]>,
) {
    let mapped_entities = options
        .entities
        .iter()
        .map(|original| {
            let rust = entity_map.translate(*original);
            serde_json::json!({
                "original": original,
                "rust": {
                    "kind": format!("{:?}", rust.kind()).to_lowercase(),
                    "index": rust.index(),
                },
            })
        })
        .collect::<Vec<_>>();
    let selected_rust_indices = options
        .entities
        .iter()
        .map(|original| entity_map.translate(*original).index() as usize)
        .collect::<BTreeSet<_>>();
    let mut engine_value = robin_util::json_value::to_json_value(diagnostic_engine)
        .expect("serialize diagnostic engine state");
    if !selected_rust_indices.is_empty() {
        let entities = engine_value
            .get_mut("world")
            .and_then(|world| world.get_mut("entities"))
            .and_then(serde_json::Value::as_array_mut)
            .expect("serialized Engine.world.entities must be an array");
        for (index, entity) in entities.iter_mut().enumerate() {
            if !selected_rust_indices.contains(&index) {
                *entity = serde_json::Value::Null;
            }
        }
    }
    let mut record = serde_json::json!({
        "schema": "robin-parity-engine-dump.v1",
        "type": "frame",
        "frame_before": frame_before,
        "frame_after": frame_after,
        "input": {
            "resolved_commands": resolved_commands,
            "selected_pcs": selected_pcs,
        },
        "original_path_events": original_path_events,
        "rust_path_events": rust_path_events,
        "visibility_queries": {
            "original": original_visibility_queries,
            "rust": rust_visibility_queries,
        },
        "original_movement_steps": original_movement_steps,
        "rust_movement_steps": rust_movement_steps,
        "original_flight_steps": original_flight_steps,
        "rust_flight_steps": rust_flight_steps,
        "rust_move_box_extractions": rust_move_box_extractions,
        "rng": {
            "cursor_before": rng_start,
            "expected_cursor_after": expected_rng_end,
            "actual_cursor_after": actual_rng_end,
            "rust_sites": rust_rng_sites,
            "rust_script_diagnostics": rust_rng_diagnostics,
            "original_frame_draws": rng_draws,
            "engine_original_replay_stream_omitted": true,
        },
        "entity_mapping": mapped_entities,
        "parity_differences": differences,
        "engine": engine_value,
    });
    if let Some(original_entities) = original_entities {
        record["original_entities"] = serde_json::to_value(original_entities)
            .expect("serialize selected Original parity dump entities");
    }
    write_jsonl_record(writer, &record);
}

pub(super) fn push_rolling_window<T>(frames: &mut VecDeque<T>, frame: T) {
    frames.push_back(frame);
    let capacity = AUTOMATIC_DUMP_PRIOR_FRAMES + 1;
    while frames.len() > capacity {
        frames.pop_front();
    }
}

pub(super) fn write_automatic_rolling_dump(
    frames: &VecDeque<RollingDumpFrame>,
    trace_path: &std::path::Path,
    header: &TraceHeader,
    entity_map: &EntityMap,
    divergent_frame: u64,
) -> PathBuf {
    assert!(
        !frames.is_empty(),
        "automatic parity dump requires at least one captured frame"
    );
    let prefix = format!("robin-parity-divergence-frame-{divergent_frame}-");
    // Replay changes cwd to the selected data directory during engine setup,
    // so current_dir is not a stable workspace anchor here. Prefer the source
    // trace's repository ancestor and retain the compile-time workspace as a
    // fallback for traces recorded outside this checkout.
    let workspace_root = trace_path
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let dump_dir = workspace_root.join(".codex-tmp").join("parity-dumps");
    std::fs::create_dir_all(&dump_dir).expect("create workspace automatic parity dump directory");
    let temporary = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".jsonl")
        .tempfile_in(&dump_dir)
        .expect("create unique automatic parity dump");
    let (file, path) = temporary
        .keep()
        .expect("persist unique automatic parity dump");
    let mut writer = BufWriter::new(file);
    let first_frame = frames.front().expect("rolling dump has a first frame");
    let last_frame = frames.back().expect("rolling dump has a last frame");
    let options = DumpOptions {
        path: path.clone(),
        from_frame: first_frame.frame_after,
        through_frame: last_frame.frame_after,
        entities: Vec::new(),
    };
    write_jsonl_record(
        &mut writer,
        &serde_json::json!({
            "schema": "robin-parity-engine-dump.v1",
            "type": "header",
            "source_trace": trace_path,
            "mission": header.mission,
            "rng_seed": header.rng_seed,
            "frame_range": {
                "from": options.from_frame,
                "through": options.through_frame,
            },
            "entity_filter": options.entities,
            "automatic_rolling_window": true,
        }),
    );
    for frame in frames {
        write_engine_dump_snapshot_frame(
            &mut writer,
            &options,
            &frame.engine,
            entity_map,
            frame.frame_before,
            frame.frame_after,
            &frame.selected_pcs,
            &frame.rng_draws,
            &frame.original_path_events,
            &frame.rust_path_events,
            &frame.original_visibility_queries,
            &frame.rust_visibility_queries,
            &frame.original_movement_steps,
            &frame.rust_movement_steps,
            &frame.original_flight_steps,
            &frame.rust_flight_steps,
            &frame.rust_move_box_extractions,
            frame.resolved_commands.clone(),
            frame.rng_start,
            frame.expected_rng_end,
            frame.actual_rng_end,
            &frame.rust_rng_sites,
            &frame.rust_rng_diagnostics,
            &frame.differences,
            None,
        );
    }
    writer.flush().expect("flush automatic parity dump");
    eprintln!("automatic parity engine dump: {}", path.display());
    path
}

pub(super) fn write_jsonl_record(writer: &mut BufWriter<File>, value: &serde_json::Value) {
    serde_json::to_writer(&mut *writer, value).expect("serialize diagnostic JSONL record");
    writer
        .write_all(b"\n")
        .expect("write diagnostic JSONL newline");
    writer.flush().expect("flush diagnostic JSONL record");
}

/// Dumps the order/sprite motion bookkeeping of one actor around a frame
/// boundary. Selected with `PARITY_DEBUG_ELEMENT=<kind>:<index>`, where kind is
/// `pc`, `soldier` or `civilian` and index is the Rust entity index — the same
/// pair the divergence report prints as `Pc(PcId(103))`. The window is
/// `PARITY_DEBUG_FROM`..=`PARITY_DEBUG_UNTIL`.
pub(super) fn print_debug_element(label: &str, engine: &Engine, frame: &TraceFrame) {
    let Some(spec) = std::env::var_os("PARITY_DEBUG_ELEMENT") else {
        return;
    };
    let frame_bound = |name: &str, fallback: u64| {
        std::env::var(name)
            .map(|value| {
                value
                    .parse::<u64>()
                    .unwrap_or_else(|_| panic!("{name} must be a u64"))
            })
            .unwrap_or(fallback)
    };
    let from = frame_bound("PARITY_DEBUG_FROM", 0);
    let until = frame_bound("PARITY_DEBUG_UNTIL", 10);
    if frame.frame_after < from || frame.frame_after > until {
        return;
    }
    let spec = spec.to_string_lossy().to_string();
    let (kind, index) = spec
        .split_once(':')
        .expect("PARITY_DEBUG_ELEMENT must look like pc:342");
    let index: u32 = index
        .parse()
        .expect("PARITY_DEBUG_ELEMENT index must be u32");
    let id = match kind {
        "pc" => EntityId::Pc(robin_engine::entity_id::PcId(index)),
        "soldier" => EntityId::Soldier(robin_engine::entity_id::SoldierId(index)),
        "civilian" => EntityId::Civilian(robin_engine::entity_id::CivilianId(index)),
        other => panic!("unsupported PARITY_DEBUG_ELEMENT kind {other}"),
    };
    // The slot may not exist yet on early frames; stay quiet until it does
    // rather than aborting the whole replay.
    let Some(entity) = engine.get_entity(id) else {
        return;
    };
    let sprite = &entity.element_data().sprite;
    let actor = entity.actor_data().expect("debug element is an actor");
    eprintln!(
        "{label} frame {} {:?} dir={} dir_goal={} posture={:?} action_state={:?} order={:?} installed={:?} last_processed_order={} actor_motion={:?} sprite_motion={:?} last_action={:?} row={} frame={}/{} command={:?} execute_init={} last_execute_order={:?}",
        frame.frame_after,
        id,
        entity.element_data().direction(),
        sprite.position_iface.get_direction_goal().as_u8(),
        entity.element_data().posture(),
        actor.action_state,
        engine.actor_order_type(id),
        actor.installed_order.map(|order| order.order_id),
        sprite.last_processed_order_id,
        actor.continuation.motion_state,
        sprite.last_motion_state,
        sprite.last_action,
        sprite.current_row,
        sprite.current_frame,
        sprite.frame_count,
        engine.actor_command(id),
        actor.execute_order_initialising,
        actor.last_execute_order_id,
    );
}

pub(super) fn print_startup_actors(
    label: &str,
    engine: &Engine,
    frame: &TraceFrame,
    entity_map: &EntityMap,
) {
    eprintln!("{label}:");
    let expected_inactive: Vec<_> = frame
        .elements
        .iter()
        .filter(|element| element.actor.is_some() && !element.active)
        .map(|element| element.entity_id)
        .collect();
    let rust_inactive: Vec<_> = engine
        .entities_with_ids_iter()
        .filter(|(_, entity)| entity.actor_data().is_some() && !entity.element_data().active)
        .map(|(id, _)| id)
        .collect();
    eprintln!("  expected inactive actors: {expected_inactive:?}");
    eprintln!("  Rust inactive actors: {rust_inactive:?}");
    let rust_hidden: Vec<_> = engine
        .entities_with_ids_iter()
        .filter(|(_, entity)| entity.element_data().hidden_in_building)
        .map(|(id, entity)| {
            (
                id,
                entity.element_data().sector(),
                entity.element_data().layer(),
                entity.element_data().position_map(),
            )
        })
        .collect();
    eprintln!("  Rust script-hidden actors: {rust_hidden:?}");
    for expected in frame.elements.iter().filter(|element| {
        element.actor.is_some()
            && (element.posture == robin_engine::element::Posture::Sitting as u32
                || element.entity_id
                    == TraceEntityId {
                        kind: TraceEntityKind::Soldier,
                        index: 119,
                    })
    }) {
        let id = entity_map.translate(expected.entity_id);
        let actual = engine
            .get_entity(id)
            .unwrap_or_else(|| panic!("mapped startup actor {id:?} is missing"));
        let ai_debug = actual.ai_controller().map(|ai| {
            (
                ai.current_state,
                ai.current_substate,
                ai.already_on_point,
                ai.last_goto_destination,
                ai.initial_view_direction,
                ai.outbox.reentrant.self_stimuli.clone(),
                ai.outbox.actor.launch_commands.clone(),
                ai.outbox.actor.orders.clone(),
            )
        });
        eprintln!(
            "  original={:?} rust={id:?} expected_posture={} rust_posture={:?} expected_dir={}/{} rust_dir={:?}/{:?} pos={:?} goal_pos={:?} action={:?} last_action={:?} alt_profile={} command={:?} order={:?} sector={:?} ai={ai_debug:?}",
            expected.entity_id,
            expected.posture,
            actual.element_data().posture(),
            expected.direction,
            expected.direction_goal,
            actual.element_data().sprite.position_iface.get_direction(),
            actual
                .element_data()
                .sprite
                .position_iface
                .get_direction_goal(),
            actual.element_data().position_map(),
            actual.element_data().sprite.position_iface.map_goal(),
            actual.actor_data().map(|actor| actor.action_state),
            actual.element_data().sprite.last_action,
            actual.element_data().sprite.use_alternate_profile,
            engine.actor_command(id),
            engine.actor_order_type(id),
            actual.element_data().sector(),
        );
    }
}

/// Wraps a Rust entity id so its `Debug` rendering also carries the original-game
/// trace index it was mapped from, e.g. `Pc(PcId(174))[orig:171]`.
///
/// The divergence report is read alongside `--dump-entity` (Original indices)
/// and the Original's own `[DBG]` logs; the id spaces frequently differ.
pub(super) struct EntityLabel {
    pub(super) id: robin_engine::entity_id::EntityId,
    pub(super) original_index: u32,
}

impl std::fmt::Debug for EntityLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}[orig:{}]", self.id, self.original_index)
    }
}
