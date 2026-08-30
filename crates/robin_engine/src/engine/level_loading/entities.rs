//! Entity construction stages in original proto/mission stream order.

use super::*;

/// Populate the behavior traits consumed by the established malignity battle
/// state machine. `weapon_*`, shooting, and endurance come from the physical
/// actor profile, allowing PCs to keep their actual character combat data
/// while borrowing only decision personality from a soldier profile.
#[allow(clippy::too_many_arguments)]
fn configure_enemy_ai_profile(
    ai: &mut crate::ai_enemy::EnemyAi,
    behavior: &crate::profiles::SoldierProfile,
    hth_weapon_id: u32,
    shooting_weapon_id: u32,
    shooting: u16,
    endurance: u16,
    profiles: &crate::profiles::ProfileManager,
    profile_number: u32,
    ale_reliability_eligible: bool,
) {
    ai.soldier_profile_courage = behavior.courage;
    ai.soldier_profile_iq = behavior.intelligence;
    ai.soldier_profile_shooting = shooting;
    ai.soldier_profile_pride = behavior.pride;
    ai.soldier_profile_rank = behavior.rank;
    ai.soldier_profile_initiative = behavior.initiative;
    ai.soldier_profile_beer = behavior.beer;
    ai.ale_reliable_distraction = ale_reliability_eligible;
    ai.soldier_profile_money = behavior.money;
    ai.soldier_profile_apple = behavior.apple;
    ai.soldier_profile_whistle = behavior.whistle;
    ai.soldier_profile_duty = behavior.duty;
    ai.soldier_profile_endurance = endurance;
    ai.is_vip = false;
    ai.hth_weapon_id = hth_weapon_id;
    ai.is_archer_unit = if shooting_weapon_id == 0 {
        false
    } else {
        profiles.get_bow(shooting_weapon_id).unwrap_or_else(|| {
            panic!(
                "combat profile {profile_number} requires missing bow profile {shooting_weapon_id}"
            )
        });
        true
    };
    if let Some(weapon) = profiles.get_hth_weapon(hth_weapon_id) {
        ai.sword_range = weapon.distance[crate::weapons::WeaponDistance::Default as usize];
        ai.sword_is_charge_weapon = weapon.charge;
    }
}

impl EngineInner {
    pub(super) fn spawn_proto_entities_stage(
        &mut self,
        assets: &mut LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
    ) {
        // ── Entity spawn order ──
        //
        // Elements are added to the script-elements array in the order
        // they appear in the proto + mission files. Script handles carry
        // tagged 0-based indices into this array, so getting the order
        // right is essential — otherwise script natives like `Deactivate(GetActorScript(N))`
        // hit the wrong entity, leaving initially-hidden enemies/scrolls/FX
        // visible at mission start.
        //
        // Load order:
        //   1. Proto animation/patch FX in their source chunk order
        //   2. Mission PATCH_2 FX  (shipped files place it before ELEMENT)
        //   3. Mission ELEMENT chunk sub-chunks in file order:
        //        BETE animals (skipped) → GOOD beam-mes (no script entry) →
        //        CIVI civilians → PRIS PCs-to-rescue → EVIL soldiers → TGET targets
        //   4. BONU bonuses
        //   5. PARC scrolls
        //   6. GUYS tenants  (not ported as entities — see note below;
        //                      `InitOccupant` consumes GUYS already)
        //   7. PCs from beam-mes  (one slot per beam-me, NULL if unfilled)
        //
        // Some types (PRIS, GUYS) are not yet spawned; we push None placeholders
        // for them so the script-position-to-entity-index mapping stays aligned.

        let mut proto_patch_handles = None;
        let mut animations_spawned = false;
        let mut patches_spawned = false;
        let mut proto_element_chunks = loaded.proto.element_chunk_order.clone();
        if proto_element_chunks.is_empty() {
            // Backward-compatible fallback for programmatically constructed
            // levels and old serialized test fixtures.
            proto_element_chunks.extend([
                crate::level_data::ProtoElementChunk::Animation,
                crate::level_data::ProtoElementChunk::Patch,
            ]);
        }

        for chunk in proto_element_chunks {
            match chunk {
                crate::level_data::ProtoElementChunk::Animation if !animations_spawned => {
                    spawn_proto_animation_fx_entities(self, assets, &loaded.proto.animations);
                    animations_spawned = true;
                }
                crate::level_data::ProtoElementChunk::Patch if !patches_spawned => {
                    proto_patch_handles = Some(spawn_patch_fx_entities(
                        self,
                        assets,
                        &loaded.proto.patches,
                        0,
                    ));
                    patches_spawned = true;
                }
                _ => {}
            }
        }

        // A malformed/custom file could omit one of the chunks while a test
        // constructs the corresponding vector directly. Do not silently drop
        // those entities.
        if !animations_spawned && !loaded.proto.animations.is_empty() {
            spawn_proto_animation_fx_entities(self, assets, &loaded.proto.animations);
        }
        if !patches_spawned && !loaded.proto.patches.is_empty() {
            proto_patch_handles = Some(spawn_patch_fx_entities(
                self,
                assets,
                &loaded.proto.patches,
                0,
            ));
        }

        assets.entities.patch_animation_entities = std::sync::Arc::new(
            proto_patch_handles.unwrap_or_else(|| vec![None; loaded.proto.patches.len()]),
        );
        tracing::info!(
            "Spawned {} proto animations and {} proto patch FX entities in source order",
            loaded.proto.animations.len(),
            loaded.proto.patches.len(),
        );
    }

    pub(super) fn spawn_civilians_and_rescue_pcs_stage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &mut LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
        config: SimConfig,
    ) -> Result<(), EngineError> {
        let profiles = assets.profile_manager.clone();
        let char_base_dir = "Data/Characters";
        let bank_signature = assets.bank_signature;
        let highlander2 = config.highlander2;
        // Mission PATCH_2 precedes ELEMENT in shipped mission streams. The
        // patch animations therefore belong here in the flat script-element
        // array, between proto FX and mission actors (RHengine.cpp's mission
        // chunk loop calls InitializePatchFromProtoStream immediately).
        let mission_patch_handles = spawn_patch_fx_entities(
            self,
            assets,
            &loaded.mission.mission_patches,
            loaded.proto.patches.len(),
        );
        std::sync::Arc::make_mut(&mut assets.entities.patch_animation_entities)
            .extend(mission_patch_handles);
        tracing::info!(
            "Spawned {} mission patch FX entities",
            loaded.mission.mission_patches.len(),
        );

        // Every freshly-constructed NPC (soldier or civilian, regardless
        // of camp) seeds `invulnerable` from the session's highlander2
        // construction mode.

        // Spawn civilians (CIVI sub-chunk, before soldiers in the ELEMENT chunk).
        // The index is Original's NPC-only construction register.
        // TODO(original-parity): retain the next register number at engine
        // scope if missions can be reloaded in-process or scripts gain
        // dynamic NPC construction; Original's global counter never resets.
        for (npc_register_number, raw) in loaded.mission.civilians.iter().enumerate() {
            let mut sprite = crate::sprite::Sprite::default();
            let frame_kind = if self.world.weather.is_forest_level {
                crate::sprite_script::FrameKind::Character
            } else {
                crate::sprite_script::FrameKind::CharacterBlipped
            };
            let civ_profile = profiles.get_civilian(raw.profile_number).ok_or_else(|| {
                EngineError::ProfileSpriteLoadFailed {
                    kind: "civilian",
                    profile_id: raw.profile_number,
                    reason: "profile is missing from the loaded CPF".to_owned(),
                }
            })?;

            if let Err(e) = sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                frame_kind,
                char_base_dir,
                &civ_profile.filename,
                &civ_profile.profile_name,
                bank_signature,
                Some(self.world.weather.ambiance.to_sprite_ambiance()),
            ) {
                tracing::error!(
                    "Failed to load sprite for civilian profile {}: {e}",
                    raw.profile_number,
                );
            }

            let cached_camp = raw
                .allegiance
                .map(crate::element::Camp::from_allegiance_id)
                .unwrap_or_else(|| {
                    if civ_profile.attitude == crate::profiles::Attitude::Hostile {
                        crate::element::Camp::Lacklandists
                    } else {
                        crate::element::Camp::Royalists
                    }
                });
            let cached_civilian_type = civ_profile.civilian_type;

            let mut ai = crate::ai_friendly::FriendlyAi::default();
            ai.base.path_id = crate::ai::PathId::new(raw.path_id);
            ai.base.initial_action = raw.action;

            // Civilians hardcode pathfinder index 0 and take the move box
            // from the grid's slot 0.
            let civ_pathfinder = crate::position_interface::PathfinderIndex::new(0).unwrap();
            let civ_half_diag = self
                .world
                .fast_grid
                .try_move_box_half_diagonal(0)
                .unwrap_or_else(|| panic!("civilian pathfinder move-box slot 0 is missing"));
            sprite.position_iface.configure_for_actor(
                civ_pathfinder,
                civ_half_diag,
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
            );
            sprite.apply_placement(
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
                raw.layer,
                Some(Self::resolve_sparse_position_handle(assets, raw.sector)),
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::from_u32(raw.material),
                crate::position_interface::ObstacleHandle::from_serialized_pointer(
                    raw.obstacle_index,
                ),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::from_serialized_pointer(
                        raw.obstacle_index,
                    ),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );
            prime_mission_start_sprite(
                &mut sprite,
                raw.action,
                raw.direction,
                &format!("civilian profile {}", raw.profile_number),
            );
            let entity = Entity::Civilian(crate::element::ActorCivilian {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ActorCivilian,
                    // Civilians are also blipped on non-forest levels,
                    // same as soldiers.
                    blipped: !self.world.weather.is_forest_level,
                    posture: crate::element::Posture::Upright,
                    sprite,
                    ..Default::default()
                },
                actor: crate::element::ActorData {
                    script_class: raw.script_class.clone().unwrap_or_default(),
                    ..Default::default()
                },
                human: crate::element::HumanData {
                    invulnerable: highlander2,
                    ..Default::default()
                },
                npc: crate::element::NpcData {
                    ai: crate::element::AiActorData {
                        register_number: u16::try_from(npc_register_number)
                            .expect("civilian NPC register number exceeds u16"),
                        money: raw.money,
                        ai_brain: crate::element::AiBrain::Friendly(Box::new(ai)),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                civilian: crate::element::CivilianData {
                    civilian_profile_index: crate::profiles::CivilianProfileIdx(raw.profile_number),
                    cached_camp,
                    cached_civilian_type,
                    beggar_scroll_sets: raw.beggar_scroll_sets.clone(),
                    ..Default::default()
                },
            });
            let eid = self.add_entity(entity);
            // FriendlyAi is constructed before the entity is inserted, so
            // its Original `mpMe` equivalent cannot be initialized until the
            // stable runtime handle is known. Soldiers perform the same
            // backfill below; civilians need it as well because their alert,
            // panic, and owner-local callback paths read `base.me`.
            if let Some(e) = self.world.entities.get_mut(eid)
                && let Some(ai) = e.ai_controller_mut()
            {
                ai.me = eid.index();
                ai.owner_entity_id = Some(eid);
            }
            // Civilians contribute to the same level-money pool the
            // debriefing screen surfaces, even though the pool is named
            // "soldier_money".
            self.mission_domain.mission_stat.soldier_money += raw.money;
        }

        // PRIS sub-chunk: PCs to rescue.
        // These are full PC actors that the player must rescue during
        // the mission; spawned playable=false so they're NPCs until
        // rescued.
        for raw in &loaded.mission.pcs_to_rescue {
            let char_profile = profiles.get_character(raw.profile_index).ok_or_else(|| {
                EngineError::ProfileSpriteLoadFailed {
                    kind: "rescue PC",
                    profile_id: raw.profile_index,
                    reason: "profile is missing from the loaded CPF".to_owned(),
                }
            })?;
            let mut sprite = crate::sprite::Sprite::default();
            if let Err(e) = sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                crate::sprite_script::FrameKind::Character,
                char_base_dir,
                &char_profile.filename,
                &char_profile.profile_name,
                bank_signature,
                Some(self.world.weather.ambiance.to_sprite_ambiance()),
            ) {
                tracing::error!(
                    "Failed to load sprite for rescue PC profile {}: {e}",
                    raw.profile_index,
                );
            }

            let kind = crate::character_kind::CharacterKind::from_profile(
                &char_profile.filename,
                &char_profile.profile_name,
            );
            let is_robin = kind.is_some_and(|k| k.is_robin());
            let (has_lockpick, has_climb, has_jump) =
                crate::element::PcData::movement_auth_from_profile(char_profile);

            // Set the sprite's move box + pathfinder index from the
            // character profile right after `LoadFrameInfo`, the same
            // way the beam-me path does, so anti-collision has a valid
            // bbox on the very first tick.  Falls back cleanly when the
            // profile lookup missed — `configure_for_actor` is a no-op
            // with a zero half-diagonal in that case.
            let pc_pathfinder_idx = char_profile.pathfinder_index;
            let pc_half_diag = self
                .world
                .fast_grid
                .try_move_box_half_diagonal(char_profile.pathfinder_index as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "rescue PC profile {} references missing pathfinder slot {}",
                        raw.profile_index, pc_pathfinder_idx
                    )
                });
            let initial_position = MapPoint::new(raw.position_x as f32, raw.position_y as f32);
            sprite.position_iface.configure_for_actor(
                crate::position_interface::PathfinderIndex::new(u16::from(pc_pathfinder_idx))
                    .expect("u8 rescue-PC pathfinder index cannot equal 0xffff"),
                pc_half_diag,
                initial_position,
            );
            // The sector must be both motion and area; warn instead of
            // asserting so a corrupt mission file still loads.
            let sector_motion_area = self
                .world
                .fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(raw.sector as i16))
                .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
                .map(|gs| gs.sector_type.is_motion() && gs.sector_type.is_area())
                .unwrap_or(false);
            if !sector_motion_area {
                tracing::warn!(
                    "Rescue PC profile {} at ({},{}) sector {} is not a motion+area sector",
                    raw.profile_index,
                    raw.position_x,
                    raw.position_y,
                    raw.sector,
                );
            }
            // Validate the rescue PC's obstacle: it must be a projection
            // area and the map position must be inside the obstacle's
            // screen box.  Warn instead of aborting so a corrupt mission
            // stream still loads.
            if raw.obstacle_index != 0xFFFF {
                match assets
                    .static_sight_obstacles
                    .get(raw.obstacle_index as usize)
                {
                    None => tracing::warn!(
                        "Rescue PC profile {} references out-of-range obstacle {}",
                        raw.profile_index,
                        raw.obstacle_index,
                    ),
                    Some(obs) => {
                        if !obs.is_projection_area() {
                            tracing::warn!(
                                "Rescue PC profile {} at ({},{}) not lying on projection area (obstacle {})",
                                raw.profile_index,
                                raw.position_x,
                                raw.position_y,
                                raw.obstacle_index,
                            );
                        }
                        if !obs.box_projection.contains_point(initial_position) {
                            tracing::warn!(
                                "Rescue PC profile {} at ({},{}) map position not lying in projection area screen box (obstacle {})",
                                raw.profile_index,
                                raw.position_x,
                                raw.position_y,
                                raw.obstacle_index,
                            );
                        }
                    }
                }
            }
            // `sprite.center` (loaded from the sprite info via
            // `load_frame_info` above) is the authoritative C++ sprite
            // anchor used by rendering and gameplay hotspot lookups.
            sprite.apply_placement(
                initial_position,
                raw.layer,
                Some(Self::resolve_sparse_position_handle(assets, raw.sector)),
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::from_u32(raw.material),
                crate::position_interface::ObstacleHandle::from_serialized_pointer(
                    raw.obstacle_index,
                ),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::from_serialized_pointer(
                        raw.obstacle_index,
                    ),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );
            prime_mission_start_sprite(
                &mut sprite,
                raw.action,
                raw.direction,
                &format!("rescue PC profile {}", raw.profile_index),
            );
            // Seed old position fields so `is_moving()` is false on the
            // first post-spawn tick.
            let current_position = sprite.position_iface.get_position();
            sprite.position_iface.set_old_position(current_position);
            sprite.position_iface.set_old_map_position(initial_position);
            // Display order is computed by the host-side
            // `compute_display_order` pass (`engine/display_state.rs`)
            // that runs before render and input hit-test on every tick
            // — the rescue PC is picked up automatically before any
            // consumer reads display order.

            // Map the PRIS chunk's authored animation to the starting
            // (posture, action_state) pair.  Without this, rescue PCs
            // always spawn in UPRIGHT/WAITING regardless of the
            // level-authored action.
            let (initial_posture, initial_action_state) = map_pc_initial_action(raw.action);

            // Create or reuse the campaign `PcDescription` before
            // spawning the entity so PcData can point at the same
            // dynamic status block the C++ PC's mpStatus referenced.
            //   * Non-VIP   → always a fresh description with full pockets.
            //   * VIP, none → same as non-VIP.
            //   * VIP, dup  → reuse the existing description and heal to
            //                 `LIFEPOINTS_PC` (guest-star return).
            // Previously this was deferred to `rescue_pc_by_profile_name`,
            // which left the reuse + heal branch unreachable for guest
            // stars and meant rescue PCs were not flagged `instanced` at
            // spawn.
            let profile_idx = crate::profiles::CharacterProfileIdx(raw.profile_index);
            let difficulty = config.difficulty;
            let char_idx = {
                let profile = char_profile;
                let campaign = &mut self.mission_domain.campaign;
                let existing = campaign.get_character_by_profile(profile_idx);
                let char_idx = match (profile.vip, existing) {
                    (true, Some(idx)) => {
                        if let Some(desc) = campaign.characters.get_mut(idx) {
                            desc.status.life_points = crate::pc_status::LIFEPOINTS_PC;
                        }
                        idx
                    }
                    _ => {
                        let mut status =
                            crate::pc_status::PcStatus::from_profile(profile, true, difficulty);
                        if profile.vip {
                            // Original maps the seven fixed French profile
                            // identities to localized menu IDs 144..150 and
                            // performs no random draws. Mod-added VIP profiles
                            // have no retail menu-text slot, so their stable
                            // authored profile name is the only valid name.
                            status.name = if let Some(name) =
                                assets.fixed_vip_names.get(&profile.profile_name)
                            {
                                name.clone()
                            } else if kind.is_none() {
                                // TODO(mod-localization): allow profile patches
                                // to provide locale-specific display names.
                                if profile.display_name.is_empty() {
                                    profile.profile_name.clone()
                                } else {
                                    profile.display_name.clone()
                                }
                            } else {
                                return Err(EngineError::MissionLevelStage {
                                    stage: "rescue PCs",
                                    reason: format!(
                                        "missing localized fixed VIP name for profile {:?}",
                                        profile.profile_name
                                    ),
                                });
                            };
                        } else {
                            const ORIGINAL_NAME_COUNT: usize = 22;
                            const MAX_ATTEMPTS: usize = 10;
                            if assets.peasant_firstnames.len() != ORIGINAL_NAME_COUNT
                                || assets.peasant_surnames.len() != ORIGINAL_NAME_COUNT
                            {
                                return Err(EngineError::MissionLevelStage {
                                    stage: "rescue PCs",
                                    reason: format!(
                                        "Original rescue-PC name generation requires 22 firstnames \
                                         and 22 surnames, loaded {}/{}",
                                        assets.peasant_firstnames.len(),
                                        assets.peasant_surnames.len(),
                                    ),
                                });
                            }

                            let mut generated = None;
                            for _ in 0..MAX_ATTEMPTS {
                                // RHPCStatus::GenerateName calls rand() once
                                // for each half, in this exact order.
                                let first_idx = crate::sim_rng::usize(
                                    sim,
                                    crate::sim_rng::RngSite::RescuePcFirstName,
                                    0..ORIGINAL_NAME_COUNT,
                                );
                                let surname_idx = crate::sim_rng::usize(
                                    sim,
                                    crate::sim_rng::RngSite::RescuePcSurname,
                                    0..ORIGINAL_NAME_COUNT,
                                );
                                let name = format!(
                                    "{} {}",
                                    assets.peasant_firstnames[first_idx],
                                    assets.peasant_surnames[surname_idx],
                                );
                                if !campaign.is_peasant_name_registered(&name) {
                                    // Register immediately: every later PRIS
                                    // constructor checks the names accepted by
                                    // all earlier constructors in source order.
                                    campaign.register_peasant_name(name.clone());
                                    generated = Some(name);
                                    break;
                                }
                            }
                            status.name = generated.unwrap_or_else(|| "Misteryman".to_owned());
                        }
                        let desc = crate::campaign::PcDescription {
                            character_profile_idx: Some(profile_idx),
                            instanced: false,
                            status,
                        };
                        campaign.add_to_characters(desc, &profiles)
                    }
                };
                if let Some(desc) = campaign.characters.get_mut(char_idx) {
                    desc.instanced = true;
                }
                char_idx
            };
            let list_index =
                u8::try_from(char_idx).map_err(|_| EngineError::MissionLevelStage {
                    stage: "rescue PCs",
                    reason: format!(
                        "campaign character index {char_idx} does not fit in PcData::list_index"
                    ),
                })?;

            let actor_ai = if raw.decision_policy == crate::human_control::DecisionPolicy::EnemyAi {
                let behavior_profile_id =
                    raw.ai_profile
                        .as_deref()
                        .ok_or_else(|| EngineError::MissionLevelStage {
                            stage: "rescue PCs",
                            reason: format!(
                                "enemy-AI hero profile {} requires a readable ai_profile",
                                raw.profile_index
                            ),
                        })?;
                let behavior_profile_index = profiles
                    .soldier_idx_by_identifier(behavior_profile_id)
                    .map_err(|reason| EngineError::ProfileSpriteLoadFailed {
                        kind: "enemy-AI hero",
                        profile_id: raw.profile_index,
                        reason,
                    })?;
                let behavior_profile =
                    profiles
                        .get_soldier(behavior_profile_index)
                        .unwrap_or_else(|| {
                            panic!("resolved hero AI profile {behavior_profile_id:?} disappeared")
                        });
                let mut ai = crate::ai_enemy::EnemyAi::new(0);
                ai.base.initial_action = raw.action;
                configure_enemy_ai_profile(
                    &mut ai,
                    behavior_profile,
                    char_profile.hth_weapon_id,
                    char_profile.shooting_weapon_id,
                    char_profile.shooting,
                    char_profile.endurance,
                    &profiles,
                    behavior_profile_index.0,
                    false,
                );
                Some(Box::new(crate::element::AiActorData {
                    ai_brain: crate::element::AiBrain::Enemy(Box::new(ai)),
                    ..Default::default()
                }))
            } else {
                if raw.decision_policy == crate::human_control::DecisionPolicy::FriendlyAi {
                    return Err(EngineError::MissionLevelStage {
                        stage: "rescue PCs",
                        reason: format!(
                            "hero profile {} requests friendly_ai, which has no hero decision runtime",
                            raw.profile_index
                        ),
                    });
                }
                None
            };

            let entity = Entity::Pc(crate::element::ActorPc {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ActorPc,
                    sprite,
                    posture: initial_posture,
                    ..Default::default()
                },
                actor: crate::element::ActorData {
                    script_class: raw.script_class.clone().unwrap_or_default(),
                    action_state: initial_action_state,
                    ..Default::default()
                },
                human: crate::element::HumanData {
                    time_hulk: crate::element::HULK_LENGTH,
                    invulnerable: config.highlander,
                    ..Default::default()
                },
                pc: crate::element::PcData {
                    robin: is_robin,
                    profile_index: profile_idx,
                    list_index,
                    cached_camp: raw
                        .allegiance
                        .map(crate::element::Camp::from_allegiance_id)
                        .unwrap_or(crate::element::Camp::Royalists),
                    campaign_description_index: Some(char_idx as u32),
                    kind,
                    has_lockpick,
                    has_climb,
                    has_jump,
                    immortal: config.highlander,
                    playable: raw.playable,
                    interface_hidden: !raw.playable,
                    command_interface: raw.command_interface,
                    mission_role: raw.mission_role,
                    combat_stance: raw.combat_stance,
                    ai: actor_ai,
                    ..Default::default()
                },
            });
            let eid = self.add_entity(entity);
            if let Some(ai) = self
                .world
                .entities
                .get_mut(eid)
                .and_then(Entity::ai_controller_mut)
            {
                ai.me = eid.index();
                ai.owner_entity_id = Some(eid);
            }
            // The low-priority idle order is enqueued by the post-spawn
            // `ensure_wait_element` loop further below, which iterates
            // every actor (rescue PCs included) before the first tick.
        }

        Ok(())
    }

    pub(super) fn spawn_soldiers_stage(
        &mut self,
        assets: &mut LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
        config: SimConfig,
    ) -> Result<(), EngineError> {
        let profiles = assets.profile_manager.clone();
        let char_base_dir = "Data/Characters";
        let bank_signature = assets.bank_signature;
        let highlander2 = config.highlander2;
        // Spawn soldiers (EVIL sub-chunk). Original's one NPC-only counter
        // continues after every CIVI constructor.
        for (soldier_index, raw) in loaded.mission.soldiers.iter().enumerate() {
            let npc_register_number = loaded
                .mission
                .civilians
                .len()
                .checked_add(soldier_index)
                .and_then(|value| u16::try_from(value).ok())
                .expect("soldier NPC register number exceeds u16");
            let mut sprite = crate::sprite::Sprite::default();
            let frame_kind = if raw.revealed || self.world.weather.is_forest_level {
                crate::sprite_script::FrameKind::Character
            } else {
                crate::sprite_script::FrameKind::CharacterBlipped
            };
            let profile_number = if let Some(identifier) = raw.profile_id.as_deref() {
                profiles
                    .soldier_idx_by_identifier(identifier)
                    .map_err(|reason| EngineError::ProfileSpriteLoadFailed {
                        kind: "soldier",
                        profile_id: raw.profile_number,
                        reason,
                    })?
                    .0
            } else {
                raw.profile_number
            };
            let soldier_profile = profiles.get_soldier(profile_number).ok_or_else(|| {
                EngineError::ProfileSpriteLoadFailed {
                    kind: "soldier",
                    profile_id: profile_number,
                    reason: "profile is missing from the loaded CPF".to_owned(),
                }
            })?;

            sprite
                .load_frame_info(
                    assets.sprite_scriptor_mut(),
                    frame_kind,
                    char_base_dir,
                    &soldier_profile.filename,
                    &soldier_profile.profile_name,
                    bank_signature,
                    Some(self.world.weather.ambiance.to_sprite_ambiance()),
                )
                .map_err(|e| EngineError::ProfileSpriteLoadFailed {
                    kind: "soldier",
                    profile_id: profile_number,
                    reason: e.to_string(),
                })?;

            let mut cached_max_lp = soldier_profile.life_point as i16;
            let cached_camp = raw
                .allegiance
                .map(crate::element::Camp::from_allegiance_id)
                .unwrap_or_else(|| {
                    if soldier_profile.hostile {
                        crate::element::Camp::Lacklandists
                    } else {
                        crate::element::Camp::Royalists
                    }
                });
            // Legacy RHM files encoded commandability indirectly: every
            // Royalist soldier was eligible for the optional troop-control
            // UI. Resolve that historical convention once at the data
            // boundary. Hackable descriptors author the command interface
            // explicitly and never infer it from allegiance.
            let (command_interface, mission_role) = if raw.allegiance.is_none()
                && raw.command_interface == crate::human_control::CommandInterface::None
                && cached_camp == crate::element::Camp::Royalists
            {
                (
                    crate::human_control::CommandInterface::TacticalOrders,
                    crate::human_control::MissionRole::TacticalAlly,
                )
            } else {
                (raw.command_interface, raw.mission_role)
            };

            // Modify life points for Lacklandist (enemy) soldiers based
            // on difficulty level.  VIPs are excluded from the modifier.
            // We scale cached_max_lp itself so both cached_max_life_points
            // and initial life_points start at the difficulty-adjusted
            // value.
            if self.is_hostile_to_player_camp(cached_camp) && !soldier_profile.vip {
                let diff = config.difficulty;
                cached_max_lp = diff.rules().enemy_life_points(cached_max_lp as u16, 10000) as i16;
            }

            // drunk_level must fit in u8.
            assert!(
                raw.drunk_level < 256,
                "soldier drunk_level out of range: {}",
                raw.drunk_level
            );
            // company_number must fit in u16.
            assert!(
                raw.company_number < 0x10000,
                "soldier company_number out of range: {}",
                raw.company_number
            );

            // Build the AI controller now so init_ai picks it up later.
            // path_id / alert_path_id / initial_action / blood_alcohol
            // all live on the AI base.
            let mut ai = crate::ai_enemy::EnemyAi::new(0);
            ai.base.path_id = crate::ai::PathId::new(raw.path_id);
            ai.base.alert_path_id = crate::ai::PathId::new(raw.alert_path_id);
            ai.base.initial_action = raw.action;
            ai.base.blood_alcohol = raw.drunk_level as u8;
            // company_number is u16, range asserted above.
            ai.company_number = raw.company_number as u16;
            ai.tower_guard = raw.tower_guard;
            // Copy courage from soldier profile for the approach logic.
            // Also pull the soldier's sword range from the HtH weapon
            // profile's distance[Default] entry.
            configure_enemy_ai_profile(
                &mut ai,
                soldier_profile,
                soldier_profile.hth_weapon_id,
                soldier_profile.shooting_weapon_id,
                soldier_profile.shooting,
                soldier_profile.endurance,
                &profiles,
                profile_number,
                config.item_gameplay.ale_reliable_distraction && !soldier_profile.vip,
            );

            // Set the sprite's move box + pathfinder index from the
            // soldier profile right after `LoadFrameInfo`.
            let soldier_pathfinder_idx = soldier_profile.pathfinder_index;
            let soldier_half_diag = self
                .world
                .fast_grid
                .try_move_box_half_diagonal(soldier_pathfinder_idx as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "soldier profile {profile_number} references missing pathfinder slot \
                         {soldier_pathfinder_idx} (table has {})",
                        self.world.fast_grid.level.move_box_half_diagonals.len()
                    )
                });
            sprite.position_iface.configure_for_actor(
                crate::position_interface::PathfinderIndex::new(u16::from(soldier_pathfinder_idx))
                    .expect("u8 soldier pathfinder index cannot equal 0xffff"),
                soldier_half_diag,
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
            );
            sprite.apply_placement(
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
                raw.layer,
                Some(Self::resolve_sparse_position_handle(assets, raw.sector)),
                // Apply initial facing from level data (0-15 sector).
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::from_u32(raw.material),
                crate::position_interface::ObstacleHandle::from_serialized_pointer(
                    raw.obstacle_index,
                ),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::from_serialized_pointer(
                        raw.obstacle_index,
                    ),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );
            prime_mission_start_sprite(
                &mut sprite,
                raw.action,
                raw.direction,
                &format!("soldier profile {profile_number}"),
            );

            let entity = Entity::Soldier(crate::element::ActorSoldier {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ActorSoldier,
                    // Non-forest levels start soldiers as blipped shadows that
                    // get revealed by proximity detection (SeesBlip) or the
                    // Listen ability.
                    blipped: !raw.revealed && !self.world.weather.is_forest_level,
                    // Default posture is Upright.  Without an explicit
                    // initializer posture defaults to `Undefined`, which
                    // stranded freshly-spawned soldiers because the
                    // `Command::Wait` fallback in tick.rs only maps known
                    // postures to idle animations — Undefined returned None
                    // and no bored animation got pushed.
                    posture: crate::element::Posture::Upright,
                    sprite,
                    ..Default::default()
                },
                actor: crate::element::ActorData {
                    // Record the script class name here; per-actor
                    // Initialize() is dispatched by initialize_mission_script.
                    script_class: raw.script_class.clone().unwrap_or_default(),
                    ..Default::default()
                },
                human: crate::element::HumanData {
                    invulnerable: highlander2,
                    ..Default::default()
                },
                npc: crate::element::NpcData {
                    // cached_max_lp was already difficulty-scaled above.
                    life_points: cached_max_lp,
                    ai: crate::element::AiActorData {
                        register_number: npc_register_number,
                        money: raw.money,
                        ai_brain: crate::element::AiBrain::Enemy(Box::new(ai)),
                        ..Default::default()
                    },
                },
                soldier: crate::element::SoldierData {
                    soldier_profile_index: crate::profiles::SoldierProfileIdx(profile_number),
                    cached_max_life_points: cached_max_lp,
                    cached_camp,
                    // Seed the cached rider flag from the profile at spawn,
                    // same pattern as `ai.is_vip = p.vip` above.
                    rider: soldier_profile.rider,
                    command_interface,
                    mission_role,
                    combat_stance: raw.combat_stance,
                    ..Default::default()
                },
            });
            let eid = self.add_entity(entity);
            // AiController was built with `EnemyAi::new(0)` above because
            // the entity id isn't known until `add_entity` returns — backfill
            // `ai.base.me` and `owner_entity_id` so trace logs, filter-event
            // dispatch, and any `self.me`/`self.owner_entity_id` reads see
            // the real id instead of 0.
            if let Some(e) = self.world.entities.get_mut(eid)
                && let Some(ai) = e.ai_controller_mut()
            {
                ai.me = eid.index();
                ai.owner_entity_id = Some(eid);
            }
            tracing::trace!(
                eid = eid.index(),
                path_id = raw.path_id,
                action = raw.action,
                "spawn soldier"
            );
            // Track soldier load-order → EntityId for patrol ID resolution.
            assets.entities.soldier_entity_ids.push(eid);
            assets
                .entities
                .soldier_subordinate_ids
                .push(raw.subordinate_ids.clone());
            // Every soldier contributes its money to the level pool, and
            // hostile (Lacklandists) soldiers increment the total soldier
            // count used by debriefing / campaign-stat sync.  Without this,
            // the "Level money" and "enemies encountered" rows on the
            // debriefing screen stay at 0 — and the `money` console cheat
            // miscomputes the delta.
            self.mission_domain.mission_stat.soldier_money += raw.money;
            self.mission_domain
                .mission_stat
                .record_soldier_encounter(cached_camp);
            if self.is_hostile_to_player_camp(cached_camp) {
                self.mission_domain.mission_stat.total_soldier_count += 1;
            }
        }

        Ok(())
    }

    pub(super) fn spawn_targets_stage(
        &mut self,
        assets: &mut LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
    ) {
        let bank_signature = assets.bank_signature;
        // Spawn targets.
        //
        // Each target stores its own RHS file name and profile in the
        // mission stream, loaded as an animation (from
        // `Data/Animations/<ambiance>/`), then `ForceAnimation(action,
        // direction)` is applied. Accessory targets reuse the object-master
        // profile preloaded before mission entities, matching the original
        // filename/profile-keyed SpriteScriptor cache.
        let sprite_ambiance = Some(self.world.weather.ambiance.to_sprite_ambiance());
        for raw in &loaded.mission.targets {
            let mut sprite = crate::sprite::Sprite::default();
            let (frame_kind, base_dir) = if raw.character_sprite {
                (
                    crate::sprite_script::FrameKind::Character,
                    "Data/Characters",
                )
            } else {
                (
                    crate::sprite_script::FrameKind::Animation,
                    "Data/Animations",
                )
            };

            match sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                frame_kind,
                base_dir,
                &raw.filename,
                &raw.profile_name,
                bank_signature,
                sprite_ambiance,
            ) {
                Ok(()) => {
                    // Apply `ForceAnimation(action, direction)`.
                    match crate::order::OrderType::try_from(raw.action) {
                        Ok(anim) => sprite.force_animation(anim, raw.direction as u16),
                        Err(_) => tracing::error!(
                            "Target action {} is not a valid OrderType — animation not forced",
                            raw.action,
                        ),
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to load target sprite scripts for '{}' profile '{}': {e}",
                        raw.filename,
                        raw.profile_name,
                    );
                }
            }

            // Rendering properties come from the blit type byte:
            //   0      → Blocky
            //   non-0  → NeedShadow
            let rendering_properties = if raw.blit_type != 0 {
                crate::element::RenderingProperties::NeedShadow
            } else {
                crate::element::RenderingProperties::Blocky
            };
            sprite.apply_placement(
                // First set the map position to the raw sprite position
                // so the plane projection can derive a baseline 3D
                // location. The map is later overwritten with the action
                // point (see below).
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
                raw.layer,
                Some(Self::resolve_sparse_position_handle(assets, raw.sector)),
                // Apply initial facing from level data (0-15 sector).
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::default(),
                crate::position_interface::ObstacleHandle::from_serialized_pointer(
                    raw.obstacle_index,
                ),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::from_serialized_pointer(
                        raw.obstacle_index,
                    ),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );

            // When the authored Z is non-negative, override the
            // plane-derived 3D with an explicit lift.  Elevated targets
            // (wall-mounted levers, second-storey apple shelves) render
            // at the authored height and feed the correct Z into bow-aim
            // / 3D hit queries.  Negative `position_z` means "no explicit
            // lift; keep the plane projection computed above".
            if raw.position_z >= 0 {
                sprite
                    .position_iface
                    .set_position(crate::coordinates::WorldPoint3D {
                        x: raw.position_x as f32,
                        y: raw.position_y as f32 + raw.position_z as f32,
                        z: raw.position_z as f32,
                    });
            }
            // Capture the current 3D position into `set_old_position`
            // so sprite-motion diffs across the first tick see a stable
            // baseline rather than the pre-placement zero.
            let current_position = sprite.position_iface.get_position();
            sprite.position_iface.set_old_position(current_position);
            let visual_map = current_position.to_map();
            sprite
                .position_iface
                .set_cached_sprite_position(MapPoint::new(
                    (visual_map.x - sprite.center.x).floor(),
                    (visual_map.y - sprite.center.y).floor(),
                ));

            // Overwrite the map position with the action point *without*
            // touching the 3D elevation we just set.  The target renders
            // at the sprite position but its action point — the spot the
            // PC walks to when interacting — lives at action_position.
            let action_point =
                MapPoint::new(raw.action_position_x as f32, raw.action_position_y as f32);
            sprite
                .position_iface
                .set_map_position_preserving_3d(action_point);
            sprite.position_iface.set_old_map_position(action_point);
            let entity = Entity::Target(crate::element::ElementTarget {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::Target,
                    sprite,
                    ..Default::default()
                },
                fx: crate::element::FxData {
                    // Targets are primary gameplay elements and are always
                    // drawn regardless of FX display options.
                    force_display: true,
                    ..Default::default()
                },
                target: crate::element::TargetData {
                    action_filter: crate::element::TargetFilter::from_bits_truncate(
                        raw.action_filter,
                    ),
                    action_position: MapPoint::new(
                        raw.action_position_x as f32,
                        raw.action_position_y as f32,
                    ),
                    action_sector: raw.action_sector,
                    action_layer: raw.action_layer,
                    position_z: raw.position_z,
                    sprite_filename: raw.filename.clone(),
                    sprite_profile_name: raw.profile_name.clone(),
                    display_polyline: raw
                        .polyline
                        .iter()
                        .map(|&(x, y)| MapPoint::new(x as f32, y as f32))
                        .collect(),
                    rendering_properties,
                    // Per-target script class name. Empty string = no
                    // script, matching proto default.
                    script_class: raw.script_class.clone().unwrap_or_default(),
                    ..Default::default()
                },
            });
            self.add_entity(entity);
        }
    }

    pub(super) fn spawn_bonuses_stage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &mut LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
    ) {
        let char_base_dir = "Data/Characters";
        let bank_signature = assets.bank_signature;
        // Preload sprites for sim-tick spawn paths so they can hit the
        // scriptor cache through `&LevelAssets` — enforces the "engine
        // mutation only during perform_hourglass" invariant.
        self.preload_scroll_amulet_sprite(assets);
        self.preload_campaign_peasant_sprites(assets);

        // Spawn bonuses.
        //
        // Each bonus type has its own RHS file and profile name living
        // next to the character sprites; the bonus is constructed from
        // the corresponding pre-loaded master sprite.
        for raw in &loaded.mission.bonuses {
            let (sprite_file, profile_name, object_type) =
                match bonus_type_to_sprite_asset(raw.bonus_type) {
                    Some(t) => t,
                    None => {
                        tracing::error!(
                            "Unknown bonus type {} in mission file — skipping",
                            raw.bonus_type,
                        );
                        continue;
                    }
                };

            // Decode the bonus type to get the associated player action.
            let bonus_kind = crate::element::BonusItemType::from_u16(raw.bonus_type)
                .unwrap_or_else(|| panic!("unknown BonusItemType ordinal {}", raw.bonus_type));
            let associated_action = bonus_kind.to_action();

            // SetQuantity:
            //   * Ransom maps 1..=5 to 100/500/1000/2500/5000.
            //   * Blazon keeps the raw quantity and forces the animation row
            //     to BonusOne..BonusFive.
            //   * Everything else stores the raw quantity as-is.
            // The level stream holds the 1..=5 ordinal.
            let (stored_quantity, blazon_anim) = match bonus_kind {
                crate::element::BonusItemType::Ransom => {
                    let real = match raw.quantity {
                        1 => 100,
                        2 => 500,
                        3 => 1000,
                        4 => 2500,
                        5 => 5000,
                        q => {
                            tracing::error!(
                                "Ransom quantity {q} out of range [1,5]; using raw value",
                            );
                            q
                        }
                    };
                    (real, None)
                }
                crate::element::BonusItemType::Blazon => {
                    let anim = match raw.quantity {
                        1 => Some(crate::order::OrderType::BonusOne),
                        2 => Some(crate::order::OrderType::BonusTwo),
                        3 => Some(crate::order::OrderType::BonusThree),
                        4 => Some(crate::order::OrderType::BonusFour),
                        5 => Some(crate::order::OrderType::BonusFive),
                        q => {
                            tracing::error!(
                                "Blazon quantity {q} out of range [1,5]; leaving default animation",
                            );
                            None
                        }
                    };
                    (raw.quantity, anim)
                }
                _ => (raw.quantity, None),
            };

            let mut sprite = crate::sprite::Sprite::default();
            if let Err(e) = sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                crate::sprite_script::FrameKind::Object,
                char_base_dir,
                sprite_file,
                profile_name,
                bank_signature,
                Some(self.world.weather.ambiance.to_sprite_ambiance()),
            ) {
                tracing::error!(
                    "Failed to load bonus sprite '{sprite_file}' profile '{profile_name}': {e}",
                );
            } else {
                if let Some(anim) = blazon_anim {
                    // Blazons display the row for their current quantity:
                    // force_animation with the BonusOne..BonusFive row.
                    // Direction comes from the level data (applied below on
                    // the ElementData); the direction is set AFTER
                    // constructing the bonus, so we pass 0 here.
                    sprite.force_animation(anim, 0);
                }
                // Force a random sprite frame *after* the quantity-driven
                // animation row, so every bonus (including Blazons whose
                // animation row was just forced) ends up on a random frame
                // within its current row.  Sequencing must be
                // force_animation → force_random_sprite_frame because
                // `force_animation` resets `current_frame` to 0.
                sprite.force_random_sprite_frame(
                    sim,
                    crate::sim_rng::RngSite::LevelBonusInitialFrame,
                );
            }
            sprite.apply_placement(
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
                raw.layer,
                Some(Self::resolve_sparse_position_handle(assets, raw.sector)),
                // Apply initial facing from level data (0-15 sector).
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::default(),
                crate::position_interface::ObstacleHandle::from_serialized_pointer(
                    raw.obstacle_index,
                ),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::from_serialized_pointer(
                        raw.obstacle_index,
                    ),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );
            // RHElementBonus::ReadFromMissionLevel computes the placed 3D
            // position, then copies both current coordinates into the old
            // coordinates before the element enters its first Hourglass
            // (original-code/RHElementBonus.cpp:484-487). Without this the
            // first parity frame reports every bonus as moving from zero.
            let current_position = sprite.position_iface.get_position();
            let current_map_position = sprite.position_iface.map_position();
            sprite.position_iface.set_old_position(current_position);
            sprite
                .position_iface
                .set_old_map_position(current_map_position);
            let entity = Entity::Bonus(crate::element::ElementBonus {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ObjectBonus,
                    // Bonuses are blipped on non-forest levels.
                    blipped: !self.world.weather.is_forest_level,
                    sprite,
                    ..Default::default()
                },
                object: crate::element::ObjectData {
                    quantity: stored_quantity,
                    object_type,
                    associated_action,
                    ..Default::default()
                },
            });
            self.add_entity(entity);
            // Each RANSOM bonus feeds its mapped value
            // (100/500/1000/2500/5000) into `bonus_money`. Without this,
            // the "Level money" debriefing row and the `money` console
            // cheat always see 0 for bonus money.
            if matches!(bonus_kind, crate::element::BonusItemType::Ransom) {
                self.mission_domain.mission_stat.bonus_money += stored_quantity as u32;
            }
        }
    }

    pub(super) fn spawn_scrolls_stage(
        &mut self,
        assets: &mut LevelAssets,
        loaded: &crate::level_data::LoadedLevel,
        config: SimConfig,
    ) -> Vec<crate::element::EntityId> {
        let char_base_dir = "Data/Characters";
        let bank_signature = assets.bank_signature;
        // Spawn scrolls (PARC chunk).
        //
        // Each scroll uses the "BONUS_Parchment" / "BONUS Parchemin"
        // sprite pair.
        //
        // `force_visible` on the raw scroll is only used once during init
        // (`if force_visible → SetStatus(Visible)`). Collect the handles here
        // and flush them after `load_mission_script` so native-visible scroll
        // state still begins at the historical script boundary.
        let mut force_visible_scroll_ids: Vec<crate::element::EntityId> = Vec::new();
        // Reset the scroll-id → EntityId map (repopulated per-level).
        // Reserved capacity matches the PARC chunk count exactly.
        assets.entities.scroll_entity_ids.clear();
        assets
            .entities
            .scroll_entity_ids
            .reserve(loaded.mission.scrolls.len());
        for raw in &loaded.mission.scrolls {
            let mut sprite = crate::sprite::Sprite::default();
            if let Err(e) = sprite.load_frame_info(
                assets.sprite_scriptor_mut(),
                crate::sprite_script::FrameKind::Object,
                char_base_dir,
                "BONUS_Parchment",
                "BONUS Parchemin",
                bank_signature,
                Some(self.world.weather.ambiance.to_sprite_ambiance()),
            ) {
                tracing::error!("Failed to load scroll sprite: {e}");
            } else {
                let anim = if raw.tutorial {
                    crate::order::OrderType::BonusTwo
                } else {
                    crate::order::OrderType::BonusOne
                };
                sprite.force_animation(anim, raw.direction as u16);
                // Random sprite frame is picked later by
                // `EngineInner::initialize_all_scrolls` at mission start
                // (not at load).
            }

            // The mission stream's `action` field is ignored — the
            // initial animation is overridden based on the tutorial
            // flag, applied by `force_animation` above.
            let _ = raw.action;

            // SetActive(true) when `is_to_be_replaced_by_amulet`
            // (Easy + presence[Easy]==false, so the scroll spawns in
            // place to later morph into an amulet), else
            // SetActive(presence[difficulty]).  Skipping this left every
            // scroll active on Medium/Hard even when its presence flag
            // was cleared, which the render / focus paths would then
            // happily expose.
            let difficulty = config.difficulty;
            let difficulty_idx = difficulty.to_u32() as usize;
            let is_to_be_replaced_by_amulet = difficulty.rules().legacy_level
                == crate::player_profile::LegacyDifficultyLevel::Easy
                && !raw.presence[0];
            let scroll_active = if is_to_be_replaced_by_amulet {
                true
            } else {
                raw.presence.get(difficulty_idx).copied().unwrap_or(false)
            };
            sprite.apply_placement(
                MapPoint::new(raw.position_x as f32, raw.position_y as f32),
                raw.layer,
                Some(Self::resolve_sparse_position_handle(assets, raw.sector)),
                (raw.direction & 15) as i16,
                crate::element::GameMaterial::default(),
                crate::position_interface::ObstacleHandle::from_serialized_pointer(
                    raw.obstacle_index,
                ),
                crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                    crate::position_interface::ObstacleHandle::from_serialized_pointer(
                        raw.obstacle_index,
                    ),
                    assets.static_sight_obstacles.as_slice(),
                ),
            );
            // RHElementScroll::ReadFromMissionLevel performs the same
            // current-to-old settlement immediately after ComputePositionAll
            // (original-code/RHElementScroll.cpp:259-262).
            let current_position = sprite.position_iface.get_position();
            let current_map_position = sprite.position_iface.map_position();
            sprite.position_iface.set_old_position(current_position);
            sprite
                .position_iface
                .set_old_map_position(current_map_position);
            let entity = Entity::Scroll(crate::element::ElementScroll {
                element: crate::element::ElementData {
                    kind: crate::element::ElementKind::ObjectScroll,
                    sprite,
                    active: scroll_active,
                    // Stamp `CUSTOM_DOT_INVISIBLE` (= 0) at construction.
                    // Without this the scroll's default
                    // `custom_minimap_dot = 1` leaks into
                    // `minimap::classify_default` and pre-reveal scrolls
                    // would paint a minimap dot before the PC even talks
                    // to a beggar.
                    custom_minimap_dot: 0,
                    ..Default::default()
                },
                object: crate::element::ObjectData {
                    object_type: crate::element::ObjectType::Scroll,
                    ..Default::default()
                },
                presence: raw.presence,
                tutorial: raw.tutorial,
                script_class: raw.script_class.clone().unwrap_or_default(),
                script_hourglass_timeout: 0,
            });
            let scroll_eid = self.add_entity(entity);
            assets.entities.scroll_entity_ids.push(scroll_eid);

            // `force_visible` flips the canonical scroll-domain status to
            // Visible. Capture ids here and flush them once the mission script
            // is loaded below.
            if raw.force_visible {
                force_visible_scroll_ids.push(scroll_eid);
            }
        }

        // NOTE: GUYS (tenants) chunk does NOT add entities to the script
        // elements array.  Tenants just register existing entities as
        // building occupants; `InitOccupant` runs after
        // the MissionLevelBuilder so it can operate on fully-
        // initialised entities.

        force_visible_scroll_ids
    }
}
