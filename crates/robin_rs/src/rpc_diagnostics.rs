//! Read-only RPC diagnostic builders. No request queue or transport ownership.
use crate::http_server::ReplayStatus;
use robin_assets::decompile as assets_decompile;
use robin_engine::engine::{Engine, LevelAssets};
use robin_engine::{
    coordinates as engine_coordinates, element as engine_element, engine as engine_api,
    natives as engine_natives, position_interface as engine_position_interface,
    profiles as engine_profiles, scb as engine_scb, weapons as engine_weapons,
};
pub(crate) fn info_json() -> serde_json::Value {
    serde_json::json!({
        "name": "robin-hood-script-rpc",
        "endpoints": [
            {"method": "GET",  "path": "/natives",            "desc": "list every NativeFn (index, name, params, return type)"},
            {"method": "GET",  "path": "/engine-dump",        "desc": "full serialized engine for ad-hoc debug"},
            {"method": "GET",  "path": "/level-assets",       "desc": "level-scoped static assets for ad-hoc debug, including static fast-grid sectors plus runtime fast-grid flags"},
            {"method": "GET",  "path": "/host-debug",         "desc": "host/UI state for ad-hoc debug, including trajectory preview and mouse hover fields"},
            {"method": "GET",  "path": "/script",             "desc": "mission-script class & function listing"},
            {"method": "GET",  "path": "/script/decompile",   "desc": "decompile to TypeScript-like pseudocode (?class=Foo)"},
            {"method": "POST", "path": "/native",             "desc": "invoke one native: {op, args, this?}"},
            {"method": "POST", "path": "/batch",              "desc": "invoke many natives on one tick: {calls: [{op, args, this?}]}"},
            {"method": "POST", "path": "/console",            "desc": "run a debug-console command: {command: '...'}"},
            {"method": "POST", "path": "/command",            "desc": "apply a PlayerCommand (externally-tagged JSON enum)"},
            {"method": "GET",  "path": "/screenshot",         "desc": "PNG at the requested frame. Query: frame (absolute sim frame), full_map, w, h (aspect-preserving max bounds), hide_ui, view_cones, pc_sight, motion_graph, all_obstacles, elevation, noise, sound_source, actor_info, script_zones, door, projection_areas, railroad, probability, company_number, combat_energy, light_zones, animation_lines, seek_points, fps, sprite_masks, entity_ids (bool flags)"},
            {"method": "POST", "path": "/step-forward",       "desc": "Run N engine ticks with --start-paused. Body {n: N, auto_dismiss: bool, dismissals: [{kind, result}], synchronized_multiplayer: bool}; live multiplayer requires explicit synchronized_multiplayer=true on the host and reconnects peers from the result."},
            {"method": "POST", "path": "/step-back",          "desc": "Rewind N frames via the rewind buffer. Body {n: N, auto_dismiss, dismissals}; the modal policy matches step-forward. Fails if target frame is older than the oldest retained snapshot."},
            {"method": "POST", "path": "/go-to-frame",        "desc": "Seek to an absolute simulation frame in live play or recording ordinal in replay. Body {frame: N, auto_dismiss, dismissals}; replay ordinals include all saves and reloads."},
        ],
    })
}

pub(crate) fn list_natives_json() -> serde_json::Value {
    let mut entries = Vec::new();
    for i in 0u32..512 {
        if let Ok(n) = engine_natives::NativeFn::try_from(i) {
            let name: &'static str = n.into();
            let sig = engine_natives::native_signature_by_name(name);
            entries.push(serde_json::json!({
                "index": i,
                "name": name,
                "return_type": sig.map(|s| s.return_type),
                "params": sig.map(|s| {
                    s.params.iter().map(|p| serde_json::json!({"type": p.ty, "name": p.name})).collect::<Vec<_>>()
                }),
            }));
        }
    }
    serde_json::json!({"natives": entries})
}

/// Build the stable public state response without acquiring request authority.
pub(crate) fn snapshot_state(engine: &Engine, replay: Option<ReplayStatus>) -> serde_json::Value {
    let replay = replay.map(|s| {
        serde_json::json!({
            "frame": s.frame,
            "total": s.total,
            "paused": s.paused,
        })
    });
    serde_json::json!({
        "frame": engine.frame_counter(),
        "map": engine.mission_map_name(),
        "replay": replay,
    })
}

pub(crate) fn snapshot_host_debug(
    engine: &Engine,
    frontend: &crate::host::HostFrontend,
    local_seat: robin_engine::player_command::PlayerId,
    assets: &LevelAssets,
) -> serde_json::Value {
    let selected_action = engine.selected_action_for_seat(local_seat);
    let selected_pc = engine.hero_selection(local_seat).first().copied();
    let selected_pc_state = selected_pc.and_then(|id| {
        engine.get_entity(id).map(|entity| {
            serde_json::json!({
                "id": id,
                "kind": entity.kind(),
                "pc_current_action": entity.pc_data().map(|pc| pc.current_action),
                "actor_action_state": entity.actor_data().map(|actor| actor.action_state),
                "position_map": entity.element_data().position_map(),
                "position_3d": entity.element_data().position(),
                "layer": entity.element_data().layer(),
                "direction": entity.element_data().direction(),
            })
        })
    });
    let preview = frontend.trajectory_preview();
    let last_preview_point = preview.points().last().map(|point| {
        serde_json::json!({
            "position": point.position,
            "time": point.time,
        })
    });
    let bow_hover = match (
        selected_action,
        selected_pc,
        frontend.input.feedback.focused_entity_id,
    ) {
        (engine_profiles::Action::Bow, Some(pc_id), Some(target_id)) => {
            let (target_status, shoot_mode) =
                engine.can_shoot_with_bow_at(assets, pc_id, target_id);
            Some(serde_json::json!({
                "target_id": target_id,
                "target_status": format!("{target_status:?}"),
                "shoot_mode": format!("{shoot_mode:?}"),
                "range_debug": bow_range_debug(engine, assets, pc_id, target_id),
            }))
        }
        _ => None,
    };

    serde_json::json!({
        "frame": engine.frame_counter(),
        "selected_action": selected_action,
        "selection": engine.hero_selection(local_seat),
        "selected_pc": selected_pc_state,
        "valid_trajectory": preview.is_valid(),
        "trajectory_preview_points_len": preview.points().len(),
        "trajectory_preview_start": preview.start(),
        "trajectory_preview_last": last_preview_point,
        "trajectory_preview_layer": preview.layer(),
        "net_crumpled": preview.crumpled(),
        "time_no_mouse_move": preview.hover_ticks(),
        "mouse_map_prev": preview.previous_mouse(),
        "trajectory_mark_count": preview.mark_count(),
        "bow_hover": bow_hover,
        "input": {
            "focused_entity_id": frontend.input.feedback.focused_entity_id,
            "target_drag": frontend.input.gestures.target_drag,
            "double_status_bar_entity_id": frontend.input.feedback.double_status_bar_entity_id,
            "selected_layer": frontend.input.spatial_hit().selected_layer,
            "selected_sector_idx": frontend.input.spatial_hit().selected_sector_idx,
            "selected_patch_idx": frontend.input.spatial_hit().selected_patch_idx,
            "hovered_door_idx": frontend.input.spatial_hit().hovered_door_idx,
            "valid_position_for_move": frontend.input.spatial_hit().valid_position_for_move,
            "mouse_opacity": frontend.input.feedback.mouse_opacity,
            "mouse_shadow_color": frontend.input.feedback.mouse_shadow_color,
            "left_mouse_down": frontend.input.left_mouse_down(),
            "right_mouse_down": frontend.input.controls.right_mouse_down,
            "is_dragging": frontend.input.is_dragging(),
            "is_alt": frontend.input.controls.is_alt,
        },
    })
}

fn bow_debug_ground_y_raw(point: engine_coordinates::WorldPoint3D) -> f32 {
    point.y
}

fn bow_debug_ground_y_projected(point: engine_coordinates::WorldPoint3D) -> f32 {
    point.to_map().y
}

fn game_sector_0_to_15_with_aspect(x: f32, y: f32, aspect_ratio: f32) -> u8 {
    const COS_PI_SIXTEENTH: f32 = 0.980_785_25;
    const SIN_PI_SIXTEENTH: f32 = 0.195_090_32;
    const TAN_PI_EIGHTH: f32 = 0.414_213_57;

    let mut rotated_x = x * COS_PI_SIXTEENTH * aspect_ratio - y * SIN_PI_SIXTEENTH;
    let mut rotated_y = x * SIN_PI_SIXTEENTH * aspect_ratio + y * COS_PI_SIXTEENTH;

    let west = rotated_x < 0.0;
    if west {
        rotated_x = -rotated_x;
    }

    let south = rotated_y > 0.0;
    if !south {
        rotated_y = -rotated_y;
    }

    let east_west = rotated_y < rotated_x;
    let skew = if east_west {
        rotated_y > rotated_x * TAN_PI_EIGHTH
    } else {
        rotated_x > rotated_y * TAN_PI_EIGHTH
    };

    let mut sector = 0u8;
    if west {
        sector |= 8;
    }
    if west ^ south {
        sector |= 4;
    }
    if west ^ south ^ east_west {
        sector |= 2;
    }
    if west ^ south ^ east_west ^ skew {
        sector |= 1;
    }
    sector
}

fn bow_profile_debug(
    engine: &Engine,
    assets: &LevelAssets,
    entity_id: engine_element::EntityId,
) -> Option<serde_json::Value> {
    let entity = engine.get_entity(entity_id)?;
    let (bow_profile_idx, shooting_ability) = match entity {
        engine_element::Entity::Pc(pc) => {
            let idx = usize::from(pc.pc.profile_index);
            let profile = assets.profile_manager.characters.get(idx)?;
            if profile.shooting_weapon_id == 0 {
                return None;
            }
            (profile.shooting_weapon_id, profile.shooting as u32)
        }
        engine_element::Entity::Soldier(soldier) => {
            let idx = usize::from(soldier.soldier.soldier_profile_index);
            let profile = assets.profile_manager.soldiers.get(idx)?;
            if profile.shooting_weapon_id == 0 {
                return None;
            }
            (profile.shooting_weapon_id, profile.shooting as u32)
        }
        _ => return None,
    };

    let bow_profile = assets.profile_manager.get_bow(bow_profile_idx)?;
    let bow_state = engine_weapons::BowState::new(bow_profile_idx, bow_profile, 1);
    Some(serde_json::json!({
        "bow_profile_idx": bow_profile_idx,
        "shooting_ability": shooting_ability,
        "normal_range": bow_profile.normal_shoot.range,
        "long_range": bow_profile.long_shoot.range,
        "has_long_shoot": bow_profile.has_long_shoot,
        "max_range": bow_state.get_max_range(bow_profile),
    }))
}

fn bow_target_points_debug(
    engine: &Engine,
    target_id: engine_element::EntityId,
) -> Option<serde_json::Value> {
    let target = engine.get_entity(target_id)?;
    let range_target = if target.is_human() {
        target.compute_belt_point()
    } else {
        Some(target.element_data().position())
    };
    let preview_target = if target.is_human() {
        target.compute_belt_point()
    } else if target.is_fx_target() {
        target.compute_target_center()
    } else {
        Some(target.element_data().position())
    };

    Some(serde_json::json!({
        "id": target_id,
        "kind": target.kind(),
        "is_human": target.is_human(),
        "is_fx_target": target.is_fx_target(),
        "position_3d": target.element_data().position(),
        "position_map": target.element_data().position_map(),
        "belt_point": target.compute_belt_point(),
        "eyes_point": target.compute_eyes_point(None),
        "fx_center": target.compute_target_center(),
        "range_target_point": range_target,
        "preview_target_point": preview_target,
    }))
}

fn bow_range_math_debug(
    hand_point: engine_coordinates::WorldPoint3D,
    target_point: engine_coordinates::WorldPoint3D,
    max_range: f32,
    forest_target: bool,
) -> serde_json::Value {
    const THROW_ANGLE_BOW: f32 = 0.3;
    let rel_height = hand_point.z - target_point.z;
    let base_radius = if rel_height > 0.0 {
        max_range + rel_height * THROW_ANGLE_BOW.tan()
    } else {
        max_range
    };
    let radius = if forest_target {
        base_radius * 2.0
    } else {
        base_radius
    };

    let dx = target_point.x - hand_point.x;
    let dy_raw = bow_debug_ground_y_raw(target_point) - bow_debug_ground_y_raw(hand_point);
    let dy_projected =
        bow_debug_ground_y_projected(target_point) - bow_debug_ground_y_projected(hand_point);
    let dz = target_point.z - hand_point.z;
    let dy_range_raw = dy_raw * engine_position_interface::INVERSE_ASPECT_RATIO_PROJECTILES;
    let dy_range_projected =
        dy_projected * engine_position_interface::INVERSE_ASPECT_RATIO_PROJECTILES;
    let square_distance_raw = dx * dx + dy_range_raw * dy_range_raw;
    let square_distance_projected = dx * dx + dy_range_projected * dy_range_projected;
    let radius_square = radius * radius;
    let dist_3d_raw = (dx * dx + dy_raw * dy_raw + dz * dz).sqrt();
    let dist_3d_projected = (dx * dx + dy_projected * dy_projected + dz * dz).sqrt();

    serde_json::json!({
        "hand_point": hand_point,
        "target_point": target_point,
        "target_delta": {
            "dx": dx,
            "dy_raw_game": dy_raw,
            "dy_projected_y_minus_z": dy_projected,
            "dz": dz,
        },
        "range": {
            "max_range": max_range,
            "rel_height": rel_height,
            "throw_angle_bow": THROW_ANGLE_BOW,
            "base_radius": base_radius,
            "forest_target": forest_target,
            "radius": radius,
            "radius_square": radius_square,
            "dy_raw_times_projectile_aspect": dy_range_raw,
            "dy_projected_times_projectile_aspect": dy_range_projected,
            "square_distance_raw_game_y": square_distance_raw,
            "square_distance_projected_y_minus_z": square_distance_projected,
            "in_range_raw_game_y": square_distance_raw < radius_square,
            "in_range_projected_y_minus_z": square_distance_projected < radius_square,
            "dist_3d_raw_game_y": dist_3d_raw,
            "dist_3d_projected_y_minus_z": dist_3d_projected,
        },
        "direction": {
            "iso_sector_raw_game_y": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_raw),
            "rust_iso_sector_projected_y_minus_z": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_projected),
            "game_sector_aspect_raw_game_y": game_sector_0_to_15_with_aspect(
                dx,
                dy_raw,
                engine_position_interface::ASPECT_RATIO,
            ),
            "game_sector_aspect_projected_y_minus_z": game_sector_0_to_15_with_aspect(
                dx,
                dy_projected,
                engine_position_interface::ASPECT_RATIO,
            ),
        },
    })
}

fn bow_range_debug(
    engine: &Engine,
    assets: &LevelAssets,
    pc_id: engine_element::EntityId,
    target_id: engine_element::EntityId,
) -> serde_json::Value {
    let Some(shooter) = engine.get_entity(pc_id) else {
        return serde_json::json!({"error": "missing_shooter", "pc_id": pc_id});
    };
    let Some(target) = engine.get_entity(target_id) else {
        return serde_json::json!({"error": "missing_target", "target_id": target_id});
    };
    let Some(hand_point) = shooter.compute_hand_point(None) else {
        return serde_json::json!({"error": "missing_shooter_hand_point", "pc_id": pc_id});
    };

    let bow_profile = bow_profile_debug(engine, assets, pc_id);
    let max_range = bow_profile
        .as_ref()
        .and_then(|profile| profile.get("max_range"))
        .and_then(serde_json::Value::as_u64)
        .map(|v| v as f32);
    let range_target_point = if target.is_human() {
        target.compute_belt_point()
    } else {
        Some(target.element_data().position())
    };
    let preview_target_point = if target.is_human() {
        target.compute_belt_point()
    } else if target.is_fx_target() {
        target.compute_target_center()
    } else {
        Some(target.element_data().position())
    };
    let forest_target = !target.is_human() && engine.weather().is_forest_level;
    let range_math = match (range_target_point, max_range) {
        (Some(point), Some(max_range)) => Some(bow_range_math_debug(
            hand_point,
            point,
            max_range,
            forest_target,
        )),
        _ => None,
    };
    let preview_direction = preview_target_point.map(|point| {
        let dx = point.x - shooter.element_data().position().x;
        let dy_raw = point.y - shooter.element_data().position().y;
        let dy_projected =
            bow_debug_ground_y_projected(point) - bow_debug_ground_y_projected(shooter.element_data().position());
        serde_json::json!({
            "source_position_3d": shooter.element_data().position(),
            "preview_target_point": point,
            "dx": dx,
            "dy_raw_game": dy_raw,
            "dy_projected_y_minus_z": dy_projected,
            "iso_sector_raw_game_y": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_raw),
            "rust_iso_sector_projected_y_minus_z": engine_position_interface::vector_to_sector_0_to_15_iso(dx, dy_projected),
            "game_sector_aspect_raw_game_y": game_sector_0_to_15_with_aspect(
                dx,
                dy_raw,
                engine_position_interface::ASPECT_RATIO,
            ),
            "game_sector_aspect_projected_y_minus_z": game_sector_0_to_15_with_aspect(
                dx,
                dy_projected,
                engine_position_interface::ASPECT_RATIO,
            ),
        })
    });

    serde_json::json!({
        "shooter": {
            "id": pc_id,
            "kind": shooter.kind(),
            "position_3d": shooter.element_data().position(),
            "position_map": shooter.element_data().position_map(),
            "hand_point": hand_point,
            "direction": shooter.element_data().direction(),
            "posture": shooter.element_data().posture(),
            "pc_current_action": shooter.pc_data().map(|pc| pc.current_action),
            "actor_action_state": shooter.actor_data().map(|actor| actor.action_state),
        },
        "target": bow_target_points_debug(engine, target_id),
        "bow_profile": bow_profile,
        "forest_target": forest_target,
        "range_math": range_math,
        "preview_direction": preview_direction,
    })
}

pub(crate) fn engine_dump_json(engine: &Engine) -> Result<serde_json::Value, String> {
    crate::json_value::to_json_value(engine).map_err(|e| e.to_string())
}

pub(crate) fn level_assets_json(
    engine: &Engine,
    assets: &LevelAssets,
) -> Result<serde_json::Value, String> {
    let mut root = serde_json::Map::new();
    root.insert("schema".into(), serde_json::json!("level-assets.v1"));
    root.insert(
        "counts".into(),
        serde_json::json!({
            "level_grid": {
                "lines": assets.navigation.level_grid.lines.len(),
                "sectors": assets.navigation.level_grid.sectors.len(),
                "masks": assets.navigation.level_grid.masks.len(),
                "jump_lines": assets.navigation.level_grid.jump_lines.len(),
                "blocks": assets.navigation.level_grid.blocks.len(),
                "layers": assets.navigation.level_grid.layers.len(),
                "level_repulsive_points": assets.navigation.level_grid.level_repulsive_points.len(),
                "shadow_data": assets.navigation.level_grid.shadow_data.len(),
            },
            "pathfinder_graph": {
                "nodes": assets.navigation.pathfinder_graph.nodes.len(),
                "layers": assets.navigation.pathfinder_graph.layers.len(),
                "links": assets.navigation.pathfinder_graph.static_data.links.len(),
                "link_configs": assets.navigation.pathfinder_graph.static_data.link_configs.len(),
                "move_layers": assets.navigation.pathfinder_graph.static_data.move_layers.len(),
                "alternative_move_layers": assets.navigation.pathfinder_graph.static_data.alternative_move_layers.len(),
            },
            "profiles": {
                "characters": assets.profile_manager.characters.len(),
                "soldiers": assets.profile_manager.soldiers.len(),
                "civilians": assets.profile_manager.civilians.len(),
                "hth_weapons": assets.profile_manager.hth_weapons.len(),
                "bows": assets.profile_manager.bows.len(),
                "missions": assets.profile_manager.missions.len(),
            },
            "mission_script_programs": assets.scripts.mission_programs.len(),
            "hiking_paths": assets.navigation.hiking_paths.len(),
            "static_sight_obstacles": assets.environment.static_sight_obstacles.len(),
            "accessory_sprite_prototypes": assets.accessory_sprite_prototypes.len(),
            "water_zones": assets.environment.water_zones.zones.len(),
            "material_sectors": assets.environment.material_sectors.sectors.len(),
            "script_locations": assets.scripts.location_count,
            "script_points": assets.scripts.point_count,
            "script_buildings": assets.scripts.building_count,
            "script_hiking_paths": assets.scripts.hiking_path_count,
        }),
    );
    root.insert(
        "pixel_opacity_attached".into(),
        serde_json::json!(assets.attachments.pixel_opacity.is_some()),
    );
    insert_json(&mut root, "fast_grid_runtime", engine.fast_grid())?;

    let mut asset = serde_json::Map::new();
    insert_json(&mut asset, "sprite_scriptor", &*assets.sprite_scriptor)?;
    insert_json(&mut asset, "level_grid", &*assets.navigation.level_grid)?;
    insert_json(
        &mut asset,
        "pathfinder_graph",
        &*assets.navigation.pathfinder_graph,
    )?;
    insert_json(&mut asset, "hiking_paths", &*assets.navigation.hiking_paths)?;
    insert_json(&mut asset, "profile_manager", &*assets.profile_manager)?;
    insert_json(&mut asset, "bank_signature", &assets.bank_signature)?;
    insert_json(
        &mut asset,
        "mission_script_programs",
        &*assets.scripts.mission_programs,
    )?;
    insert_json(&mut asset, "peasant_firstnames", &assets.peasant_firstnames)?;
    insert_json(&mut asset, "peasant_surnames", &assets.peasant_surnames)?;
    insert_json(
        &mut asset,
        "accessory_sprite_prototypes",
        &assets.accessory_sprite_prototypes,
    )?;
    insert_json(
        &mut asset,
        "exclamation_durations",
        &assets.audio.exclamation_durations(),
    )?;
    insert_json(
        &mut asset,
        "source_durations",
        &assets.audio.source_durations(),
    )?;
    insert_json(
        &mut asset,
        "sound_source_required_ids",
        &assets.audio.sound_source_required_ids,
    )?;
    insert_json(
        &mut asset,
        "patch_entity_handles",
        &assets.entities.patch_animation_entities,
    )?;
    insert_json(
        &mut asset,
        "scroll_entity_ids",
        &assets.entities.scroll_entity_ids,
    )?;
    insert_json(
        &mut asset,
        "all_soldier_entity_ids",
        &assets.entities.soldier_entity_ids,
    )?;
    insert_json(
        &mut asset,
        "soldier_subordinate_ids",
        &assets.entities.soldier_subordinate_ids,
    )?;
    insert_json(&mut asset, "water_zones", &assets.environment.water_zones)?;
    insert_json(
        &mut asset,
        "material_sectors",
        &assets.environment.material_sectors,
    )?;
    insert_json(
        &mut asset,
        "static_sight_obstacles",
        &*assets.environment.static_sight_obstacles,
    )?;
    insert_json(
        &mut asset,
        "script_location_count",
        &assets.scripts.location_count,
    )?;
    insert_json(
        &mut asset,
        "script_point_count",
        &assets.scripts.point_count,
    )?;
    insert_json(
        &mut asset,
        "script_location_positions",
        &assets.scripts.location_positions,
    )?;
    insert_json(
        &mut asset,
        "script_location_layers",
        &assets.scripts.location_layers,
    )?;
    insert_json(
        &mut asset,
        "script_location_sectors",
        &assets.scripts.location_sectors,
    )?;
    insert_json(
        &mut asset,
        "script_building_count",
        &assets.scripts.building_count,
    )?;
    insert_json(
        &mut asset,
        "script_hiking_path_count",
        &assets.scripts.hiking_path_count,
    )?;
    insert_json(
        &mut asset,
        "script_zone_grid_indices",
        &assets.scripts.zone_grid_indices,
    )?;
    root.insert("assets".into(), serde_json::Value::Object(asset));

    Ok(serde_json::Value::Object(root))
}

fn insert_json<T>(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: &T,
) -> Result<(), String>
where
    T: serde::Serialize + ?Sized,
{
    object.insert(
        key.into(),
        crate::json_value::to_json_value(value).map_err(|e| e.to_string())?,
    );
    Ok(())
}

// ──────────────────────────────────────────────────────────────────
// Replay export transport adapter
// ──────────────────────────────────────────────────────────────────

pub(crate) fn frame_console_response_to_json(
    response: engine_api::FrameConsoleResponse,
) -> serde_json::Value {
    use engine_api::FrameConsoleResponse as R;

    match response {
        R::Ok(message) => serde_json::json!({"kind": "ok", "message": message}),
        R::Unknown => serde_json::json!({"kind": "unknown"}),
        R::NotImplemented(command) => {
            serde_json::json!({"kind": "not_implemented", "command": command})
        }
        R::LoadCampaignRequested(path) => serde_json::json!({
            "kind": "host_followup",
            "variant": "LoadCampaignRequested",
            "path": path,
        }),
        R::DeityInvoked => serde_json::json!({
            "kind": "host_followup",
            "variant": "DeityInvoked",
        }),
    }
}

pub(crate) fn snapshot_script(engine: &Engine) -> serde_json::Value {
    let Some(script) = engine.mission_script() else {
        return serde_json::json!({"loaded": false});
    };
    let scb = script.scb();
    let counts = script.instance_counts();
    let classes: Vec<_> = scb
        .classes
        .iter()
        .map(|c| {
            let funcs: Vec<&str> = c.functions.iter().map(|f| f.name.as_str()).collect();
            let members: Vec<&str> = c.member_variables.iter().map(|m| m.name.as_str()).collect();
            serde_json::json!({
                "name": c.class_name,
                "source_filename": c.source_file,
                "functions": funcs,
                "members": members,
                "quad_count": c.quads.len(),
            })
        })
        .collect();
    serde_json::json!({
        "loaded": true,
        "version": scb.version,
        "class_count": classes.len(),
        "actor_instances": counts.actors,
        "zone_instances": counts.zones,
        "target_instances": counts.targets,
        "scroll_instances": counts.scrolls,
        "waypoint_instances": counts.waypoints,
        "classes": classes,
    })
}

pub(crate) fn decompile_script(engine: &Engine, class: Option<&str>) -> serde_json::Value {
    let Some(script) = engine.mission_script() else {
        return serde_json::json!({"error": "no mission script loaded"});
    };
    let scb = script.scb();
    let source = if let Some(name) = class {
        // Single-class mode: rebuild a minimal ScbFile holding just
        // this class so the existing whole-file decompiler can run on
        // it without us reaching into its private per-class entry
        // points.
        let Some(c) = scb.classes.iter().find(|c| c.class_name == name) else {
            return serde_json::json!({"error": format!("class not found: {name}")});
        };
        let scb_one = engine_scb::ScbFile {
            version: scb.version,
            classes: vec![c.clone()],
        };
        assets_decompile::decompile(&scb_one)
    } else {
        assets_decompile::decompile(scb)
    };
    serde_json::json!({"source": source})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_and_script_diagnostics_preserve_wire_shapes_without_mutation() {
        let mut assets = LevelAssets::new();
        let mut engine =
            Engine::new_for_test(800.0, 600.0, Default::default(), &mut assets).unwrap();
        engine.test_set_frame_counter(42);
        let before = engine.encode_native_snapshot();
        assert_eq!(
            snapshot_state(&engine, None),
            serde_json::json!({
                "frame": 42, "map": engine.mission_map_name(), "replay": null
            })
        );
        assert_eq!(
            snapshot_state(
                &engine,
                Some(ReplayStatus {
                    frame: 3,
                    total: 9,
                    paused: true
                })
            ),
            serde_json::json!({"frame": 42, "map": engine.mission_map_name(),
                "replay": {"frame": 3, "total": 9, "paused": true}})
        );
        assert_eq!(
            snapshot_script(&engine),
            serde_json::json!({"loaded": false})
        );
        assert_eq!(
            decompile_script(&engine, None),
            serde_json::json!({"error": "no mission script loaded"})
        );
        assert_eq!(engine.encode_native_snapshot(), before);
    }

    #[test]
    fn console_response_keeps_existing_wire_text_and_variants() {
        use engine_api::FrameConsoleResponse as R;
        assert_eq!(
            frame_console_response_to_json(R::Unknown),
            serde_json::json!({"kind": "unknown"})
        );
        assert_eq!(
            frame_console_response_to_json(R::Ok("done".into())),
            serde_json::json!({"kind": "ok", "message": "done"})
        );
        assert_eq!(
            frame_console_response_to_json(R::NotImplemented("X".into())),
            serde_json::json!({"kind": "not_implemented", "command": "X"})
        );
        assert_eq!(
            frame_console_response_to_json(R::DeityInvoked),
            serde_json::json!({"kind": "host_followup", "variant": "DeityInvoked"})
        );
    }
}
