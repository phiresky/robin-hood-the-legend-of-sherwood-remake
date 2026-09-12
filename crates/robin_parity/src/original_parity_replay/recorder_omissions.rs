//! Facts omitted by the Original recorder, not native trace version adapters.
//! TODO(parity-recorder): retire reconstruction only once its input facts are
//! emitted by the recorder and covered by exact replay evidence.
use super::{
    BTreeMap, BTreeSet, EntityId, GameCode, Path, PlayerCommand, Sha256, TRACE_NATIVE_SUFFIX,
    TRACE_SCHEMA_VERSION, TraceAction, TraceActor, TraceCommand, TraceElement, TraceEntityId,
    TraceEntityKind, TraceJsonValue, trace_schema_is_supported,
};
use sha2::Digest as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LegacyBlockedBoxTuple {
    pub(super) min_x: u32,
    pub(super) min_y: u32,
    pub(super) max_x: u32,
    pub(super) max_y: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LegacyBlockedBoxValidity {
    Unknown,
    Unset,
    Set,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LegacyStoppableMotionOrder {
    pub(super) id: u32,
    pub(super) stop_animation: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LegacyBlockedBoxShadow {
    pub(super) tuple: LegacyBlockedBoxTuple,
    pub(super) validity: LegacyBlockedBoxValidity,
    pub(super) last_processed_order_id: u32,
    pub(super) pending_motion_order_id: Option<u32>,
    pub(super) stoppable_motion_order: Option<LegacyStoppableMotionOrder>,
    pub(super) deviated: Option<bool>,
    pub(super) direct_validity_observed: bool,
}

pub(super) fn initial_legacy_blocked_box_shadows(
    save: &robin_engine::legacy_save::body::LegacySaveBody,
) -> BTreeMap<u32, LegacyBlockedBoxShadow> {
    save.element_payloads
        .records
        .iter()
        .filter_map(|record| {
            let sprite = record.payload.actor_sprite()?;
            let blocked = sprite.position.blocked_box;
            Some((
                record.header.creation_order,
                LegacyBlockedBoxShadow {
                    tuple: LegacyBlockedBoxTuple {
                        min_x: blocked.top_left.x.to_bits(),
                        min_y: blocked.top_left.y.to_bits(),
                        max_x: blocked.bottom_right.x.to_bits(),
                        max_y: blocked.bottom_right.y.to_bits(),
                    },
                    validity: if blocked.bounds_are_set {
                        LegacyBlockedBoxValidity::Set
                    } else {
                        LegacyBlockedBoxValidity::Unset
                    },
                    last_processed_order_id: sprite.last_processed_order_id,
                    pending_motion_order_id: None,
                    stoppable_motion_order: None,
                    deviated: Some(sprite.position.deviated),
                    direct_validity_observed: false,
                },
            ))
        })
        .collect()
}

pub(super) fn legacy_blocked_box_tuple(
    runtime: &serde_json::Value,
) -> Option<LegacyBlockedBoxTuple> {
    let blocked = runtime.pointer("/position/blocked_box")?;
    if blocked.is_null() {
        return None;
    }
    let bits = |pointer: &str| {
        blocked
            .pointer(pointer)
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    };
    Some(LegacyBlockedBoxTuple {
        min_x: bits("/min/x/bits")?,
        min_y: bits("/min/y/bits")?,
        max_x: bits("/max/x/bits")?,
        max_y: bits("/max/y/bits")?,
    })
}

/// Old schema-16 runtime capture omitted the bounding-box bounds-set marker
/// and printed its stale coordinate words unconditionally. Track Original's
/// validity bit from transitions which prove it without touching Rust state.
/// Sprite motion changes the processed movement-order id and then
/// clears the blocked box; only blocked-box updates mutate its bounds and
/// make the box valid again.
///
/// TODO(parity-recorder): have new original-game captures emit no value when
/// the blocked area is absent. Exact tuple reuse remains ambiguous
/// unless the captured anti-collision state also proves the unset-to-set
/// update path.
pub(super) fn canonicalize_legacy_blocked_box(
    runtime: &mut serde_json::Value,
    creation_order: u32,
    reset_by_new_movement_order: bool,
    current_motion_order_id: Option<u32>,
    current_stoppable_motion_order: Option<LegacyStoppableMotionOrder>,
    shadows: &mut BTreeMap<u32, LegacyBlockedBoxShadow>,
) -> bool {
    let blocked_is_null = runtime
        .pointer("/position/blocked_box")
        .is_some_and(serde_json::Value::is_null);
    let tuple = legacy_blocked_box_tuple(runtime);
    let deviated = runtime
        .pointer("/position/deviated")
        .and_then(serde_json::Value::as_bool);
    if tuple.is_none() && !blocked_is_null {
        return false;
    }
    let Some(last_processed_order_id) = runtime
        .pointer("/sprite/last_processed_order_id")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    else {
        return false;
    };

    let Some(shadow) = shadows.get_mut(&creation_order) else {
        // Runtime-created actors have no saved validity bit. Their first old
        // capture is observationally ambiguous, so retain it until a later
        // transition proves whether the cache is valid.
        if let Some(tuple) = tuple {
            shadows.insert(
                creation_order,
                LegacyBlockedBoxShadow {
                    tuple,
                    validity: LegacyBlockedBoxValidity::Unknown,
                    last_processed_order_id,
                    pending_motion_order_id: current_motion_order_id,
                    stoppable_motion_order: current_stoppable_motion_order,
                    deviated,
                    direct_validity_observed: false,
                },
            );
        }
        return blocked_is_null;
    };

    let tuple_changed = tuple.is_some_and(|tuple| tuple != shadow.tuple);
    let same_tuple_revalidated = tuple.is_some_and(|tuple| {
        tuple == shadow.tuple
            && shadow.validity == LegacyBlockedBoxValidity::Unset
            && legacy_blocked_box_revalidated_by_deviation(runtime, tuple, shadow.deviated)
    });
    let order_changed = last_processed_order_id != shadow.last_processed_order_id;
    if blocked_is_null {
        // New recordings carry the validity bit directly by emitting null.
        shadow.validity = LegacyBlockedBoxValidity::Unset;
        shadow.direct_validity_observed = true;
    } else if shadow.direct_validity_observed {
        shadow.validity = LegacyBlockedBoxValidity::Set;
    } else if reset_by_new_movement_order && order_changed {
        shadow.validity = LegacyBlockedBoxValidity::Unset;
    }
    if let Some(tuple) = tuple {
        // Motion processing resets before any blocked-box update in the same step,
        // so a changed tuple proves that the later update revalidated it.
        if tuple_changed || same_tuple_revalidated {
            shadow.validity = LegacyBlockedBoxValidity::Set;
        }
        shadow.tuple = tuple;
    }
    shadow.last_processed_order_id = last_processed_order_id;
    shadow.pending_motion_order_id = current_motion_order_id;
    shadow.stoppable_motion_order = current_stoppable_motion_order;
    shadow.deviated = deviated;

    if shadow.validity == LegacyBlockedBoxValidity::Unset {
        if let Some(blocked) = runtime.pointer_mut("/position/blocked_box") {
            *blocked = serde_json::Value::Null;
        }
        true
    } else {
        false
    }
}

pub(super) fn legacy_blocked_box_revalidated_by_deviation(
    runtime: &serde_json::Value,
    tuple: LegacyBlockedBoxTuple,
    prior_deviated: Option<bool>,
) -> bool {
    let Some(position) = runtime.pointer("/position") else {
        return false;
    };
    if prior_deviated != Some(false)
        || position
            .pointer("/deviated")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || position
            .pointer("/anti_collision_on")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || position
            .pointer("/blocked_count")
            .and_then(serde_json::Value::as_u64)
            != Some(0)
    {
        return false;
    }
    let bits = |pointer: &str| {
        position
            .pointer(pointer)
            .and_then(serde_json::Value::as_u64)
            .and_then(|bits| u32::try_from(bits).ok())
    };
    let (Some(map_x), Some(map_y), Some(old_x), Some(old_y)) = (
        bits("/map/x/bits"),
        bits("/map/y/bits"),
        bits("/old_map/x/bits"),
        bits("/old_map/y/bits"),
    ) else {
        return false;
    };
    let map_x = f32::from_bits(map_x);
    let map_y = f32::from_bits(map_y);
    if old_x == map_x.to_bits() && old_y == map_y.to_bits() {
        return false;
    }
    let half = 0.49_f32;
    tuple.min_x == (map_x - half).to_bits()
        && tuple.min_y == (map_y - half).to_bits()
        && tuple.max_x == (map_x + half).to_bits()
        && tuple.max_y == (map_y + half).to_bits()
}

/// Whether the captured actor has just entered the original game's matching execution state
/// which performs sprite motion and therefore resets box-blocked state.
///
/// A movement *sequence* may currently be playing an in-place transition
/// through action processing, and a non-movement sequence may use a locomotion
/// transition action. Both helpers return the same movement-start value, so
/// sequence shape plus motion state is not enough to prove a reset.
pub(super) fn original_motion_executor_order_id(
    actor: &TraceActor,
    entity_id: EntityId,
) -> Option<u32> {
    let sequence = actor.sequence_element.as_ref()?;
    // Several locomotion transition actions are also installed in generic
    // turn sequences. The original game explicitly routes those through action processing
    // when the sequence is not movement.
    sequence.movement.as_ref()?;
    let order = sequence.current_order.as_ref()?.to_json();
    let action = order
        .get("action")
        .and_then(serde_json::Value::as_u64)
        .and_then(|action| u32::try_from(action).ok())
        .and_then(|action| robin_engine::order::OrderType::try_from(action).ok())?;
    if !robin_engine::engine::original_actor_order_uses_motion_executor(entity_id, action) {
        return None;
    }
    order
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .and_then(|order_id| u32::try_from(order_id).ok())
}

pub(super) fn original_stoppable_current_motion_order(
    actor: &TraceActor,
) -> Option<LegacyStoppableMotionOrder> {
    let sequence = actor.sequence_element.as_ref()?;
    sequence.movement.as_ref()?;
    let order = sequence.current_order.as_ref()?.to_json();
    let action = order
        .get("action")
        .and_then(serde_json::Value::as_u64)
        .and_then(|action| u32::try_from(action).ok())
        .and_then(|action| robin_engine::order::OrderType::try_from(action).ok())?;
    let stop_animation = match action {
        robin_engine::order::OrderType::WalkingUpright => {
            robin_engine::order::OrderType::TransitionWalkingUprightWaitingUpright
        }
        robin_engine::order::OrderType::RunningUpright => {
            robin_engine::order::OrderType::TransitionRunningUprightWaitingUpright
        }
        robin_engine::order::OrderType::WalkingCrouched => {
            robin_engine::order::OrderType::TransitionWalkingCrouchedWaitingCrouched
        }
        _ => return None,
    };
    let id = order
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .and_then(|order_id| u32::try_from(order_id).ok())?;
    Some(LegacyStoppableMotionOrder {
        id,
        stop_animation: stop_animation as u32,
    })
}

pub(super) fn original_reset_blocked_box_this_frame(
    actor: &TraceActor,
    entity_id: EntityId,
    last_processed_order_id: u32,
    moved_this_frame: bool,
    prior_pending_motion_order_id: Option<u32>,
    prior_stoppable_motion_order: Option<LegacyStoppableMotionOrder>,
    prior_last_processed_order_id: Option<u32>,
) -> bool {
    let Some(sequence) = actor.sequence_element.as_ref() else {
        return false;
    };
    let current_order_id = sequence
        .current_order
        .as_ref()
        .map(TraceJsonValue::to_json)
        .and_then(|order| {
            order
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .and_then(|order_id| u32::try_from(order_id).ok())
        });
    // Stopping movement rewrites the sole live walking/running order in place and
    // assigns a new ID. The rewritten transition may execute and then be replaced
    // before frame recording, leaving its new ID only in last_processed. The
    // preceding capture proves the exact stoppable movement source; the
    // current non-movement order and lack of displacement prove this is the
    // hidden handoff rather than ordinary progress. This is schema-16
    // session-003-0001 frame-1176.
    let hidden_stop_movement_rewrite = prior_stoppable_motion_order
        .zip(prior_last_processed_order_id)
        .is_some_and(|(stoppable_order, prior_last_processed_order_id)| {
            stoppable_order.id == prior_last_processed_order_id
                && actor.animation == stoppable_order.stop_animation
        })
        && sequence.movement.is_none()
        && current_order_id
            .is_some_and(|current_order_id| current_order_id != last_processed_order_id)
        && !moved_this_frame;
    if hidden_stop_movement_rewrite {
        return true;
    }

    let Some(current_motion_order_id) = original_motion_executor_order_id(actor, entity_id) else {
        return false;
    };

    let starts_visible_order =
        actor.motion_state == robin_engine::sprite::MotionState::Start as u32;
    let advanced_to_next_order =
        current_motion_order_id != last_processed_order_id && moved_this_frame;
    let began_previously_observed_order = prior_pending_motion_order_id
        == Some(last_processed_order_id)
        && current_motion_order_id == last_processed_order_id;
    // A distance-producing order may reach its waypoint and advance before
    // frame recording. Its final motion latch is then IN_PROGRESS and the current
    // order is the successor, but `last_processed_order_id` still proves that
    // Motion processing initialized the just-executed predecessor. This is the
    // schema-16 session-003-0006 frame-3769 shape.
    // A new movement order can also remain current after its first update. In
    // that case the preceding capture proves it was pending, and the changed
    // raw `last_processed_order_id` (checked by the shadow) proves it began.
    // This is the schema-16 session-003-0001 frame-600 shape.
    // Motion processing initializes/resets before deciding whether an order's
    // tolerance permits displacement. A preceding capture of the pending
    // exact movement ID, followed by both current and last_processed changing
    // to that ID, proves the reset even for a stationary order. This is
    // schema-16 session-003-0006 frame-4394 (RunningWithSword, tolerance 65).
    starts_visible_order || advanced_to_next_order || began_previously_observed_order
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RefreshOrientationSignature {
    pub(super) actor: TraceEntityId,
    pub(super) action: TraceAction,
    pub(super) mouse_map_bits: [u32; 2],
    pub(super) target_bits: [u32; 3],
}

impl RefreshOrientationSignature {
    pub(super) fn from_command(command: &TraceCommand) -> Option<Self> {
        let TraceCommand::OrientActionAt {
            actor,
            action,
            mouse_map,
            target,
            ..
        } = command
        else {
            return None;
        };
        Some(Self {
            actor: *actor,
            action: *action,
            mouse_map_bits: [mouse_map.x.bits, mouse_map.y.bits],
            target_bits: [target.x.bits, target.y.bits, target.z.bits],
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum LegacyRefreshOrientationProvenance {
    #[default]
    None,
    ActionSelected {
        actor: TraceEntityId,
        action: TraceAction,
    },
    FirstOrdinaryOrientation(RefreshOrientationSignature),
}

impl LegacyRefreshOrientationProvenance {
    pub(super) fn advance(
        self,
        commands: &[TraceCommand],
        popup_nested_refresh: bool,
    ) -> LegacyRefreshOrientationProvenance {
        if popup_nested_refresh {
            return Self::None;
        }

        if let Self::ActionSelected { actor, action } = self
            && let [command] = commands
            && let Some(signature) = RefreshOrientationSignature::from_command(command)
            && (signature.actor, signature.action) == (actor, action)
        {
            return Self::FirstOrdinaryOrientation(signature);
        }

        match commands {
            [TraceCommand::SelectAction { pc, action, .. }] => Self::ActionSelected {
                actor: *pc,
                action: *action,
            },
            [
                TraceCommand::SelectPc { pc: selected, .. },
                TraceCommand::SelectAction { pc, action, .. },
            ] if selected == pc => Self::ActionSelected {
                actor: *pc,
                action: *action,
            },
            _ => Self::None,
        }
    }

    pub(super) fn proves_single_popup_orientation_is_late(
        self,
        commands: &[TraceCommand],
        popup_nested_refresh: bool,
    ) -> bool {
        let Self::FirstOrdinaryOrientation(previous) = self else {
            return false;
        };
        popup_nested_refresh
            && matches!(commands, [command] if RefreshOrientationSignature::from_command(command) == Some(previous))
    }
}

/// Split refresh-owned orientation records from commands that entered through
/// the input phase of this simulation boundary.
///
/// The original game processes input before its simulation update, whereas
/// Orientation processing is called from the element-refresh pass
/// during the refresh pass. Ordinarily a refresh
/// orientation left over from the preceding host pass is already at the front
/// of the next frame's command queue. When an orientation follows this
/// boundary's matching `MSG_SELECT_ACTION`, it came from a refresh reached
/// later in the same host pass and therefore must not affect the actor's
/// earlier execution/action-processing step.
pub(super) fn split_refresh_owned_orientations(
    commands: Vec<TraceCommand>,
    popup_nested_refresh: bool,
    ordinary_refresh_eligible: &[(TraceEntityId, TraceAction)],
    force_single_popup_orientation_late: bool,
) -> (Vec<TraceCommand>, Vec<TraceCommand>) {
    let mut actions_selected_this_boundary = BTreeMap::new();
    let mut before_hourglass = Vec::with_capacity(commands.len());
    let mut after_hourglass = Vec::new();

    // A frame can contain both the ordinary refresh orientation left by the
    // preceding host pass and a synchronous popup refresh reached during this
    // Hourglass. Original records both into the same flat command stream. The
    // popup refresh is later, so for each actor/action pair only its final
    // resolved orientation belongs after Hourglass.
    let mut final_popup_orientations = Vec::new();
    if popup_nested_refresh {
        for (index, command) in commands.iter().enumerate() {
            if let TraceCommand::OrientActionAt { actor, action, .. } = command {
                if let Some(entry) =
                    final_popup_orientations
                        .iter_mut()
                        .find(|(known_actor, known_action, _, _)| {
                            known_actor == actor && known_action == action
                        })
                {
                    entry.2 = index;
                    entry.3 += 1;
                } else {
                    final_popup_orientations.push((*actor, *action, index, 1_u32));
                }
            }
        }
    }

    for (index, command) in commands.into_iter().enumerate() {
        match &command {
            TraceCommand::SelectAction { pc, action, .. } => {
                actions_selected_this_boundary.insert(*pc, *action);
            }
            TraceCommand::CancelAction { pc: Some(pc), .. } => {
                actions_selected_this_boundary.remove(pc);
            }
            TraceCommand::CancelAction { pc: None, .. } => {
                actions_selected_this_boundary.clear();
            }
            TraceCommand::OrientActionAt { actor, action, .. }
                if final_popup_orientations.iter().any(
                    |(known_actor, known_action, known_index, count)| {
                        known_actor == actor
                            && known_action == action
                            && *known_index == index
                            && (*count > 1
                                || force_single_popup_orientation_late
                                || !ordinary_refresh_eligible.contains(&(*actor, *action)))
                    },
                ) =>
            {
                after_hourglass.push(command);
                continue;
            }
            TraceCommand::OrientActionAt { actor, action, .. }
                if actions_selected_this_boundary.get(actor) == Some(action) =>
            {
                after_hourglass.push(command);
                continue;
            }
            _ => {}
        }
        before_hourglass.push(command);
    }

    (before_hourglass, after_hourglass)
}

pub(super) fn advance_trace_qa_recording_state(recording: &mut bool, command: &TraceCommand) {
    if matches!(command, TraceCommand::StartRecordingMacro { .. }) {
        *recording = true;
        return;
    }
    if *recording
        && matches!(
            command,
            TraceCommand::GroupMove { .. }
                | TraceCommand::LaunchInteraction { .. }
                | TraceCommand::LaunchGroundTarget { .. }
                | TraceCommand::DropAleAt { .. }
                | TraceCommand::LaunchSelfAbility { .. }
                | TraceCommand::LaunchScrollRead { .. }
                | TraceCommand::SwordStrike { .. }
                | TraceCommand::CrouchDown
                | TraceCommand::StandUp
        )
    {
        // These are the command shapes stored by the engine's QA hook. Their
        // successful original-game handlers stop macro recording globally.
        *recording = false;
    }
}

/// Recover the quit-mission message omitted by old schema-16 recorders.
///
/// The engine tick can report success without advancing only
/// through its leading won-quit branch. The UI message which set that flag
/// was not captured, so this exact terminal envelope proves the omitted input.
// TODO(parity-trace): record mission quitting in the original game and remove this
// compatibility inference after every surviving schema-16 trace includes it.
pub(super) fn is_legacy_retained_terminal_success(
    schema: u32,
    frame_before: u64,
    frame_after: u64,
    simulation_body_ran: bool,
    game_code: i32,
) -> bool {
    schema == TRACE_SCHEMA_VERSION
        && frame_before == frame_after
        && !simulation_body_ran
        && game_code == GameCode::LevelSucceeded as i32
}

/// Stable host identity for campaign attempts synthesized by this replay tool.
///
/// Original predates native attempt history, so an archived trace cannot carry
/// the host nonce which current live sessions attach to terminal commands. Use
/// the logical recording-family name: chained `-session-NNNN` files belong to
/// one host run and therefore receive one identity, independent of their disk
/// location. The domain separator keeps this namespace distinct from future
/// deterministic tool identities.
pub(super) fn replay_campaign_run_id(trace_path: &Path, session_index: u32) -> u64 {
    let file_name = trace_path.file_name().unwrap_or_else(|| {
        panic!(
            "parity trace path {} has no file name for campaign identity",
            trace_path.display()
        )
    });
    let file_name = file_name.to_string_lossy();
    let file_name = file_name
        .strip_suffix(TRACE_NATIVE_SUFFIX)
        .unwrap_or(&file_name);
    let session_suffix = format!("-session-{session_index:04}");
    let logical_stem = file_name.strip_suffix(".jsonl.zst").unwrap_or(file_name);
    let recording_family = logical_stem
        .strip_suffix(&session_suffix)
        .unwrap_or(logical_stem);

    let mut digest = Sha256::new();
    digest.update(b"robin-original-parity-campaign-run-v1\0");
    digest.update(recording_family.as_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    let identity = u64::from_le_bytes(bytes);
    // Zero is reserved as an invalid durable campaign-history identity.
    if identity == 0 { u64::MAX } else { identity }
}

pub(super) fn append_legacy_retained_terminal_success_repair(
    commands_before_hourglass: &mut Vec<PlayerCommand>,
    commands_after_hourglass: &mut Vec<PlayerCommand>,
    difficulty: robin_engine::player_profile::DifficultyLevel,
    campaign_run_id: u64,
    schema: u32,
    frame_before: u64,
    frame_after: u64,
    simulation_body_ran: bool,
    game_code: i32,
    already_applied: &mut bool,
) -> bool {
    if *already_applied
        || !is_legacy_retained_terminal_success(
            schema,
            frame_before,
            frame_after,
            simulation_body_ran,
            game_code,
        )
    {
        return false;
    }

    // The omitted original-game quit-mission message both applies campaign/stat updates
    // and arms the won-quit flag before the retained simulation boundary. Rust keeps
    // terminal campaign updates in a post-hourglass command so achievement
    // evidence is finalized only after that final engine boundary, matching the
    // live Rust session transaction. Both phases still complete before parity
    // compares the Original terminal snapshot.
    *already_applied = true;
    commands_before_hourglass.push(PlayerCommand::QuitMissionRequested);
    commands_after_hourglass.push(PlayerCommand::ApplyQuitMissionUpdates {
        exit_code: GameCode::LevelSucceeded,
        difficulty,
        completed_at_unix_seconds: None,
        campaign_run_nonce: Some(campaign_run_id),
    });
    true
}

/// Legacy schemas 12 through 16 do not record the host lifecycle event that produced some
/// presentation-only random sprite-frame selection. It does retain the exact
/// RNG values and callsites, so the missing RNG boundary can be reconstructed
/// without inventing any retained engine side effects.
///
/// Admit only an otherwise ordinary in-progress frame with no resolved host
/// command and a previously unseen terminal callsite burst longer than the
/// retained scroll set. Besides random frame selection's homogeneous burst,
/// Original's mobile-element presentation update consumes one X/Y pair per
/// visual child, producing a repeated pair of
/// adjacent callsites. Requiring distinct retained values further rejects
/// accidental ordinary-callsite runs. Numeric addresses are build-specific and
/// deliberately never classified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LegacyPresentationEntityState {
    pub(super) entity_id: TraceEntityId,
    pub(super) creation_order: u32,
    pub(super) kind: TraceEntityKind,
    pub(super) active: bool,
    pub(super) position_bits: [u32; 2],
}

pub(super) fn legacy_presentation_entity_states(
    elements: &[TraceElement],
) -> Vec<LegacyPresentationEntityState> {
    elements
        .iter()
        .map(|element| LegacyPresentationEntityState {
            entity_id: element.entity_id,
            creation_order: element.creation_order,
            kind: element.kind,
            active: element.active,
            position_bits: [element.position_map.x.bits, element.position_map.y.bits],
        })
        .collect()
}

/// Teleporting creates five no-supplier unconscious-star titbits at
/// the old position and five at the new position. The following draw refreshes
/// each transient titbit and randomizes its sprite frame once. Schema 16
/// omits both the host Draw boundary and these presentation-only entities, but
/// retains the actor/target teleport transition.
pub(super) fn has_legacy_teleport_star_lifecycle(
    previous: Option<&[LegacyPresentationEntityState]>,
    current: &[LegacyPresentationEntityState],
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if previous.len() != current.len() {
        return false;
    }

    let mut activated_moved_pcs = 0;
    let mut deactivated_targets = 0;
    for (before, after) in previous.iter().zip(current) {
        if before.entity_id != after.entity_id
            || before.creation_order != after.creation_order
            || before.kind != after.kind
        {
            return false;
        }
        match (before.kind, before.active, after.active) {
            (TraceEntityKind::Pc, false, true) if before.position_bits != after.position_bits => {
                activated_moved_pcs += 1;
            }
            (TraceEntityKind::Target, true, false) => deactivated_targets += 1,
            (TraceEntityKind::Pc | TraceEntityKind::Target, before_active, after_active)
                if before_active != after_active =>
            {
                return false;
            }
            _ => {}
        }
    }
    activated_moved_pcs == 1 && deactivated_targets == 1
}

pub(super) fn legacy_presentation_sprite_rng_burst(
    schema: u32,
    trace_commands_were_empty: bool,
    game_code: i32,
    simulation_body_ran: bool,
    scroll_count: usize,
    has_teleport_star_lifecycle: bool,
    gameplay_callsite_offsets: &[u32],
    gameplay_values: &[u32],
) -> Option<usize> {
    if !trace_schema_is_supported(schema)
        || game_code != GameCode::LevelInProgress as i32
        || !simulation_body_ran
        || gameplay_callsite_offsets.len() != gameplay_values.len()
    {
        return None;
    }

    let terminal_callsite = *gameplay_callsite_offsets.last()?;
    let homogeneous_suffix_start = gameplay_callsite_offsets
        .iter()
        .rposition(|&offset| offset != terminal_callsite)
        .map_or(0, |index| index + 1);
    let alternating_suffix_start = (gameplay_callsite_offsets.len() >= 2).then(|| {
        let second = gameplay_callsite_offsets[gameplay_callsite_offsets.len() - 2];
        if second == terminal_callsite {
            return gameplay_callsite_offsets.len();
        }
        let mut start = gameplay_callsite_offsets.len() - 2;
        while start >= 2
            && gameplay_callsite_offsets[start - 2] == second
            && gameplay_callsite_offsets[start - 1] == terminal_callsite
        {
            start -= 2;
        }
        start
    });
    let suffix_start = alternating_suffix_start
        .filter(|&start| gameplay_callsite_offsets.len() - start >= 4)
        .unwrap_or(homogeneous_suffix_start);
    let (ordinary_prefix, presentation_suffix) = gameplay_callsite_offsets.split_at(suffix_start);
    let suffix_values = &gameplay_values[suffix_start..];
    let homogeneous = suffix_start == homogeneous_suffix_start;
    let large_unretained_burst = if homogeneous {
        trace_commands_were_empty && scroll_count != 0 && presentation_suffix.len() > scroll_count
    } else {
        // Three visual children are sufficient to distinguish the mobile
        // element's repeated X/Y vibration loop from one ordinary paired
        // gameplay calculation. The exact post-tick cursor check below still
        // requires this whole suffix, and only this suffix, to be unconsumed.
        presentation_suffix.len() >= 6
    };
    let exact_teleport_star_burst = homogeneous
        && trace_commands_were_empty
        && presentation_suffix.len() == 10
        && has_teleport_star_lifecycle;
    let callsites_were_seen = if homogeneous {
        ordinary_prefix.contains(&terminal_callsite)
    } else {
        ordinary_prefix.contains(&presentation_suffix[0])
            || ordinary_prefix.contains(&presentation_suffix[1])
    };
    if (!large_unretained_burst && !exact_teleport_star_burst)
        || callsites_were_seen
        || (homogeneous
            && suffix_values
                .iter()
                .enumerate()
                .any(|(index, value)| suffix_values[..index].contains(value)))
    {
        return None;
    }
    Some(presentation_suffix.len())
}

/// Admit a legacy presentation-only RNG candidate only when the retained
/// simulation body left exactly that suffix unconsumed.
///
/// A homogeneous original-game event run is not sufficient evidence by itself:
/// ordinary gameplay sites such as `BoredAnimationChoice` can produce a first,
/// distinct run longer than the retained scroll set. In that case Rust already
/// consumes the complete frame batch and replaying the candidate would consume
/// the same values twice. Conversely, a genuinely omitted host presentation
/// boundary leaves precisely the candidate suffix between the post-tick cursor
/// and the recorded frame end.
pub(super) fn missing_legacy_presentation_sprite_rng_draws(
    candidate: Option<usize>,
    rust_rng_after_tick: usize,
    original_rng_end: usize,
) -> Option<usize> {
    let missing = original_rng_end.checked_sub(rust_rng_after_tick)?;
    (missing != 0 && candidate == Some(missing)).then_some(missing)
}

/// Recover repeated falling-arrow presentation passes omitted by legacy
/// schemas.  A single pass may contain several falling arrows, so compare the
/// leading original-game event run with the engine's exact pending-pass draw
/// count.  The callsite must already have correlated with `ArrowFallingFrame`
/// on an earlier exact frame in this trace; numeric offsets are build-specific.
pub(super) fn legacy_additional_arrow_refresh_draws(
    schema: u32,
    gameplay_callsite_offsets: &[u32],
    known_arrow_falling_callsites: &BTreeSet<u32>,
    pending_pass_draws: usize,
) -> Option<usize> {
    if !trace_schema_is_supported(schema) || pending_pass_draws == 0 {
        return None;
    }
    let callsite = *gameplay_callsite_offsets.first()?;
    if !known_arrow_falling_callsites.contains(&callsite) {
        return None;
    }
    let leading_draws = gameplay_callsite_offsets
        .iter()
        .take_while(|&&candidate| candidate == callsite)
        .count();
    (leading_draws > pending_pass_draws).then_some(leading_draws - pending_pass_draws)
}
