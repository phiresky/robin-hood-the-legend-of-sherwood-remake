//! Mission-team, production, campaign, PC, and Rust-extension dispatch.

use super::*;

impl NativeContext<'_, '_> {
    pub(super) fn dispatch_campaign(&mut self, native: NativeFn, stack: &mut NativeStack) -> i32 {
        use NativeFn::*;

        match native {
            // --- mission team ---
            GetPCFromMissionTeam => {
                let idx = stack.pop_i32();
                self.campaign.as_ref().map_or(0, |campaign| {
                    campaign
                        .mission_team_indices
                        .get(idx as usize)
                        .and_then(|&char_idx| campaign.characters.get(char_idx))
                        .and_then(|desc| desc.character_profile_idx)
                        .map_or(0, |pi| u32::from(pi) as i32)
                })
            }
            AddPCToMissionTeam => {
                let actor = stack.pop_i32();
                // If the handle refers to a live entity, it
                // must actually be a PC; non-PCs warn and skip
                // the campaign update + mark.  In a Sherwood HUD
                // context there's no live entity, so the
                // `resolve_profile` fallback via raw profile
                // index is the only signal available.
                let entity_is_pc = self.get_entity(actor).map(|e| e.is_pc());
                let mut added = false;
                if entity_is_pc == Some(false) {
                    tracing::warn!("AddPCToMissionTeam: actor {actor} is not a PC");
                } else {
                    let profile_idx = self.resolve_profile(actor);
                    if let Some(campaign) = self.campaign.as_mut() {
                        if let Some(pi) = profile_idx {
                            if let Some(char_idx) = campaign.get_character_by_profile(pi) {
                                campaign.add_to_mission_team(char_idx);
                                added = true;
                            }
                        } else {
                            tracing::warn!("AddPCToMissionTeam: cannot resolve actor {actor}");
                        }
                    }
                }
                // Mark only on the success branch.
                if added {
                    self.engine.commands.push(EngineCommand::MarkPc {
                        actor_handle: actor,
                    });
                }
                0
            }
            RemovePCFromMissionTeam => {
                let actor = stack.pop_i32();
                // Reject non-PC actors with a warning and skip
                // the update.
                let entity_is_pc = self.get_entity(actor).map(|e| e.is_pc());
                if entity_is_pc == Some(false) {
                    tracing::warn!("RemovePCFromMissionTeam: actor {actor} is not a PC");
                } else {
                    let profile_idx = self.resolve_profile(actor);
                    if let Some(campaign) = self.campaign.as_mut() {
                        if let Some(pi) = profile_idx {
                            if let Some(char_idx) = campaign.get_character_by_profile(pi) {
                                campaign.remove_from_mission_team(char_idx);
                            }
                        } else {
                            tracing::warn!("RemovePCFromMissionTeam: cannot resolve actor {actor}");
                        }
                    }
                }
                0
            }
            GetNumberOfObligatoryPCsInMissionTeam => self.campaign.as_ref().map_or(0, |campaign| {
                let profiles = &self.bindings.profile_manager;
                campaign
                    .next_mission_idx
                    .and_then(|mi| campaign.missions.get(mi))
                    .and_then(|m| m.profile_idx)
                    .and_then(|pi| profiles.missions.get(pi as usize))
                    .map_or(0, |mp| mp.required_character_indices.len() as i32)
            }),
            GetObligatoryPCFromMissionTeam => {
                let idx = stack.pop_i32();
                // Returns a live PC actor handle (not a profile
                // index) for the indexed required-character
                // slot.  Resolve via the inverse of
                // canonical PC entity profile fields.
                let profile_manager = self.bindings.profile_manager.clone();
                let required_profile: Option<u32> = self.campaign.as_ref().and_then(|c| {
                    c.next_mission_idx
                        .and_then(|mi| c.missions.get(mi))
                        .and_then(|m| m.profile_idx)
                        .and_then(|pi| profile_manager.missions.get(pi as usize))
                        .and_then(|mp| mp.required_character_indices.get(idx as usize))
                        .copied()
                });
                if let Some(char_profile_idx) = required_profile {
                    let needle = crate::profiles::CharacterProfileIdx(char_profile_idx);
                    if let Some(handle) = self
                        .pc_handles()
                        .into_iter()
                        .find(|&handle| self.pc_profile_index(handle) == Some(needle))
                    {
                        handle
                    } else {
                        tracing::warn!(
                            "GetObligatoryPCFromMissionTeam: no live PC actor for profile {char_profile_idx}"
                        );
                        0
                    }
                } else {
                    0
                }
            }
            IsPCObligatoryInMissionTeam => {
                let actor = stack.pop_i32();
                let profile_idx = self.resolve_profile(actor);
                self.campaign.as_ref().map_or(0, |campaign| {
                    let profiles = &self.bindings.profile_manager;
                    let Some(pi) = profile_idx else { return 0 };
                    let is_required = campaign
                        .next_mission_idx
                        .and_then(|mi| campaign.missions.get(mi))
                        .and_then(|m| m.profile_idx)
                        .and_then(|mpi| profiles.missions.get(mpi as usize))
                        .is_some_and(|mp| mp.required_character_indices.contains(&u32::from(pi)));
                    if is_required { 1 } else { 0 }
                })
            }
            IsMenToBlazonConversionMode => {
                if self.script_domains.mission_ui.men_to_blazon_conversion_mode {
                    1
                } else {
                    0
                }
            }

            // --- beam-me / spawning ---
            GetNumberOfBeamMes => {
                self.campaign.as_ref().map_or(5, |campaign| {
                    let profiles = &self.bindings.profile_manager;
                    // Only valid from the Sherwood HQ mission.
                    let current_loc = campaign
                        .current_mission_idx
                        .and_then(|idx| campaign.missions.get(idx))
                        .and_then(|m| m.profile_idx)
                        .and_then(|pi| profiles.missions.get(pi as usize))
                        .map(|mp| mp.location);
                    if current_loc != Some(crate::profiles::MissionLocation::Sherwood) {
                        tracing::warn!(
                            "Script error: GetNumberOfBeamMes called from non-Sherwood mission"
                        );
                        return 0;
                    }
                    campaign
                        .next_mission_idx
                        .and_then(|idx| campaign.missions.get(idx))
                        .and_then(|m| m.profile_idx)
                        .and_then(|pi| profiles.missions.get(pi as usize))
                        .map_or(5, |mp| mp.number_of_beam_mes as i32)
                })
            }
            MoveBeamMe => {
                let loc = stack.pop_i32();
                let idx = stack.pop_i32();
                self.move_beam_me(idx, loc);
                0
            }
            GetActorForBeamMe => {
                let idx = stack.pop_i32();
                self.get_actor_for_beam_me(idx)
            }

            // --- production / sector ---
            //
            // The engine drains these queues in
            // `apply_production_registrations` (engine/script.rs) —
            // it resolves each location handle to a script zone
            // sector, sets the sector's production type, and pushes
            // per-sector geometry into the campaign production
            // table.  Nothing to do here beyond queuing.
            RegisterAsProductionSector => {
                let speed = stack.pop_i32();
                let loc = stack.pop_i32();
                let prod_type = stack.pop_i32();
                self.script_domains
                    .production_initialization
                    .sectors
                    .push((prod_type, loc, speed));
                0
            }
            AddProductionPoint => {
                let loc = stack.pop_i32();
                let prod_type = stack.pop_i32();
                self.script_domains
                    .production_initialization
                    .points
                    .push((prod_type, loc));
                0
            }
            GetNumberOfActorsInSector => {
                // Warn when the handle is not a script-sector.
                // Script-location handles are tagged indices laid
                // out `[points..., sectors...]`; a sector payload
                // index is in `[point_count, location_count)`.
                let loc = stack.pop_i32();
                if loc == 0 {
                    return 0;
                }
                if !self.is_script_sector_handle(loc) {
                    tracing::warn!(
                        "Script Error: GetNumberOfActorsInSector on non-sector handle {loc}"
                    );
                    return 0;
                }
                self.zone_occupant_handles(loc)
                    .map_or(0, |occ| occ.len() as i32)
            }
            GetActorInSector => {
                // Same sector-handle type guard as
                // `GetNumberOfActorsInSector`.
                let idx = stack.pop_i32();
                let loc = stack.pop_i32();
                if loc == 0 {
                    return 0;
                }
                if !self.is_script_sector_handle(loc) {
                    tracing::warn!("Script Error: GetActorInSector on non-sector handle {loc}");
                    return 0;
                }
                match self.zone_occupant_handles(loc) {
                    Some(occ) => {
                        if idx >= 0 && (idx as usize) < occ.len() {
                            occ[idx as usize]
                        } else {
                            tracing::warn!(
                                "GetActorInSector: index {idx} out of range (max={})",
                                occ.len()
                            );
                            0
                        }
                    }
                    None => 0,
                }
            }

            // --- blazon / campaign ---
            WinBlazon => {
                let actor = stack.pop_i32();
                self.win_blazon(actor);
                0
            }
            LoseBlazon => {
                let actor = stack.pop_i32();
                self.lose_blazon(actor);
                0
            }
            IsBlazonWon => {
                let actor = stack.pop_i32();
                self.is_blazon_won(actor)
            }
            IsBonusItemPickedUp => {
                let actor = stack.pop_i32();
                self.is_bonus_item_picked_up(actor)
            }
            ConfiscateMoney => {
                let actor = stack.pop_i32();
                self.confiscate_money(actor);
                0
            }
            AddPCToGang => {
                let actor = stack.pop_i32();
                let profile_idx = self.resolve_profile(actor);
                let profiles = self.bindings.profile_manager.clone();
                if let Some(campaign) = self.campaign.as_mut() {
                    if let Some(pi) = profile_idx {
                        if let Some(char_idx) = campaign.get_character_by_profile(pi) {
                            campaign.add_to_gang(char_idx, &profiles);
                            // Also calls `mission_stat.add_new_pc`
                            // — required for the post-mission
                            // stat screen and save data.
                            //
                            // The original passes the PC's
                            // `Status->wsName`, which by then has
                            // either the localized peasant name
                            // from `GenerateName` or the
                            // SPECIAL_PEASANT override stamped
                            // in by `SetPersistentProperty(NAME,
                            // …)`.  We haven't ported
                            // `generate_name`, and `name_override`
                            // resolves through `MenuTextLookup`
                            // at display time, so we capture the
                            // stable profile name as the
                            // fallback (matching the
                            // `mission_stat.remove_new_pc(profile_name)`
                            // key used by the kill cascade in
                            // `engine/melee.rs`) plus the
                            // override slot for the debriefing
                            // render to resolve.
                            if let Some(desc) = campaign.characters.get(char_idx) {
                                let fallback = profiles
                                    .get_character(pi)
                                    .map(|cp| cp.profile_name.clone())
                                    .unwrap_or_default();
                                let name_override = desc.status.name_override;
                                self.mission_stat
                                    .as_mut()
                                    .expect("AddPCToGang requires live mission statistics")
                                    .add_new_pc(fallback, name_override);
                            }
                        }
                    } else {
                        tracing::warn!("AddPCToGang: cannot resolve actor {actor}");
                    }
                }
                0
            }
            AddFarmerToGang => {
                let bow_exp = stack.pop_i32();
                let sword_exp = stack.pop_i32();
                let farmer_type = stack.pop_i32();
                let profiles = self.bindings.profile_manager.clone();
                if let Some(campaign) = self.campaign.as_mut() {
                    // 1-indexed script → 0-indexed profile.
                    let char_idx =
                        campaign.add_new_peasant_to_gang(Some((farmer_type - 1) as u16), &profiles);
                    if let Some(desc) = campaign.characters.get_mut(char_idx) {
                        desc.status.human_status.set_capacity(
                            crate::pc_status::SkillName::HandToHand,
                            sword_exp as u32,
                        );
                        desc.status
                            .human_status
                            .set_capacity(crate::pc_status::SkillName::Bow, bow_exp as u32);
                    }
                }
                0
            }
            SetExperiences => {
                let bow_exp = stack.pop_i32();
                let sword_exp = stack.pop_i32();
                let actor = stack.pop_i32();
                // The original has one backing status here, not separate live and
                // persistent copies. `RHElementActorPC` is constructed with
                // `&pDescription->PCStatus` (`RHelementactorpc.cpp`), and
                // `RHElementActorHuman::SetCapacity` writes through that pointer
                // (`RHelementactorhuman.cpp`). `RHCampaign::Serialize` then writes
                // the same PC descriptions. Rust likewise keeps the PC status on
                // the campaign description, so this actor-scoped call must update
                // that serialized backing state.
                //
                // Validate the actor is a PC handle to surface
                // script bugs that pass NPCs.
                if !matches!(self.get_entity(actor), Some(Entity::Pc(_))) {
                    tracing::warn!("Script error: SetExperiences passed non-PC actor {actor}");
                    return 0;
                }
                let profile_idx = self.resolve_profile(actor);
                if let Some(campaign) = self.campaign.as_mut()
                    && let Some(pi) = profile_idx
                    && let Some(char_idx) = campaign.get_character_by_profile(pi)
                    && let Some(desc) = campaign.characters.get_mut(char_idx)
                {
                    desc.status
                        .human_status
                        .set_capacity(crate::pc_status::SkillName::HandToHand, sword_exp as u32);
                    desc.status
                        .human_status
                        .set_capacity(crate::pc_status::SkillName::Bow, bow_exp as u32);
                }
                0
            }
            TransformHandleTargetToTakeTarget => {
                let actor = stack.pop_i32();
                self.transform_handle_target_to_take_target(actor);
                0
            }

            // --- PC queries ---
            GetRobin => {
                // Returns the first spawned PC where `is_robin`
                // is true, else 0.  The result is a live Actor
                // handle — never a profile index.
                self.robin_handle()
            }
            GetRelic => {
                let idx = stack.pop_i32();
                self.get_relic(idx)
            }
            GetPCType => {
                let actor = stack.pop_i32();
                // Gate on `ActorExists` and `IsPC()` before
                // reading the profile, so passing a junk handle
                // or a non-PC actor surfaces as a distinct
                // warning rather than falling through to the
                // "unknown filename" path.
                if !self.actor_exists(actor) {
                    tracing::warn!("Script Error: Trying to get the PC type of an invalid actor!");
                    return -1;
                }
                if !matches!(self.get_entity(actor), Some(Entity::Pc(_))) {
                    tracing::warn!("Script Error: Trying to get the PC type of a non-PC!");
                    return -1;
                }
                self.campaign.as_ref().expect("campaign required");
                let profile_idx = self.resolve_profile(actor);
                profile_idx
                    .and_then(|pi| self.bindings.profile_manager.get_character(pi))
                    .map_or(-1, |cp| {
                        match cp.filename.as_str() {
                            "RobinTown" | "RobinHood" => 0, // PC_TYPE_ROBIN
                            "LittleJohn" => 1,              // PC_TYPE_JOHN
                            "Friar Tuck" => 2,              // PC_TYPE_TUCK
                            "Stuteley" => 3,                // PC_TYPE_STUTELEY
                            "WillScarlet" => 4,             // PC_TYPE_SCARLET
                            "LadyMarian" => 5,              // PC_TYPE_MARIAN
                            "MerryManA" => 6,               // PC_TYPE_FARMER_A
                            "MerryManB" => 7,               // PC_TYPE_FARMER_B
                            "MerryManC" => 8,               // PC_TYPE_FARMER_C
                            _ => {
                                tracing::warn!(
                                    "Script Error: PC with unknown type! (filename '{}')",
                                    cp.filename
                                );
                                -1
                            }
                        }
                    })
            }
            SelectActorPC => {
                let select = stack.pop_i32();
                let actor = stack.pop_i32();
                if actor != 0 && !matches!(self.get_entity(actor), Some(Entity::Pc(_))) {
                    tracing::warn!("Script Error: Trying to select an invalid or non-PC actor!");
                    return 0;
                }
                self.apply_script_selection(actor, select != 0);
                self.simulation_barriers
                    .commands
                    .push(DeferredCommand::SelectPC {
                        actor,
                        select: select != 0,
                    });
                0
            }
            IsPCSelected => {
                let actor = stack.pop_i32();
                // Returns true on validation failure (invalid
                // actor or non-PC handle) after warning, so
                // scripts that null-check the PC don't
                // infinite-loop.
                if !matches!(self.get_entity(actor), Some(Entity::Pc(_))) {
                    tracing::warn!("Script Error: The Actor in IsPCSelected is invalid.");
                    return 1;
                }
                if self.selected_pc_handles().contains(&actor) {
                    1
                } else {
                    0
                }
            }
            GetNumberOfSelectedPCs => self.selected_pc_handles().len() as i32,
            GetSelectedPC => {
                let idx = stack.pop_i32();
                // Logs a warning when the index is out of range
                // before returning NULL.  Treat negative indices
                // as out-of-range too.
                let selected = self.selected_pc_handles();
                if idx < 0 || (idx as usize) >= selected.len() {
                    tracing::error!(
                        "Script Error: GetSelectedPC index {idx} out of range (count {})",
                        selected.len()
                    );
                    return 0;
                }
                selected.get(idx as usize).copied().unwrap_or(0)
            }
            // ── Spellforge / Lua-only natives ──
            Reveal => {
                // Spellforge name for "make this actor visible
                // (un-blip)". Behaves like `UnBlip` but is the
                // imperative-style name surfaced to Lua. Returns
                // 1 iff the actor was previously blipped.
                let actor = stack.pop_i32();
                if !self.actor_exists(actor) {
                    tracing::warn!("Script Error: Reveal with invalid actor handle {actor}");
                    return 0;
                }
                let was_blipped = self
                    .get_entity(actor)
                    .is_some_and(|e| e.element_data().blipped);
                if let Some(entity) = self.get_entity_mut(actor) {
                    entity.reveal_blip();
                }
                i32::from(was_blipped)
            }
            SequenceReveal => {
                // Sequence-recorded variant of `Reveal`. Same
                // semantics as `RecordUnBlip` — emits a `Unblip`
                // sequence element at the current recording
                // level.
                let actor = stack.pop_i32();
                if !self.is_actor_handle(actor) {
                    tracing::warn!("Script Error: SequenceReveal illegal actor handle {actor}");
                    return 0;
                }
                let level = self.recording_level();
                let elem = SequenceElement::new(level, Command::Unblip, self.actor_id(actor));
                self.record_element(elem)
            }
            IsActorOutOfAction => {
                // Spellforge's English name for `IsActorHS`
                // (Hors Service). Returns true iff the actor is
                // dead, tied, or unconscious. Same semantics as
                // `IsActorHS` — kept in lockstep to avoid drift
                // if scripts mix the two names.
                let actor = stack.pop_i32();
                let Some(e) = self.get_entity(actor) else {
                    tracing::warn!(
                        "Script Error: IsActorOutOfAction with invalid actor handle {actor}"
                    );
                    return 0;
                };
                if !e.is_actor() {
                    tracing::warn!(
                        "Script Error: IsActorOutOfAction with non-actor handle {actor}"
                    );
                    return 0;
                }
                let posture = e.element_data().posture;
                let dead = e.is_dead();
                let tied = posture == Posture::Tied;
                let unconscious = e.human_data().is_some_and(|h| h.unconscious);
                i32::from(dead || tied || unconscious)
            }
            AddObjective => {
                let is_main = stack.pop_i32();
                let id = stack.pop_i32();
                self.short_briefings
                    .as_mut()
                    .expect("AddObjective requires live mission objectives")
                    .add(id as u32, is_main != 0);
                0
            }
            CompleteObjective => {
                let id = stack.pop_i32();
                self.short_briefings
                    .as_mut()
                    .expect("CompleteObjective requires live mission objectives")
                    .mark_done(id as u32);
                0
            }
            SetPatrolShouldRun => {
                // This Rust extension has never had a patrol-domain field or
                // consumer in the port. Retain its registry ABI but do not
                // manufacture a write-only queue. Implementing the AI behavior
                // is intentionally outside this architecture-only cleanup.
                let _should_run = stack.pop_i32();
                let _actor = stack.pop_i32();
                // TODO(ai parity): implement once patrol run state has a
                // canonical owner and an Original/Spellforge behavior oracle.
                0
            }
            ComputeLocationBetween => {
                // Both args must be points; both must share
                // layer and sector.  The result inherits pA's
                // layer/sector.
                let lambda_bits = stack.pop_i32();
                let loc_b = stack.pop_i32();
                let loc_a = stack.pop_i32();
                let lambda = f32::from_bits(lambda_bits as u32);
                if !self.is_script_point(loc_a) || !self.is_script_point(loc_b) {
                    tracing::error!(
                        "Script Error in ComputeLocationBetween: non-point handle(s) {loc_a}, {loc_b}"
                    );
                    return 0;
                }
                let layer_sector_a = self.resolve_location_layer_sector(loc_a);
                let layer_sector_b = self.resolve_location_layer_sector(loc_b);
                // If both sides resolve to a layer/sector, they
                // must match.  If only one (or neither) resolves
                // — e.g. computed locations that inherited no
                // metadata — we accept the call and inherit
                // whatever metadata is available; the source
                // point just carries its own layer/sector
                // forward.
                if let (Some(a), Some(b)) = (layer_sector_a, layer_sector_b)
                    && a != b
                {
                    tracing::error!(
                        "Script Error in ComputeLocationBetween: locations span different layers/sectors (a={a:?}, b={b:?})"
                    );
                    return 0;
                }
                match (
                    self.resolve_location_pos(loc_a),
                    self.resolve_location_pos(loc_b),
                ) {
                    (Some(pos_a), Some(pos_b)) => {
                        let x = pos_a.0 + lambda * (pos_b.0 - pos_a.0);
                        let y = pos_a.1 + lambda * (pos_b.1 - pos_a.1);
                        // Inherit layer/sector from pA.  If pA
                        // is a computed location with no
                        // metadata, the result also has none.
                        self.create_computed_location_full(x, y, layer_sector_a)
                    }
                    _ => {
                        tracing::warn!(
                            "ComputeLocationBetween: invalid location handle(s) {loc_a}, {loc_b}"
                        );
                        0
                    }
                }
            }
            _ => unreachable!("native {native:?} has no immediate dispatch domain"),
        }
    }
}
