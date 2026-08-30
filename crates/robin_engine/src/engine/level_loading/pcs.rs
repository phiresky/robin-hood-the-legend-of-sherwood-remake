//! Beam-me player-character assignment and spawning.

use super::*;

impl EngineInner {
    pub(super) fn spawn_beam_me_pcs_stage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &mut LevelAssets,
        loaded: &mut crate::level_data::LoadedLevel,
    ) -> Result<(), EngineError> {
        let profiles = assets.profile_manager.clone();
        let char_base_dir = "Data/Characters";
        let bank_signature = assets.bank_signature;
        // ── Spawn PCs at beam-me points ─────────────────────────────
        // We split this into two phases to satisfy the borrow checker:
        // Phase A computes assignments from campaign data (borrows self.mission_domain.campaign),
        // Phase B creates entities (borrows assets.sprite_scriptor, self.world.entities).
        // (char_idx, profile_idx, beam_me_idx, pre-shuffle Sherwood placement roll)
        let pc_spawn_plan: Vec<(
            usize,
            crate::profiles::CharacterProfileIdx,
            usize,
            Option<super::teleport::SherwoodPlacementRoll>,
        )>;
        let is_sherwood;
        {
            let campaign = &mut self.mission_domain.campaign;

            // Determine Sherwood-camp flag up front — the Sherwood
            // branch needs to run between Phase 1 and Phase 2, and the
            // final post-spawn `ResetMissionTeam` call reads the same
            // flag.
            is_sherwood = campaign.current_mission_idx.is_some_and(|idx| {
                campaign
                    .missions
                    .get(idx)
                    .and_then(|m| m.profile_idx)
                    .and_then(|pi| profiles.missions.get(pi as usize))
                    .is_some_and(|p| p.location == crate::profiles::MissionLocation::Sherwood)
            });

            // Reset instanced flags for all gang members
            for &gi in &campaign.gang_indices {
                if let Some(desc) = campaign.characters.get_mut(gi) {
                    desc.instanced = false;
                }
            }

            // Build mission team list: (char_idx, profile_idx)
            let team: Vec<(usize, crate::profiles::CharacterProfileIdx)> = campaign
                .mission_team_indices
                .iter()
                .map(|&char_idx| {
                    let description = campaign.characters.get(char_idx).ok_or_else(|| {
                        EngineError::MissionLevelStage {
                            stage: "beam-me assignment",
                            reason: format!(
                                "mission team references missing campaign character {char_idx}"
                            ),
                        }
                    })?;
                    let profile_idx = description.character_profile_idx.ok_or_else(|| {
                        EngineError::MissionLevelStage {
                            stage: "beam-me assignment",
                            reason: format!(
                                "campaign character {char_idx} has no character profile"
                            ),
                        }
                    })?;
                    Ok((char_idx, profile_idx))
                })
                .collect::<Result<_, EngineError>>()?;

            const MAX_NUMBER_OF_CHARACTER: usize = 5;

            // Snapshot each team member's remembered Sherwood slot now so
            // that later mutations (Phase 1 or the Phase-B write-back of
            // `beam_me_index_in_sherwood`) don't race with Sherwood-branch
            // placement below.
            let team_remembered_sherwood_slot: Vec<i16> = team
                .iter()
                .map(|&(char_idx, _)| {
                    campaign
                        .characters
                        .get(char_idx)
                        .map(|d| d.status.beam_me_index_in_sherwood)
                        .unwrap_or(-1)
                })
                .collect();

            let mut assignments: Vec<Option<usize>> = vec![None; loaded.mission.beam_mes.len()];
            let mut instanced = vec![false; team.len()];
            // Pre-rolled placement values are kept by team member until
            // Phase B can create and mutate the corresponding entity.
            let mut sherwood_placement_rolls = vec![None; team.len()];

            // Phase 1: Handle required characters.
            for (bm_idx, beam_me) in loaded.mission.beam_mes.iter().enumerate() {
                if beam_me.required_pc == 0 {
                    continue;
                }
                let required_names: &[&str] = match beam_me.required_pc {
                    1 => &["Frere Tuck"],
                    2 => &["Lady Marianne"],
                    3 => &["Petit Jean"],
                    4 => &["Robin des bois", "Robin des villes"],
                    5 => &["Stutely"],
                    6 => &["Will Ecarlate"],
                    _ => {
                        tracing::error!(
                            "Unknown required_pc value {} at beam-me {}",
                            beam_me.required_pc,
                            bm_idx,
                        );
                        continue;
                    }
                };
                let found = team.iter().enumerate().find(|(ti, (_, pidx))| {
                    if instanced[*ti] {
                        return false;
                    }
                    // Case-insensitive name match against the character
                    // profile name.
                    profiles.get_character(*pidx).is_some_and(|p| {
                        required_names
                            .iter()
                            .any(|n| p.profile_name.eq_ignore_ascii_case(n))
                    })
                });
                if let Some((ti, _)) = found {
                    instanced[ti] = true;
                    assignments[bm_idx] = Some(ti);
                } else {
                    tracing::error!(
                        "Beam-me {} requires character type {} but no match in mission team",
                        bm_idx,
                        beam_me.required_pc,
                    );
                }
            }

            // Sherwood branch: when the current mission is the Sherwood
            // camp, seat every mission-team member whose remembered
            // `beam_me_index_in_sherwood` recalls a slot from the
            // previous Sherwood stay back at that slot, then permute the
            // unused beam-mes 100 times so the remaining free-for-all
            // slots Phase 2 assigns to are randomised.  The per-PC
            // position jitter + random facing (`randomize_position`)
            // happens in Phase B after each sherwood-returner is
            // actually spawned.
            if is_sherwood {
                for ti in 0..team.len() {
                    if instanced[ti] {
                        // Phase 1 already seated this member by required
                        // name; skip to avoid double-spawning.
                        continue;
                    }
                    let remembered = team_remembered_sherwood_slot[ti];
                    if remembered < 0 {
                        continue;
                    }
                    let bm_idx = remembered as usize;
                    if bm_idx >= loaded.mission.beam_mes.len() {
                        tracing::warn!(
                            "Sherwood-return PC (team_idx {ti}) remembered beam_me {remembered} \
                             but only {} slots exist — ignoring",
                            loaded.mission.beam_mes.len(),
                        );
                        continue;
                    }
                    if assignments[bm_idx].is_some() {
                        tracing::warn!(
                            "Sherwood-return PC (team_idx {ti}) wanted beam_me {bm_idx} which is \
                             already assigned — skipping",
                        );
                        continue;
                    }
                    assignments[bm_idx] = Some(ti);
                    instanced[ti] = true;
                    // Original creates and randomizes every remembered PC
                    // before consuming the 200 beam-me shuffle draws.
                    sherwood_placement_rolls[ti] =
                        Some(super::teleport::roll_sherwood_placement(sim));
                }

                // Shuffle the free beam-mes 100 times.  Both the
                // `BeamMe` vector and the parallel `assignments` vector
                // are swapped together so that the "used" flag attached
                // to an occupied slot travels with its beam-me (a PC
                // Sherwood-placed above was baked at the slot's
                // original beam-me; we preserve that identity rather
                // than the slot index).  We pull from the deterministic
                // sim RNG so replay / rollback stay reproducible.
                let n = loaded.mission.beam_mes.len();
                if n > 0 {
                    shuffle_sherwood_slots(sim, n, |a, b| {
                        if a != b {
                            loaded.mission.beam_mes.swap(a, b);
                            assignments.swap(a, b);
                        }
                    });
                }
            }

            // Phase 2: Fill remaining beam-me slots, honoring the
            // per-beam-me action requirements.
            //
            //   1. Build the list of "available" team members (those not
            //      already instanced by Phase 1).  For non-Sherwood
            //      missions the list is capped at MAX_NUMBER_OF_CHARACTER
            //      (= 5; extra mission-team members stay behind as
            //      reservists for this mission).
            //   2. Iterate until every slot is filled or no candidates
            //      remain.  In each pass: for each unsolved beam-me,
            //      collect the team members valid for the slot (action-
            //      capability intersection with the beam-me's
            //      `action_required` flags).  Assign when there is
            //      exactly one candidate, or, once `force_decision`
            //      kicks in, when there is at least one.
            //   3. Dead end (no slot solved this iteration): first flip
            //      `force_decision` to let multi-candidate slots
            //      resolve.  If that still fails, fall through to a
            //      brute force pass that fills each remaining slot with
            //      whichever character is left even if the action
            //      requirements don't match — better than leaving the
            //      slot empty.
            let mut available: Vec<usize> = (0..team.len()).filter(|&i| !instanced[i]).collect();
            if !is_sherwood && available.len() > MAX_NUMBER_OF_CHARACTER {
                available.truncate(MAX_NUMBER_OF_CHARACTER);
            }

            let slot_valid_for = |beam_me: &crate::level_data::BeamMe,
                                  mission_team_index: usize|
             -> bool {
                // Original's IsCharacterValidForThisSlot is passed an index
                // into mMissionTeam, but (despite that contract) indexes
                // maGang with it. Preserve this observable bug: it controls
                // which campaign PC is assigned to each scripted beam-me and
                // therefore the static actor/script identities used by saves.
                let gang_character_index = *campaign
                    .gang_indices
                    .get(mission_team_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "Original beam-me gang-index bug addressed missing gang slot {mission_team_index}"
                        )
                    });
                let profile_idx = campaign
                    .characters
                    .get(gang_character_index)
                    .and_then(|description| description.character_profile_idx)
                    .unwrap_or_else(|| {
                        panic!(
                            "Original beam-me gang slot {mission_team_index} references an absent or profile-less campaign character {gang_character_index}"
                        )
                    });
                let Some(profile) = profiles.get_character(profile_idx) else {
                    return false;
                };
                use crate::profiles::Action;
                let mut archer = false;
                let mut lever_main = false;
                let mut lockpicker_main = false;
                let mut stuner = false;
                let mut eater = false;
                for a in &profile.actions {
                    match *a {
                        Action::Bow => archer = true,
                        Action::Lever => lever_main = true,
                        Action::Lockpick => lockpicker_main = true,
                        Action::Hit | Action::HitHard => stuner = true,
                        Action::Eat | Action::Guzzle => eater = true,
                        _ => {}
                    }
                }
                let mut carrier = false;
                let mut climber = false;
                let mut jumper = false;
                let mut tailor = false;
                let mut searcher = false;
                let mut lever_ctx = false;
                let mut lockpicker_ctx = false;
                for a in &profile.contextual_actions {
                    match *a {
                        Action::FarmerCarry | Action::LittleJohnCarry => carrier = true,
                        Action::Climb => climber = true,
                        Action::Jump => jumper = true,
                        Action::Tie => tailor = true,
                        Action::Search => searcher = true,
                        Action::Lockpick => lockpicker_ctx = true,
                        Action::Lever => lever_ctx = true,
                        _ => {}
                    }
                }
                let req = &beam_me.action_required;
                if req.archery && !archer {
                    return false;
                }
                if req.carry && !carrier {
                    return false;
                }
                if req.climb && !climber {
                    return false;
                }
                if req.jump && !jumper {
                    return false;
                }
                if req.lever && !(lever_main || lever_ctx) {
                    return false;
                }
                if req.lockpick && !(lockpicker_main || lockpicker_ctx) {
                    return false;
                }
                if req.stun && !stuner {
                    return false;
                }
                if req.tie && !tailor {
                    return false;
                }
                if req.eat && !eater {
                    return false;
                }
                if req.search && !searcher {
                    return false;
                }
                true
            };

            let mut force_decision = false;
            while assignments.iter().any(|a| a.is_none()) && !available.is_empty() {
                let mut solved_this_pass = false;
                for (bm_idx, beam_me) in loaded.mission.beam_mes.iter().enumerate() {
                    if assignments[bm_idx].is_some() {
                        continue;
                    }
                    let candidates: Vec<usize> = available
                        .iter()
                        .copied()
                        .filter(|&ti| slot_valid_for(beam_me, ti))
                        .collect();
                    let pick =
                        if candidates.len() == 1 || (force_decision && !candidates.is_empty()) {
                            Some(candidates[0])
                        } else {
                            None
                        };
                    if let Some(ti) = pick {
                        assignments[bm_idx] = Some(ti);
                        instanced[ti] = true;
                        available.retain(|&x| x != ti);
                        solved_this_pass = true;
                        // Reset force_decision on success so the next
                        // pass re-prefers single-candidate slots.
                        force_decision = false;
                        if available.is_empty() {
                            break;
                        }
                    }
                }
                if !solved_this_pass {
                    if !force_decision {
                        force_decision = true;
                        continue;
                    }
                    // Brute-force fill for slots with no valid candidate.
                    for (bm_idx, _) in loaded.mission.beam_mes.iter().enumerate() {
                        if assignments[bm_idx].is_some() || available.is_empty() {
                            continue;
                        }
                        let ti = available.remove(0);
                        assignments[bm_idx] = Some(ti);
                        instanced[ti] = true;
                    }
                    break;
                }
            }

            // Collect the spawn plan and mark instanced in campaign.
            pc_spawn_plan = loaded
                .mission
                .beam_mes
                .iter()
                .enumerate()
                .filter_map(|(bm_idx, _)| {
                    let ti = assignments[bm_idx]?;
                    let (char_idx, profile_idx) = team[ti];
                    // Mark character as instanced in the campaign
                    if let Some(desc) = campaign.characters.get_mut(char_idx) {
                        desc.instanced = true;
                    }
                    Some((char_idx, profile_idx, bm_idx, sherwood_placement_rolls[ti]))
                })
                .collect();
        }

        // Phase B: Create entities (no longer borrowing self.mission_domain.campaign).
        // Add one script entry per beam-me: the PC if assigned, or None
        // to keep script entity indices aligned.
        let mut pc_count = 0u32;
        for (bm_idx, beam_me) in loaded.mission.beam_mes.iter().enumerate() {
            // Find the spawn plan entry for this beam-me, if any
            let plan_entry = pc_spawn_plan.iter().find(|&&(_, _, bi, _)| bi == bm_idx);

            if let Some(&(char_idx, mut profile_idx, _, sherwood_placement_roll)) = plan_entry {
                if let Some(override_index) = beam_me.profile_override {
                    profile_idx = crate::profiles::CharacterProfileIdx(override_index);
                }
                // A "Robin des villes" PC in a forest level is rewritten
                // to "Robin des bois", and vice-versa in a town level.
                // Swap both `profile_idx` and the campaign character's
                // stored `character_profile_idx`.
                {
                    use crate::character_kind::CharacterKind;
                    let want_town = !self.world.weather.is_forest_level;
                    let current_kind = profiles
                        .get_character(profile_idx)
                        .and_then(|p| CharacterKind::from_profile(&p.filename, &p.profile_name));
                    if let Some(CharacterKind::RobinHood { is_town }) = current_kind
                        && is_town != want_town
                    {
                        let target_kind = CharacterKind::RobinHood { is_town: want_town };
                        if let Some(new_idx) = profiles.characters.iter().position(|p| {
                            CharacterKind::from_profile(&p.filename, &p.profile_name)
                                == Some(target_kind)
                        }) {
                            let new_profile_idx =
                                crate::profiles::CharacterProfileIdx(new_idx as u32);
                            profile_idx = new_profile_idx;
                            if let Some(desc) =
                                self.mission_domain.campaign.characters.get_mut(char_idx)
                            {
                                desc.character_profile_idx = Some(new_profile_idx);
                            }
                        } else {
                            tracing::warn!(
                                "Robin forest/town swap: profile '{:?}' not found; keeping {:?}",
                                target_kind,
                                profile_idx,
                            );
                        }
                    }
                }
                let profile = profiles.get_character(profile_idx).ok_or_else(|| {
                    EngineError::MissionLevelStage {
                        stage: "beam-me PC spawn",
                        reason: format!(
                            "campaign character {char_idx} references missing profile {profile_idx}"
                        ),
                    }
                })?;
                if beam_me.profile_override.is_some() {
                    let description = self
                        .mission_domain
                        .campaign
                        .characters
                        .get_mut(char_idx)
                        .expect("beam-me campaign character disappeared after assignment");
                    description.character_profile_idx = Some(profile_idx);
                    description.status.name = profile.profile_name.clone();
                }

                // PCs always use the Character frame kind, unlike NPCs
                // which use the level's frame_kind (Character vs CharacterBlipped).
                let mut sprite = crate::sprite::Sprite::default();
                if let Err(e) = sprite.load_frame_info(
                    assets.sprite_scriptor_mut(),
                    crate::sprite_script::FrameKind::Character,
                    char_base_dir,
                    &profile.filename,
                    &profile.profile_name,
                    bank_signature,
                    Some(self.world.weather.ambiance.to_sprite_ambiance()),
                ) {
                    tracing::error!(
                        "Failed to load sprite for PC '{}' (profile {}): {e}",
                        profile.profile_name,
                        profile_idx,
                    );
                }
                // Load the alternate profile track when the character
                // profile flags `valid_alternative_profile`.  Used for
                // disguise / variant animations.
                if profile.valid_alternative_profile
                    && !profile.alternative_profile_name.is_empty()
                    && let Err(e) = sprite.load_alternate_profile(
                        assets.sprite_scriptor_mut(),
                        crate::sprite_script::FrameKind::Character,
                        char_base_dir,
                        &profile.filename,
                        &profile.alternative_profile_name,
                        bank_signature,
                        None,
                    )
                {
                    tracing::error!(
                        "Failed to load alternate sprite profile '{}' for PC '{}' (profile {}): {e}",
                        profile.alternative_profile_name,
                        profile.profile_name,
                        profile_idx,
                    );
                }

                let kind = crate::character_kind::CharacterKind::from_profile(
                    &profile.filename,
                    &profile.profile_name,
                );
                let is_robin = beam_me.robin_role || kind.is_some_and(|k| k.is_robin());
                let (has_lockpick, has_climb, has_jump) =
                    crate::element::PcData::movement_auth_from_profile(profile);

                // Map the beam-me's initial animation to a (posture,
                // action_state) pair. Apply it up front so the PC starts
                // in the correct pose. The HIDDEN titbit for Spy/Tree
                // postures is added by the regular titbit sync pass once
                // it sees a hidden posture, so we don't add it manually
                // here.
                let (initial_posture, initial_action_state) = map_pc_initial_action(beam_me.action);
                let list_index =
                    u8::try_from(char_idx).map_err(|_| EngineError::MissionLevelStage {
                        stage: "beam-me PC spawn",
                        reason: format!(
                            "campaign character index {char_idx} does not fit in PcData::list_index"
                        ),
                    })?;

                // Set the sprite's move box + pathfinder index from the
                // character profile right after `LoadFrameInfo` so
                // anti-collision has a valid bbox on the very first tick.
                let pc_pathfinder_idx = profile.pathfinder_index;
                let pc_half_diag = self
                    .world
                    .fast_grid
                    .try_move_box_half_diagonal(pc_pathfinder_idx as usize)
                    .unwrap_or_else(|| {
                        panic!(
                            "PC profile {profile_idx} references missing pathfinder slot \
                             {pc_pathfinder_idx}"
                        )
                    });
                sprite.position_iface.configure_for_actor(
                    crate::position_interface::PathfinderIndex::new(u16::from(pc_pathfinder_idx))
                        .expect("u8 PC pathfinder index cannot equal 0xffff"),
                    pc_half_diag,
                    beam_me.position,
                );
                // Validate every beam-me's layer range and sector
                // motion/area bits.  Warn instead of asserting so a
                // corrupt mission file still loads (the existing
                // motion/area lookup will collapse the beam-me into the
                // fallback path downstream).
                if beam_me.layer > self.world.fast_grid.level.special_layer {
                    tracing::warn!(
                        "Beam-me {} at ({},{}) lies on out-of-range layer {} (special_layer={})",
                        bm_idx,
                        beam_me.position.x,
                        beam_me.position.y,
                        beam_me.layer,
                        self.world.fast_grid.level.special_layer,
                    );
                }
                let sector_motion_area = self
                    .world
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&crate::sector::SectorNumber::new(beam_me.sector as i16))
                    .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
                    .map(|gs| gs.sector_type.is_motion() && gs.sector_type.is_area())
                    .unwrap_or(false);
                if !sector_motion_area {
                    tracing::warn!(
                        "Beam-me {} at ({},{}) sector {} is not a motion+area sector",
                        bm_idx,
                        beam_me.position.x,
                        beam_me.position.y,
                        beam_me.sector,
                    );
                }
                // Out-of-range material silently falls back to the grid
                // default material.
                let material = crate::element::GameMaterial::from_u32_with_default(
                    beam_me.material,
                    assets.material_sectors.default_material,
                );
                // Validate that the beam-me's obstacle index is a
                // projection area and the beam-me position is inside its
                // screen box.  We warn so a corrupt mission still loads.
                if beam_me.projection_area != 0xFFFF {
                    match assets
                        .static_sight_obstacles
                        .get(beam_me.projection_area as usize)
                    {
                        None => tracing::warn!(
                            "Beam-me {} references out-of-range projection area {}",
                            bm_idx,
                            beam_me.projection_area,
                        ),
                        Some(obs) => {
                            if !obs.is_projection_area() {
                                tracing::warn!(
                                    "Beam-me {} at ({},{}) not lying on projection area (obstacle {})",
                                    bm_idx,
                                    beam_me.position.x,
                                    beam_me.position.y,
                                    beam_me.projection_area,
                                );
                            }
                            if !obs.box_projection.contains_point(beam_me.position) {
                                tracing::warn!(
                                    "Beam-me {} at ({},{}) map position not lying in projection area screen box (obstacle {})",
                                    bm_idx,
                                    beam_me.position.x,
                                    beam_me.position.y,
                                    beam_me.projection_area,
                                );
                            }
                        }
                    }
                }
                sprite.apply_placement(
                    beam_me.position,
                    beam_me.layer,
                    Some(Self::resolve_sparse_position_handle(assets, beam_me.sector)),
                    // Apply initial facing from the beam-me point (0-15 sector).
                    (beam_me.direction & 15) as i16,
                    material,
                    crate::position_interface::ObstacleHandle::from_serialized_pointer(
                        beam_me.projection_area,
                    ),
                    crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                        crate::position_interface::ObstacleHandle::from_serialized_pointer(
                            beam_me.projection_area,
                        ),
                        assets.static_sight_obstacles.as_slice(),
                    ),
                );
                prime_mission_start_sprite(
                    &mut sprite,
                    beam_me.action,
                    beam_me.direction,
                    &format!("beam-me {bm_idx}"),
                );
                // Seed old position fields so `is_moving()` is false on
                // the first post-spawn tick (matches the Target spawn
                // path above).
                let current_position = sprite.position_iface.get_position();
                sprite.position_iface.set_old_position(current_position);
                sprite.position_iface.set_old_map_position(beam_me.position);
                // Seed `disabled_actions` from per-slot ammo /
                // purse-ransom checks so a slot whose counter is empty
                // (or whose purse threshold isn't met) starts greyed out
                // instead of waiting for the first runtime ammo update.
                let disabled_actions: Vec<bool> = {
                    let pc_status = &self
                        .mission_domain
                        .campaign
                        .characters
                        .get(char_idx)
                        .ok_or_else(|| EngineError::MissionLevelStage {
                            stage: "beam-me PC spawn",
                            reason: format!(
                                "campaign character {char_idx} disappeared before entity creation"
                            ),
                        })?
                        .status;
                    let ransom = self
                        .mission_domain
                        .campaign
                        .get_value(crate::campaign::CampaignValue::Ransom);
                    let purse_threshold = crate::inventory::COINS_PER_PURSE as i32
                        * crate::inventory::COIN_VALUE as i32;
                    (0..crate::profiles::NUMBER_OF_PC_ACTIONS)
                        .map(|slot| {
                            let action = profile.actions[slot];
                            if action == crate::profiles::Action::NoAction {
                                return false;
                            }
                            let ammo_empty = crate::inventory::action_uses_ammo(action)
                                && pc_status.get_ammo(action) == 0;
                            let purse_underfunded = action == crate::profiles::Action::Purse
                                && ransom < purse_threshold;
                            ammo_empty || purse_underfunded
                        })
                        .collect()
                };
                // RHElementActorPC keeps the campaign description's
                // RHPCStatus as its live status object. Seed the entity-owned
                // mirror from that same description rather than PcData's
                // full-health/empty-pocket defaults.
                let pc_status = self
                    .mission_domain
                    .campaign
                    .characters
                    .get(char_idx)
                    .expect("beam-me campaign character disappeared after validation")
                    .status
                    .clone();
                let entity = Entity::Pc(crate::element::ActorPc {
                    element: crate::element::ElementData {
                        kind: crate::element::ElementKind::ActorPc,
                        sprite,
                        // Initial posture from `InitializeAction`.
                        posture: initial_posture,
                        ..Default::default()
                    },
                    actor: crate::element::ActorData {
                        // The per-actor Initialize() dispatch in
                        // `EngineInner::initialize_mission_script` picks
                        // up this string and creates a persistent
                        // ScriptInstance via `MissionScript::bind_actor`.
                        script_class: beam_me.script.clone().unwrap_or_default(),
                        // Action state from `InitializeAction`.
                        action_state: initial_action_state,
                        ..Default::default()
                    },
                    human: crate::element::HumanData {
                        time_hulk: crate::element::HULK_LENGTH,
                        invulnerable: self.control.sim_config.highlander,
                        ..Default::default()
                    },
                    pc: crate::element::PcData {
                        life_points: pc_status.life_points,
                        robin: is_robin,
                        profile_index: profile_idx,
                        list_index,
                        campaign_description_index: Some(char_idx as u32),
                        kind,
                        has_lockpick,
                        has_climb,
                        has_jump,
                        immortal: self.control.sim_config.highlander,
                        beam_me_index: beam_me.index as i16,
                        disabled_actions,
                        disabled_actions_temp: vec![false; crate::profiles::NUMBER_OF_PC_ACTIONS],
                        ammo: crate::element::PcAmmoData {
                            ales: pc_status.num_ales,
                            arrows: pc_status.num_arrows,
                            apples: pc_status.num_apples,
                            rations: pc_status.num_rations,
                            stones: pc_status.num_stones,
                            wasp_nests: pc_status.num_wasp_nests,
                            nets: pc_status.num_nets,
                            plants: pc_status.num_plants,
                            purses: pc_status.num_purses,
                        },
                        // Kept for save/restore parity.
                        initial_action: beam_me.action,
                        ..Default::default()
                    },
                });
                let spawned_eid = self.add_entity(entity);
                pc_count += 1;

                // When the mission is the Sherwood camp, remember the
                // beam-me slot this PC landed on so that a later
                // Sherwood visit can restore the same position;
                // otherwise clear the slot ("goes out of Sherwood =>
                // looses his place").  The write happens here in
                // Phase B because this is the only point we have both
                // the post-shuffle beam-me and the char_idx in scope
                // with a live mut-borrow path to `campaign.characters`.
                if let Some(desc) = self.mission_domain.campaign.characters.get_mut(char_idx) {
                    desc.status.beam_me_index_in_sherwood = if is_sherwood {
                        beam_me.index as i16
                    } else {
                        -1
                    };
                }

                // Sherwood returners get their position + facing
                // jittered by `randomize_position`.  Non-returners keep
                // the beam-me's exact position/facing.
                if let Some(roll) = sherwood_placement_roll {
                    self.apply_randomized_position(spawned_eid, roll);
                }
            } else {
                // No PC for this beam-me — push None to keep script indices aligned.
                self.world.entities.push(None);
            }
        }
        tracing::info!("Spawned {} PCs at beam-me positions", pc_count);

        // Sherwood camp built — clear the mission team so that UI paths
        // that read `mission_team_indices` while the player is back in
        // Sherwood don't see the team from whichever mission we just
        // finished.
        if is_sherwood {
            self.mission_domain.campaign.reset_mission_team();
        }

        Ok(())
    }
}
