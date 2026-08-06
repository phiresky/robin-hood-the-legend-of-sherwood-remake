//! Actor identity, state, visibility, interaction, and property dispatch.

use super::*;

impl NativeContext<'_, '_> {
    pub(super) fn dispatch_actors(&mut self, native: NativeFn, stack: &mut NativeStack) -> i32 {
        use NativeFn::*;

        match native {
            // --- entity type checks ---
            // Original: original-code/RHScript.cpp, RHScript::ThisActor
            // returns the callback's pScriptThis verbatim.
            ThisActor => self.call_frame.script_this(),
            // Original: original-code/RHScript.cpp,
            // RHScript::GetNumberOfActorsInEngine returns
            // marrayElementsScript.Size().
            GetNumberOfActorsInEngine => self.entities.len() as i32,
            IsActorAnimation => {
                let handle = stack.pop_i32();
                if handle == 0 {
                    return 0;
                }
                match self.get_entity(handle) {
                    Some(e) if e.is_fx() => 1,
                    _ => 0,
                }
            }
            IsActorObject => {
                let handle = stack.pop_i32();
                if handle == 0 {
                    return 0;
                }
                if !self.actor_exists(handle) {
                    tracing::error!(
                        "Script error (IsActorObject): invalid actor handle {handle:#x}"
                    );
                    return 0;
                }
                if self.get_entity(handle).unwrap().is_object() {
                    1
                } else {
                    0
                }
            }
            IsActorCharacter => {
                let handle = stack.pop_i32();
                if handle == 0 {
                    return 0;
                }
                if !self.actor_exists(handle) {
                    tracing::error!(
                        "Script error (IsActorCharacter): invalid actor handle {handle:#x}"
                    );
                    return 0;
                }
                if self.get_entity(handle).unwrap().is_actor() {
                    1
                } else {
                    0
                }
            }
            IsActorPC => {
                let handle = stack.pop_i32();
                if handle == 0 {
                    return 0;
                }
                if !self.actor_exists(handle) {
                    tracing::error!("Script error (IsActorPC): invalid actor handle {handle:#x}");
                    return 0;
                }
                if self.get_entity(handle).unwrap().is_pc() {
                    1
                } else {
                    0
                }
            }
            IsActorNPC => {
                let handle = stack.pop_i32();
                if handle == 0 {
                    return 0;
                }
                if !self.actor_exists(handle) {
                    tracing::error!("Script error (IsActorNPC): invalid actor handle {handle:#x}");
                    return 0;
                }
                if self.get_entity(handle).unwrap().is_npc() {
                    1
                } else {
                    0
                }
            }
            IsActorSoldier => {
                let handle = stack.pop_i32();
                if handle == 0 {
                    return 0;
                }
                if !self.actor_exists(handle) {
                    tracing::error!(
                        "Script error (IsActorSoldier): invalid actor handle {handle:#x}"
                    );
                    return 0;
                }
                if self.get_entity(handle).unwrap().is_soldier() {
                    1
                } else {
                    0
                }
            }
            IsActorCivilian => {
                let handle = stack.pop_i32();
                if handle == 0 {
                    return 0;
                }
                if !self.actor_exists(handle) {
                    tracing::error!(
                        "Script error (IsActorCivilian): invalid actor handle {handle:#x}"
                    );
                    return 0;
                }
                if self.get_entity(handle).unwrap().is_civilian() {
                    1
                } else {
                    0
                }
            }
            IsActorAnimal => {
                // No animals in this port; shipped scripts never
                // actually query this (verified across all .scb
                // files in datadirs/fullgame_linux), but we keep
                // the native slot so the enum discriminants align
                // with the shipped SCB indices.  Always returns 0.
                let _handle = stack.pop_i32();
                0
            }
            IsActorCart => {
                let handle = stack.pop_i32();
                self.get_entity(handle)
                    .and_then(crate::element::Entity::as_fx)
                    .is_some_and(|fx| fx.fx.mobile_index.is_some()) as i32
            }
            IsActorActive => {
                let handle = stack.pop_i32();
                if !self.actor_exists(handle) {
                    tracing::error!(
                        "Script error (IsActorActive): invalid actor handle {handle:#x}"
                    );
                    return 0;
                }
                if self.get_entity(handle).unwrap().is_active() {
                    1
                } else {
                    0
                }
            }
            IsActorRider => {
                let handle = stack.pop_i32();
                if handle == 0 {
                    return 0;
                }
                if !self.actor_exists(handle) {
                    tracing::error!(
                        "Script error (IsActorRider): invalid actor handle {handle:#x}"
                    );
                    return 0;
                }
                let entity = self.get_entity(handle).unwrap();
                if !entity.is_soldier() {
                    return 0;
                }
                if entity.soldier_data().is_some_and(|s| s.rider) {
                    1
                } else {
                    0
                }
            }
            IsUnblipped => {
                let handle = stack.pop_i32();
                if !self.actor_exists(handle) {
                    tracing::error!("Script error (IsUnblipped): invalid actor handle {handle:#x}");
                    return 0;
                }
                if !self.get_entity(handle).unwrap().element_data().blipped {
                    1
                } else {
                    0
                }
            }

            // --- actor state ---
            GetActorPosture => {
                // Remaps the internal `Posture` enum to the
                // script-visible `ID_*` constants.  The two
                // numeric spaces do NOT coincide — e.g. `Upright`
                // is internal-1 but `ID_UPRIGHT` is script-0.
                // Two arms are conditional: LYING with
                // `unconscious` → `ID_KO` (17); CARRIED with
                // `life_points <= 0` → `ID_DEAD` (15).  Returns
                // -1 on invalid / non-human actor, warns and
                // returns -1 for unmapped variants.
                let actor = stack.pop_i32();
                let Some(entity) = self.get_entity(actor) else {
                    tracing::error!("Script Error: GetActorPosture invalid actor {actor}");
                    return -1;
                };
                if !entity.is_human() {
                    tracing::error!("Script Error: GetActorPosture target {actor} is not human");
                    return -1;
                }
                let posture = entity.element_data().posture;
                let unconscious = entity.human_data().map(|h| h.unconscious).unwrap_or(false);
                let is_dead = entity.is_dead();
                match posture {
                    Posture::Upright => 0,
                    Posture::Lying => {
                        if unconscious {
                            17
                        } else {
                            2
                        }
                    }
                    Posture::OnLadder => 4,
                    Posture::Siesta => 5,
                    Posture::Carried => {
                        if is_dead {
                            15
                        } else {
                            6
                        }
                    }
                    Posture::Flying => 8,
                    Posture::OnWall => 9,
                    Posture::Crouched => 10,
                    Posture::CarryingCorpse => 11,
                    Posture::Dead | Posture::DeadBack => 15,
                    Posture::Sitting => 16,
                    _ => {
                        tracing::warn!(
                            "GetActorPosture: unmapped posture {:?} on actor {actor}",
                            posture
                        );
                        -1
                    }
                }
            }
            SetActorPosture => {
                // The script-level argument uses the `ID_*`
                // namespace, NOT the internal `Posture` enum
                // discriminants — using `Posture::try_from` on the
                // raw value silently corrupts every script call.
                // This arm dispatches on the script IDs and
                // follows RHScript.cpp's exact Stop/broadcast/state/Wait
                // order against live canonical owners.
                //
                let val = stack.pop_i32();
                let actor = stack.pop_i32();

                // ActorExists + IsHuman gates: warn and return on
                // failure of either.
                let Some(entity) = self.get_entity(actor) else {
                    tracing::warn!("Script Error: SetActorPosture invalid actor {actor}");
                    return 0;
                };
                if !entity.is_human() {
                    tracing::warn!("Script Error: SetActorPosture target {actor} is not human");
                    return 0;
                }

                match val {
                    4 | 5 | 6 | 8 | 9 | 11 => {
                        // Warn + return; never touches state.
                        // No Wait().
                        tracing::warn!(
                            "Script Error: SetActorPosture cannot set posture {val} from script"
                        );
                    }
                    0 | 2 | 7 | 10 | 15 | 16 | 17 | 100 => {
                        let request = crate::interp::SynchronousScriptRequest::SetActorPosture {
                            actor,
                            posture: val,
                            native_return: 0,
                        };
                        self.pending_yield = Some(crate::interp::NativeYield {
                            resume: crate::interp::ResumePolicy::Fixed(request.native_return()),
                            operation: crate::interp::NativeOperation::EngineAction(request),
                        });
                    }
                    _ => {
                        tracing::warn!("Script Error: SetActorPosture illegal ID {val}");
                    }
                }
                0
            }
            GetActorDirection => {
                let actor = stack.pop_i32();
                self.get_entity(actor)
                    .map_or(0, |e| e.element_data().direction() as i32)
            }
            SetActorDirection => {
                // Sets direction instantly; if the element is an
                // FX target it additionally upgrades
                // rendering_properties to NeedShadow (workaround
                // for level-09 "tie soldier" sprite reuse).
                // `rendering_properties` lives on `TargetData`,
                // so the upgrade only applies to the
                // `Entity::Target` variant.
                let dir = stack.pop_i32();
                let actor = stack.pop_i32();
                if let Some(entity) = self.get_entity_mut(actor) {
                    entity
                        .element_data_mut()
                        .set_direction_instantly(dir as i16);
                    if let Entity::Target(t) = entity {
                        t.target.rendering_properties =
                            crate::element_kinds::RenderingProperties::NeedShadow;
                    }
                }
                0
            }
            GetActorLocation => {
                // Allocates a script point and stamps the
                // actor's current (layer, sector) onto it so
                // subsequent SetActorLocation round-trips
                // preserve the sector.
                let actor = stack.pop_i32();
                match self.get_entity(actor) {
                    Some(entity) => {
                        let pos = entity.element_data().position_map();
                        let layer = entity.element_data().layer();
                        let sector = entity.element_data().sector().map(|s| s.get());
                        let meta = sector.map(|s| (layer, s));
                        self.create_computed_location_full(pos.x, pos.y, meta)
                    }
                    None => {
                        tracing::warn!("GetActorLocation: invalid actor handle {actor}");
                        0
                    }
                }
            }
            SetActorLocation => {
                let loc = stack.pop_i32();
                let actor = stack.pop_i32();
                if self.get_entity(actor).is_none() {
                    tracing::warn!("SetActorLocation: invalid actor handle {actor}");
                    return 0;
                }
                let request = crate::interp::SynchronousScriptRequest::SetActorLocation {
                    actor,
                    location: loc,
                    native_return: 1,
                };
                self.pending_yield = Some(crate::interp::NativeYield {
                    resume: crate::interp::ResumePolicy::OperationResult,
                    operation: crate::interp::NativeOperation::EngineAction(request),
                });
                1
            }
            IsInside => {
                let loc = stack.pop_i32();
                let actor = stack.pop_i32();
                if actor == 0 || loc == 0 {
                    return 0;
                }
                // Geometric polygon point-in-test recomputed
                // every call so results stay correct immediately
                // after teleport natives ("works also after
                // teleports"). The authoritative occupant list
                // is only refreshed on explicit
                // Add/CleanFromScriptZone natives or on the
                // next-frame tick, so we recompute here when we
                // have polygon geometry installed.
                let zone_idx = Self::location_index(loc)
                    .and_then(|idx| idx.checked_sub(self.bindings.script_point_count));
                if let Some(zi) = zone_idx
                    && let Some(&grid_idx) = self.bindings.script_zone_grid_indices.get(zi)
                    && let Some(zone) = self.fast_grid.level.sectors.get(grid_idx as usize)
                    && let Some(entity) = self.get_entity(actor)
                {
                    let ed = entity.element_data();
                    // Filter invisible objects.
                    if !ed.active || ed.in_honolulu {
                        return 0;
                    }
                    if zone.layer != ed.layer() {
                        return 0;
                    }
                    let pt = ed.position_map();
                    if !zone.bounding_box.contains_point(pt) {
                        return 0;
                    }
                    // Ray-casting point-in-polygon, identical to
                    // `GridSector::contains_point` (the production
                    // path used by the per-frame zone scan).
                    if zone.points.len() < 3 {
                        return 0;
                    }
                    let mut inside = false;
                    let n = zone.points.len();
                    let mut j = n - 1;
                    for i in 0..n {
                        let vi = zone.points[i];
                        let vj = zone.points[j];
                        if (vi.y > pt.y) != (vj.y > pt.y) {
                            let x_intersect = (vj.x - vi.x) * (pt.y - vi.y) / (vj.y - vi.y) + vi.x;
                            if pt.x < x_intersect {
                                inside = !inside;
                            }
                        }
                        j = i;
                    }
                    i32::from(inside)
                } else {
                    // Fall back to the cache when geometry isn't
                    // available (handle out of zone range, or
                    // pre-load test fixtures that never installed
                    // polygons).
                    i32::from(
                        self.zone_occupant_handles(loc)
                            .is_some_and(|occ| occ.contains(&actor)),
                    )
                }
            }
            IsInsideBuilding => {
                let bld = stack.pop_i32();
                let actor = stack.pop_i32();
                if actor == 0 {
                    return 0;
                }
                if bld == 0 {
                    // NULL building: check if actor is inside ANY building
                    i32::from(
                        self.script_domains
                            .buildings
                            .actor_building
                            .contains_key(&actor),
                    )
                } else {
                    // Check if actor is in the specific building
                    i32::from(
                        self.script_domains.buildings.actor_building.get(&actor) == Some(&bld),
                    )
                }
            }
            UnBlip => {
                // Gates on ActorExists (warn + false on invalid
                // handle); returns true iff element was actually
                // blipped before the call.
                let actor = stack.pop_i32();
                if !self.actor_exists(actor) {
                    tracing::error!("Script error (UnBlip): invalid actor handle {actor}");
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
            GetMovementStyle => {
                // Returns 1 if action_state == MovingFast, else 0.
                let actor = stack.pop_i32();
                self.get_entity(actor).map_or(0, |e| {
                    if e.actor_data()
                        .is_some_and(|a| a.action_state == ActionState::MovingFast)
                    {
                        1
                    } else {
                        0
                    }
                })
            }
            GetCurrentAction => {
                // (1) !ActorExists → warn + return 0
                // (2) Object → return the object's animation
                // (3) !IsActor → warn + return 0
                // (4) Actor → return the front order's
                //     `order_type`.
                //
                // Actors query the live SequenceManager; objects read
                // their canonical animation field directly.
                let actor = stack.pop_i32();
                let Some(entity) = self.get_entity(actor) else {
                    tracing::warn!(
                        "Script Error: GetCurrentAction on invalid actor handle {actor}"
                    );
                    return 0;
                };
                if entity.is_object() {
                    self.current_animation(actor).map_or(0, |a| a as i32)
                } else if entity.is_actor() {
                    self.current_animation(actor).map_or(0, |a| a as i32)
                } else {
                    tracing::warn!(
                        "Script Error: GetCurrentAction on illegal actor handle {actor} (not actor, not object)"
                    );
                    0
                }
            }
            InflictPain => {
                // Launches a one-element damage sequence
                // through the sequence manager instead of
                // mutating life points inline — this lets the
                // victim's `instruct` handler queue the hit
                // animation, routes through
                // `apply_generic_damage` for posture/death
                // transitions, and honours sequence priority so
                // the damage can't preempt a higher-priority
                // non-interruptable element.
                let pain_type = stack.pop_i32();
                let amount = stack.pop_i32();
                let actor = stack.pop_i32();
                if !self.actor_exists(actor) {
                    tracing::warn!("Script Error: InflictPain on invalid actor handle {actor}");
                    return 0;
                }
                // `amount` flows straight into a u16 slot;
                // negative scripts wrap.
                let damage = amount as u16;
                let concussion = if pain_type != 0 { 100u16 } else { 0u16 };
                let target = self
                    .actor_id(actor)
                    .expect("InflictPain: actor_exists check passed but actor_id None");
                self.launch_script_sequence(Sequence::single_damage(target, damage, concussion), 0);
                // Returns true on success.
                1
            }
            SetCompanyNumber => {
                let num = stack.pop_i32();
                let actor = stack.pop_i32();
                if let Some(entity) = self.get_entity_mut(actor) {
                    if let Some(enemy) = entity.enemy_ai_mut() {
                        // `company_number` is a u16.
                        enemy.company_number = num as u16;
                    } else {
                        tracing::warn!(
                            "Script Error: SetCompanyNumber on non-soldier actor {actor}"
                        );
                    }
                }
                0
            }
            SetAlwaysAttentive => {
                // Set the AI's `forced_attentive` flag, launch
                // an `EnterAttentiveMode` sequence on the
                // false→true transition, and — on the true
                // branch with frame>1 and the NPC already on
                // GREEN — bump the alert to YELLOW.
                let val = stack.pop_i32();
                let actor = stack.pop_i32();
                let target = val != 0;
                let mut launch_enter = false;
                let frame = self.frame_counter();
                match self.get_entity_mut(actor) {
                    None => {
                        tracing::warn!("Script Error: SetAlwaysAttentive on invalid actor {actor}");
                    }
                    Some(entity) => match entity.enemy_ai_mut() {
                        None => {
                            tracing::warn!(
                                "Script Error: SetAlwaysAttentive on non-soldier actor {actor}"
                            );
                        }
                        Some(enemy) => {
                            enemy.forced_attentive = target;
                            if target && !enemy.will_be_attentive {
                                enemy.will_be_attentive = true;
                                launch_enter = true;
                            }
                            if target
                                && frame > 1
                                && enemy.base.current_music_alert_status == AlertLevel::Green
                            {
                                // Route through the soldier wrapper so view
                                // tracks the override; SetAlwaysAttentive
                                // already updated `forced_attentive` above.
                                enemy.set_alert_status(AlertLevel::Yellow);
                            }
                        }
                    },
                }
                if launch_enter && let Some(target_id) = self.actor_id(actor) {
                    let mut seq = Sequence::new();
                    seq.append_element(SequenceElement::new(
                        1,
                        Command::EnterAttentiveMode,
                        Some(target_id),
                    ));
                    self.launch_script_sequence(seq, 0);
                }
                0
            }
            SetInvisible => {
                // Warns separately on "inexisting actor" and
                // "non-human" so the scripting team can
                // distinguish the two failures.
                let val = stack.pop_i32();
                let actor = stack.pop_i32();
                match self.get_entity_mut(actor) {
                    None => {
                        tracing::warn!("SetInvisible: actor {actor} does not exist");
                    }
                    Some(entity) => match entity.human_data_mut() {
                        Some(human) => {
                            human.hollow_man = val != 0;
                        }
                        None => {
                            tracing::warn!("SetInvisible: actor {actor} is not human");
                        }
                    },
                }
                0
            }
            IsInvisible => {
                let actor = stack.pop_i32();
                self.get_entity(actor).map_or(0, |e| {
                    i32::from(e.human_data().is_some_and(|h| h.hollow_man))
                })
            }
            MakePCCrouched => {
                let actor = stack.pop_i32();
                // Route through the engine layer so the full
                // sequence/animation/path rewrite happens with
                // engine-side state.  Validation (ActorExists +
                // IsPC) and the actual `actor_make_crouched`
                // call happen in the engine-side handler.
                self.emit_engine(EngineCommand::ScriptMakePCCrouched {
                    actor_handle: actor,
                });
                0
            }
            GetActorActionState => {
                // Returns -1 on invalid actor or non-human.  The
                // Rust `ActionState` enum discriminants coincide
                // with the script `ID_ACTIONSTATE_*` constants
                // 0..=17, so a direct `as i32` cast is correct
                // on the happy path.
                let actor = stack.pop_i32();
                let Some(entity) = self.get_entity(actor) else {
                    tracing::error!("Script Error: GetActorActionState invalid actor {actor}");
                    return -1;
                };
                if !entity.is_human() {
                    tracing::error!(
                        "Script Error: GetActorActionState target {actor} is not human"
                    );
                    return -1;
                }
                entity.actor_data().map_or(-1, |a| a.action_state as i32)
            }
            SetActorActionState => {
                // Validates ActorExists + IsHuman (warn + early
                // return on either failure).  Every arm then
                // calls `set_action_state(s) + Wait()`.  The
                // trailing Wait() launches a low-priority Wait
                // sequence element on the actor, displacing any
                // in-flight sequence so the freshly-stamped
                // action state actually takes hold.
                let val = stack.pop_i32();
                let actor = stack.pop_i32();
                let Some(entity) = self.get_entity(actor) else {
                    tracing::warn!("Script Error: SetActorActionState invalid actor {actor}");
                    return 0;
                };
                if !entity.is_human() {
                    tracing::warn!("Script Error: SetActorActionState target {actor} is not human");
                    return 0;
                }
                let Ok(s) = ActionState::try_from(val as u32) else {
                    tracing::warn!("SetActorActionState: invalid value {val}");
                    return 0;
                };
                self.pending_yield = Some(crate::interp::NativeYield {
                    operation: crate::interp::NativeOperation::EngineAction(
                        crate::interp::SynchronousScriptRequest::SetActorActionState {
                            actor,
                            state: s as i32,
                            native_return: 0,
                        },
                    ),
                    resume: crate::interp::ResumePolicy::Fixed(0),
                });
                0
            }

            // --- vision / interaction ---
            Sees => {
                // Sees(Actor npc, Actor target) -> bool
                //
                // Validates both handles (NPC observer + Human
                // target) and then returns whether the NPC's
                // visibility computation for the target is > 0.
                //
                // The pre-rewrite version peeked the EventView
                // stimulus queue, which is racy: the queue is
                // drained by the AI state machine on the same
                // tick, so a script call after the AI tick saw
                // `false` for a target the NPC was actively
                // engaging.  The synchronous `compute_visibility`
                // call gives the right answer regardless of tick
                // phase.
                //
                let target_h = stack.pop_i32();
                let npc_h = stack.pop_i32();

                // Four warn + return-false validation gates.
                let Some(npc_entity) = self.get_entity(npc_h) else {
                    tracing::warn!(
                        "Script Error: Trying to test if an invalid actor element ({npc_h}) sees another actor."
                    );
                    return 0;
                };
                if npc_entity.ai_controller().is_none() {
                    tracing::warn!(
                        "Script Error: Trying to test if a non-NPC element ({npc_h}) sees another actor."
                    );
                    return 0;
                }
                let npc_index = u32::try_from(
                    Self::actor_handle_index(npc_h)
                        .expect("validated Sees NPC handle must decode as an actor"),
                )
                .expect("validated Sees NPC slot exceeds u32");
                let npc_id = crate::element::EntityId::new(
                    npc_index,
                    match npc_entity {
                        Entity::Pc(_) => crate::entity_id::EntityIdKind::Pc,
                        Entity::Soldier(_) => crate::entity_id::EntityIdKind::Soldier,
                        Entity::Civilian(_) => crate::entity_id::EntityIdKind::Civilian,
                        Entity::Fx(_) => crate::entity_id::EntityIdKind::Fx,
                        Entity::Target(_) => crate::entity_id::EntityIdKind::Target,
                        Entity::Bonus(_) => crate::entity_id::EntityIdKind::Bonus,
                        Entity::Scroll(_) => crate::entity_id::EntityIdKind::Scroll,
                        Entity::Projectile(_) => crate::entity_id::EntityIdKind::Projectile,
                        Entity::Net(_) => crate::entity_id::EntityIdKind::Net,
                    },
                );
                let Some(target_entity) = self.get_entity(target_h) else {
                    tracing::warn!(
                        "Script Error: Trying to test if a NPC sees an invalid actor element ({target_h})."
                    );
                    return 0;
                };
                if target_entity.human_data().is_none() {
                    tracing::warn!(
                        "Script Error: Trying to test if a NPC sees a non-human ({target_h})."
                    );
                    return 0;
                }

                // Read everything we need off the live entity
                // store for fields that move per-frame (position,
                // direction, posture, view parameters, eye Z).
                // Building membership comes from the canonical script
                // domain borrowed for this native session.
                let npc_dir = npc_entity.element_data().direction();
                let npc_layer = npc_entity.element_data().layer();
                let Some(npc_data) = npc_entity.npc_data() else {
                    tracing::warn!(
                        "Script Error: NPC {npc_h} has no NpcData (view parameters missing)."
                    );
                    return 0;
                };
                let view_radius = npc_data.view_radius;
                let eye_status = npc_data.eye_status;
                let real_half_aperture = npc_data.real_half_aperture;
                let view_direction = npc_data.view_direction;
                let viewer_eye_3d = npc_entity
                    .compute_eyes_point(None)
                    .expect("Sees validated an NPC observer, which must have an eye point");
                let viewer_eye = crate::coordinates::MapPoint::from_world_xyz(
                    viewer_eye_3d.x,
                    viewer_eye_3d.y,
                    npc_entity.element_data().position().z,
                );

                let viewer_building = self
                    .script_domains
                    .buildings
                    .actor_building
                    .get(&npc_h)
                    .copied();
                let viewer_building_sector = viewer_building
                    .and_then(|h| crate::position_interface::SectorHandle::new(h as u16));
                let viewer_in_building = viewer_building.is_some();

                // Target side.
                let tgt_layer = target_entity.element_data().layer();
                let tgt_posture = target_entity.element_data().posture;
                let tgt_action_state = target_entity
                    .actor_data()
                    .expect("Sees validated a human target, which must have actor data")
                    .action_state;
                let tgt_active = target_entity.element_data().active;
                let target_building = self
                    .script_domains
                    .buildings
                    .actor_building
                    .get(&target_h)
                    .copied();
                let tgt_building_sector = target_building
                    .and_then(|h| crate::position_interface::SectorHandle::new(h as u16));
                let tgt_in_building = target_building.is_some();
                let tgt_unconscious = target_entity
                    .human_data()
                    .expect("Sees validated a human target, which must have human data")
                    .unconscious;
                let tgt_passing_door = target_entity
                    .actor_data()
                    .expect("Sees validated a human target, which must have actor data")
                    .active_door_pass
                    .is_some();
                let tgt_is_pc = matches!(target_entity, Entity::Pc(_));
                // Use the same target-point helper as the authoritative
                // AI detection pass. The 3D point supplies only the
                // posture-adjusted Z used by the close-range test.
                let tgt_detection_3d = target_entity
                    .compute_detection_point()
                    .expect("Sees validated a human target, which must have a detection point");
                let target_point = crate::stealth::detection_point_xy(
                    target_entity.element_data().position_map(),
                    tgt_posture,
                    target_entity.element_data().direction(),
                );

                // Different layer ⇒ no LOS; the sight raycast
                // wouldn't cross floors.  Same layer guard the
                // AI detection path uses before
                // VisibilityQuery construction.
                if tgt_layer != npc_layer {
                    return 0;
                }

                let view_forward = (view_direction[0], view_direction[1]);
                let golden_eye_mode = self.ai_global().golden_eye_mode;
                let target_in_same_building =
                    viewer_in_building && tgt_building_sector == viewer_building_sector;

                let sight_obstacle_list = self
                    .sight_obstacles
                    .expect("Sees requires live canonical sight-obstacle views");
                let target_obstacle_handle = target_entity.element_data().obstacle_index();
                let target_obstacle = target_obstacle_handle.map(|handle| {
                    sight_obstacle_list
                        .get(usize::from(handle))
                        .unwrap_or_else(|| {
                            panic!(
                                "Sees target {target_h} references missing sight obstacle {}",
                                usize::from(handle)
                            )
                        })
                });
                let is_night_or_fog = matches!(
                    self.weather
                        .expect("script native requires a live WeatherState query view")
                        .ambiance,
                    crate::engine::Ambiance::Night | crate::engine::Ambiance::Fog
                );
                let forest_180_degree_view = self
                    .weather
                    .expect("script native requires a live WeatherState query view")
                    .is_forest_level
                    && npc_entity.camp() == Camp::Royalists;
                if forest_180_degree_view && !npc_entity.is_active() {
                    return 0;
                }
                let universal_frame = *self
                    .frame_counter
                    .expect("Sees requires the live universal frame counter");
                let q = crate::ai_vision::VisibilityQuery {
                    viewer_los: viewer_eye,
                    viewer_world: viewer_eye_3d,
                    viewer_direction: npc_dir,
                    view_forward,
                    view_radius,
                    viewer_eye_status: eye_status,
                    real_half_aperture,
                    viewer_in_building,
                    target_in_same_building,
                    forest_180_degree_view,
                    golden_eye_mode,
                    effective_view_radius: view_radius as f32,
                    target_is_active_and_outside_building: tgt_active && !tgt_in_building,
                    target_los: target_point,
                    target_world: tgt_detection_3d,
                    target_posture: tgt_posture,
                    target_action_state: tgt_action_state,
                    target_is_pc: tgt_is_pc,
                    sight_obstacles: sight_obstacle_list,
                    fast_grid: &self.fast_grid,
                    layer: npc_layer,
                    target_unconscious: tgt_unconscious,
                    target_passing_door: tgt_passing_door,
                };
                if crate::ai_vision::compute_visibility_with_effective_radius(&q, || {
                    if let Some(radius) = self.view_radius_cache.as_ref().and_then(|cache| {
                        cache.get(target_obstacle_handle, npc_id, universal_frame)
                    }) {
                        return radius;
                    }
                    let radius = crate::ai_vision::compute_view_radius(
                        viewer_eye_3d,
                        view_radius,
                        view_forward,
                        real_half_aperture,
                        is_night_or_fog,
                        &self.fast_grid,
                        sight_obstacle_list,
                        target_obstacle,
                    );
                    if radius != 0.0
                        && let Some(cache) = self.view_radius_cache.as_mut()
                    {
                        cache.set(target_obstacle_handle, npc_id, universal_frame, radius);
                    }
                    radius
                }) > 0.0
                {
                    1
                } else {
                    0
                }
            }
            EnableViewCone => {
                // Validates IsNPC (warn only, no early return)
                // and then either (a) triggers the Ezekiel2517
                // "Dies irae" cheat — a 10000 HP info-priority
                // damage sequence — or (b) stores the actor as
                // the engine's single selected view element.
                // We emulate (b) by setting the per-AI debug
                // flag on the target and clearing it on every
                // other NPC, so calling twice on the same actor
                // keeps it selected, and calling on a different
                // actor replaces the selection.
                let actor = stack.pop_i32();
                if !self.actor_exists(actor) {
                    return 0;
                }
                // NPC-type check (warn, no early return).
                let is_npc = self
                    .get_entity(actor)
                    .is_some_and(|e| e.ai_controller().is_some());
                if !is_npc {
                    tracing::warn!(
                        "Script Error: Trying to enable the view cone of an element which is not a NPC."
                    );
                }
                if self.ai_global().ezekiel_2517 {
                    // "Dies irae" cheat: 10000 HP info-priority
                    // damage on the target (asserts IsHuman).
                    if let Some(target) = self.actor_id(actor)
                        && self
                            .get_entity(actor)
                            .is_some_and(|e| e.human_data().is_some())
                    {
                        self.launch_script_sequence(Sequence::single_damage(target, 10000, 0), 0);
                    }
                } else if is_npc {
                    let target_id = self.actor_id(actor);
                    for (entity_id, entity) in self.occupied_entities_mut() {
                        let Some(ai) = entity.ai_controller_mut() else {
                            continue;
                        };
                        ai.debug_view_cone_enabled = target_id == Some(entity_id);
                    }
                }
                0
            }
            PrototypeFilterEvent => {
                unreachable!("PrototypeFilterEvent is handled as explicit VM control flow")
            }
            SendMessage => {
                let msg = stack.pop_i32();
                let actor = stack.pop_i32();
                // Non-null non-actor handle → warn + no dispatch.
                if actor != 0 && !self.is_actor_handle(actor) {
                    tracing::error!("Script Error : trying to send a message to non actor object.");
                    return 0;
                }
                // RHScript::SendMessage constructs and launches the sequence
                // element inline. Launching through the live manager keeps
                // its sequence id ordered with recorded Thanx sequences and
                // yields to ProcessMessage before this callback resumes.
                tracing::trace!(target_actor = actor, message = msg, "script SendMessage");
                let mut sequence = Sequence::new();
                sequence.append_element(self.build_send_message_element(1, actor, msg, 0, 0));
                self.launch_script_sequence(sequence, 0);
                0
            }
            SendMessageWithArguments => {
                let arg2 = stack.pop_i32();
                let arg1 = stack.pop_i32();
                let msg = stack.pop_i32();
                let actor = stack.pop_i32();
                // Same IsActor guard as SendMessage.
                if actor != 0 && !self.is_actor_handle(actor) {
                    tracing::error!("Script Error : trying to send a message to non actor object.");
                    return 0;
                }
                let mut sequence = Sequence::new();
                sequence.append_element(self.build_send_message_element(1, actor, msg, arg1, arg2));
                self.launch_script_sequence(sequence, 0);
                0
            }

            // --- action / property ---
            SetActionAvailable => {
                // Validates `IsPC(actor)` then `action ∈ [0, 5]`,
                // forwards an enable/disable message, and returns
                // true / false. The C++ message handlers do not
                // mutate RHElementActorPC::mpbDisabledActions; real
                // persistent disables are maintained by PC ammo /
                // action methods.
                let _avail = stack.pop_i32();
                let action_idx = stack.pop_i32();
                let actor = stack.pop_i32();
                let Some(entity) = self.get_entity(actor) else {
                    tracing::error!("Script Error: SetActionAvailable invalid actor {actor}");
                    return 0;
                };
                if !entity.is_pc() {
                    tracing::error!("Script Error: SetActionAvailable target {actor} is not a PC");
                    return 0;
                }
                if !(0..=5).contains(&action_idx) {
                    tracing::error!(
                        "Script Error: SetActionAvailable action index {action_idx} out of range"
                    );
                    return 0;
                }
                1
            }
            IsActionAvailable => {
                // Consults BOTH the persistent and temporary
                // disabled-action masks — an action is available
                // only if neither slot is set.
                let action_idx = stack.pop_i32();
                let actor = stack.pop_i32();
                let Some(entity) = self.get_entity(actor) else {
                    tracing::error!("Script Error: IsActionAvailable invalid actor {actor}");
                    return 0;
                };
                let Some(pc) = entity.pc_data() else {
                    tracing::error!("Script Error: IsActionAvailable target {actor} is not a PC");
                    return 0;
                };
                if !(0..=5).contains(&action_idx) {
                    tracing::error!(
                        "Script Error: IsActionAvailable action index {action_idx} out of range"
                    );
                    return 0;
                }
                let idx = action_idx as usize;
                let disabled_persistent = pc.disabled_actions.get(idx).copied().unwrap_or(false);
                let disabled_temp = pc.disabled_actions_temp.get(idx).copied().unwrap_or(false);
                if disabled_persistent || disabled_temp {
                    0
                } else {
                    1
                }
            }
            SetPersistentProperty => {
                // The property id and the amount are narrowed to a signed byte
                // before they reach the implementation, so a script constant
                // above 127 arrives negative (250 becomes -6). Missions rely on
                // this: H07 sets the Sheriff's life points to 250, which lands
                // as -6 and kills him.
                let amount = i32::from(stack.pop_i32() as i8);
                let prop = i32::from(stack.pop_i32() as i8);
                let actor = stack.pop_i32();
                self.set_persistent_property(actor, prop, amount) as i32
            }
            GetPersistentProperty => {
                let prop = stack.pop_i32();
                let actor = stack.pop_i32();
                self.get_persistent_property(actor, prop)
            }
            IsAnyCivilianDead => {
                if self.npc_status_aggregates().0 {
                    1
                } else {
                    0
                }
            }
            IsAnyEnemyDead => {
                if self.npc_status_aggregates().1 {
                    1
                } else {
                    0
                }
            }
            GetOverallEnemyAlert => self.npc_status_aggregates().2,
            GetOverallCivilianAlert => self.npc_status_aggregates().3,
            HasPCAction => {
                let action_code = stack.pop_i32();
                let actor = stack.pop_i32();
                // Guards: ActorExists + IsPC; warn and return
                // false on either failure.
                let Some(entity) = self.get_entity(actor) else {
                    tracing::warn!(
                        "Script Error: Trying to call HasPCAction for invalid actor element."
                    );
                    return 0;
                };
                if entity.pc_data().is_none() {
                    tracing::warn!("Script Error: Trying to call HasPCAction for non-PC.");
                    return 0;
                }
                let Ok(script_action) = crate::profiles::ScriptAction::try_from(action_code as u32)
                else {
                    tracing::warn!("Script Error: HasPCAction with bad action ID {action_code}");
                    return 0;
                };
                let action = script_action.to_action();
                // Direct PC->profile lookup (no raw-profile-index fallback for this native).
                let Some(profile_idx) = self.pc_profile_index(actor) else {
                    return 0;
                };
                self.campaign.as_ref().expect("campaign required");
                self.bindings
                    .profile_manager
                    .get_character(profile_idx)
                    .map_or(0, |cp| {
                        let has =
                            cp.actions.contains(&action) || cp.contextual_actions.contains(&action);
                        if has { 1 } else { 0 }
                    })
            }
            HasAnyPCAction => {
                let action_code = stack.pop_i32();
                let Ok(script_action) = crate::profiles::ScriptAction::try_from(action_code as u32)
                else {
                    tracing::warn!("Script Error: HasAnyPCAction with bad action ID {action_code}");
                    return 0;
                };
                let action = script_action.to_action();
                // Iterate the spawned-PC array (not the
                // campaign-wide gang list). Handles are derived from the
                // canonical entity slots for every query.
                let profiles = &self.bindings.profile_manager;
                for handle in self.pc_handles() {
                    let Some(profile_idx) = self.pc_profile_index(handle) else {
                        continue;
                    };
                    let Some(cp) = profiles.get_character(profile_idx) else {
                        continue;
                    };
                    if cp.actions.contains(&action) || cp.contextual_actions.contains(&action) {
                        return 1;
                    }
                }
                0
            }
            HasAnyActivePCAction => {
                // Like HasAnyPCAction but also requires the PC to be "playable"
                // (alive, active, not guarded). Check entity state for playable,
                // then campaign profile for action availability.
                let action_code = stack.pop_i32();
                let Ok(script_action) = crate::profiles::ScriptAction::try_from(action_code as u32)
                else {
                    tracing::warn!(
                        "Script Error: HasAnyActivePCAction with bad action ID {action_code}"
                    );
                    return 0;
                };
                let action = script_action.to_action();

                // Filter solely on `playable`, not death.  The
                // death pipeline is responsible for clearing
                // `playable`; do not double-filter here.
                let playable_profiles: Vec<crate::profiles::CharacterProfileIdx> = self
                    .entities
                    .occupied()
                    .filter_map(|(_, entity)| {
                        let pc = entity.pc_data()?;
                        if !pc.playable {
                            return None;
                        }
                        Some(pc.profile_index)
                    })
                    .collect();

                let profiles = &self.bindings.profile_manager;
                for pi in &playable_profiles {
                    let Some(cp) = profiles.get_character(*pi) else {
                        continue;
                    };
                    if cp.actions.contains(&action) || cp.contextual_actions.contains(&action) {
                        return 1;
                    }
                }
                0
            }
            HasAnyActionSelected => {
                // Checks whether the PC is selected and has a
                // non-NoAction action selected.
                let actor = stack.pop_i32();
                if !self.actor_exists(actor) {
                    tracing::error!("Script Error: HasAnyActionSelected for invalid actor {actor}");
                    return 0;
                }
                let entity = self.get_entity(actor).unwrap();
                if !entity.is_pc() {
                    tracing::error!("Script Error: HasAnyActionSelected for non-PC {actor}");
                    return 0;
                }
                // Must be selected
                if !self.selected_pc_handles().contains(&actor) {
                    return 0;
                }
                // Check if any action is selected (non-NoAction)
                if entity
                    .pc_data()
                    .is_some_and(|pc| pc.current_action != Action::NoAction)
                {
                    1
                } else {
                    0
                }
            }

            _ => self.dispatch_ai(native, stack),
        }
    }
}
