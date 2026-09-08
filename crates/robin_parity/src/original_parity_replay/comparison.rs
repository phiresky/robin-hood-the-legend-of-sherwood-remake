//! Extracted comparison boundary; wire layouts remain in the parent.
use super::*;

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
pub(super) fn trace_entity_kind_name(name: &str) -> Option<TraceEntityKind> {
    Some(match name {
        "pc" => TraceEntityKind::Pc,
        "soldier" => TraceEntityKind::Soldier,
        "civilian" => TraceEntityKind::Civilian,
        "fx" => TraceEntityKind::Fx,
        "target" => TraceEntityKind::Target,
        "bonus" => TraceEntityKind::Bonus,
        "scroll" => TraceEntityKind::Scroll,
        "projectile" => TraceEntityKind::Projectile,
        "net" => TraceEntityKind::Net,
        _ => return None,
    })
}

pub(super) fn entity_kind_name(kind: robin_engine::element::EntityIdKind) -> &'static str {
    match kind {
        robin_engine::element::EntityIdKind::Pc => "pc",
        robin_engine::element::EntityIdKind::Soldier => "soldier",
        robin_engine::element::EntityIdKind::Civilian => "civilian",
        robin_engine::element::EntityIdKind::Fx => "fx",
        robin_engine::element::EntityIdKind::Target => "target",
        robin_engine::element::EntityIdKind::Bonus => "bonus",
        robin_engine::element::EntityIdKind::Scroll => "scroll",
        robin_engine::element::EntityIdKind::Projectile => "projectile",
        robin_engine::element::EntityIdKind::Net => "net",
    }
}

/// Replace Original allocation identities in authoritative runtime snapshots
/// with their Rust-side isomorphic identities before structural
/// comparison. Native VM table indices and sequence ordinals are semantic and
/// intentionally remain untouched.
pub(super) fn canonicalize_authoritative_snapshot(
    value: &mut serde_json::Value,
    entity_map: &EntityMap,
) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                canonicalize_authoritative_snapshot(value, entity_map);
            }
        }
        serde_json::Value::Object(object) => {
            let entity_reference = if object.len() == 2 {
                object
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .and_then(trace_entity_kind_name)
                    .zip(object.get("index").and_then(serde_json::Value::as_u64))
            } else {
                None
            };
            if let Some((kind, index)) = entity_reference {
                let index = u32::try_from(index)
                    .unwrap_or_else(|_| panic!("Original entity index {index} exceeds u32"));
                let id = entity_map.translate(TraceEntityId { kind, index });
                *value = serde_json::json!({
                    "kind": entity_kind_name(id.kind()),
                    "index": id.index(),
                });
                return;
            }

            for (key, child) in object {
                canonicalize_authoritative_snapshot(child, entity_map);
                if matches!(
                    key.as_str(),
                    "area" | "sector" | "sector_goal" | "sector_in" | "sector_out"
                ) && let Some(original) = child.as_i64()
                    && original >= 0
                {
                    let original = u16::try_from(original).unwrap_or_else(|_| {
                        panic!("Original snapshot sector {original} exceeds u16")
                    });
                    *child = entity_map.translate_sector(original).into();
                }
            }
        }
        _ => {}
    }
}

/// Whether the original game exposes indeterminate position-interface old-position
/// storage for a bonus constructed during this replay.
///
/// Position-interface construction does not initialize
/// `mpointOldPosition`/`mpointOldMap`. Both runtime bonus construction paths
/// (bonus and ale elements used by DROP_ALE) place the object
/// through position copying, which sets only the current
/// position and never starts a new move. These stationary
/// objects never acquire a defined old position during their live runtime.
///
/// Save/mission-start bonuses are deliberately excluded: serialization does
/// retain the complete old position, and their creation orders lie below the
/// boundary captured when the initial entity bijection is built. All other
/// fields on runtime bonuses remain authoritative and are still compared.
pub(super) fn original_runtime_bonus_has_undefined_old_position(
    kind: TraceEntityKind,
    creation_order: u32,
    runtime_creation_order_boundary: u32,
) -> bool {
    kind == TraceEntityKind::Bonus && creation_order >= runtime_creation_order_boundary
}

/// Remove initialization storage that the original game leaves indeterminate on a
/// dynamically-created projectile.
///
/// The original game does not initialize position goals, radius, or
/// door direction here; it likewise leaves frame countdown and
/// behind-display-order state untouched. The latter boolean is only
/// meaningful with a display-order reference. In addition, ordinary arrow
/// trajectory points do not initialize `material` when
/// material determination is disabled, although the projectile tick copies it
/// into the position interface. An out-of-domain material therefore proves
/// that this trace observed that undefined slot rather than game state.
///
/// The move box is a representation edge rather than undefined data:
/// The original-game 2D bounding box initializes zero corners but keeps its bounds-set bit
/// false. The trace omits that bit and emits a point box after translation;
/// Rust represents the same unset box as `null`.
pub(super) fn project_runtime_projectile_constructor_storage(value: &mut serde_json::Value) {
    let Some(runtime) = value.as_object_mut() else {
        return;
    };
    if let Some(position) = runtime
        .get_mut("position")
        .and_then(serde_json::Value::as_object_mut)
    {
        for field in ["door_direction", "goal_world", "radius"] {
            position.remove(field);
        }
        if let Some(material) = position.get("material") {
            // The material field is a signed enum in the runtime snapshot. The
            // uninitialized trajectory slot can therefore surface as either
            // negative allocator residue or a large positive integer.
            let is_out_of_domain_integer = material
                .as_i64()
                .is_some_and(|material| !(0..=10).contains(&material));
            if is_out_of_domain_integer {
                position.remove("material");
            }
        }
        let point_box = position.get("move_box").is_some_and(|move_box| {
            move_box
                .get("min")
                .zip(move_box.get("max"))
                .is_some_and(|(min, max)| min == max)
        });
        if point_box {
            position.insert("move_box".to_owned(), serde_json::Value::Null);
        }
    }
    if let Some(sprite) = runtime
        .get_mut("sprite")
        .and_then(serde_json::Value::as_object_mut)
    {
        sprite.remove("flight_countdown");
        if sprite
            .get("display_order_reference")
            .is_some_and(serde_json::Value::is_null)
        {
            sprite.remove("behind_display_order_reference");
        }
    }
}

pub(super) fn compare_frame(
    engine: &Engine,
    assets: &LevelAssets,
    frame: &TraceFrame,
    actual_game_code: i32,
    entity_map: &EntityMap,
    late_movement_retranslations: &[EntityId],
    legacy_additive_omissions: bool,
    legacy_missing_draw_view: bool,
    legacy_blocked_box_shadows: &mut BTreeMap<u32, LegacyBlockedBoxShadow>,
) -> Vec<String> {
    let mut differences = Vec::new();

    if frame.game_code != actual_game_code {
        differences.push(format!(
            "frame.game_code: original={} rust={actual_game_code}",
            frame.game_code
        ));
    }
    let selected: Vec<EntityId> = frame
        .selected_pcs
        .iter()
        .copied()
        .map(|id| entity_map.translate(id))
        .collect();
    if engine.selected_hero_ids() != selected {
        differences.push(format!(
            "selected_pcs: original={selected:?} rust={:?}",
            engine.selected_hero_ids()
        ));
    }

    // Actor state is generally the most actionable parity signal. Report it
    // before the (much larger) background-FX table.
    let mut elements: Vec<_> = frame.elements.iter().collect();
    elements.sort_by_key(|element| {
        let priority = match element.entity_id.kind {
            TraceEntityKind::Pc => 0,
            TraceEntityKind::Soldier => 1,
            TraceEntityKind::Civilian => 2,
            _ => 3,
        };
        (priority, element.entity_id.index)
    });
    for expected in elements {
        // Background FX animation is presentation state. It remains in the
        // trace for a later renderer-parity pass, but does not belong in the
        // first logical gameplay comparison.
        if expected.entity_id.kind == TraceEntityKind::Fx {
            continue;
        }
        let id = entity_map.translate(expected.entity_id);
        // Diff paths name the RUST id, but `--dump-entity`, the trace's
        // `elements[].entity_id.index` and the Original's logs all use the ORIGINAL
        // index, and the two are frequently unequal (e.g. Original pc:171 is
        // Rust Pc(PcId(174))). Four investigations lost hours to that mismatch, so
        // every diff root spells the pairing out.
        let id_label = EntityLabel {
            id,
            original_index: expected.entity_id.index,
        };
        let Some(actual) = engine.get_entity(id) else {
            differences.push(format!("{id_label:?}: missing in Rust entity table"));
            continue;
        };
        let element = actual.element_data();
        assert_eq!(
            expected.ai.is_some(),
            expected.detection.is_some(),
            "Original NPC trace state must contain both ai and detection payloads for {:?}",
            expected.entity_id
        );
        if entity_map.creation_order_is_exact(expected.creation_order) {
            compare(
                &mut differences,
                id,
                "creation_order",
                expected.creation_order,
                engine.original_creation_order(id),
            );
        }
        compare(
            &mut differences,
            id,
            "kind",
            expected.kind,
            trace_kind_for_entity(actual),
        );
        compare(
            &mut differences,
            id,
            "entity_id.kind",
            expected.entity_id.kind,
            expected.kind,
        );
        compare(
            &mut differences,
            id,
            "active",
            expected.active,
            element.active,
        );
        compare(
            &mut differences,
            id,
            "blipped",
            expected.blipped,
            element.blipped,
        );
        // `class_id` is redundant with the concrete kind above. Rust uses
        // that typed kind for dispatch rather than retaining Original's raw
        // numeric RTTI token. `surface_id` is a DrawManager allocation handle;
        // Rust has no corresponding per-entity render surface. Both fields
        // remain deserialized for diagnostics, but neither is logical state.
        if expected.actor.is_some() {
            compare(
                &mut differences,
                id,
                "unreachable",
                expected.unreachable,
                element.unreachable,
            );
            if expected.posture != element.posture as u32 {
                let ai = actual.ai_controller();
                differences.push(format!(
                    "{id:?}.posture: original={} rust={} (rust initial_action={:?} stay_home={:?} likes_to_sit={:?} sector={:?})",
                    expected.posture,
                    element.posture as u32,
                    ai.map(|ai| ai.initial_action),
                    ai.map(|ai| ai.is_stay_at_home),
                    ai.map(|ai| ai.likes_to_sit_around),
                    element.sector(),
                ));
            }
        }
        compare_point(
            &mut differences,
            id,
            "position_map",
            expected.position_map,
            element.position_map(),
        );
        let pi = &element.sprite.position_iface;
        let undefined_runtime_bonus_old_position =
            original_runtime_bonus_has_undefined_old_position(
                expected.kind,
                expected.creation_order,
                entity_map.runtime_creation_order_boundary,
            );
        if !undefined_runtime_bonus_old_position {
            compare_point(
                &mut differences,
                id,
                "old_position_map",
                expected.old_position_map,
                pi.old_map_position(),
            );
        }
        compare_point_with_absolute_tolerance(
            &mut differences,
            id,
            "position_goal_map",
            expected.position_goal_map,
            pi.map_goal(),
            0.011,
        );
        compare_float(
            &mut differences,
            id,
            "elevation",
            expected.elevation,
            pi.get_elevation(),
        );
        if !undefined_runtime_bonus_old_position {
            compare_float(
                &mut differences,
                id,
                "old_elevation",
                expected.old_elevation,
                pi.old_elevation(),
            );
        }
        let increment_map = pi.raw_increment_map();
        compare_point(
            &mut differences,
            id,
            "increment_map",
            expected.increment_map,
            MapPoint::new(increment_map.x, increment_map.y),
        );
        if let Some(expected_increment_map_valid) = expected.increment_map_valid {
            compare(
                &mut differences,
                id,
                "increment_map_valid",
                expected_increment_map_valid,
                pi.is_increment_map_computed(),
            );
        }
        if !undefined_runtime_bonus_old_position {
            let movement_map = element.position_map() - pi.old_map_position();
            compare_point(
                &mut differences,
                id,
                "movement_map",
                expected.movement_map,
                MapPoint::new(movement_map.x, movement_map.y),
            );
        }
        let actual_sector = element.sector().map_or(u16::MAX, |sector| sector.get());
        let mapped_building_sector = expected.sector != actual_sector
            && entity_map.sectors_equivalent(expected.sector, actual_sector);
        compare(
            &mut differences,
            id,
            "layer",
            expected.layer,
            element
                .optional_layer()
                .map_or(u16::MAX, robin_engine::position_interface::Layer::get),
        );
        compare(
            &mut differences,
            id,
            "layer_goal",
            expected.layer_goal,
            pi.optional_layer_goal()
                .map_or(u16::MAX, robin_engine::position_interface::Layer::get),
        );
        if !mapped_building_sector {
            compare(
                &mut differences,
                id,
                "sector",
                expected.sector,
                actual_sector,
            );
        }
        compare(
            &mut differences,
            id,
            "direction",
            expected.direction,
            i16::from(pi.get_direction().as_u8()),
        );
        compare(
            &mut differences,
            id,
            "direction_goal",
            expected.direction_goal,
            i16::from(pi.get_direction_goal().as_u8()),
        );
        if !undefined_runtime_bonus_old_position {
            compare(
                &mut differences,
                id,
                "moving",
                expected.moving,
                pi.is_moving(),
            );
            compare(
                &mut differences,
                id,
                "moving_map",
                expected.moving_map,
                pi.is_moving_map(),
            );
        }
        compare(
            &mut differences,
            id,
            "sprite_row",
            expected.sprite_row,
            element.sprite.current_row,
        );
        compare(
            &mut differences,
            id,
            "sprite_frame",
            expected.sprite_frame,
            element.sprite.current_frame,
        );
        if !legacy_additive_omissions {
            compare(
                &mut differences,
                id,
                "sprite_frame_count",
                expected.sprite_frame_count,
                element.sprite.frame_count,
            );
        }
        let mut expected_runtime = expected.runtime.to_json();
        if !expected_runtime.is_null() {
            let original_last_processed_order_id = expected_runtime
                .pointer("/sprite/last_processed_order_id")
                .and_then(serde_json::Value::as_u64)
                .and_then(|order_id| u32::try_from(order_id).ok());
            let reset_by_new_movement_order = expected
                .actor
                .as_ref()
                .zip(original_last_processed_order_id)
                .is_some_and(|(actor, last_processed_order_id)| {
                    let moved_this_frame = expected.position_map.x.bits
                        != expected.old_position_map.x.bits
                        || expected.position_map.y.bits != expected.old_position_map.y.bits;
                    original_reset_blocked_box_this_frame(
                        actor,
                        id,
                        last_processed_order_id,
                        moved_this_frame,
                        legacy_blocked_box_shadows
                            .get(&expected.creation_order)
                            .and_then(|shadow| shadow.pending_motion_order_id),
                        legacy_blocked_box_shadows
                            .get(&expected.creation_order)
                            .and_then(|shadow| shadow.stoppable_motion_order),
                        legacy_blocked_box_shadows
                            .get(&expected.creation_order)
                            .map(|shadow| shadow.last_processed_order_id),
                    )
                });
            let current_motion_order_id = expected
                .actor
                .as_ref()
                .and_then(|actor| original_motion_executor_order_id(actor, id));
            let current_stoppable_motion_order = expected
                .actor
                .as_ref()
                .and_then(original_stoppable_current_motion_order);
            canonicalize_legacy_blocked_box(
                &mut expected_runtime,
                expected.creation_order,
                reset_by_new_movement_order,
                current_motion_order_id,
                current_stoppable_motion_order,
                legacy_blocked_box_shadows,
            );
            canonicalize_original_runtime_representation(&mut expected_runtime);
            if expected.kind == TraceEntityKind::Projectile
                && expected.creation_order >= entity_map.runtime_creation_order_boundary
            {
                project_runtime_projectile_constructor_storage(&mut expected_runtime);
            }
            if legacy_missing_draw_view {
                // TODO: A schema that records the draw viewport must compare
                // these presentation-cache fields exactly.
                project_missing_draw_view_sprite_cache(&mut expected_runtime);
            }
            canonicalize_authoritative_snapshot(&mut expected_runtime, entity_map);
            let actual_runtime = engine.parity_entity_runtime_state(id, assets);
            collect_json_subset_differences(
                &format!("{id_label:?}.runtime"),
                &expected_runtime,
                &actual_runtime,
                &mut differences,
            );
        }
        if let Some(expected_actor) = &expected.actor {
            let actual_actor = actual
                .actor_data()
                .unwrap_or_else(|| panic!("trace reports actor state for non-actor {id:?}"));
            let execution_telemetry_is_logical = original_actor_execution_telemetry_is_logical(
                expected.creation_order,
                &frame.sequence_lifecycle_events,
            );
            if expected_actor.action_state != actual_actor.action_state as u32 {
                differences.push(format!(
                    "{id:?}.actor.action_state: original={} rust={} (original animation={} command={} motion={}; rust last_action={:?} command={:?} script_class={:?})",
                    expected_actor.action_state,
                    actual_actor.action_state as u32,
                    expected_actor.animation,
                    expected_actor.command,
                    expected_actor.motion_state,
                    element.sprite.last_action,
                    engine.actor_command(id),
                    actual_actor.script_class,
                ));
            }
            compare(
                &mut differences,
                id,
                "actor.wait_time",
                expected_actor.wait_time,
                engine.actor_legacy_wait_time(id),
            );
            // Older legacy recordings can
            // observe a dangling actor order after the actor's
            // owner slot invalidates and retranslates its movements. The
            // pointed-to allocator storage is not logical game state; the
            // replacement is compared through movement, path, and later
            // actor snapshots instead.
            if execution_telemetry_is_logical
                && original_actor_animation_is_logical(id, late_movement_retranslations)
            {
                compare(
                    &mut differences,
                    id,
                    "actor.animation",
                    expected_actor.animation,
                    engine
                        .actor_order_type(id)
                        .unwrap_or(robin_engine::order::OrderType::NonanimationEnd)
                        as u32,
                );
            }
            // Player-character strangling execution leaves its
            // local `motionState` uninitialized while either participant is
            // still turning, and the actor update copies that raw
            // return into the motion state. Preserve comparison for every value
            // in the original game's declared motion values, including its error sentinel, but do
            // not require Rust to manufacture captured stack bytes.
            if execution_telemetry_is_logical
                && original_motion_state_is_defined(expected_actor.motion_state)
            {
                compare(
                    &mut differences,
                    id,
                    "actor.motion_state",
                    expected_actor.motion_state,
                    actual_actor.continuation.motion_state as u32,
                );
            }
            compare(
                &mut differences,
                id,
                "actor.command",
                command_from_stable_name(&expected_actor.command_name),
                engine.actor_command(id),
            );
            if !legacy_additive_omissions {
                compare(
                    &mut differences,
                    id,
                    "actor.passing_door_directly",
                    expected_actor.passing_door_directly,
                    actual_actor.passing_door_directly,
                );
                let expected_pass_key = expected_actor
                    .active_pass_door
                    .as_ref()
                    .map(trace_pass_door_key);
                let actual_pass = engine
                    .actor_selected_pass_door(id)
                    .map(|(gate_id, direction)| (gate_id.get(), direction != 0));
                if !active_pass_door_keys_match(
                    expected_actor.active_pass_door.as_ref(),
                    actual_pass,
                ) {
                    differences.push(format!(
                        "{id:?}.actor.active_pass_door.(gate_id,direct): original={expected_pass_key:?} rust={actual_pass:?}"
                    ));
                }
            }
            if let Some(expected_sequence) = &expected_actor.sequence_element {
                compare(
                    &mut differences,
                    id,
                    "actor.sequence_element.command",
                    command_from_stable_name(&expected_sequence.command_name),
                    engine.actor_command(id),
                );
            }
        }
        if let Some(expected_human) = &expected.human {
            let actual_camp = actual.camp();
            let actual_life = match actual {
                Entity::Pc(pc) => pc.pc.life_points,
                Entity::Soldier(soldier) => soldier.npc.life_points,
                Entity::Civilian(civilian) => civilian.npc.life_points,
                _ => panic!("trace reports life_points for non-human {id:?}"),
            };
            compare(
                &mut differences,
                id,
                "life_points",
                expected_human.life_points,
                actual_life,
            );
            compare(
                &mut differences,
                id,
                "dead",
                expected_human.dead,
                actual.is_dead(),
            );
            compare(
                &mut differences,
                id,
                "unconscious",
                expected_human.unconscious,
                actual
                    .human_data()
                    .unwrap_or_else(|| panic!("trace reports human state for non-human {id:?}"))
                    .unconscious,
            );
            compare(
                &mut differences,
                id,
                "human.camp",
                expected_human.camp.as_str(),
                camp_name(actual_camp),
            );
            compare(
                &mut differences,
                id,
                "human.original_camp",
                expected_human.original_camp,
                camp_ordinal(actual_camp),
            );
            compare(
                &mut differences,
                id,
                "human.vip",
                expected_human.vip,
                entity_is_vip(actual, assets),
            );
            compare(
                &mut differences,
                id,
                "human.civilian",
                expected_human.civilian,
                actual.is_civilian(),
            );
            if !legacy_additive_omissions {
                let expected_opponents: Vec<EntityId> = expected_human
                    .opponents
                    .iter()
                    .copied()
                    .map(|opponent| entity_map.translate(opponent))
                    .collect();
                let actual_human = actual.human_data().unwrap_or_else(|| {
                    panic!("trace reports human opponents for non-human {id:?}")
                });
                compare(
                    &mut differences,
                    id,
                    "human.opponents",
                    expected_opponents,
                    actual_human.opponents.ids(),
                );
                let expected_jump_lines: Vec<Option<[u32; 4]>> = expected_human
                    .opponent_jump_lines
                    .iter()
                    .map(|line| line.as_ref().map(trace_jump_line_bits))
                    .collect();
                let actual_jump_lines: Vec<Option<[u32; 4]>> = actual_human
                    .opponents
                    .iter_with_jump_lines()
                    .map(|(_, line_index)| line_index)
                    .map(|line_index| {
                        line_index.map(|line_index| {
                            let line = engine
                                .fast_grid()
                                .level
                                .jump_lines
                                .get(usize::from(line_index))
                                .unwrap_or_else(|| {
                                    panic!(
                                        "human {id:?} opponent jump-line index {line_index} is out of range"
                                    )
                                });
                            runtime_jump_line_bits(line)
                        })
                    })
                    .collect();
                compare(
                    &mut differences,
                    id,
                    "human.opponent_jump_lines",
                    expected_jump_lines,
                    actual_jump_lines,
                );
            }
        }
        if let Some(expected_pc) = &expected.pc {
            use robin_engine::profiles::Action;

            let ammo = &expected_pc.ammo;
            for (field, expected_count, action) in [
                ("ales", ammo.ales, Action::Ale),
                ("apples", ammo.apples, Action::Apple),
                ("arrows", ammo.arrows, Action::Bow),
                ("nets", ammo.nets, Action::Net),
                ("plants", ammo.plants, Action::Heal),
                ("purses", ammo.purses, Action::Purse),
                ("rations", ammo.rations, Action::Eat),
                ("stones", ammo.stones, Action::Stone),
                ("wasp_nests", ammo.wasp_nests, Action::WaspNest),
            ] {
                compare(
                    &mut differences,
                    id,
                    &format!("pc.ammo.{field}"),
                    expected_count,
                    engine.get_pc_ammo_count(id, action),
                );
            }
        }
        if let Some(expected_ai) = &expected.ai {
            let actual_ai = actual
                .ai_controller()
                .unwrap_or_else(|| panic!("trace reports AI state for non-NPC {id:?}"));
            compare(
                &mut differences,
                id,
                "ai.state",
                expected_ai.state,
                actual_ai.current_state as u32,
            );
            compare(
                &mut differences,
                id,
                "ai.substate",
                expected_ai.substate,
                actual_ai.current_substate as u32,
            );
            if !legacy_additive_omissions {
                compare(
                    &mut differences,
                    id,
                    "ai.script_locked",
                    expected_ai.script_locked,
                    actual_ai.ai_is_script_locked(),
                );
                compare(
                    &mut differences,
                    id,
                    "ai.locked",
                    expected_ai.locked,
                    actual_ai.ai_is_locked(),
                );
                compare(
                    &mut differences,
                    id,
                    "ai.locks",
                    expected_ai.locks,
                    actual_ai.locks_flag_field.bits(),
                );
                compare(
                    &mut differences,
                    id,
                    "ai.was_busy",
                    expected_ai.was_busy,
                    actual_ai.was_busy,
                );
                compare(
                    &mut differences,
                    id,
                    "ai.very_busy",
                    expected_ai.very_busy,
                    engine.is_very_very_busy(id),
                );
                compare(
                    &mut differences,
                    id,
                    "ai.macro_timer_running",
                    expected_ai.macro_timer_running,
                    actual_ai.macro_timer_is_running,
                );
                compare(
                    &mut differences,
                    id,
                    "ai.macro_timer_ring",
                    expected_ai.macro_timer_ring,
                    actual_ai.when_does_macro_timer_ring,
                );
                // The cursor is only a position while it still lies inside the
                // waypoint block it was authored against. Advancing the patrol path
                // or breaking a macro leaves the cursor behind on the previous
                // waypoint's data, and the Original can only report an offset when
                // its pointer still falls within the current waypoint's block.
                // That is a question of identity, not of content: two waypoints can
                // carry byte-identical macro data, so the retained stream has to be
                // the one taken from the waypoint the path currently stands on.
                let current_waypoint = actual_ai
                    .has_patrol_path
                    .then_some(actual_ai.patrol_path.as_ref())
                    .flatten()
                    .map(|path| (path.hiking_path_index, path.current_waypoint_index));
                let actual_macro_cursor = current_waypoint
                    .filter(|current| {
                        actual_ai.macro_command_waypoint == Some(*current)
                            && !actual_ai.macro_command.is_empty()
                            && actual_ai.macro_command_offset <= actual_ai.macro_command.len()
                    })
                    .map(|_| {
                        u16::try_from(actual_ai.macro_command_offset).unwrap_or_else(|_| {
                            panic!(
                                "NPC {id:?} macro cursor {} exceeds the original game's 16-bit domain",
                                actual_ai.macro_command_offset
                            )
                        })
                    });
                compare(
                    &mut differences,
                    id,
                    "ai.macro_cursor",
                    expected_ai.macro_cursor,
                    actual_macro_cursor,
                );
                compare(
                    &mut differences,
                    id,
                    "ai.macro_remaining",
                    expected_ai.macro_remaining,
                    actual_ai.number_of_remaining_macro_bytes,
                );
                compare(
                    &mut differences,
                    id,
                    "ai.macro_in_progress",
                    expected_ai.macro_in_progress,
                    actual_ai.macro_in_progress,
                );
                let expected_us: Vec<EntityId> = expected_ai
                    .list_us
                    .iter()
                    .copied()
                    .map(|human| entity_map.translate(human))
                    .collect();
                let actual_us: Vec<EntityId> = actual_ai
                    .list_us
                    .iter()
                    .map(|&handle| {
                        engine.entity_id_for_index(handle).unwrap_or_else(|| {
                            panic!("AI list_us handle {handle} refers to a vacant entity slot")
                        })
                    })
                    .collect();
                compare(&mut differences, id, "ai.list_us", expected_us, actual_us);
                let expected_them: Vec<EntityId> = expected_ai
                    .list_them
                    .iter()
                    .copied()
                    .map(|human| entity_map.translate(human))
                    .collect();
                let actual_them: Vec<EntityId> = actual
                    .enemy_ai()
                    .map(|enemy| {
                        enemy
                            .list_them
                            .iter()
                            .map(|&handle| {
                                engine.entity_id_for_index(handle).unwrap_or_else(|| {
                                panic!(
                                    "AI list_them handle {handle} refers to a vacant entity slot"
                                )
                            })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                compare(
                    &mut differences,
                    id,
                    "ai.list_them",
                    expected_them,
                    actual_them,
                );
                let expected_line = expected_ai.my_line_jump.as_ref().map(trace_jump_line_bits);
                let actual_line_index = actual.enemy_ai().and_then(|enemy| enemy.my_line_jump);
                if expected_line.is_some() && actual.enemy_ai().is_none() {
                    panic!("trace reports a non-null my_line_jump for non-enemy entity {id:?}");
                }
                let actual_line = actual_line_index.map(|line_index| {
                    let line = engine
                        .fast_grid()
                        .level
                        .jump_lines
                        .get(line_index as usize)
                        .unwrap_or_else(|| {
                            panic!("enemy {id:?} my_line_jump index {line_index} is out of range")
                        });
                    runtime_jump_line_bits(line)
                });
                compare(
                    &mut differences,
                    id,
                    "ai.my_line_jump",
                    expected_line,
                    actual_line,
                );
            }
        }
        if let Some(expected_detection) = &expected.detection {
            let npc = actual
                .npc_data()
                .unwrap_or_else(|| panic!("trace reports detection state for non-NPC {id:?}"));
            let controller = actual
                .ai_controller()
                .unwrap_or_else(|| panic!("trace reports detection state for AI-less NPC {id:?}"));
            compare(
                &mut differences,
                id,
                "detection.suspects",
                expected_detection.suspects.as_slice(),
                npc.detection_suspects.as_slice(),
            );
            compare(
                &mut differences,
                id,
                "detection.maximal_suspect",
                expected_detection.maximal_suspect,
                npc.maximal_detection_suspect,
            );
            compare(
                &mut differences,
                id,
                "detection.maximal_visibility",
                expected_detection.maximal_visibility,
                controller.max_visibility,
            );
            compare(
                &mut differences,
                id,
                "detection.view_status",
                expected_detection.view_status,
                npc.eye_status as u8,
            );
            compare(
                &mut differences,
                id,
                "detection.alert_status",
                expected_detection.alert_status,
                controller.view_alert_status as u32,
            );

            let actual_detectables_len = npc.detectable_lists.iter().map(Vec::len).sum::<usize>();
            compare(
                &mut differences,
                id,
                "detection.detectables.length",
                expected_detection.detectables.len(),
                actual_detectables_len,
            );
            if expected_detection.detectables.len() != actual_detectables_len
                && std::env::var_os("PARITY_DEBUG_DETECTABLE_DIFF").is_some()
            {
                // Side-by-side dump of both flattened detectable lists for the
                // NPC whose length diverged. Unlike `PARITY_DEBUG_DETECTABLE_LIST`
                // this needs no frame/creation-order filter: it fires exactly at
                // the first divergent frame, which is where the identity of the
                // added/removed entry (bucket + target) has to be read off.
                eprintln!(
                    "DETDIFF owner={id:?} original_len={} rust_len={}",
                    expected_detection.detectables.len(),
                    actual_detectables_len
                );
                for (i, d) in expected_detection.detectables.iter().enumerate() {
                    eprintln!(
                        "DETDIFF original[{i}] type={} target={:?}->{:?} seen_now={} seen_last={} heard_last={} shadow_now={} shadow_last={} vis={:?}",
                        d.detectable_type,
                        d.target,
                        entity_map.translate(d.target),
                        d.seen_now,
                        d.seen_last_frame,
                        d.heard_last_frame,
                        d.shadow_seen_now,
                        d.shadow_seen_last_frame,
                        d.last_visibility
                    );
                }
                for (i, d) in npc
                    .detectable_lists
                    .iter()
                    .flat_map(|list| list.iter())
                    .enumerate()
                {
                    eprintln!(
                        "DETDIFF rust[{i}] type={} target={:?} seen_now={} seen_last={} heard_last={} shadow_now={} shadow_last={} vis={}",
                        detectable_type_ordinal(d.detectable_type),
                        d.element,
                        d.seen_now,
                        d.seen_last_frame,
                        d.heard_last_frame,
                        d.shadow_seen_now,
                        d.shadow_seen_last_frame,
                        d.last_visibility
                    );
                }
            }
            for (detectable_index, (expected_detectable, actual_detectable)) in expected_detection
                .detectables
                .iter()
                .zip(npc.detectable_lists.iter().flat_map(|list| list.iter()))
                .enumerate()
            {
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "type",
                    expected_detectable.detectable_type,
                    detectable_type_ordinal(actual_detectable.detectable_type),
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "target",
                    entity_map.translate(expected_detectable.target),
                    actual_detectable.element.unwrap_or_else(|| {
                        panic!("NPC {id:?} detectable {detectable_index} has no target element")
                    }),
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "seen_now",
                    expected_detectable.seen_now,
                    actual_detectable.seen_now,
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "seen_last_frame",
                    expected_detectable.seen_last_frame,
                    actual_detectable.seen_last_frame,
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "heard_last_frame",
                    expected_detectable.heard_last_frame,
                    actual_detectable.heard_last_frame,
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "shadow_seen_now",
                    expected_detectable.shadow_seen_now,
                    actual_detectable.shadow_seen_now,
                );
                compare_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "shadow_seen_last_frame",
                    expected_detectable.shadow_seen_last_frame,
                    actual_detectable.shadow_seen_last_frame,
                );
                compare_float_indexed(
                    &mut differences,
                    id,
                    "detection.detectables",
                    detectable_index,
                    "last_visibility",
                    expected_detectable.last_visibility,
                    actual_detectable.last_visibility,
                );
            }
        }
    }
    differences
}

pub(super) fn compare<T: std::fmt::Debug + PartialEq>(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: T,
    actual: T,
) {
    if expected != actual {
        differences.push(format!(
            "{id:?}.{field}: original={expected:?} rust={actual:?}"
        ));
    }
}

pub(super) fn original_motion_state_is_defined(raw: u32) -> bool {
    // Original-game sprite states: DONE, START, IN_PROGRESS, TERMINATED,
    // ABORTED, ERROR.
    raw <= 5
}

pub(super) fn compare_indexed<T: std::fmt::Debug + PartialEq>(
    differences: &mut Vec<String>,
    id: EntityId,
    collection: &str,
    index: usize,
    field: &str,
    expected: T,
    actual: T,
) {
    if expected != actual {
        differences.push(format!(
            "{id:?}.{collection}[{index}].{field}: original={expected:?} rust={actual:?}"
        ));
    }
}

pub(super) fn trace_kind_for_entity(entity: &Entity) -> TraceEntityKind {
    match entity {
        Entity::Pc(_) => TraceEntityKind::Pc,
        Entity::Soldier(_) => TraceEntityKind::Soldier,
        Entity::Civilian(_) => TraceEntityKind::Civilian,
        Entity::Fx(_) => TraceEntityKind::Fx,
        Entity::Target(_) => TraceEntityKind::Target,
        Entity::Bonus(_) => TraceEntityKind::Bonus,
        Entity::Scroll(_) => TraceEntityKind::Scroll,
        Entity::Projectile(_) => TraceEntityKind::Projectile,
        Entity::Net(_) => TraceEntityKind::Net,
    }
}

pub(super) fn camp_name(camp: robin_engine::element::Camp) -> &'static str {
    match camp {
        robin_engine::element::Camp::Royalists => "royalists",
        robin_engine::element::Camp::Lacklandists => "lacklandists",
        robin_engine::element::Camp::Error => "error",
        robin_engine::element::Camp::Custom(_) => "custom",
    }
}

pub(super) fn camp_ordinal(camp: robin_engine::element::Camp) -> i32 {
    match camp {
        robin_engine::element::Camp::Royalists => 0,
        robin_engine::element::Camp::Lacklandists => 1,
        robin_engine::element::Camp::Error => 2,
        robin_engine::element::Camp::Custom(id) => i32::from(id),
    }
}

pub(super) fn entity_is_vip(entity: &Entity, assets: &LevelAssets) -> bool {
    match entity {
        Entity::Pc(pc) => {
            assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "parity VIP lookup is missing PC character profile {:?}",
                        pc.pc.profile_index
                    )
                })
                .vip
        }
        Entity::Soldier(soldier) => {
            assets
                .profile_manager
                .get_soldier(soldier.soldier.soldier_profile_index)
                .unwrap_or_else(|| {
                    panic!(
                        "parity VIP lookup is missing soldier profile {:?}",
                        soldier.soldier.soldier_profile_index
                    )
                })
                .vip
        }
        Entity::Civilian(civilian) => {
            civilian.civilian.cached_civilian_type == robin_engine::profiles::CivilianType::Vip
        }
        _ => panic!("parity VIP lookup called for non-human entity"),
    }
}

pub(super) fn detectable_type_ordinal(
    detectable_type: robin_engine::element::DetectableType,
) -> u32 {
    match detectable_type {
        robin_engine::element::DetectableType::Enemy => 0,
        robin_engine::element::DetectableType::Body => 1,
        robin_engine::element::DetectableType::Object => 2,
        robin_engine::element::DetectableType::Friend => 3,
        robin_engine::element::DetectableType::MissedFriend => 4,
        robin_engine::element::DetectableType::Beggar => 5,
        robin_engine::element::DetectableType::None => 6,
    }
}

pub(super) fn compare_float(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: TraceFloat,
    actual: f32,
) {
    compare_float_with_absolute_tolerance(differences, id, field, expected, actual, 0.0);
}

pub(super) fn compare_float_with_absolute_tolerance(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: TraceFloat,
    actual: f32,
    absolute_tolerance: f32,
) {
    let expected_value = expected.value();
    let scale = expected_value.abs().max(actual.abs()).max(1.0);
    let logically_equal = (expected_value.is_nan() && actual.is_nan())
        || (expected_value.is_finite()
            && actual.is_finite()
            && (expected_value - actual).abs() <= (1.0e-5 * scale).max(absolute_tolerance));
    if !logically_equal {
        differences.push(format!(
            "{id:?}.{field}: original={} (0x{:08x}) rust={} (0x{:08x})",
            expected.value(),
            expected.bits,
            actual,
            actual.to_bits()
        ));
    }
}

pub(super) fn compare_point(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: TracePoint,
    actual: MapPoint,
) {
    compare_float_component(differences, id, field, "x", expected.x, actual.x, 0.0);
    compare_float_component(differences, id, field, "y", expected.y, actual.y, 0.0);
}

pub(super) fn compare_point_with_absolute_tolerance(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    expected: TracePoint,
    actual: MapPoint,
    absolute_tolerance: f32,
) {
    compare_float_component(
        differences,
        id,
        field,
        "x",
        expected.x,
        actual.x,
        absolute_tolerance,
    );
    compare_float_component(
        differences,
        id,
        field,
        "y",
        expected.y,
        actual.y,
        absolute_tolerance,
    );
}

pub(super) fn compare_float_component(
    differences: &mut Vec<String>,
    id: EntityId,
    field: &str,
    component: &str,
    expected: TraceFloat,
    actual: f32,
    absolute_tolerance: f32,
) {
    let expected_value = expected.value();
    let scale = expected_value.abs().max(actual.abs()).max(1.0);
    let logically_equal = (expected_value.is_nan() && actual.is_nan())
        || (expected_value.is_finite()
            && actual.is_finite()
            && (expected_value - actual).abs() <= (1.0e-5 * scale).max(absolute_tolerance));
    if !logically_equal {
        differences.push(format!(
            "{id:?}.{field}.{component}: original={} (0x{:08x}) rust={} (0x{:08x})",
            expected.value(),
            expected.bits,
            actual,
            actual.to_bits()
        ));
    }
}

pub(super) fn compare_float_indexed(
    differences: &mut Vec<String>,
    id: EntityId,
    collection: &str,
    index: usize,
    field: &str,
    expected: TraceFloat,
    actual: f32,
) {
    let expected_value = expected.value();
    let scale = expected_value.abs().max(actual.abs()).max(1.0);
    let logically_equal = (expected_value.is_nan() && actual.is_nan())
        || (expected_value.is_finite()
            && actual.is_finite()
            && (expected_value - actual).abs() <= 1.0e-5 * scale);
    if !logically_equal {
        differences.push(format!(
            "{id:?}.{collection}[{index}].{field}: original={} (0x{:08x}) rust={} (0x{:08x})",
            expected.value(),
            expected.bits,
            actual,
            actual.to_bits()
        ));
    }
}

pub(super) fn original_actor_animation_is_logical(
    id: EntityId,
    late_movement_retranslations: &[EntityId],
) -> bool {
    !late_movement_retranslations.contains(&id)
}

/// The original game leaves selected order and motion state untouched when an instruction
/// completes during `Translate` after the actor's Hourglass slot has already
/// run. The sequence callback may have removed or replaced its logical current
/// order by capture time, but the actor's execution telemetry is not refreshed
/// until the next actor slot and cannot affect gameplay.
///
/// This is the actor-instruction early return immediately after
/// translation, before the normal assignments to motion state and order. Require
/// the recorded early-return event so all other actor snapshots retain exact
/// telemetry comparison.
pub(super) fn original_actor_execution_telemetry_is_logical(
    creation_order: u32,
    lifecycle: &[TraceSequenceLifecycleEvent],
) -> bool {
    let completed_during_translation = lifecycle.iter().any(|event| {
        event.event == "actor_instruct_result"
            && event.phase == "completed_during_translation"
            && event.actor_creation_order == Some(creation_order)
    });
    !completed_during_translation
}

pub(super) fn compare_visibility_queries(
    expected: &[TraceVisibilityQuery],
    actual: &[robin_engine::sight_obstacle::ParityVisibilityQuery],
) -> Vec<String> {
    let mut differences = Vec::new();
    if expected.len() != actual.len() {
        differences.push(format!(
            "frame.visibility_queries.length: original={} rust={}",
            expected.len(),
            actual.len()
        ));
    }
    for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
        let actual_origin = actual.origin.map(f32::to_bits);
        let actual_destination = actual.destination.map(f32::to_bits);
        let expected_origin = [
            expected.origin.x.bits,
            expected.origin.y.bits,
            expected.origin.z.bits,
        ];
        let expected_destination = [
            expected.destination.x.bits,
            expected.destination.y.bits,
            expected.destination.z.bits,
        ];
        if expected_origin != actual_origin {
            differences.push(format!(
                "frame.visibility_queries[{index}].origin: original_bits={expected_origin:?} rust_bits={actual_origin:?}"
            ));
        }
        if expected_destination != actual_destination {
            differences.push(format!(
                "frame.visibility_queries[{index}].destination: original_bits={expected_destination:?} rust_bits={actual_destination:?}"
            ));
        }
        if expected.result != actual.result {
            differences.push(format!(
                "frame.visibility_queries[{index}].result: original={} rust={}",
                expected.result, actual.result
            ));
        }
    }
    differences
}

struct TracePathRequestRef<'a> {
    actor: TraceEntityId,
    antagonist: Option<TraceEntityId>,
    layer: u16,
    area: u16,
    source: &'a TracePoint,
    goal: &'a TracePoint,
    half_diagonal_index: u16,
    half_diagonal: &'a TracePoint,
    animation: u32,
    reverse: bool,
    speed: u8,
    tolerance: &'a TraceFloat,
    use_first_point: bool,
}

impl TracePathEvent {
    fn request(&self) -> TracePathRequestRef<'_> {
        match self {
            TracePathEvent::Queued {
                actor,
                antagonist,
                layer,
                area,
                source,
                goal,
                half_diagonal_index,
                half_diagonal,
                animation,
                reverse,
                speed,
                tolerance,
                use_first_point,
            }
            | TracePathEvent::Completed {
                actor,
                antagonist,
                layer,
                area,
                source,
                goal,
                half_diagonal_index,
                half_diagonal,
                animation,
                reverse,
                speed,
                tolerance,
                use_first_point,
                ..
            } => TracePathRequestRef {
                actor: *actor,
                antagonist: *antagonist,
                layer: *layer,
                area: *area,
                source,
                goal,
                half_diagonal_index: *half_diagonal_index,
                half_diagonal,
                animation: *animation,
                reverse: *reverse,
                speed: *speed,
                tolerance,
                use_first_point: *use_first_point,
            },
        }
    }
}

pub(super) fn compare_path_events(
    expected: &[TracePathEvent],
    actual: &[robin_engine::pathfinder::ParityPathEvent],
    entity_map: &EntityMap,
) -> Vec<String> {
    use robin_engine::pathfinder::ParityPathEvent;

    let mut differences = Vec::new();
    if expected.len() != actual.len() {
        differences.push(format!(
            "frame.path_events.length: original={} rust={}",
            expected.len(),
            actual.len()
        ));
    }
    for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
        let expected_phase = match expected {
            TracePathEvent::Queued { .. } => "queued",
            TracePathEvent::Completed { .. } => "completed",
        };
        let (actual_phase, actual_request) = match actual {
            ParityPathEvent::Queued(request) => ("queued", request),
            ParityPathEvent::Completed { request, .. } => ("completed", request),
        };
        if expected_phase != actual_phase {
            differences.push(format!(
                "frame.path_events[{index}].phase: original={expected_phase} rust={actual_phase}"
            ));
        }

        let expected_request = expected.request();
        let prefix = format!("frame.path_events[{index}]");
        macro_rules! compare_path_field {
            ($field:expr, $expected:expr, $actual:expr) => {
                if $expected != $actual {
                    differences.push(format!(
                        "{}.{field}: original={:?} rust={:?}",
                        prefix,
                        $expected,
                        $actual,
                        field = $field
                    ));
                }
            };
        }
        compare_path_field!(
            "actor",
            entity_map.translate(expected_request.actor),
            actual_request.actor
        );
        compare_path_field!(
            "antagonist",
            expected_request
                .antagonist
                .map(|entity| entity_map.translate(entity)),
            actual_request.antagonist
        );
        compare_path_field!("layer", expected_request.layer, actual_request.layer);
        compare_path_field!("area", expected_request.area, actual_request.area);
        compare_path_field!(
            "source.bits",
            [
                expected_request.source.x.bits,
                expected_request.source.y.bits
            ],
            [
                actual_request.source.x.to_bits(),
                actual_request.source.y.to_bits()
            ]
        );
        compare_path_field!(
            "goal.bits",
            [expected_request.goal.x.bits, expected_request.goal.y.bits],
            [
                actual_request.goal.x.to_bits(),
                actual_request.goal.y.to_bits()
            ]
        );
        compare_path_field!(
            "half_diagonal_index",
            expected_request.half_diagonal_index,
            actual_request.half_diagonal_index
        );
        compare_path_field!(
            "half_diagonal.bits",
            [
                expected_request.half_diagonal.x.bits,
                expected_request.half_diagonal.y.bits
            ],
            [
                actual_request.half_diagonal.x.to_bits(),
                actual_request.half_diagonal.y.to_bits()
            ]
        );
        compare_path_field!(
            "animation",
            expected_request.animation,
            actual_request.animation
        );
        compare_path_field!("reverse", expected_request.reverse, actual_request.reverse);
        compare_path_field!("speed", expected_request.speed, actual_request.speed);
        compare_path_field!(
            "tolerance.bits",
            expected_request.tolerance.bits,
            actual_request.tolerance.to_bits()
        );
        compare_path_field!(
            "use_first_point",
            expected_request.use_first_point,
            actual_request.use_first_point
        );

        if let (
            TracePathEvent::Completed {
                valid: expected_valid,
                waypoints: expected_waypoints,
                ..
            },
            ParityPathEvent::Completed {
                valid: actual_valid,
                waypoints: actual_waypoints,
                ..
            },
        ) = (expected, actual)
        {
            compare_path_field!("valid", *expected_valid, *actual_valid);
            compare_path_field!(
                "waypoints.length",
                expected_waypoints.len(),
                actual_waypoints.len()
            );
            for (waypoint, (expected, actual)) in
                expected_waypoints.iter().zip(actual_waypoints).enumerate()
            {
                compare_path_field!(
                    &format!("waypoints[{waypoint}].bits"),
                    [expected.x.bits, expected.y.bits],
                    [actual.x.to_bits(), actual.y.to_bits()]
                );
            }
        }
    }
    differences
}

/// Compare an authoritative recorded JSON projection against a richer Rust
/// projection. Every value emitted by the original game must match; Rust-only values
/// are intentionally ignored so the engine may expose additional diagnostics
/// without retroactively changing the trace schema.
pub(super) fn collect_json_subset_differences(
    path: &str,
    expected: &serde_json::Value,
    actual: &serde_json::Value,
    differences: &mut Vec<String>,
) {
    if differences.len() >= 64 || expected == actual {
        return;
    }
    match (expected, actual) {
        (serde_json::Value::Array(expected), serde_json::Value::Array(actual)) => {
            if expected.len() != actual.len() {
                differences.push(format!(
                    "{path}.length: original={} rust={}",
                    expected.len(),
                    actual.len()
                ));
            }
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                collect_json_subset_differences(
                    &format!("{path}[{index}]"),
                    expected,
                    actual,
                    differences,
                );
            }
        }
        (serde_json::Value::Object(expected), serde_json::Value::Object(actual)) => {
            // `value` is a human-readable rendering of the authoritative f32
            // bit pattern. nlohmann-json and serde-json can choose adjacent
            // decimal spellings for the same bits.
            if expected.len() == 2
                && actual.len() == 2
                && expected.contains_key("bits")
                && expected.contains_key("value")
                && actual.contains_key("bits")
                && actual.contains_key("value")
            {
                collect_json_subset_differences(
                    &format!("{path}.bits"),
                    &expected["bits"],
                    &actual["bits"],
                    differences,
                );
                return;
            }

            for (key, expected) in expected {
                match actual.get(key) {
                    Some(actual) => collect_json_subset_differences(
                        &format!("{path}.{key}"),
                        expected,
                        actual,
                        differences,
                    ),
                    None => differences.push(format!(
                        "{path}.{key}: original={expected:?} rust=<missing>"
                    )),
                }
                if differences.len() >= 64 {
                    break;
                }
            }
            differences.sort();
            differences.dedup();
        }
        _ => differences.push(format!("{path}: original={expected:?} rust={actual:?}")),
    }
}
