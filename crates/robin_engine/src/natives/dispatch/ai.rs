//! AI state, attention, pathing, noise, and avoidance dispatch.

use super::*;

impl NativeContext<'_, '_> {
    pub(super) fn dispatch_ai(&mut self, native: NativeFn, stack: &mut NativeStack) -> i32 {
        use NativeFn::*;

        match native {
            // --- AI ---
            SetAIAlertStatus => {
                // Reject (1) missing actor, (2) PCs, (3)
                // non-NPCs, (4) illegal alert values — each with
                // its own warning + false return.  The actual
                // alert write + music propagation still happens
                // via the per-frame overall-alert sweep.
                let val = stack.pop_i32();
                let actor = stack.pop_i32();
                let Some(entity) = self.get_entity_mut(actor) else {
                    tracing::error!("Script Error: SetAIAlertStatus invalid actor {actor}");
                    return 0;
                };
                if entity.is_pc() {
                    tracing::error!("Script Error: SetAIAlertStatus target {actor} is a PC");
                    return 0;
                }
                if !entity.is_npc() {
                    tracing::error!("Script Error: SetAIAlertStatus target {actor} is not an NPC");
                    return 0;
                }
                let Ok(level) = AlertLevel::try_from(val as u32) else {
                    tracing::error!("Script Error: SetAIAlertStatus illegal alert value {val}");
                    return 0;
                };
                // Route soldiers through the enemy-side wrapper
                // so the forced-attentive view-override is
                // applied; civilians fall through to the base
                // setter (override is soldier-only and would
                // always be `false` for them).
                if let Some(enemy) = entity.enemy_ai_mut() {
                    enemy.set_alert_status(level);
                } else if let Some(ai) = entity.ai_controller_mut() {
                    ai.set_alert_status(level);
                }
                1
            }
            GetAIAlertStatus => {
                let actor = stack.pop_i32();
                // Warn + return false on missing actor / non-NPC.
                let Some(entity) = self.get_entity(actor) else {
                    tracing::error!("Script Error: GetAIAlertStatus invalid actor {actor}");
                    return 0;
                };
                let Some(ai) = entity.ai_controller() else {
                    tracing::error!("Script Error: GetAIAlertStatus target {actor} is not an NPC");
                    return 0;
                };
                // Read the view-parameter alert status — the
                // field that `SetAlertStatus` pins to YELLOW for
                // forced-attentive soldiers on Green.
                ai.view_alert_status as i32
            }
            SetAIState => {
                let val = stack.pop_i32();
                let actor = stack.pop_i32();
                let Some(entity) = self.get_entity_mut(actor) else {
                    tracing::error!("Script Error: SetAIState invalid actor {actor}");
                    return 0;
                };
                if !entity.is_npc() {
                    tracing::error!("Script Error: SetAIState target {actor} is not an NPC");
                    return 0;
                }

                // The public AISTATE_* values are not the internal enum after
                // SEEKING: MENACING=4, FLEEING=5, ATTACKING=6, and the
                // SCRIPT_DRIVEN pseudo-state is 7. Match RHScript's switch
                // literally so rejected states return false without yielding.
                match val {
                    0 => {
                        tracing::error!(
                            "Script Error: Sleeping state cannot be set by script on actor {actor}"
                        );
                        return 0;
                    }
                    2 => {
                        tracing::error!(
                            "Script Error: SetAIState illegal state value {val} on actor {actor}"
                        );
                        return 0;
                    }
                    4 => {
                        tracing::error!(
                            "Script Error: Menacing state cannot be set by script on actor {actor}"
                        );
                        return 0;
                    }
                    6 => {
                        tracing::error!(
                            "Script Error: Attacking state cannot be set by script on actor {actor}"
                        );
                        return 0;
                    }
                    1 | 3 | 5 | 7 => {}
                    _ => {
                        tracing::error!(
                            "Script Error: SetAIState illegal state value {val} on actor {actor}"
                        );
                        return 0;
                    }
                }

                let effect = match (val, entity) {
                    (1, Entity::Soldier(s)) if s.npc.ai_brain.enemy().is_some() => {
                        crate::interp::ScriptAiStateNativeEffect::Default
                    }
                    (1, Entity::Civilian(c)) if c.npc.ai_brain.friendly().is_some() => {
                        crate::interp::ScriptAiStateNativeEffect::Default
                    }
                    (3, Entity::Soldier(s)) if s.npc.ai_brain.enemy().is_some() => {
                        crate::interp::ScriptAiStateNativeEffect::Seeking
                    }
                    (3, Entity::Civilian(_)) => {
                        tracing::error!(
                            "Script Error: SetAIState(SEEKING) on civilian NPC {actor}"
                        );
                        return 0;
                    }
                    (5, Entity::Soldier(s)) if s.npc.ai_brain.enemy().is_some() => {
                        crate::interp::ScriptAiStateNativeEffect::Fleeing
                    }
                    (5, Entity::Civilian(c)) if c.npc.ai_brain.friendly().is_some() => {
                        crate::interp::ScriptAiStateNativeEffect::Fleeing
                    }
                    (7, Entity::Soldier(s)) if s.npc.ai_brain.enemy().is_some() => {
                        crate::interp::ScriptAiStateNativeEffect::ScriptDriven
                    }
                    (7, Entity::Civilian(c)) if c.npc.ai_brain.friendly().is_some() => {
                        crate::interp::ScriptAiStateNativeEffect::ScriptDriven
                    }
                    (_, Entity::Soldier(_)) => {
                        panic!("accepted SetAIState soldier {actor} requires Enemy AI")
                    }
                    (_, Entity::Civilian(_)) => {
                        panic!("accepted SetAIState civilian {actor} requires Friendly AI")
                    }
                    _ => unreachable!("validated SetAIState owner stopped being an NPC"),
                };

                self.pending_yield = Some(crate::interp::NativeYield {
                    operation: crate::interp::NativeOperation::EngineAction(
                        crate::interp::SynchronousScriptRequest::ApplyAiStateNative {
                            actor,
                            effect,
                            native_return: 1,
                        },
                    ),
                    resume: crate::interp::ResumePolicy::Fixed(1),
                });
                1
            }
            GetAIState => {
                let actor = stack.pop_i32();
                self.get_entity(actor).map_or(0, |e| {
                    e.ai_controller()
                        .map_or(0, |ai| ai.current_state.to_script_code())
                })
            }
            SetAIAttitude => {
                // Retired stub: just logs "attitudes are fixed
                // in the profiles and cannot be changed" and
                // returns false.  Profile-sourced attitudes
                // remain authoritative.
                let _val = stack.pop_i32();
                let _actor = stack.pop_i32();
                tracing::warn!("SetAIAttitude called but attitudes are fixed in profiles (no-op)");
                0
            }
            GetAIAttitude => {
                // Switches on the NPC's camp: Royalists → 0
                // (FRIENDLY), Lacklandists → 1 (HOSTILE).
                // Attitude is not a stored field at the script
                // boundary — it is a pure function of camp
                // membership.
                let actor = stack.pop_i32();
                match self.get_entity(actor) {
                    None => {
                        tracing::error!("Script Error: GetAIAttitude invalid actor {actor}");
                        0
                    }
                    Some(e) if !e.is_npc() => {
                        tracing::error!("Script Error: GetAIAttitude target {actor} is not an NPC");
                        0
                    }
                    Some(e) => match e.camp() {
                        Camp::Royalists => 0,
                        Camp::Lacklandists => 1,
                        Camp::Error => 0,
                    },
                }
            }
            SetAILevel => {
                // Retired stub: the body is entirely commented
                // out.  Validate handle + NPC-ness to preserve
                // diagnostic output, but do NOT mutate any state
                // — no field exists and no ported caller reads a
                // derived value.
                let _value = stack.pop_i32();
                let _property = stack.pop_i32();
                let actor = stack.pop_i32();
                match self.get_entity(actor) {
                    None => {
                        tracing::error!("Script Error: SetAILevel invalid actor {actor}");
                        return 0;
                    }
                    Some(e) if !e.is_npc() => {
                        tracing::error!("Script Error: SetAILevel target {actor} is not an NPC");
                        return 0;
                    }
                    _ => {}
                }
                1
            }
            StareActor => {
                // StareActor(Actor npc, Actor target, int duration_frames)
                // Makes npc face toward target for duration frames. 0 = stop staring.
                let duration = stack.pop_i32();
                let target = stack.pop_i32();
                let actor = stack.pop_i32();
                let target_handle = u32::try_from(target)
                    .ok()
                    .and_then(std::num::NonZeroU32::new)
                    .map(std::num::NonZeroU32::get);
                if duration > 0
                    && let Some(target_handle) = target_handle
                    && self.get_entity(target_handle as i32).is_none()
                {
                    tracing::error!("Script Error: StareActor invalid target {target_handle}");
                    return 0;
                }
                if let Some(entity) = self.get_entity_mut(actor)
                    && let Some(ai) = entity.ai_controller_mut()
                {
                    if duration > 0
                        && let Some(target_handle) = target_handle
                    {
                        ai.stare_target_actor = Some(target_handle);
                        ai.stare_target_position = None;
                        ai.stare_remaining = duration as u32;
                    } else {
                        ai.stare_target_actor = None;
                        ai.stare_target_position = None;
                        ai.stare_remaining = 0;
                    }
                }
                0
            }
            StareLocation => {
                // StareLocation(Actor npc, Location loc, int duration_frames)
                // Makes npc face toward a location for duration frames.
                let duration = stack.pop_i32();
                let loc = stack.pop_i32();
                let actor = stack.pop_i32();
                let resolved_pos =
                    self.resolve_location_pos(loc)
                        .map(|(x, y)| crate::ai::Position {
                            x,
                            y,
                            ..Default::default()
                        });
                if let Some(entity) = self.get_entity_mut(actor)
                    && let Some(ai) = entity.ai_controller_mut()
                {
                    if duration > 0 {
                        ai.stare_target_actor = None;
                        ai.stare_target_position = resolved_pos;
                        ai.stare_remaining = duration as u32;
                    } else {
                        ai.stare_target_actor = None;
                        ai.stare_target_position = None;
                        ai.stare_remaining = 0;
                    }
                }
                0
            }
            AssignPath => {
                let way = stack.pop_i32();
                let actor = stack.pop_i32();
                if !self
                    .get_entity(actor)
                    .is_some_and(|entity| entity.ai_controller().is_some())
                {
                    return 0;
                }
                let request = crate::interp::SynchronousScriptRequest::AssignPath {
                    actor,
                    way,
                    native_return: 0,
                };
                self.pending_yield = Some(crate::interp::NativeYield {
                    resume: crate::interp::ResumePolicy::Fixed(request.native_return()),
                    operation: crate::interp::NativeOperation::EngineAction(request),
                });
                0
            }
            AssignPost => {
                // AssignPost(Actor, Location, int direction) -> 0
                // Drops the active patrol path, installs the
                // post as the NPC's new initial-pos /
                // view-direction anchor, clears the three
                // authored flags, and — when not script-locked
                // and in the default state — fires
                // EventReturnToDuty so the NPC walks to the
                // post.
                let direction = stack.pop_i32();
                let loc = stack.pop_i32();
                let actor = stack.pop_i32();
                let Some(resolved_xy) = self.resolve_location_pos(loc) else {
                    tracing::warn!("AssignPost: invalid location handle {loc}");
                    return 0;
                };
                if !self
                    .get_entity(actor)
                    .is_some_and(|entity| entity.ai_controller().is_some())
                {
                    return 0;
                }
                let request = crate::interp::SynchronousScriptRequest::AssignPost {
                    actor,
                    post_x: resolved_xy.0,
                    post_y: resolved_xy.1,
                    direction,
                    native_return: 0,
                };
                self.pending_yield = Some(crate::interp::NativeYield {
                    resume: crate::interp::ResumePolicy::Fixed(request.native_return()),
                    operation: crate::interp::NativeOperation::EngineAction(request),
                });
                0
            }
            ForceBattleDecision => {
                // Soldier-only.  `decision >= 100` peels off an
                // `always` prefix (always=true, decrement by
                // 100) that is translated into
                // `!reset_battle_decision` on the soldier.  The
                // decision ID is mapped through a
                // BATTLE_DECISION_* → `Decision` switch; unknown
                // IDs warn and skip the mutation.
                let mut decision_arg = stack.pop_i32();
                let actor = stack.pop_i32();

                let Some(entity) = self.get_entity_mut(actor) else {
                    tracing::warn!(
                        "Script Error: ForceBattleDecision on illegal actor handle {actor}"
                    );
                    return 0;
                };
                if !entity.is_soldier() {
                    tracing::warn!(
                        "Script Error: ForceBattleDecision on non-soldier actor {actor}"
                    );
                    return 0;
                }

                let b_always = if decision_arg >= 100 {
                    decision_arg -= 100;
                    true
                } else {
                    false
                };

                use crate::ai::Decision;
                let decision = match decision_arg {
                    0 => Decision::Cassos,
                    1 => Decision::Fight,
                    2 => Decision::Observe,
                    3 => Decision::Reserve,
                    4 => Decision::AlertSoldiers,
                    5 => Decision::RunAndAlertSoldiers,
                    6 => Decision::Menace,
                    7 => Decision::Shoot,
                    8 => Decision::ArcherStepBack,
                    9 => Decision::LookForHelp,
                    10 => Decision::LookForHelpIfNobodyElseDoes,
                    11 => Decision::CoverBehindShieldBearer,
                    12 => Decision::TooProudToAttack,
                    13 => Decision::TowerGuardAlert,
                    14 => Decision::TowerGuardObserve,
                    15 => Decision::ArcherObserve,
                    16 => Decision::RunToArcheryPoint,
                    99 => Decision::None,
                    other => {
                        tracing::warn!(
                            "Script Error: Illegal identifier {other} for battle decision."
                        );
                        return 0;
                    }
                };

                if let Some(enemy) = entity.enemy_ai_mut() {
                    enemy.forced_next_battle_decision = decision;
                    enemy.reset_battle_decision = !b_always;
                }
                0
            }
            MakeNoise => {
                // MakeNoise(Location, int id) -> 0
                // `id` is a *script-local* selector —
                // `SCRIPT_NOISE_LOGS = 0`,
                // `SCRIPT_NOISE_DRAWBRIDGE = 1` — NOT the
                // `NoiseType` enum value.  Error order: id check
                // first, then NULL-location check.  (The original
                // would plow on with an uninitialised noise type
                // on a bad id; we drop the noise instead, which
                // is strictly safer.)
                let noise_id = stack.pop_i32();
                let loc = stack.pop_i32();
                let noise_type = match noise_id {
                    0 => crate::ai::NoiseType::Logs,
                    1 => crate::ai::NoiseType::Drawbridge,
                    _ => {
                        tracing::error!("Script Error: Illegal noise ID {noise_id}");
                        return 0;
                    }
                };
                let Some((origin_x, origin_y)) = self.resolve_location_pos(loc) else {
                    tracing::error!("Script error : MakeNoise on NULL-location (handle {loc})");
                    return 0;
                };
                // Emit a deferred command so the engine runs the
                // full `broadcast_noise` path (deafness, state
                // filter, `AddNoiseToDisplay`), identical to the
                // gameplay callsites.
                let Some((layer, sector)) = self.resolve_location_layer_sector(loc) else {
                    tracing::error!(
                        "Script error: MakeNoise location {loc} has no layer/sector metadata"
                    );
                    return 0;
                };
                self.emit_engine(EngineCommand::MakeNoise {
                    noise_type,
                    x: origin_x,
                    y: origin_y,
                    layer,
                    sector,
                });
                tracing::debug!(
                    "MakeNoise: scripted {noise_type:?} at ({origin_x},{origin_y}) \
                     layer {layer} sector {sector}"
                );
                0
            }
            SetPathWalkingStyle => {
                let style = stack.pop_i32();
                let actor = stack.pop_i32();
                // ActorExists + IsNPC guards, both warn +
                // early-return on miss.
                if !self.actor_exists(actor) {
                    tracing::error!(
                        "Script Error: Trying to set path walking style of an invalid actor element."
                    );
                    return 0;
                }
                let Some(entity) = self.get_entity_mut(actor) else {
                    return 0;
                };
                if entity.ai_controller().is_none() {
                    tracing::error!("Script Error: Trying to set path walking style of a non-NPC.");
                    return 0;
                }
                let Some(ai) = entity.ai_controller_mut() else {
                    return 0;
                };
                // The original switch only names WALKING(0)→0
                // and RUNNING(1)→GOTO_RUN — cases 2/3
                // (WALKING_NONINTERRUPTABLE /
                // RUNNING_NONINTERRUPTABLE) leave the flags
                // uninitialised (a bug).  Treat 0/2 as "clear
                // RUN" and 1/3 as "insert RUN" to match the
                // non-buggy intent.
                if style & 1 == 1 {
                    ai.default_path_walking_flags.insert(GotoFlags::RUN);
                } else {
                    ai.default_path_walking_flags.remove(GotoFlags::RUN);
                }
                // SetPathWalkingFlags re-launches the current patrol GoTo
                // inline. Yield to the engine before the VM executes its next
                // statement: a following Thanx() may launch a recorded
                // sequence, and Original registers the speed-change Move
                // before that sequence.
                let needs_relaunch = ai.has_patrol_path
                    && matches!(
                        ai.current_substate,
                        crate::ai::Substate::DefaultGotoRoute | crate::ai::Substate::DefaultEnroute
                    );
                if needs_relaunch {
                    let request = crate::interp::SynchronousScriptRequest::SetPathWalkingStyle {
                        actor,
                        native_return: 0,
                    };
                    self.pending_yield = Some(crate::interp::NativeYield {
                        resume: crate::interp::ResumePolicy::Fixed(request.native_return()),
                        operation: crate::interp::NativeOperation::EngineAction(request),
                    });
                }
                0
            }
            GetSoldierRank => {
                let actor = stack.pop_i32();
                self.get_entity(actor).map_or(0, |e| {
                    if let Some(soldier) = e.soldier_data() {
                        self.bindings
                            .profile_manager
                            .get_soldier(soldier.soldier_profile_index)
                            .map_or(0, |p| p.rank as i32)
                    } else {
                        0
                    }
                })
            }
            SwitchToAlertPath => {
                // Gates on ActorExists + IsSoldier, then:
                //   if (alert_path_id is some) {
                //       changed_to_alert_path = true;
                //       path.init(alert_path_id);
                //       has_patrol_path = true;
                //   }
                //   if (state == Default) {
                //       return_to_duty(sim, );
                //   }
                let actor = stack.pop_i32();
                let Some(entity) = self.get_entity(actor) else {
                    tracing::error!(
                        "Script Error: SwitchToAlertPath with invalid soldier ({actor})"
                    );
                    return 0;
                };
                if !entity.is_soldier() {
                    tracing::error!("Script Error: SwitchToAlertPath with non-soldier ({actor})");
                    return 0;
                }
                let request = crate::interp::SynchronousScriptRequest::SwitchToAlertPath {
                    actor,
                    native_return: 0,
                };
                self.pending_yield = Some(crate::interp::NativeYield {
                    resume: crate::interp::ResumePolicy::Fixed(request.native_return()),
                    operation: crate::interp::NativeOperation::EngineAction(request),
                });
                0
            }
            SetNPCEmoticon => {
                let duration = stack.pop_i32();
                let emoticon_type = stack.pop_i32();
                let actor = stack.pop_i32();
                let frame = self.frame_counter();
                let Some(entity) = self.get_entity_mut(actor) else {
                    tracing::warn!("Script Error: SetNPCEmoticon invalid actor {actor}");
                    return 0;
                };
                if !entity.is_npc() {
                    tracing::warn!("Script Error: SetNPCEmoticon target {actor} is not an NPC");
                    return 0;
                }
                let Ok(et) = EmoticonType::try_from(emoticon_type as u32) else {
                    tracing::warn!(
                        "Script Error: SetNPCEmoticon invalid emoticon id {emoticon_type}"
                    );
                    return 0;
                };
                if let Some(ai) = entity.ai_controller_mut() {
                    // NONE clears the expiration flag and
                    // ignores `duration`; otherwise the
                    // expiration is always written from
                    // `frame + duration` (u16 cast — negative
                    // wraps to a huge unsigned, zero expires
                    // next frame).
                    ai.current_emoticon_type = et;
                    if et == EmoticonType::None {
                        ai.emoticon_has_expiration_date = false;
                    } else {
                        ai.emoticon_has_expiration_date = true;
                        ai.emoticon_expiration_date = frame + (duration as u16) as u32;
                    }
                }
                0
            }
            ForbidNPCRemark => {
                // ForbidNPCRemark(Actor, int remark_id, bool forbid)
                // Adds or removes a remark ID from this NPC's forbidden list.
                // Both trailing arguments are narrowed to a signed byte before
                // they reach the implementation, the same way
                // `SetPersistentProperty` narrows its own.
                let forbid = i32::from(stack.pop_i32() as i8);
                let remark_id = i32::from(stack.pop_i32() as i8);
                let actor = stack.pop_i32();
                if let Some(entity) = self.get_entity_mut(actor)
                    && let Some(ai) = entity.ai_controller_mut()
                {
                    let id = remark_id as u32;
                    if forbid != 0 {
                        if !ai.forbidden_remark_ids.contains(&id) {
                            ai.forbidden_remark_ids.push(id);
                        }
                    } else {
                        ai.forbidden_remark_ids.retain(|&r| r != id);
                    }
                }
                0
            }
            DeclareAsCombatTrainer => {
                // DeclareAsCombatTrainer(Actor) -> 0
                // Two field sets on a soldier:
                // `set_combat_trainer(true)` on the AI and
                // `set_invulnerable(true)` on the human base
                // (the damage/concussion pipeline reads the
                // flag).
                let actor = stack.pop_i32();
                if let Some(entity) = self.get_entity_mut(actor) {
                    if let Some(enemy_ai) = entity.enemy_ai_mut() {
                        enemy_ai.combat_trainer = true;
                    } else {
                        tracing::warn!("DeclareAsCombatTrainer: actor {actor} is not a soldier");
                    }
                    if let Some(human) = entity.human_data_mut() {
                        human.invulnerable = true;
                    }
                }
                0
            }
            AddAsSubordinate => {
                // Gates on eight conditions before mutating,
                // then appends the subordinate to the chief's
                // theoretical patrol (deduped) and triggers an
                // `initialize_patrol` to rebuild the active
                // patrol / missed-members lists and stamp the
                // chief on every accepted minion.
                let subordinate = stack.pop_i32();
                let actor = stack.pop_i32();

                // Guard 1: subordinate exists.
                let Some(sub_entity) = self.get_entity(subordinate) else {
                    tracing::error!(
                        "Script Error: AddAsSubordinate with invalid subordinate ({subordinate})"
                    );
                    return 0;
                };
                // Guard 2: subordinate is an NPC.
                if !sub_entity.is_npc() {
                    tracing::error!(
                        "Script Error: AddAsSubordinate with non-NPC subordinate ({subordinate})"
                    );
                    return 0;
                }
                // Guard 3: subordinate has no existing chief.
                let sub_has_chief = sub_entity
                    .ai_controller()
                    .is_some_and(|ai| ai.patrol_chief.is_some());
                if sub_has_chief {
                    tracing::error!(
                        "Script Error: AddAsSubordinate with subordinate ({subordinate}) who already is in a patrol"
                    );
                    return 0;
                }
                // Guard 4: subordinate is not itself a chief
                // (HasPatrol == !theoretical_patrol.is_empty()).
                let sub_has_patrol = sub_entity
                    .ai_controller()
                    .is_some_and(|ai| !ai.theoretical_patrol.is_empty());
                if sub_has_patrol {
                    tracing::error!(
                        "Script Error: AddAsSubordinate with subordinate ({subordinate}) who is himself a patrol chief"
                    );
                    return 0;
                }

                // Guard 5: chief exists.
                let Some(chief_entity) = self.get_entity(actor) else {
                    tracing::error!("Script Error: AddAsSubordinate with invalid chief ({actor})");
                    return 0;
                };
                // Guard 6: chief is an NPC.
                if !chief_entity.is_npc() {
                    tracing::error!("Script Error: AddAsSubordinate with non-NPC chief ({actor})");
                    return 0;
                }
                // Guard 7: chief has no chief of its own.
                let chief_has_chief = chief_entity
                    .ai_controller()
                    .is_some_and(|ai| ai.patrol_chief.is_some());
                if chief_has_chief {
                    tracing::error!(
                        "Script Error: AddAsSubordinate with chief ({actor}) who is himself in a patrol"
                    );
                    return 0;
                }
                // Guard 8: subordinate ≠ chief.
                if subordinate == actor {
                    tracing::error!(
                        "Script Error: AddAsSubordinate with subordinate ({subordinate}) == chief"
                    );
                    return 0;
                }

                let Some(sub_id) = self.actor_id(subordinate) else {
                    tracing::error!(
                        "Script Error: AddAsSubordinate with invalid subordinate handle {subordinate}"
                    );
                    return 0;
                };
                let mut appended_len = None;
                if let Some(entity) = self.get_entity_mut(actor)
                    && let Some(ai) = entity.ai_controller_mut()
                {
                    // Dedup before pushing — same as the
                    // upstream `add_patrol_member` helper.
                    if !ai.theoretical_patrol.contains(&sub_id) {
                        ai.theoretical_patrol.push(sub_id);
                        // Force the chief's active patrol to be
                        // rebuilt on the next `tick_patrol_coordination`
                        // pass (engine/ai/mod.rs:5121).  The
                        // deferred pass rebuilds the active
                        // patrol lists and stamps the chief on
                        // every accepted member via
                        // `chief_assigns`.
                        ai.patrol.clear();
                        ai.missed_patrol_members.clear();
                        ai.needs_patrol_reinit = true;
                        appended_len = Some(ai.theoretical_patrol.len());
                    }
                }
                // AddPatrolMember only re-initializes the patrol when the
                // member was actually new, and it does so before returning to
                // the mission script. Yield through the typed engine barrier
                // so subsequent natives (notably UnlockAI) observe the
                // subordinate's freshly assigned patrol chief. The roster
                // length is captured here because the barrier drains after the
                // whole script chunk, by which point later appends would
                // otherwise widen this pass beyond what it saw.
                if let Some(member_count) = appended_len {
                    self.emit_barrier(DeferredCommand::AddAsSubordinateInitialize {
                        chief: actor,
                        member_count,
                    });
                }
                0
            }
            RemoveAllSubordinates => {
                let actor = stack.pop_i32();

                let Some(entity) = self.get_entity(actor) else {
                    tracing::error!(
                        "Script Error: RemoveAllSubordinates with invalid chief ({actor})"
                    );
                    return 0;
                };
                if !entity.is_npc() {
                    tracing::error!(
                        "Script Error: RemoveAllSubordinates with non-NPC chief ({actor})"
                    );
                    return 0;
                }

                // ClearPatrol synchronously returns every default-state
                // subordinate to duty before the script continues. That work
                // needs EngineInner + LevelAssets, so suspend this VM at the
                // native boundary and let the engine complete it before the
                // next instruction runs.
                self.pending_yield = Some(crate::interp::NativeYield {
                    operation: crate::interp::NativeOperation::EngineAction(
                        crate::interp::SynchronousScriptRequest::RemoveAllSubordinates {
                            actor,
                            native_return: 0,
                        },
                    ),
                    resume: crate::interp::ResumePolicy::Fixed(0),
                });
                0
            }
            AddRepulsivePoint => {
                // AddRepulsivePoint(Location, float radius, float action_radius, int flags) -> int
                // Creates a repulsive point that NPCs avoid during pathfinding.
                // Returns the auto-generated ID for the new point.
                //
                // Gates on `is_script_point(loc)` and warns +
                // returns 0 for sector-typed locations.
                //
                // Repulsive points carry their script-point
                // layer through `Position.level` so
                // `gather_static_repulsive_points`'s layer
                // filter compares against the authored layer.
                let flags = stack.pop_i32();
                let action_radius = f32::from_bits(stack.pop_i32() as u32);
                let radius = f32::from_bits(stack.pop_i32() as u32);
                let loc = stack.pop_i32();
                if !self.is_script_point(loc) {
                    tracing::error!(
                        "Script Error: AddRepulsivePoint requires a point location (got handle {loc})"
                    );
                    return 0;
                }
                let Some((x, y)) = self.resolve_location_pos(loc) else {
                    tracing::error!(
                        "Script Error: AddRepulsivePoint cannot resolve location {loc}"
                    );
                    return 0;
                };
                let (level, sector_num) = self.resolve_location_layer_sector(loc).unwrap_or((0, 0));
                let position = crate::ai::Position {
                    x,
                    y,
                    level,
                    sector: crate::position_interface::SectorHandle::new(sector_num),
                };
                let ai_global = self.ai_global_mut();
                let id = ai_global.next_repulsive_point_id;
                ai_global.next_repulsive_point_id += 1;
                ai_global
                    .repulsive_points
                    .push(crate::ai::RepulsivePoint::new(
                        id,
                        position,
                        radius,
                        action_radius,
                        flags,
                    ));
                id
            }
            DeleteRepulsivePoint => {
                // DeleteRepulsivePoint(int id) -> 0
                // Removes a repulsive point by its ID.
                let id = stack.pop_i32();
                let ai_global = self.ai_global_mut();
                let before = ai_global.repulsive_points.len();
                ai_global.repulsive_points.retain(|p| p.id != id);
                if ai_global.repulsive_points.len() == before {
                    tracing::warn!("DeleteRepulsivePoint: no point with id {id}");
                }
                0
            }

            _ => self.dispatch_world(native, stack),
        }
    }
}
