//! Script-global, registry-handle, mission-state, and foundational dispatch.

use super::*;

impl NativeContext<'_, '_> {
    pub(super) fn dispatch_script_core(
        &mut self,
        native: NativeFn,
        stack: &mut NativeStack,
    ) -> i32 {
        use NativeFn::*;

        match native {
            // --- victory ---
            ForceCheckVictory => {
                self.script_domains.mission_ui.force_check = true;
                0
            }

            // --- globals ---
            InitGlobal => {
                let value = stack.pop_i32();
                let id = stack.pop_i32();
                self.script_state.globals.insert(id, value);
                0
            }
            SetGlobal => {
                let value = stack.pop_i32();
                let id = stack.pop_i32();
                // Script globals must be created by InitGlobal
                // first; SetGlobal on an un-init'd id warns and
                // no-ops.
                if let std::collections::btree_map::Entry::Occupied(mut e) =
                    self.script_state.globals.entry(id)
                {
                    e.insert(value);
                } else {
                    tracing::warn!("Script Error: Non-valid ID for script global {id}");
                }
                0
            }
            GetGlobal => {
                let id = stack.pop_i32();
                // Returns -1 with a warning on an un-init'd id.
                match self.script_state.globals.get(&id) {
                    Some(v) => *v,
                    None => {
                        tracing::warn!("Script Error: Non-valid ID for script global {id}");
                        -1
                    }
                }
            }

            // --- sequence manager ---
            Start => {
                // If a recording is already active, warn and
                // return 0 *without mutating state*; otherwise
                // allocate, set sequence_level = 1, return 1.
                if self.script_state.sequence_recorder.recording.is_some() {
                    tracing::error!(
                        "Script error in Start: cannot start a new record sequence while another is still being recorded"
                    );
                    0
                } else {
                    self.script_state.sequence_recorder.recording = Some(RecordingSession::new());
                    self.script_state.sequence_recorder.sequence_id = 1;
                    1
                }
            }
            Thanx => {
                // Errors out on "no active recording" (false) and
                // on "empty recording" (false).  Happy path
                // launches the sequence and returns true.
                if let Some(rec) = self.script_state.sequence_recorder.recording.take() {
                    match rec.finalize() {
                        Some(seq) => {
                            self.launch_script_sequence(seq);
                            1
                        }
                        None => {
                            tracing::error!("Script Error: Trying to launch an empty sequence");
                            0
                        }
                    }
                } else {
                    tracing::error!(
                        "Script error in Thanx: End a sequence recording without ever started it"
                    );
                    0
                }
            }
            Then => {
                // If there's no active recording (sequence_level
                // < 1), warn and return 0 *without mutating
                // state*; else advance the level and return the
                // current sequence level.
                if let Some(rec) = &mut self.script_state.sequence_recorder.recording {
                    let level = rec.advance_level();
                    self.script_state.sequence_recorder.sequence_id = level as i32;
                    level as i32
                } else {
                    tracing::error!(
                        "Script error in Then: called outside of a Start/Thanx sequence"
                    );
                    0
                }
            }

            // --- pure functions ---
            IsNull => {
                let h = stack.pop_i32();
                if h == 0 { 1 } else { 0 }
            }
            IsActorEqual => {
                let b = stack.pop_i32();
                let a = stack.pop_i32();
                if a == b { 1 } else { 0 }
            }
            IsActorDead => {
                // Gated on `ActorExists && IsHuman`, returns
                // life-points <= 0.  Use the life-points-based
                // `Element::is_dead()` rather than posture so the
                // native flips true the moment HP reaches 0,
                // before the death animation rewrites posture.
                let actor = stack.pop_i32();
                self.get_entity(actor)
                    .map_or(0, |e| i32::from(e.human_data().is_some() && e.is_dead()))
            }
            IsActorKO => {
                let actor = stack.pop_i32();
                self.get_entity(actor).map_or(0, |e| {
                    i32::from(e.human_data().is_some_and(|h| h.unconscious))
                })
            }
            IsActorTied => {
                let actor = stack.pop_i32();
                self.get_entity(actor)
                    .map_or(0, |e| i32::from(e.element_data().posture == Posture::Tied))
            }
            IsActorHS => {
                // `ActorExists && IsHuman` then
                // `IsDead() || IsTied() || IsUnconscious()`.
                // (The previous arm read `in_honolulu`, an
                // off-map flag, which broke any mission script
                // gating on incapacitation.)
                let actor = stack.pop_i32();
                let Some(e) = self.get_entity(actor) else {
                    tracing::warn!("Script Error: IsActorHS with invalid actor handle {actor}");
                    return 0;
                };
                if !e.is_actor() {
                    tracing::warn!("Script Error: IsActorHS with non-actor handle {actor}");
                    return 0;
                }
                let posture = e.element_data().posture;
                let dead = e.is_dead();
                let tied = posture == Posture::Tied;
                let unconscious = e.human_data().is_some_and(|h| h.unconscious);
                i32::from(dead || tied || unconscious)
            }

            // --- actor stop / activation ---

            // Cancels the actor's current sequence element and any
            // pending sequence elements at script priority.  The
            // sequence manager lives on the engine, so we queue a
            // deferred command.
            StopActor => {
                let actor = stack.pop_i32();
                if self.get_entity(actor).is_some_and(|e| e.is_actor()) {
                    self.simulation_barriers
                        .commands
                        .push(DeferredCommand::StopActor { actor });
                } else {
                    tracing::warn!("StopActor: invalid or non-actor handle {actor}");
                }
                0
            }

            // God is null (as everybody knows).  Used by scripts as
            // a sentinel actor handle in conditional logic.
            God => 0,

            // Only codes 31 (select-all) and 0 (unselect-all) are
            // supported — other values warn and return.
            Select => {
                let code = stack.pop_i32();
                match code {
                    31 => {
                        self.apply_script_selection(0, true);
                        self.simulation_barriers
                            .commands
                            .push(DeferredCommand::SelectPC {
                                actor: 0,
                                select: true,
                            });
                    }
                    0 => {
                        self.apply_script_selection(0, false);
                        self.simulation_barriers
                            .commands
                            .push(DeferredCommand::SelectPC {
                                actor: 0,
                                select: false,
                            });
                    }
                    _ => tracing::warn!(
                        "Select: only codes 31 (select all) and 0 (unselect all) supported, got {code}"
                    ),
                }
                // Returns 1 unconditionally (including the warn
                // branch).
                1
            }

            // Deactivate dispatch:
            //   - Mobile  → SetActiveAll(false) (propagates to sub-sprites)
            //   - PC      → SetPlayable(false) + clear quick-action icons
            //   - General → SetActive(false)
            Deactivate => {
                let actor = stack.pop_i32();
                if self.script_activate_actor(actor, false) {
                    1
                } else {
                    0
                }
            }

            // Inverse of Deactivate.
            Activate => {
                let actor = stack.pop_i32();
                if self.script_activate_actor(actor, true) {
                    1
                } else {
                    0
                }
            }

            // --- AI control ---

            // For NPCs sets the AI script-lock flag (with a
            // "remember events" bit for replaying stimuli on
            // unlock).  PCs produce a script error (no-op here).
            LockAI => {
                let remember = stack.pop_i32() != 0;
                let actor = stack.pop_i32();
                self.script_lock_ai(actor, remember);
                0
            }

            // Inverse of LockAI.  The animal-kick branch is gone
            // with the rest of the animal system.
            UnlockAI => {
                let actor = stack.pop_i32();
                self.script_unlock_ai(actor);
                0
            }

            // Sets the NPC or PC freeze flag, which causes the
            // actor's per-frame Hourglass tick to early-return.
            Freeze => {
                let freeze = stack.pop_i32() != 0;
                let actor = stack.pop_i32();
                self.script_freeze_actor(actor, freeze);
                0
            }

            // Flips the engine-global freeze flag which gates AI,
            // combat, movement, and animation ticks.  Deferred to
            // avoid needing engine access.
            FreezeAll => {
                let freeze = stack.pop_i32() != 0;
                self.simulation_barriers
                    .commands
                    .push(DeferredCommand::FreezeAll { freeze });
                0
            }

            // --- location / distance ---
            NoWhere => 0,
            GetDistance => {
                // Both arguments must be points; on a sector or
                // null handle, warn and return 0.
                let loc_b = stack.pop_i32();
                let loc_a = stack.pop_i32();
                if !self.is_script_point(loc_a) {
                    tracing::error!(
                        "Script Error: 1st argument of GetDistance is no point (handle {loc_a})"
                    );
                    0
                } else if !self.is_script_point(loc_b) {
                    tracing::error!(
                        "Script Error: 2nd argument of GetDistance is no point (handle {loc_b})"
                    );
                    0
                } else {
                    match (
                        self.resolve_location_pos(loc_a),
                        self.resolve_location_pos(loc_b),
                    ) {
                        (Some(pos_a), Some(pos_b)) => {
                            let dx = pos_b.0 - pos_a.0;
                            let dy = pos_b.1 - pos_a.1;
                            (dx * dx + dy * dy).sqrt() as i32
                        }
                        _ => {
                            tracing::warn!(
                                "GetDistance: invalid location handle(s) {loc_a}, {loc_b}"
                            );
                            0
                        }
                    }
                }
            }
            Rand => {
                let max = stack.pop_i32();
                crate::sim_rng::script_rand(crate::sim_rng::RngSite::ScriptRand, max)
                    .unwrap_or_else(|error| panic!("{error}"))
            }
            PrintConsole => {
                // Originally blits "%d\n" into the in-game
                // debug-console overlay.  Closest analogue here is
                // a tracing line — the debug overlay is dev-only.
                let value = stack.pop_i32();
                tracing::info!(target: "rh_script_console", "{value}");
                0
            }

            // --- custom values (campaign-backed) ---
            // Range-check id against script-side index 0..=19
            // (CUSTOM_VALUE_1 .. CUSTOM_VALUE_20) and warn+return
            // on out-of-range.
            GetCustomCampaignValue => {
                let id = stack.pop_i32();
                let Some(value) = crate::campaign::CampaignValue::custom(id) else {
                    tracing::warn!("GetCustomCampaignValue: invalid index {id} (must be 0..=19)");
                    return 0;
                };
                self.campaign
                    .as_ref()
                    .expect("GetCustomCampaignValue requires an active campaign")
                    .values[value]
            }
            SetCustomCampaignValue => {
                let value = stack.pop_i32();
                let id = stack.pop_i32();
                let Some(slot) = crate::campaign::CampaignValue::custom(id) else {
                    tracing::warn!("SetCustomCampaignValue: invalid index {id} (must be 0..=19)");
                    return 0;
                };
                self.campaign
                    .as_mut()
                    .expect("SetCustomCampaignValue requires an active campaign")
                    .values[slot] = value;
                0
            }
            // Validate id in script-side range 0..=9
            // (CUSTOM_NPC_VALUE_1 .. CUSTOM_NPC_VALUE_10),
            // ActorExists, and IsNPC; each with a warn + return
            // -1 on failure.
            GetCustomNPCValue => {
                let id = stack.pop_i32();
                let actor = stack.pop_i32();
                if !(0..=9).contains(&id) {
                    tracing::warn!("GetCustomNPCValue: invalid index {id} (must be 0..=9)");
                    return -1;
                }
                match self.get_entity(actor) {
                    None => {
                        tracing::warn!("GetCustomNPCValue: actor {actor} does not exist");
                        -1
                    }
                    Some(e) if !e.is_npc() => {
                        tracing::warn!("GetCustomNPCValue: actor {actor} is not an NPC");
                        -1
                    }
                    Some(entity) => {
                        entity
                            .npc_data()
                            .expect("GetCustomNPCValue validated an NPC")
                            .custom_values[id as usize]
                    }
                }
            }
            SetCustomNPCValue => {
                let value = stack.pop_i32();
                let id = stack.pop_i32();
                let actor = stack.pop_i32();
                if !(0..=9).contains(&id) {
                    tracing::warn!("SetCustomNPCValue: invalid index {id} (must be 0..=9)");
                    return 0;
                }
                match self.get_entity_mut(actor) {
                    None => {
                        tracing::warn!("SetCustomNPCValue: actor {actor} does not exist");
                    }
                    Some(e) if !e.is_npc() => {
                        tracing::warn!("SetCustomNPCValue: actor {actor} is not an NPC");
                    }
                    Some(entity) => {
                        entity
                            .npc_data_mut()
                            .expect("SetCustomNPCValue validated an NPC")
                            .custom_values[id as usize] = value;
                    }
                }
                0
            }

            // --- bitwise ops ---
            BitwiseAnd => {
                let b = stack.pop_i32();
                let a = stack.pop_i32();
                a & b
            }
            BitwiseOr => {
                let b = stack.pop_i32();
                let a = stack.pop_i32();
                a | b
            }
            BitwiseXor => {
                let b = stack.pop_i32();
                let a = stack.pop_i32();
                a ^ b
            }

            // --- PC actions ---
            HasAnyPCActionWhoIsInThisLevelOrCouldMaybeComeFromSherwood => {
                // Iterates the spawned PCs (not the campaign-wide
                // gang list).  For each live PC carrying the
                // requested action it returns true if the PC is
                // alive; otherwise checks for a Sherwood
                // replacement (non-VIP, portrait still displayed,
                // profile in Sherwood).  The portrait-displayed
                // gate tracks "death is recent / corpse still
                // active in the UI" — i.e. the PC entity is still
                // spawned in the level.  Iterating `pc_handles`
                // (live PC entities, alive or corpse) is the
                // natural source: once the corpse is despawned
                // the PC drops out of entity storage.
                let action_code = stack.pop_i32();
                let Ok(script_action) = crate::profiles::ScriptAction::try_from(action_code as u32)
                else {
                    tracing::warn!("Script Error: HasAnyPCAction with bad action ID {action_code}");
                    return 0;
                };
                let action = script_action.to_action();

                let Some(campaign) = self.campaign.as_ref() else {
                    return 0;
                };
                let profiles = &self.bindings.profile_manager;

                for handle in self.pc_handles() {
                    let Some(profile_idx) = self.pc_profile_index(handle) else {
                        continue;
                    };
                    let Some(cp) = profiles.get_character(profile_idx) else {
                        continue;
                    };

                    let has_action =
                        cp.actions.contains(&action) || cp.contextual_actions.contains(&action);
                    if !has_action {
                        continue;
                    }

                    let is_dead = self.get_entity(handle).is_some_and(|e| e.is_dead());

                    if !is_dead {
                        return 1;
                    }

                    // Dead PC — can we get a replacement from Sherwood?
                    // (Live entity proxy already covers the
                    // portrait-displayed guard.)
                    if !cp.vip && campaign.is_in_sherwood(profile_idx) {
                        return 1;
                    }
                }

                0
            }
            // --- profile/campaign-backed queries (reading real data) ---
            // Returns the count of PC actors currently spawned in
            // the running mission, not the campaign roster.
            GetNumberOfPCs => self.pc_handles().len() as i32,
            GetPC => {
                // Returns a live PC actor handle that scripts pass
                // straight into other natives.  Indexes the
                // canonical entity storage.
                let idx = stack.pop_i32();
                if idx < 0 {
                    0
                } else {
                    self.pc_handles().get(idx as usize).copied().unwrap_or(0)
                }
            }
            GetRansomMoney => self
                .campaign
                .as_ref()
                .map(|c| c.get_value(crate::campaign::CampaignValue::Ransom))
                .unwrap_or_else(|| {
                    tracing::warn!("Script Error: GetRansomMoney called outside campaign mode");
                    -1
                }),
            SetRansomMoney => {
                let val = stack.pop_i32();
                if self.campaign.is_some() {
                    let frame_counter = self.frame_counter();
                    self.set_campaign_value(
                        crate::campaign::CampaignValue::Ransom,
                        val,
                        frame_counter,
                    );
                } else {
                    tracing::warn!("Script Error: SetRansomMoney called outside campaign mode");
                }
                0
            }
            GetDifficultyLevel => crate::player_profile::DifficultyLevel::current().to_u32() as i32,
            GetSizeOfMissionTeam => self
                .campaign
                .as_ref()
                .map_or(0, |c| c.get_size_of_mission_team() as i32),
            // Forwards to `Campaign::is_mission_team_valid`.
            // Returns 0 when no campaign/next-mission context is
            // established (script calling before a mission is
            // chosen) so SCB doesn't see a spurious "team valid".
            IsMissionTeamValid => {
                let profiles = self.bindings.profile_manager.clone();
                self.campaign.as_ref().map_or(0, |c| {
                    if c.next_mission_idx.is_some() {
                        c.is_mission_team_valid(&profiles) as i32
                    } else {
                        0
                    }
                })
            }
            GetNumberOfPCsAlive => {
                // Iterate the loaded PC roster and count those
                // with life-points > 0 — a per-mission, per-tick
                // aliveness count, using canonical entity storage and
                // the life-points-based `Entity::is_dead`.
                self.pc_handles()
                    .iter()
                    .filter(|&&h| self.get_entity(h).is_some_and(|e| !e.is_dead()))
                    .count() as i32
            }
            AreAllBlazonsWon => {
                // Compares the live blazon inventory against the
                // campaign's max, so a spent/lost blazon can flip
                // it back to false even if its mission is still
                // marked done.
                let profiles = self.bindings.profile_manager.clone();
                self.campaign.as_ref().map_or(0, |campaign| {
                    let current = campaign.get_value(crate::campaign::CampaignValue::Blazon);
                    let max = campaign.get_max_number_of_blazons(&profiles) as i32;
                    if current >= max { 1 } else { 0 }
                })
            }
            SecretAgentsAreBackInSherwood => self
                .campaign
                .as_ref()
                .map_or(0, |c| if c.are_reservists_back() { 1 } else { 0 }),
            // Returns the packed 16-bit mission ID (e.g.
            // `'A','1' → 0x3141`), NOT the sequential profile_idx.
            GetLastPlayedMission => self.campaign.as_ref().map_or(0, |campaign| {
                campaign
                    .last_mission_idx
                    .and_then(|idx| campaign.missions.get(idx))
                    .and_then(|m| m.profile_idx)
                    .and_then(|pi| self.bindings.profile_manager.missions.get(pi as usize))
                    .map_or(0, |mp| mp.id as i32)
            }),
            GetNextPlayedMission => self.campaign.as_ref().map_or(0, |campaign| {
                campaign
                    .next_mission_idx
                    .and_then(|idx| campaign.missions.get(idx))
                    .and_then(|m| m.profile_idx)
                    .and_then(|pi| self.bindings.profile_manager.missions.get(pi as usize))
                    .map_or(0, |mp| mp.id as i32)
            }),

            // --- entity handle / script lookup ---
            // Handles are opaque non-null VM values with 0-based payload indices.
            // C++ appends its separate mobile-master array after the normal
            // script-element array. Rust exposes the first masked child as
            // the opaque handle while retaining the original script index.
            GetActorScript => {
                let idx = stack.pop_i32();
                let script_count = self.standard_actor_script_count();
                if idx == -1 {
                    0
                } else if idx < 0 {
                    panic!("GetActorScript: negative actor ID {idx}");
                } else if (idx as usize) < script_count {
                    if self.entities.id_at_legacy_slot(idx as u32).is_some() {
                        Self::actor_handle_from_index(idx as usize)
                    } else {
                        // legacy implementation returns `marrayElementsScript[idx]`
                        // directly; in-range NULL entries are valid
                        // placeholders (e.g. unfilled BeamMe slots)
                        // and return a null script handle without an
                        // SBError.
                        0
                    }
                } else if let Some(owner) = self.mobile_owner_id(idx - script_count as i32) {
                    Self::actor_handle_from_index(owner.index() as usize)
                } else {
                    tracing::debug!("Script Error: invalid actor ID {idx} (normal={script_count})");
                    0
                }
            }
            GetDoorScript => Self::script_index_to_handle(
                stack.pop_i32(),
                self.script_domains.interactables.doors.len(),
                "door",
                ScriptHandleKind::Door,
            ),
            GetPatchScript => Self::script_index_to_handle(
                stack.pop_i32(),
                self.script_domains.interactables.patches.len(),
                "patch",
                ScriptHandleKind::Patch,
            ),
            GetLocationScript => Self::script_index_to_handle(
                stack.pop_i32(),
                self.bindings.script_location_count,
                "location",
                ScriptHandleKind::Location,
            ),
            GetSoundSourceScript => {
                // If the slot was nulled by a prior
                // `DestroySoundSource`, log the "already been
                // destroyed" error and return NULL.  The generic
                // `script_index_to_handle` only bounds-checks
                // `sources.len()`, which `delete` does not shrink
                // — so query canonical per-slot liveness and overlay any
                // destruction queued earlier in this callback.
                let idx = stack.pop_i32();
                if idx == -1 {
                    0
                } else if idx >= 0 && (idx as usize) < self.sound_source_count() {
                    if self.sound_source_alive(idx as usize) {
                        Self::sound_source_handle_from_index(idx as usize)
                    } else {
                        tracing::error!(
                            "Script Error: trying to get a sound source that has already been destroyed ({idx})"
                        );
                        0
                    }
                } else {
                    tracing::error!(
                        "Script Error: invalid sound source ID {idx} (max={})",
                        self.sound_source_count()
                    );
                    0
                }
            }
            GetBuildingScript => Self::script_index_to_handle(
                stack.pop_i32(),
                self.bindings.script_building_count,
                "building",
                ScriptHandleKind::Building,
            ),
            GetWayScript => Self::script_index_to_handle(
                stack.pop_i32(),
                self.bindings.script_hiking_path_count,
                "way",
                ScriptHandleKind::Way,
            ),

            // --- Reverse index lookup (handle → script index) ---
            //
            // There is a separate native per object type, but the
            // All reverse lookups decode the tagged 0-based payload.
            // `GetSoundSourceIndex` also gates
            // on the sound subsystem being ready and on canonical
            // per-slot liveness — split out below.
            GetActorIndex | GetDoorIndex | GetPatchIndex | GetLocationIndex | GetBuildingIndex
            | GetWayIndex => {
                let handle = stack.pop_i32();
                let idx = match native {
                    GetActorIndex => self.actor_script_index(handle),
                    GetDoorIndex => Self::door_index(handle),
                    GetPatchIndex => Self::patch_index(handle),
                    GetLocationIndex => Self::location_index(handle),
                    GetBuildingIndex => Self::building_index(handle),
                    GetWayIndex => Self::way_index(handle),
                    _ => unreachable!(),
                };
                idx.map_or(-1, |i| i as i32)
            }
            GetSoundSourceIndex => {
                //   - start with idx = -1
                //   - only proceed if the sound subsystem is ready
                //   - look up the handle against the live
                //     sound-source array; an unknown source logs
                //     and still returns -1.
                let handle = stack.pop_i32();
                let Some(idx) = Self::sound_source_index(handle) else {
                    return -1;
                };
                // Proxy for "sound is ready": no slots ⇒ no sound
                // subsystem in this build/level.
                if self.sound_source_count() == 0 {
                    return -1;
                }
                if idx >= self.sound_source_count() || !self.sound_source_alive(idx) {
                    tracing::error!(
                        "ScriptError: unknown sound source in GetSoundSourceIndex (handle {handle})"
                    );
                    return -1;
                }
                idx as i32
            }

            _ => self.dispatch_sequences(native, stack),
        }
    }
}
