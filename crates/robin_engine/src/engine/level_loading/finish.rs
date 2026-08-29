//! Mobile-element construction and final mission-domain attachment stages.

use super::super::scroll_reveal::ScrollStatus;
use super::*;

impl EngineInner {
    pub(super) fn spawn_mobile_elements_stage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &mut LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
    ) -> Result<(), EngineError> {
        let bank_signature = assets.bank_signature;
        assets.entities.mobile_element_count = loaded.mission.mobile_elements.len();
        // Spawn the masked child sprites owned by mission mobile elements.
        // Released data contains only the five chariot profiles. The mobile
        // master is simulation-only, matching C++: only its child
        // RHElementFXMasked objects enter the general element/render array.
        for (mobile_index, raw_mobile) in loaded.mission.mobile_elements.iter().enumerate() {
            if raw_mobile.sprites.is_empty() {
                return Err(EngineError::MissionLevelStage {
                    stage: "mobile elements",
                    reason: format!(
                        "mobile {mobile_index} has no first masked child for its Hourglass owner boundary"
                    ),
                });
            }
            let mobile_index_u16 =
                u16::try_from(mobile_index).map_err(|_| EngineError::MissionLevelStage {
                    stage: "mobile elements",
                    reason: format!("mobile index {mobile_index} does not fit in u16"),
                })?;
            let path = assets
                .hiking_paths
                .get(usize::from(raw_mobile.path_index))
                .ok_or_else(|| EngineError::MissionLevelStage {
                    stage: "mobile elements",
                    reason: format!(
                        "mobile {mobile_index} references missing hiking path {}",
                        raw_mobile.path_index
                    ),
                })?
                .clone();
            let start_waypoint = path
                .waypoints
                .get(raw_mobile.start_waypoint as usize)
                .ok_or_else(|| EngineError::MissionLevelStage {
                    stage: "mobile elements",
                    reason: format!(
                        "mobile {mobile_index} references missing start waypoint {}",
                        raw_mobile.start_waypoint
                    ),
                })?;
            let start = MapPoint::new(start_waypoint.x as f32, start_waypoint.y as f32);
            let mut sprite_ids = Vec::with_capacity(raw_mobile.sprites.len());

            for raw in &raw_mobile.sprites {
                let fname = &raw.sprite.frame_profile_name;
                let profile = &raw.sprite.profile_name;
                let mut sprite = crate::sprite::Sprite::default();
                let path = crate::sprite_script::SpriteScriptor::resolve_rhs_path(
                    crate::sprite_script::FrameKind::Animation,
                    "Data/Animations",
                    fname,
                    Some(crate::engine::Ambiance::Day.to_sprite_ambiance()),
                )
                .map_err(|e| EngineError::MissionLevelStage {
                    stage: "mobile elements",
                    reason: format!(
                        "failed to resolve mobile {mobile_index} sprite '{fname}': {e}"
                    ),
                })?;
                let cache_key = format!("{fname}/{profile}");
                let info = assets
                    .sprite_scriptor_mut()
                    .load(
                        &path,
                        profile,
                        &cache_key,
                        crate::sprite_script::FrameKind::Animation,
                        |file| {
                            let mut sig = 0u32;
                            file.serialize_u32(&mut sig)
                                .map_err(|e| format!("read signature: {e}"))?;
                            if sig != bank_signature {
                                return Err(format!(
                                    "bank signature mismatch: file {sig:#x} != bank {bank_signature:#x}"
                                ));
                            }
                            Ok(())
                        },
                    )
                    .map_err(|e| EngineError::MissionLevelStage {
                        stage: "mobile elements",
                        reason: format!(
                            "failed to load mobile {mobile_index} sprite '{fname}' profile '{profile}': {e}"
                        ),
                    })?;
                sprite.scripts = info.scripts.clone();
                sprite.conversion = info.conversion.clone();
                sprite.center = info.center;
                sprite.current_width = info.size.x as u16;
                sprite.current_height = info.size.y as u16;
                sprite.frame_profile_name = fname.clone();
                sprite.profile_cache_key = cache_key;
                sprite.apply_placement(
                    mobile_sprite_map_position(
                        raw.sprite.position_x,
                        raw.sprite.position_y,
                        sprite.center,
                        start,
                    ),
                    start_waypoint.level,
                    crate::position_interface::SectorHandle::new(start_waypoint.sector),
                    0,
                    crate::element::GameMaterial::default(),
                    None,
                    None,
                );
                sprite
                    .position_iface
                    .set_move_box(crate::coordinates::MoveBox::from_corners(
                        crate::coordinates::MapVec::new(-50.0, -50.0),
                        crate::coordinates::MapVec::new(50.0, 50.0),
                    ));
                if !sprite.has_animation(crate::order::OrderType::WaitingUprightBored) {
                    return Err(EngineError::MissionLevelStage {
                        stage: "mobile elements",
                        reason: format!(
                            "mobile {mobile_index} sprite '{fname}' lacks WAITING_UPRIGHT_BORED"
                        ),
                    });
                }
                sprite.force_animation(crate::order::OrderType::WaitingUprightBored, 0);

                let entity = Entity::Fx(crate::element::ElementFx {
                    element: crate::element::ElementData {
                        kind: crate::element::ElementKind::Fx,
                        active: raw.active,
                        posture: crate::element::Posture::Upright,
                        sprite,
                        ..Default::default()
                    },
                    fx: crate::element::FxData {
                        restore_background: false,
                        force_display: raw.force_display,
                        animation: crate::order::OrderType::WaitingUprightBored,
                        display_polyline: raw
                            .display_polyline
                            .iter()
                            // RHElementFX reads this as absolute map geometry.
                            // The mobile master translates the sprite, but never
                            // its display-order polyline.
                            .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                            .collect(),
                        patch_index: None,
                        mobile_index: Some(mobile_index_u16),
                        animation_speed: 1.0,
                        rendering_properties: if raw.blit_type != 0 {
                            crate::element::RenderingProperties::NeedShadow
                        } else {
                            crate::element::RenderingProperties::Blocky
                        },
                    },
                });
                sprite_ids.push(self.add_entity(entity));
            }

            let mobile = crate::mobile::MobileElement::from_raw(sim, raw_mobile, &path, sprite_ids)
                .map_err(|e| EngineError::MissionLevelStage {
                    stage: "mobile elements",
                    reason: format!("failed to initialize mobile {mobile_index}: {e}"),
                })?;
            let active = mobile.active;
            let animation_speed = mobile.animation_speed();
            for &sprite_id in &mobile.sprite_ids {
                let fx = self
                    .world
                    .entities
                    .get_mut(sprite_id)
                    .and_then(crate::element::Entity::as_fx_mut)
                    .ok_or_else(|| EngineError::MissionLevelStage {
                        stage: "mobile elements",
                        reason: format!(
                            "fresh mobile {mobile_index} sprite entity {sprite_id:?} is missing"
                        ),
                    })?;
                fx.element.active = active;
                fx.fx.animation_speed = animation_speed;
            }
            self.world.mobile_elements.push(mobile);
        }
        if !self.world.mobile_elements.is_empty() {
            tracing::info!(
                "Spawned {} mobile chariot element(s)",
                self.world.mobile_elements.len()
            );
        }

        Ok(())
    }

    pub(super) fn finish_mission_identity_stage(
        &mut self,
        loaded: &crate::level_data::LoadedLevel,
        mission_name: &str,
        proto_level_name: &str,
    ) {
        // Remaining load-time subsystems (motion grid, sight obstacles,
        // background map, patrol paths, tactical info, and tenants are loaded
        // from other code paths.

        // Set night color based on ambiance — pack via draw_manager.
        let (r, g, b) = self.world.weather.ambiance.night_color_rgb();
        let _ = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
        // EngineInner format is always RGB565. Host can derive 15-bit packing
        // at render time if its display needs it.
        self.world.weather.night_color = robin_util::color::rgb565(r, g, b);

        tracing::info!(
            "EngineInner: initialized from mission '{}' / proto '{}' — \
             {} soldiers, {} civilians, {} targets, {} bonuses, {} beam-mes",
            mission_name,
            proto_level_name,
            loaded.mission.soldiers.len(),
            loaded.mission.civilians.len(),
            loaded.mission.targets.len(),
            loaded.mission.bonuses.len(),
            loaded.mission.beam_mes.len(),
        );
    }

    pub(super) fn load_mission_script_stage(
        &mut self,
        assets: &mut LevelAssets,
        mission_name: &str,
        level_directory: &str,
        script_enabled: bool,
        force_visible_scroll_ids: Vec<crate::element::EntityId>,
    ) {
        // `RHEngine::Initialize` binds StartUp only when `bScript` is set.
        // In Rust, constructing MissionScript performs that bind, so keep the
        // whole VM absent in --no-script mode while still building every
        // authored non-script domain below.
        self.scripts.mission = None;
        if script_enabled {
            let scb_path = format!("{}/{}.scb", level_directory, mission_name);
            self.load_mission_script(assets, std::path::Path::new(&scb_path));
        } else {
            tracing::info!("Mission scripting disabled; skipping mission VM and StartUp binding");
        }

        // Flush `force_visible` scroll visibility, calling SetStatus
        // (Visible) for each captured scroll.  Route through
        // `set_scroll_status` so the scroll's `custom_minimap_dot` is
        // refreshed alongside the status.
        if !force_visible_scroll_ids.is_empty() {
            let count = force_visible_scroll_ids.len();
            for eid in force_visible_scroll_ids {
                self.set_scroll_status(eid, ScrollStatus::Visible);
            }
            tracing::info!("Applied force_visible to {count} scroll(s)");
        }
    }

    pub(super) fn install_reinforcement_doors_stage(
        &mut self,
        assets: &LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
    ) {
        // Install mission-defined reinforcement doors: construct one
        // `Door(Reinforcement)` per REIN entry and insert it into the
        // gate-graph table.  The Rust port keeps a single
        // `self.script_domains.interactables.doors` list plus a filtered
        // `ai_global.reinforcement_doors` cache built below.  This has
        // to run after the authored door/lift stages so that
        // `self.script_domains.interactables.doors` exists, and before the cache filter so
        // `ai_global.reinforcement_doors` picks up these entries
        // alongside any proto-level doors with
        // `door_type == Reinforcement`.
        if let Some(ref tactic) = loaded.mission.tactic_data
            && !tactic.reinforcement_points.is_empty()
        {
            let map_bbox = self.world.fast_grid.level.map_bbox;
            let special_layer = self.world.fast_grid.level.special_layer;
            let sector_out_of_map = crate::sector::SectorNumber::new(-1);
            // Original retains one real RHSectorMotionArea pointer for the
            // outside endpoint of every reinforcement gate. Resolve that
            // exact arena object, excluding unrelated -1 click/shadow
            // sectors, and reject an absent or duplicate candidate.
            let mut out_of_map_matches = self
                .world
                .fast_grid
                .level
                .sectors
                .iter()
                .enumerate()
                .filter(|(_, sector)| {
                    sector.sector_number == sector_out_of_map
                        && sector.sector_type.is_motion()
                        && sector.sector_type.is_area()
                })
                .map(|(index, _)| {
                    crate::fast_find_grid::SectorIndex::new(
                        u32::try_from(index).expect("out-of-map runtime sector index exceeds u32"),
                    )
                    .expect("out-of-map runtime sector index equals null sentinel")
                });
            let sector_out_index = out_of_map_matches
                .next()
                .expect("reinforcement doors require the explicit out-of-map motion area");
            assert!(
                out_of_map_matches.next().is_none(),
                "multiple out-of-map motion areas make reinforcement endpoint identity ambiguous"
            );
            let mut installed = 0usize;
            for raw in &tactic.reinforcement_points {
                // `uwSector` in the REIN chunk is an Original sparse
                // marraySectors slot, not the displayed sector number.
                // RHFastFindGrid resolves it with GetSector(uwSector) and
                // stores that exact RHSector pointer on the door.
                let (sector_in, sector_in_index) =
                    Self::resolve_sparse_position_sector(assets, raw.sector);
                let runtime_sector = self
                    .world
                    .fast_grid
                    .level
                    .sectors
                    .get(usize::from(sector_in_index))
                    .unwrap_or_else(|| {
                        panic!(
                            "reinforcement sparse sector slot {} resolves outside the runtime arena",
                            raw.sector
                        )
                    });
                assert_eq!(
                    runtime_sector.sector_number, sector_in,
                    "reinforcement sparse sector slot {} has conflicting public/runtime identity",
                    raw.sector
                );
                assert!(
                    runtime_sector.sector_type.is_motion() && runtime_sector.sector_type.is_area(),
                    "reinforcement sparse sector slot {} does not resolve to a motion-area sector",
                    raw.sector
                );

                let inside = MapPoint::new(raw.x as f32, raw.y as f32);
                let (border, outside) = crate::natives::compute_border_point_bbox(
                    map_bbox,
                    (inside.x, inside.y),
                    raw.direction as i16,
                );
                // RHFastFindGrid::InitializeReinforcementPointsFromMissionStream
                // narrows both computed points through SWORD before passing
                // them to RHDoor::SetPoint{Mid,Out}.  Diagonal exits normally
                // have fractional intersections/steps, so retaining those
                // fractions shifts every later GetPointOut movement goal.
                let border = MapPoint::new(border.0 as i16 as f32, border.1 as i16 as f32);
                let outside = MapPoint::new(outside.0 as i16 as f32, outside.1 as i16 as f32);

                // Reinforcement doors get 4× WalkingUpright actions
                // by default.
                let (act_d1, act_d2, act_i1, act_i2) = crate::gate::Door::default_actions_for_type(
                    crate::gate::DoorType::Reinforcement,
                );

                self.script_domains
                    .interactables
                    .doors
                    .push(crate::gate::Door {
                        gate_type: crate::gate::GateType::Door,
                        door_type: crate::gate::DoorType::Reinforcement,
                        point_in: inside,
                        point_mid: border,
                        point_out: outside,
                        layer_in: raw.layer,
                        layer_out: special_layer,
                        sector_in,
                        sector_out: sector_out_of_map,
                        sector_in_index: Some(sector_in_index),
                        sector_out_index: Some(sector_out_index),
                        action_direct_1: act_d1,
                        action_direct_2: act_d2,
                        action_indirect_1: act_i1,
                        action_indirect_2: act_i2,
                        ..Default::default()
                    });
                // AdaptPoints is a no-op for `Reinforcement` doors
                // (only BuildingTrap / LiftHigh[Crenel] on wall lifts
                // shift `point_in`), but penalty still has to be
                // computed so A* gate-graph routing through these
                // out-of-map doors has a finite cost.
                if let Some(door) = self.script_domains.interactables.doors.last_mut() {
                    door.compute_door_penalty();
                }
                installed += 1;
            }
            // Rebuild gate-link connectivity so the new reinforcement
            // doors are routed through by `find_path_gates`.
            if installed > 0 {
                crate::gate::build_gate_links(&mut self.script_domains.interactables.doors);
            }
            if installed > 0 {
                tracing::debug!(
                    "Installed {installed} mission-REIN reinforcement doors into self.script_domains.interactables.doors",
                );
            }
        }
    }
}
