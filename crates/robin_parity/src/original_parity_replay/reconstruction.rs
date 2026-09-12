//! Reconstruct imported campaign/engine state and runtime commands before comparison.
#[cfg(feature = "client")]
use super::Host;
use super::{
    Arc, BTreeMap, BTreeSet, BinaryTraceReader, BinaryTraceRecord, Command, Engine, Entity,
    EntityMap, GestureQuality, GroupMoveGoalTranslation, LevelAssets, MapPoint, Path, PathBuf,
    PlayerCommand, ReplayDropAleResolution, ReplayGroupMoveResolution, StorageContext,
    TRACE_NATIVE_SUFFIX, TRACE_SCHEMA_VERSION, TraceCampaign, TraceCommand, TraceElement,
    TraceEntityKind, TraceFrame, TraceHeader, TraceInitialNpcTransient, TraceStartState,
    TraceStorageResult, WorldPoint3D, ensure_native_binary_trace, native_binary_trace_path,
};

pub(super) fn apply_initial_npc_transients(
    engine: &mut Engine,
    transients: &[TraceInitialNpcTransient],
) {
    let mut runtime_by_creation_order = BTreeMap::new();
    for id in engine.npc_ids() {
        let creation_order = engine.original_creation_order(id);
        assert!(
            runtime_by_creation_order
                .insert(creation_order, id)
                .is_none(),
            "two Rust NPCs share Original creation order {creation_order}"
        );
    }
    assert_eq!(
        transients.len(),
        runtime_by_creation_order.len(),
        "schema-{TRACE_SCHEMA_VERSION} initial_npc_transients must cover every NPC exactly once"
    );

    let mut seen = BTreeSet::new();
    for transient in transients {
        assert!(
            seen.insert(transient.creation_order),
            "schema-{TRACE_SCHEMA_VERSION} initial_npc_transients repeats creation order {}",
            transient.creation_order
        );
        let id = runtime_by_creation_order
            .get(&transient.creation_order)
            .copied()
            .unwrap_or_else(|| {
                panic!(
                    "schema-{TRACE_SCHEMA_VERSION} NPC transient creation order {} is absent from the Rust engine",
                    transient.creation_order
                )
            });
        engine
            .parity_replay_setup()
            .restore_npc_maximal_visibility(id, transient.maximal_visibility);
    }
}

pub(super) fn reconstruct_unrecorded_maximal_visibility(
    leaning_out: bool,
    visibilities: impl IntoIterator<Item = f32>,
) -> u16 {
    let view_speed = if leaning_out {
        robin_engine::ai_vision::LOOK_DOWN_BASE_VIEW_SPEED
    } else {
        robin_engine::ai_vision::BASE_VIEW_SPEED
    };
    visibilities
        .into_iter()
        .map(|visibility| (view_speed as f32 * visibility) as u16)
        .max()
        .unwrap_or(0)
}

/// Reconstruct the process-local maximum omitted by old interactive segments.
///
/// Original clears this value before an ordinary vision pass, but dead and
/// unconscious actors return first and retain the preceding segment's value.
/// Their serialized detectable buckets retain the visibility which supplied
/// that maximum, making this exact reconstruction possible.
pub(super) fn apply_legacy_segment_visibility_fallback(engine: &mut Engine) -> usize {
    let restorations = engine
        .npc_ids()
        .into_iter()
        .filter_map(|id| {
            let entity = engine
                .get_entity(id)
                .unwrap_or_else(|| panic!("legacy parity fallback lost NPC {id:?}"));
            let npc = entity
                .npc_data()
                .unwrap_or_else(|| panic!("legacy parity fallback found non-NPC {id:?}"));
            let retains_maximum =
                entity.is_dead() || entity.human_data().is_some_and(|human| human.unconscious);
            retains_maximum.then(|| {
                let value = reconstruct_unrecorded_maximal_visibility(
                    npc.view_lean_out,
                    npc.detectable_lists
                        .iter()
                        .flatten()
                        .map(|detectable| detectable.last_visibility),
                );
                (id, value)
            })
        })
        .collect::<Vec<_>>();
    for &(id, value) in &restorations {
        engine
            .parity_replay_setup()
            .restore_npc_maximal_visibility(id, value);
    }
    restorations.len()
}

/// An in-process load starts recording after save adoption and consequently
/// has no setup RNG prefix. A nonempty prefix proves a fresh engine, whose
/// constructor-zero process-local state is authoritative.
pub(super) fn legacy_loaded_save_retains_process_transients(prefix_draw_count: usize) -> bool {
    prefix_draw_count == 0
}

pub(super) fn preceding_interactive_session_path(
    path: &Path,
    session_index: u32,
) -> Option<PathBuf> {
    if session_index <= 1 {
        return None;
    }
    let previous = session_index.checked_sub(1)?;
    let name = path.file_name()?.to_str()?;
    let (name, native) = match name.strip_suffix(TRACE_NATIVE_SUFFIX) {
        Some(logical) => (logical, true),
        None => (name, false),
    };
    let suffix = format!("-session-{session_index:04}.jsonl.zst");
    let stem = name.strip_suffix(&suffix)?;
    let previous = format!("{stem}-session-{previous:04}.jsonl.zst");
    Some(path.with_file_name(if native {
        format!("{previous}{TRACE_NATIVE_SUFFIX}")
    } else {
        previous
    }))
}

pub(super) fn terminal_macro_waypoint(
    element: &TraceElement,
    paths: &[robin_engine::level_data::RawHikingPath],
) -> Option<(robin_engine::ai::PathId, u8, usize)> {
    let ai = element.ai.as_ref()?;
    terminal_macro_waypoint_at(
        (element.position_map.x.bits, element.position_map.y.bits),
        ai.macro_cursor,
        ai.macro_in_progress,
        paths,
    )
}

pub(super) fn terminal_macro_waypoint_at(
    position_bits: (u32, u32),
    cursor: Option<u16>,
    macro_in_progress: bool,
    paths: &[robin_engine::level_data::RawHikingPath],
) -> Option<(robin_engine::ai::PathId, u8, usize)> {
    if !macro_in_progress {
        return None;
    }
    let offset = usize::from(cursor?);
    let mut matches = paths.iter().enumerate().flat_map(|(path_index, path)| {
        path.waypoints
            .iter()
            .enumerate()
            .filter_map(move |(waypoint_index, waypoint)| {
                let robin_engine::level_data::WaypointCommand::Macro(command) = &waypoint.command
                else {
                    return None;
                };
                (offset <= command.len()
                    && position_bits.0 == f32::from(waypoint.x).to_bits()
                    && position_bits.1 == f32::from(waypoint.y).to_bits())
                .then_some((path_index, waypoint_index))
            })
    });
    let (path_index, waypoint_index) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some((
        robin_engine::ai::PathId::new(u16::try_from(path_index).ok()?)?,
        u8::try_from(waypoint_index).ok()?,
        offset,
    ))
}

pub(super) fn apply_legacy_interactive_chain_macro_fallback(
    trace_path: &Path,
    header: &TraceHeader,
    prefix_draw_count: usize,
    engine: &mut Engine,
    assets: &LevelAssets,
) -> TraceStorageResult<usize> {
    if header.schema != TRACE_SCHEMA_VERSION
        || header.start_state != TraceStartState::LoadedSave
        || header.initial_npc_transients.is_some()
        || !legacy_loaded_save_retains_process_transients(prefix_draw_count)
    {
        return Ok(0);
    }
    let Some(previous_path) = preceding_interactive_session_path(trace_path, header.session_index)
        .filter(|path| path.is_file() || native_binary_trace_path(path).is_file())
    else {
        return Ok(0);
    };
    let previous_native = ensure_native_binary_trace(&previous_path)?;
    let mut reader = BinaryTraceReader::open(&previous_native)?;
    let previous_header = reader.read_header()?.trace;
    if previous_header.schema != TRACE_SCHEMA_VERSION
        || previous_header.session_index.checked_add(1) != Some(header.session_index)
        || previous_header.mission != header.mission
        || previous_header.proto_level != header.proto_level
        || previous_header.rng_seed != header.rng_seed
    {
        return Ok(0);
    }
    let mut final_frame = None;
    loop {
        match reader.read_record()? {
            BinaryTraceRecord::Frame(frame) => final_frame = Some(frame),
            BinaryTraceRecord::End {
                final_frame: end,
                frame_count,
                ..
            } => {
                reader
                    .validate_terminator(
                        frame_count
                            .storage_context("preceding interactive trace lost its frame count")?,
                        end.storage_context("preceding interactive trace lost its final frame")?,
                    )
                    .map_err(|error| format!("invalid preceding interactive trace: {error}"))?;
                break;
            }
        }
    }
    let mut runtime = engine
        .npc_ids()
        .into_iter()
        .map(|id| (engine.original_creation_order(id), id))
        .collect::<BTreeMap<_, _>>();
    let mut restored = 0;
    for element in &final_frame
        .storage_context("preceding interactive trace has no frames")?
        .elements
    {
        let Some((path_id, waypoint, offset)) =
            terminal_macro_waypoint(element, &assets.navigation.hiking_paths)
        else {
            continue;
        };
        let Some(id) = runtime.remove(&element.creation_order) else {
            continue;
        };
        restored += usize::from(
            engine
                .parity_replay_setup()
                .restore_npc_dormant_macro_cursor(id, path_id, waypoint, offset, assets),
        );
    }
    Ok(restored)
}

impl TraceCommand {
    pub(super) fn into_player_command(
        self,
        entity_map: &EntityMap,
        engine: &Engine,
        drop_ale_resolution: Option<ReplayDropAleResolution>,
        group_move_resolution: Option<ReplayGroupMoveResolution>,
    ) -> Option<PlayerCommand> {
        assert!(
            drop_ale_resolution.is_none() || matches!(&self, Self::DropAleAt { .. }),
            "DropAle route metadata was attached to a non-DropAle command"
        );
        assert!(
            group_move_resolution.is_none() || matches!(&self, Self::GroupMove { .. }),
            "group-move route metadata was attached to a non-group-move command"
        );
        Some(match self {
            Self::BoxSelect {
                first,
                second,
                append,
            } => {
                // The Original records the root drag gesture and then records
                // each resolved nested selection message as another command.
                // Replaying both would apply selection (and its speech/echo
                // side effects) twice. Keep accepting the gesture metadata,
                // but replay only the following resolved commands.
                let _ = (first, second, append);
                return None;
            }
            Self::GroupMove {
                actors,
                destination,
                running,
                show_marker,
                goal_sector,
                goal_layer,
            } => {
                let destination: MapPoint = destination.into();
                let (goal_override, goal_sector_index_override) = match entity_map
                    .translate_group_move_goal_sector(
                        goal_sector,
                        goal_layer,
                        group_move_resolution
                            .as_ref()
                            .and_then(|resolution| resolution.unmapped_goal_search_sector),
                    ) {
                    GroupMoveGoalTranslation::Runtime(goal, index) => {
                        engine
                            .fast_grid()
                            .level
                            .sector_number_map
                            .get(&goal.0)
                            .and_then(|&index| engine.fast_grid().level.sectors.get(index))
                            .unwrap_or_else(|| {
                                panic!(
                                    "group-move Original sector {goal_sector} maps to missing Rust \
                                     position sector {}",
                                    goal.0
                                )
                            });
                        (Some(goal), Some(index))
                    }
                    GroupMoveGoalTranslation::RecordedUnmapped(goal) => {
                        let runtime_collision = engine
                            .fast_grid()
                            .level
                            .sector_number_map
                            .get(&goal.0)
                            .and_then(|&index| engine.fast_grid().level.sectors.get(index));
                        assert!(
                            runtime_collision.is_none_or(|sector| sector.sector_type.is_jump()),
                            "unmapped Original group-move sector {goal_sector} collides with Rust \
                             runtime position sector {}",
                            goal.0
                        );
                        (Some(goal), None)
                    }
                };
                PlayerCommand::GroupMove {
                    actors: actors
                        .into_iter()
                        .map(|id| entity_map.translate(id))
                        .collect(),
                    destination,
                    running,
                    show_marker,
                    goal_override,
                    goal_sector_index_override,
                    door_route_override: group_move_resolution
                        .as_ref()
                        .map(|resolution| resolution.door_route),
                    recorded_gate_routes: group_move_resolution
                        .as_ref()
                        .map(|resolution| {
                            resolution
                                .recorded_gate_routes
                                .iter()
                                .map(|(actor, gates)| {
                                    (
                                        entity_map.translate(*actor),
                                        gates
                                            .iter()
                                            .map(|&(gate, direct)| {
                                                (entity_map.translate_gate(gate), direct)
                                            })
                                            .collect(),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    recorded_failed_gate_routes: group_move_resolution
                        .map(|resolution| {
                            resolution
                                .recorded_failed_gate_routes
                                .into_iter()
                                .map(|actor| entity_map.translate(actor))
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            }
            Self::LaunchInteraction {
                actor,
                target,
                original_command: _,
                original_command_name,
                running,
            } => PlayerCommand::LaunchInteraction {
                actor: entity_map.translate(actor),
                target: entity_map.translate(target),
                command: command_from_stable_name(&original_command_name),
                running,
            },
            Self::LaunchSelfAbility {
                actor,
                original_command: _,
                original_command_name,
            } => PlayerCommand::LaunchSelfAbility {
                actor: entity_map.translate(actor),
                command: command_from_stable_name(&original_command_name),
            },
            Self::LaunchGroundTarget {
                actor,
                target,
                original_command: _,
                original_command_name,
                original_target_field,
                titbit_layer,
            } => {
                // The original game assigns these stable
                // field numbers. Translate semantically because Rust's Field
                // enum intentionally omits unrelated legacy properties.
                let target_field = match (original_command_name.as_str(), original_target_field) {
                    ("throw_purse", 30) => robin_engine::sequence::Field::PurseTarget,
                    ("throw_net", 31) => robin_engine::sequence::Field::NetTarget,
                    ("throw_wasp_nest", 32) => robin_engine::sequence::Field::WaspNestTarget,
                    (command, field) => panic!(
                        "unsupported Original ground-target command/field {command:?}/{field}"
                    ),
                };
                PlayerCommand::LaunchGroundTarget {
                    actor: entity_map.translate(actor),
                    target_pos: target.into(),
                    command: command_from_stable_name(&original_command_name),
                    target_field,
                    titbit_layer,
                }
            }
            Self::LaunchScrollRead {
                actor,
                target,
                running,
            } => PlayerCommand::LaunchScrollRead {
                actor: entity_map.translate(actor),
                target: entity_map.translate(target),
                running,
            },
            Self::SwordStrike {
                actor,
                target,
                original_command: _,
                original_command_name,
                with_seek,
                seek_distance,
            } => PlayerCommand::SwordStrikeCmd {
                actor: entity_map.translate(actor),
                target: entity_map.translate(target),
                command: command_from_stable_name(&original_command_name),
                composite: None,
                gesture_quality: GestureQuality::PERFECT,
                with_seek,
                seek_distance: trace_sword_seek_distance(with_seek, seek_distance),
            },
            Self::SelectPc { pc, append } => PlayerCommand::SelectPc {
                pc_id: entity_map.translate(pc),
                append,
            },
            Self::UnselectAllPcs => PlayerCommand::UnselectAllPcs,
            Self::StopPc { pc } => PlayerCommand::StopPc {
                pc_id: entity_map.translate(pc),
            },
            Self::SelectAction { pc, action, .. } => PlayerCommand::SelectResolvedAction {
                pc_id: entity_map.translate(pc),
                action: action.into(),
            },
            Self::CancelAction { pc, .. } => match pc {
                Some(pc) => PlayerCommand::CancelAction {
                    pc_id: entity_map.translate(pc),
                },
                None => PlayerCommand::UnselectAllActions,
            },
            Self::OrientActionAt {
                action,
                actor,
                mouse_map,
                target,
                original_action: _,
            } => PlayerCommand::PerformResolvedOrientation {
                pc_id: entity_map.translate(actor),
                action: action.into(),
                mouse_map: mouse_map.into(),
                target: target.into(),
            },
            Self::MakePcFast { entity } => PlayerCommand::MakePcFast {
                pc_id: entity_map.translate(entity),
            },
            Self::CrouchDown => PlayerCommand::CrouchDown,
            Self::StandUp => PlayerCommand::StandUp,
            Self::DropAleAt {
                actor,
                target,
                running,
            } => {
                let (
                    already_authorized,
                    goal_override,
                    goal_sector_index_override,
                    recorded_gate_path,
                ) = drop_ale_resolution
                    .map(|resolution| {
                        (
                            true,
                            Some(resolution.goal),
                            resolution.goal_sector_index,
                            resolution.recorded_gate_path,
                        )
                    })
                    .unwrap_or((false, None, None, None));
                PlayerCommand::DropAleAt {
                    actor: entity_map.translate(actor),
                    target_pos: target.into(),
                    running,
                    already_authorized,
                    goal_override,
                    goal_sector_index_override,
                    recorded_gate_path,
                }
            }
            Self::ShieldSelectProtected {
                actor,
                protected_pc,
            } => PlayerCommand::ShieldSelectProtected {
                actor: entity_map.translate(actor),
                protected_pc: entity_map.translate(protected_pc),
            },
            Self::BoxUnselect {
                first,
                second,
                append,
            } => {
                // Same shape as `BoxSelect`: the drag gesture and each
                // resolved nested unselect message are both recorded, so
                // replay only the resolved commands that follow.
                let _ = (first, second, append);
                return None;
            }
            Self::RaiseShieldWithDanger {
                actor,
                protected_pc,
                danger_point,
                danger_point_layer,
            } => {
                let danger_point: WorldPoint3D = danger_point.into();
                PlayerCommand::RaiseShieldWithDanger {
                    actor: entity_map.translate(actor),
                    protected_pc: entity_map.translate(protected_pc),
                    danger_point,
                    danger_point_layer,
                }
            }
            Self::TeleportSelected {
                destination,
                goal_sector,
                goal_layer,
            } => PlayerCommand::TeleportSelectedToPoint {
                dest: destination.into(),
                layer: goal_layer,
                // The Original records the selected sector's own number
                // (or -1 when no sector was selected), not a fast-grid
                // array index, so it transfers directly.
                sector: u16::try_from(goal_sector)
                    .ok()
                    .and_then(robin_engine::position_interface::SectorHandle::new),
            },
            Self::SelectAllPcs => PlayerCommand::SelectAllPcs,
            Self::UnselectPc { pc } => PlayerCommand::UnselectPc {
                pc_id: entity_map.translate(pc),
            },
            Self::SelectActionIndex { index } => {
                // The Original resolves the action-bar shortcut against
                // the single selected PC and does nothing at all for any
                // other selection cardinality.
                match engine.selected_hero_ids() {
                    [pc_id] => PlayerCommand::SelectAction {
                        pc_id: *pc_id,
                        action_index: index,
                    },
                    _ => return None,
                }
            }
            Self::SetLockAlt { on } => PlayerCommand::SetLockAlt(on),
            Self::KeyControl => PlayerCommand::KeyControl,
            Self::KeyReleaseControl => PlayerCommand::KeyReleaseControl,
            Self::StartMacro { pc, slot } => PlayerCommand::StartMacro {
                pc: pc.map(|pc| entity_map.translate(pc)),
                slot,
            },
            Self::DeleteMacro { pc, slot } => PlayerCommand::DeleteMacro {
                pc: pc.map(|pc| entity_map.translate(pc)),
                slot,
            },
            Self::StartRecordingMacro { pc, slot } => PlayerCommand::StartRecordingMacro {
                pc: pc.map(|pc| entity_map.translate(pc)),
                slot,
            },
            Self::ChangeQaMemory { slot } => PlayerCommand::ChangeQaMemory { slot },
            Self::HeroRefusedAction {
                actor,
                action,
                original_action: _,
                target: _,
                reason,
            } => {
                // Every refusal the recorder knows about barks the same line.
                // A new one must be taught here rather than silently replayed
                // as this one.
                match reason.as_str() {
                    "anonymous_archer_contest" | "locked_patch" => {}
                    other => panic!(
                        "unsupported refused-action reason {other:?} for {action:?} \
                         by {actor:?}"
                    ),
                }
                PlayerCommand::HeroSpeak {
                    pc_id: entity_map.translate(actor),
                    expression: robin_engine::engine::melee::HERO_UNABLE_TO_DO_SOMETHING,
                }
            }
            Self::BeggarDontTalkStamp { entity } => PlayerCommand::BeggarDontTalkStamp {
                beggar_id: entity_map.translate(entity),
            },
        })
    }
}

pub(super) fn trace_sword_seek_distance(with_seek: bool, seek_distance: f32) -> Option<f32> {
    (with_seek && !seek_distance.is_nan()).then_some(seek_distance)
}

pub(super) fn command_from_stable_name(name: &str) -> Command {
    let rust_name = match name {
        "camera_jumpto" => "CameraJumpTo".to_owned(),
        // The original game uses the low-level rolling movement and the
        // contextual player ability JUMP. Rust's historical names are Jump
        // and JumpCmd respectively.
        "roll" => "Jump".to_owned(),
        "jump" => "JumpCmd".to_owned(),
        "search" => "SearchCmd".to_owned(),
        "hit" => "HitCmd".to_owned(),
        "heal" => "HealCmd".to_owned(),
        "eat" => "EatCmd".to_owned(),
        "tie" => "TieCmd".to_owned(),
        "strangle" => "StrangleCmd".to_owned(),
        "whistle" => "WhistleCmd".to_owned(),
        "launch_postseek" => "LaunchPostSeek".to_owned(),
        "launch_quickaction" => "LaunchQuickAction".to_owned(),
        other => other
            .split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                    None => String::new(),
                }
            })
            .collect(),
    };
    serde_json::from_value(serde_json::Value::String(rust_name))
        .unwrap_or_else(|_| panic!("unsupported stable original-game command name {name:?}"))
}

/// Structural extent of the authoritative Original frame stream.
///
/// `frame_count` counts snapshots, not universal-frame increments.  Most
/// snapshots advance the universal frame once, but Original records a final
/// mission-success/interruption snapshot after the simulation tick returns
/// before incrementing the clock.  Such a record legitimately has
/// `frame_before == frame_after`, so `initial_frame + frame_count` is not the
/// stream's final frame.  The explicit frame envelopes are the authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TraceTimeline {
    pub(super) next_frame_before: u64,
    pub(super) frame_count: u64,
}

impl TraceTimeline {
    pub(super) fn new(initial_frame: u64) -> Self {
        Self {
            next_frame_before: initial_frame,
            frame_count: 0,
        }
    }

    pub(super) fn observe(&mut self, frame_before: u64, frame_after: u64) -> Result<(), String> {
        if frame_before != self.next_frame_before {
            return Err(format!(
                "frame {frame_before}->{frame_after} does not continue after frame {}",
                self.next_frame_before
            ));
        }
        let advanced_frame = frame_before
            .checked_add(1)
            .ok_or_else(|| format!("frame {frame_before} cannot advance without overflowing"))?;
        if frame_after != frame_before && frame_after != advanced_frame {
            return Err(format!(
                "frame {frame_before}->{frame_after} must either retain or advance the universal frame once"
            ));
        }
        self.next_frame_before = frame_after;
        self.frame_count = self
            .frame_count
            .checked_add(1)
            .ok_or_else(|| "parity frame count overflowed u64".to_owned())?;
        Ok(())
    }

    pub(super) fn validate_terminator(
        &self,
        frame_count: u64,
        final_frame: u64,
    ) -> Result<(), String> {
        if frame_count != self.frame_count {
            return Err(format!(
                "terminator frame_count={frame_count} disagrees with {} frame records",
                self.frame_count
            ));
        }
        if final_frame != self.next_frame_before {
            return Err(format!(
                "terminator final_frame={final_frame} disagrees with the last frame_after={}",
                self.next_frame_before
            ));
        }
        Ok(())
    }
}

pub(super) fn cross_post_initialize_frame(engine: &mut Engine, assets: &LevelAssets) {
    engine
        .parity_replay_setup()
        // Schema 16 omits the capture viewport and per-Draw camera position.
        // Keep its presentation-only edge compatibility isolated here; a
        // future schema carrying that provenance must pass `false` instead.
        .refresh_sprite_dimension_cache(assets, true);
    engine
        .advance_frame(
            assets,
            robin_engine::engine::SimulationFrameInput::no_hourglass().with_post_initialize(true),
        )
        .unwrap_or_else(|error| panic!("admit Original PostInitialize boundary: {error}"));
}

pub(super) fn register_language_data_paths_for_tool() {
    #[cfg(feature = "client")]
    robin_rs::main_entry::register_language_data_paths_for_tool();
    #[cfg(not(feature = "client"))]
    crate::register_language_data_paths();
}

pub(super) fn restore_campaign(
    trace: &TraceCampaign,
    profiles: &robin_engine::profiles::ProfileManager,
) -> robin_engine::campaign::Campaign {
    use robin_engine::campaign::{Campaign, CampaignValue, PcDescription};
    use robin_engine::mission::{Mission, MissionStatus};
    use robin_engine::pc_status::{HumanStatus, PcStatus, Skill};
    use robin_engine::profiles::CharacterProfileIdx;
    use robin_engine::sector_production::{Occupant, SectorProduction, Type};

    assert_eq!(trace.version, 1, "unsupported campaign snapshot version");
    const VALUE_KEYS: [CampaignValue; 27] = [
        CampaignValue::Amulets,
        CampaignValue::Ransom,
        CampaignValue::Score,
        CampaignValue::Blazon,
        CampaignValue::LivingSoldiers,
        CampaignValue::DeadSoldiers,
        CampaignValue::MissionLength,
        CampaignValue::Custom1,
        CampaignValue::Custom2,
        CampaignValue::Custom3,
        CampaignValue::Custom4,
        CampaignValue::Custom5,
        CampaignValue::Custom6,
        CampaignValue::Custom7,
        CampaignValue::Custom8,
        CampaignValue::Custom9,
        CampaignValue::Custom10,
        CampaignValue::Custom11,
        CampaignValue::Custom12,
        CampaignValue::Custom13,
        CampaignValue::Custom14,
        CampaignValue::Custom15,
        CampaignValue::Custom16,
        CampaignValue::Custom17,
        CampaignValue::Custom18,
        CampaignValue::Custom19,
        CampaignValue::Custom20,
    ];
    assert_eq!(
        trace.values.len(),
        VALUE_KEYS.len(),
        "campaign value table has the wrong cardinality"
    );

    let mut campaign = Campaign::default();
    for (key, value) in VALUE_KEYS.into_iter().zip(trace.values.iter().copied()) {
        campaign.values[key] = value;
    }
    campaign.ares = trace.ares;
    campaign.missions = trace
        .missions
        .iter()
        .map(|source| {
            let profile = profiles
                .missions
                .get(source.profile_index as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "campaign mission profile index {} is out of range",
                        source.profile_index
                    )
                });
            assert_eq!(profile.id, source.profile_id, "mission profile ID mismatch");
            assert!(
                profile
                    .mission_filename
                    .eq_ignore_ascii_case(&source.mission)
                    && profile
                        .proto_level_filename
                        .eq_ignore_ascii_case(&source.proto_level),
                "mission profile {} names disagree with trace {}/{}",
                source.profile_index,
                source.mission,
                source.proto_level
            );
            Mission {
                age: source.age,
                blazon_price: source.blazon_price,
                status: match source.status {
                    0 => MissionStatus::Available,
                    1 => MissionStatus::Won,
                    2 => MissionStatus::Lost,
                    other => panic!("invalid campaign mission status {other}"),
                },
                profile_idx: Some(source.profile_index),
                ares_state_override: (source.ares_state_succeeded != profile.ares_state_succeeded)
                    .then_some(source.ares_state_succeeded),
                attempt_history: Default::default(),
            }
        })
        .collect();
    let mission_count = campaign.missions.len();
    let validate_mission_index = |index: usize| {
        assert!(
            index < mission_count,
            "campaign mission index {index} is out of range {mission_count}"
        );
        index
    };
    campaign.accessible_mission_indices = trace
        .accessible_mission_indices
        .iter()
        .copied()
        .map(validate_mission_index)
        .collect();
    campaign.pending_accessible_mission_indices = trace
        .pending_accessible_mission_indices
        .iter()
        .copied()
        .map(validate_mission_index)
        .collect();
    campaign.last_mission_idx = trace.last_mission_index.map(validate_mission_index);
    campaign.current_mission_idx = trace.current_mission_index.map(validate_mission_index);
    campaign.next_mission_idx = trace.next_mission_index.map(validate_mission_index);
    campaign.blazon_mission_idx = trace.blazon_mission_index.map(validate_mission_index);
    let recent_launches: Vec<usize> = trace
        .last_played_mission_indices
        .iter()
        .copied()
        .map(validate_mission_index)
        .collect();
    campaign.reconstruct_original_save_history(&recent_launches);
    campaign.last_pseudo_mission_status = match trace.last_pseudo_mission_status {
        0 => MissionStatus::Available,
        1 => MissionStatus::Won,
        2 => MissionStatus::Lost,
        other => panic!("invalid last pseudo mission status {other}"),
    };
    campaign.last_pseudo_mission_id = trace.last_pseudo_mission_id;

    campaign.characters = trace
        .characters
        .iter()
        .map(|source| {
            let profile = profiles
                .characters
                .get(source.profile_index as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "campaign character profile index {} is out of range",
                        source.profile_index
                    )
                });
            assert_eq!(
                profile.profile_name, source.profile_name,
                "character profile name mismatch"
            );
            PcDescription {
                character_profile_idx: Some(CharacterProfileIdx(source.profile_index)),
                instanced: source.instanced,
                status: PcStatus {
                    human_status: HumanStatus {
                        hand_to_hand: Skill {
                            capacity: source.status.hand_to_hand.capacity,
                            experience: source.status.hand_to_hand.experience,
                        },
                        bow: Skill {
                            capacity: source.status.bow.capacity,
                            experience: source.status.bow.experience,
                        },
                    },
                    life_points: source.status.life_points,
                    in_coma: source.status.in_coma,
                    num_ales: source.status.ales,
                    num_arrows: source.status.arrows,
                    num_apples: source.status.apples,
                    num_rations: source.status.rations,
                    num_stones: source.status.stones,
                    num_wasp_nests: source.status.wasp_nests,
                    num_nets: source.status.nets,
                    num_plants: source.status.plants,
                    num_purses: source.status.purses,
                    name: source.status.name.clone(),
                    name_override: None,
                    beam_me_index_in_sherwood: source.status.beam_me_index_in_sherwood,
                },
            }
        })
        .collect();
    let character_count = campaign.characters.len();
    let validate_character_index = |index: usize| {
        assert!(
            index < character_count,
            "campaign character index {index} is out of range {character_count}"
        );
        index
    };
    campaign.gang_indices = trace
        .gang_indices
        .iter()
        .copied()
        .map(validate_character_index)
        .collect();
    campaign.reservist_indices = trace
        .reservist_indices
        .iter()
        .copied()
        .map(validate_character_index)
        .collect();
    campaign.mission_team_indices = trace
        .mission_team_indices
        .iter()
        .copied()
        .map(validate_character_index)
        .collect();
    campaign.peasant_names = trace.peasant_names.clone();
    campaign.reservists_are_back = trace.reservists_are_back;
    campaign.collected_relics = trace.collected_relics.clone();
    campaign.production_sectors = trace
        .production_sectors
        .iter()
        .map(|source| SectorProduction {
            prod_type: match source.r#type {
                0 => Type::MakeArrow,
                1 => Type::MakePurse,
                2 => Type::MakeStone,
                3 => Type::MakeApple,
                4 => Type::MakeAle,
                5 => Type::MakeLamblegg,
                6 => Type::MakePlant,
                7 => Type::MakeNet,
                8 => Type::MakeWaspNest,
                9 => Type::TrainBow,
                10 => Type::TrainHandToHand,
                11 => Type::Heal,
                12 => Type::Relic,
                other => panic!("invalid campaign production type {other}"),
            },
            script_zone: None,
            speed: source.speed,
            production_points: Vec::new(),
            occupants: source
                .occupants
                .iter()
                .map(|occupant| Occupant {
                    pc_description_idx: validate_character_index(occupant.character_index),
                    x: occupant.x.value(),
                    y: occupant.y.value(),
                    obstacle:
                        robin_engine::position_interface::ObstacleHandle::from_serialized_pointer(
                            occupant.obstacle,
                        ),
                })
                .collect(),
            amount: source.amount,
            produced_amount: source.produced_amount,
            max_amount_reached: source.max_amount_reached,
        })
        .collect();

    campaign
}

#[cfg(not(feature = "client"))]
pub(super) fn initialize_headless_engine(
    header: &TraceHeader,
    rng_prefix: Vec<u32>,
    timing: &robin_engine::audio_durations::AudioDurations,
) -> (Engine, LevelAssets, robin_engine::scb::ScbFile) {
    let mut profile_manager = robin_engine::profiles::ProfileManager::new();
    let mut cpf = robin_engine::sbfile::SbFile::open("Data/Configuration/profile.cpf")
        .expect("open profile.cpf");
    profile_manager
        .load_all_legacy_cpf(&mut cpf)
        .expect("parse profile.cpf");
    profile_manager.import_beam_mes("Data/Levels");

    let campaign = restore_campaign(&header.campaign, &profile_manager);
    let mission_idx = campaign
        .current_mission_idx
        .expect("recorded campaign has no current mission");
    let current_profile = campaign.missions[mission_idx].profile(&profile_manager);
    assert!(
        current_profile
            .mission_filename
            .eq_ignore_ascii_case(&header.mission)
            && current_profile
                .proto_level_filename
                .eq_ignore_ascii_case(&header.proto_level),
        "trace header mission/proto {}/{} disagrees with campaign current mission {}/{}",
        header.mission,
        header.proto_level,
        current_profile.mission_filename,
        current_profile.proto_level_filename
    );

    let profiles = Arc::new(profile_manager);
    let mut assets = LevelAssets::new();
    // The runner has already prepared its datadir and locale mounts. Capture
    // that tool authority once; replay execution must not consult live globals.
    assets.sprite_scriptor = Arc::new(robin_engine::sprite_script::SpriteScriptor::legacy_tool());
    assets.profile_manager = profiles.clone();
    crate::populate_localized_names(&mut assets)
        .expect("load localized names for deterministic PC construction");

    let mut frame_holder = robin_assets::frame_holder::FrameHolder::new();
    frame_holder
        .initialize_sprite_bank(".")
        .expect("initialize sprite bank");
    assets.bank_signature = frame_holder.signature();

    let mission_name = campaign.missions[mission_idx]
        .profile(&profiles)
        .mission_filename
        .clone();
    let script_path = format!("Data/Levels/{mission_name}.scb");
    let bytes = robin_engine::sbfile::SbFile::read_all(&script_path)
        .unwrap_or_else(|status| panic!("read mission script {script_path}: status {status}"));
    let scb = robin_assets::scb::parse_bytes(&bytes).expect("parse mission script");
    assets.scripts.mission_programs = Arc::new(BTreeMap::from([(
        mission_name,
        Arc::new(
            robin_engine::script_manager::ScriptProgram::from_scb(scb.clone())
                .expect("prepare mission script bytecode"),
        ),
    )]));

    let loaded = robin_engine::engine::level_loading::load_mission_for_campaign(
        &campaign,
        &profiles,
        "Data/Levels",
        &mut |_| {},
    )
    .expect("load mission");
    let ambiance = robin_engine::engine::Ambiance::from_raw(loaded.mission.header.ambiance);
    let bg_pixel_dims = crate::background_dimensions(
        &loaded.mission.header.map_filename,
        ambiance.directory(),
        "Data/Levels",
    )
    .expect("read background dimensions");

    let engine = Engine::new(robin_engine::engine::EngineArgs {
        campaign,
        level: robin_engine::engine::LevelLoadArgs {
            assets: &mut assets,
            level_directory: "Data/Levels",
            progress: &mut |_| {},
            loaded,
            bg_pixel_dims,
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: Vec::new(),
        rng_seed: header.rng_seed,
        original_rng_replay: Some(rng_prefix),
        sim_config: header
            .sim_config
            .to_sim_config(header.synchronous_pathfinding),
    })
    .expect("initialize engine");
    crate::populate_sound_duration_tables(&mut assets, &profiles, timing)
        .expect("load deterministic sound duration tables");
    assets.attachments.pixel_opacity = Some(Arc::new(frame_holder));
    (engine, assets, scb)
}

#[cfg(feature = "client")]
pub(super) fn initialize_engine(
    header: &TraceHeader,
    rng_prefix: Vec<u32>,
) -> (
    Engine,
    LevelAssets,
    Host,
    robin_engine::engine::level_loading::PreDecodedBackground,
    robin_engine::scb::ScbFile,
    robin_rs::ingame_menu::resources::MenuText,
) {
    let mut pm = robin_engine::profiles::ProfileManager::new();
    let mut cpf = robin_engine::sbfile::SbFile::open("Data/Configuration/profile.cpf")
        .expect("open profile.cpf");
    pm.load_all_legacy_cpf(&mut cpf).expect("parse profile.cpf");
    pm.import_beam_mes("Data/Levels");

    let campaign = restore_campaign(&header.campaign, &pm);
    let mission_idx = campaign
        .current_mission_idx
        .expect("recorded campaign has no current mission");
    let current_profile = campaign.missions[mission_idx].profile(&pm);
    assert!(
        current_profile
            .mission_filename
            .eq_ignore_ascii_case(&header.mission)
            && current_profile
                .proto_level_filename
                .eq_ignore_ascii_case(&header.proto_level),
        "trace header mission/proto {}/{} disagrees with campaign current mission {}/{}",
        header.mission,
        header.proto_level,
        current_profile.mission_filename,
        current_profile.proto_level_filename
    );

    let profiles = Arc::new(pm);
    let mut assets = LevelAssets::new();
    // Keep the client-backed parity path on the same explicit, pinned tool
    // resource boundary as the headless runner.
    assets.sprite_scriptor = Arc::new(robin_engine::sprite_script::SpriteScriptor::legacy_tool());
    assets.profile_manager = profiles.clone();
    let mut text_res = robin_assets::resource_manager::ResourceManager::legacy_tool();
    text_res
        .attach_resource_file("Data/Text/Level.res")
        .expect("load Data/Text/Level.res for Original rescue-PC names");
    (assets.peasant_firstnames, assets.peasant_surnames) =
        robin_rs::game_session::load_peasant_name_pool(&mut text_res)
            .expect("decode localized peasant names");
    assets.fixed_vip_names = robin_rs::game_session::load_fixed_vip_name_map(&mut text_res)
        .expect("decode localized VIP names");
    let _ = text_res.attach_resource_file("Data/Interface/Start.sxt");
    let menu_text = robin_rs::ingame_menu::resources::MenuText::load(&mut text_res);
    let mut host = Host::scratch(1024.0, 768.0);
    host.frontend
        .resources
        .frame_holder_before_publication_mut()
        .initialize_sprite_bank(".")
        .expect("initialize sprite bank");
    assets.bank_signature = host.frontend.resources.frame_holder().signature();

    let mission_name = campaign.missions[mission_idx]
        .profile(&profiles)
        .mission_filename
        .clone();
    let script_path = format!("Data/Levels/{mission_name}.scb");
    let resolved =
        robin_engine::sbfile::resolve_case_insensitive(std::path::Path::new(&script_path))
            .unwrap_or_else(|| PathBuf::from(&script_path));
    let bytes = std::fs::read(&resolved)
        .unwrap_or_else(|e| panic!("read mission script {}: {e}", resolved.display()));
    let scb = robin_assets::scb::parse_bytes(&bytes).expect("parse mission script");
    assets.scripts.mission_programs = Arc::new(std::collections::BTreeMap::from([(
        mission_name,
        Arc::new(
            robin_engine::script_manager::ScriptProgram::from_scb(scb.clone())
                .expect("prepare mission script bytecode"),
        ),
    )]));

    let loaded = robin_engine::engine::level_loading::load_mission_for_campaign(
        &campaign,
        &profiles,
        "Data/Levels",
        &mut |_| {},
    )
    .expect("load mission");
    let ambiance = robin_engine::engine::Ambiance::from_raw(loaded.mission.header.ambiance)
        .directory()
        .to_string();
    // This legacy parity harness owns global file setup; capture it explicitly
    // before entering the reader-only terrain API.
    let terrain_files = robin_engine::sbfile::SbFile::snapshot_legacy_file_system();
    let background = robin_rs::level_loading_host::pre_decode_background_map_with_files(
        &loaded.mission.header.map_filename,
        &ambiance,
        "Data/Levels",
        None,
        &mut |_| {},
        &terrain_files,
    )
    .expect("decode background map")
    .expect("mission has no background map");
    let bg_pixel_dims = (background.width as f32, background.height as f32);

    let engine = Engine::new(robin_engine::engine::EngineArgs {
        campaign,
        level: robin_engine::engine::LevelLoadArgs {
            assets: &mut assets,
            level_directory: "Data/Levels",
            progress: &mut |_| {},
            loaded,
            bg_pixel_dims,
        },
        ground_mark_sprite: None,
        titbit_row_frame_counts: Vec::new(),
        rng_seed: header.rng_seed,
        original_rng_replay: Some(rng_prefix),
        sim_config: header
            .sim_config
            .to_sim_config(header.synchronous_pathfinding),
    })
    .expect("initialize engine");
    robin_rs::game_session::setup_mission_audio_for_tool(
        &mut host,
        &engine,
        &mut assets,
        &profiles,
        "Data/Sounds",
    );
    // The original game's target-sprite creation writes the active bank frame's native
    // dimensions into the serialized sprite frontier. The parity engine is
    // intentionally headless, so publish the immutable frame metadata used
    // to project that post-render state without mutating the simulation.
    assets.attachments.pixel_opacity = Some(host.frontend.resources.publish_frame_holder_opacity());
    (engine, assets, host, background, scb, menu_text)
}

/// Opt-in lifecycle diagnostic paired with Original's
/// `record_frame_pre_serialize` hook. It observes, but never changes, the
/// projectile state that the parity comparison is about to publish.
pub(super) fn record_arrow_publication_before_compare(
    engine: &Engine,
    frame: &TraceFrame,
    entity_map: &EntityMap,
) {
    if std::env::var_os("PARITY_DEBUG_ARROW_PUBLICATION").is_none() {
        return;
    }
    let parse_filter = |name: &str| {
        std::env::var(name).ok().map(|value| {
            value.parse::<u32>().unwrap_or_else(|error| {
                panic!("invalid {name}={value:?} for arrow publication diagnostic: {error}")
            })
        })
    };
    if parse_filter("PARITY_DEBUG_ARROW_PUBLICATION_FRAME_AFTER")
        .is_some_and(|value| u64::from(value) != frame.frame_after)
    {
        return;
    }
    let projectile_filter =
        parse_filter("PARITY_DEBUG_ARROW_PUBLICATION_PROJECTILE_CREATION_ORDER");
    let shooter_filter = parse_filter("PARITY_DEBUG_ARROW_PUBLICATION_SHOOTER_CREATION_ORDER");

    for original in frame.elements.iter().filter(|element| {
        element.kind == TraceEntityKind::Projectile
            && projectile_filter.is_none_or(|value| value == element.creation_order)
    }) {
        let id = entity_map.translate(original.entity_id);
        let entity = engine
            .get_entity(id)
            .unwrap_or_else(|| panic!("mapped diagnostic projectile {id:?} is missing"));
        let Entity::Projectile(arrow) = entity else {
            panic!("mapped diagnostic projectile {id:?} changed entity kind");
        };
        if arrow.object.object_type != robin_engine::element_kinds::ObjectType::Arrow {
            continue;
        }
        let shooter = arrow
            .projectile
            .shooter
            .expect("diagnostic arrow is missing its required shooter");
        let shooter_creation_order = engine.original_creation_order(shooter);
        if shooter_filter.is_some_and(|value| value != shooter_creation_order) {
            continue;
        }
        let sprite = &arrow.element.sprite;
        let position = sprite.position_iface.get_position();
        eprintln!(
            "PARITY_ARROW_PUBLICATION_RUST stage=record_frame_pre_compare frame_after={} \
             projectile_creation_order={} shooter_creation_order={} active={} flying={} \
             falling={} trajectory_size={} row={} frame={} frame_count={} \
             position_bits=[{:08x},{:08x},{:08x}]",
            frame.frame_after,
            original.creation_order,
            shooter_creation_order,
            arrow.element.active,
            arrow.projectile.flying,
            arrow.projectile.falling,
            arrow.projectile.trajectory.len(),
            sprite.current_row,
            sprite.current_frame,
            sprite.frame_count,
            position.x.to_bits(),
            position.y.to_bits(),
            position.z.to_bits(),
        );
    }
}
