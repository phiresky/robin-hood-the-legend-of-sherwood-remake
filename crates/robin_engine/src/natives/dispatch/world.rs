//! Animation, patch, sound, building, door, and scroll dispatch.

use super::*;

impl NativeContext<'_, '_> {
    pub(super) fn dispatch_world(&mut self, native: NativeFn, stack: &mut NativeStack) -> i32 {
        use NativeFn::*;

        match native {
            // --- animation / patch ---
            IsAnimationActive => {
                let actor_h = stack.pop_i32();
                if actor_h == 0 {
                    tracing::warn!("Script error: IsAnimationActive with null handle");
                    0
                } else {
                    self.get_entity(actor_h)
                        .map_or(0, |entity| i32::from(entity.element_data().active))
                }
            }
            SetAnimationState => {
                // Rejects !ActorExists || !IsFX with a warning
                // + false, else `set_active(state)` and true.
                let state = stack.pop_i32();
                let actor_h = stack.pop_i32();
                let is_fx = self.get_entity(actor_h).is_some_and(|e| e.is_fx());
                if !self.actor_exists(actor_h) || !is_fx {
                    tracing::error!(
                        "Script error (SetAnimationState): invalid animation handle {actor_h}"
                    );
                    0
                } else {
                    let on = state != 0;
                    if let Some(entity) = self.get_entity_mut(actor_h) {
                        entity.element_data_mut().active = on;
                    }
                    1
                }
            }
            IsPatchApplied => {
                let h = stack.pop_i32();
                self.get_patch(h).map_or(0, |p| i32::from(p.is_applied()))
            }
            ApplyPatch => {
                let h = stack.pop_i32();
                if let Some(patch_index) = Self::patch_index(h)
                    && let Some(patch) = self
                        .script_domains
                        .interactables
                        .patches
                        .get_mut(patch_index)
                {
                    let effects = patch.apply();
                    if !effects.is_empty()
                        && let Some(patch_index) = crate::patch::PatchIndex::new(patch_index as u32)
                    {
                        self.emit_barrier(DeferredCommand::ProcessPatchEffects {
                            patch_index,
                            effects,
                        });
                    }
                }
                1
            }
            ResetPatch => {
                let h = stack.pop_i32();
                if let Some(patch_index) = Self::patch_index(h)
                    && let Some(patch) = self
                        .script_domains
                        .interactables
                        .patches
                        .get_mut(patch_index)
                {
                    let effects = patch.force_reset();
                    if !effects.is_empty()
                        && let Some(patch_index) = crate::patch::PatchIndex::new(patch_index as u32)
                    {
                        self.emit_barrier(DeferredCommand::ProcessPatchEffects {
                            patch_index,
                            effects,
                        });
                    }
                }
                1
            }
            LockPatch => {
                let val = stack.pop_i32();
                let h = stack.pop_i32();
                if let Some(patch) = self.get_patch_mut(h) {
                    if val != 0 {
                        patch.lock();
                    } else {
                        patch.unlock();
                    }
                }
                0
            }
            SetPatchAnimationActive => {
                let active = stack.pop_i32();
                let patch_h = stack.pop_i32();
                // The patch's animation is an FX entity
                // referenced by handle; flip its active flag.
                let idx = Self::patch_index(patch_h);
                if let Some(animation_h) = idx.and_then(|i| {
                    self.bindings
                        .patch_animation_entities
                        .get(i)
                        .copied()
                        .flatten()
                }) {
                    let entity = self.get_entity_mut(animation_h).unwrap_or_else(|| {
                            panic!(
                                "SetPatchAnimationActive: patch animation handle {animation_h} is missing"
                            )
                        });
                    entity.element_data_mut().active = active != 0;
                }
                // If no animation entity is mapped, this is a no-op (patch has no animation)
                0
            }
            LinkTargetToFX => {
                // Gates on `is_fx_target()` and `is_fx()` —
                // either failing logs and skips the link.  The
                // link is stored on the target's `linked_fx`
                // array, which the focus-highlight code
                // iterates to call a highlight-animation hook
                // on each linked FX while the target is
                // hovered.  Note: that highlight-animation
                // setter is a no-op in shipped builds (the body
                // is commented out), so the cascade is dead
                // code in practice — but we mirror the storage
                // so a future port of the highlight effect has
                // somewhere to read.
                let fx_h = stack.pop_i32();
                let target_h = stack.pop_i32();
                let fx_id = match self.actor_id(fx_h) {
                    Some(id) if self.get_entity(fx_h).is_some_and(|entity| entity.is_fx()) => id,
                    None => {
                        tracing::warn!(
                            "Script error (LinkTargetToFX): null/invalid FX handle {fx_h}"
                        );
                        return 0;
                    }
                    Some(_) => {
                        tracing::warn!("Script error (LinkTargetToFX): handle {fx_h} is not an FX");
                        return 0;
                    }
                };
                let Some(fx_entity) = self.get_entity(fx_h) else {
                    tracing::warn!("Script error (LinkTargetToFX): invalid FX handle {fx_h}");
                    return 0;
                };
                if !fx_entity.is_fx() {
                    tracing::warn!(
                        "HALT STEHENBLEIBEN ! Script error (LinkTargetToPatch) : Invalid FX"
                    );
                    return 0;
                }
                let Some(target_entity) = self.get_entity_mut(target_h) else {
                    tracing::warn!(
                        "Script error (LinkTargetToFX): invalid target handle {target_h}"
                    );
                    return 0;
                };
                if !target_entity.is_fx_target() {
                    tracing::warn!(
                        "HALT STEHENBLEIBEN ! Script error (LinkTargetToPatch) : Invalid target"
                    );
                    return 0;
                }
                let Entity::Target(t) = target_entity else {
                    // is_fx_target() already gated this; unreachable.
                    return 0;
                };
                t.target.linked_fx.push(fx_id);
                0
            }

            // --- sound ---
            SuspendAllSoundSources => {
                self.emit_sound(SoundCommand::SuspendAll);
                1
            }
            ResumeAllSoundSources => {
                self.emit_sound(SoundCommand::ResumeAll);
                1
            }
            ActivateSoundSource => {
                let ss_h = stack.pop_i32();
                if ss_h != 0 {
                    self.emit_sound(SoundCommand::Activate(ss_h));
                }
                1
            }
            DeactivateSoundSource => {
                let ss_h = stack.pop_i32();
                self.emit_sound(SoundCommand::Deactivate(ss_h));
                1
            }
            DestroySoundSource => {
                let ss_h = stack.pop_i32();
                if let Some(index) = Self::sound_source_index(ss_h) {
                    self.sound_sources
                        .as_mut()
                        .expect("DestroySoundSource requires live sound-source state")
                        .delete(index);
                }
                self.emit_sound(SoundCommand::Destroy(ss_h));
                1
            }

            // --- building / teleport ---
            CleanFromHisBuildingBeforeTeleport => {
                let actor_h = stack.pop_i32();
                // Remove actor from their current building's occupant list
                if let Some(&bld_h) = self.script_domains.buildings.actor_building.get(&actor_h) {
                    if let Some(idx) = Self::building_index(bld_h)
                        && let Some(occupants) =
                            self.script_domains.buildings.occupants.get_mut(idx)
                    {
                        occupants.retain(|&a| a != actor_h);
                    }
                    self.script_domains
                        .buildings
                        .actor_building
                        .remove(&actor_h);
                    1
                } else {
                    tracing::warn!(
                        "Script error: CleanFromHisBuildingBeforeTeleport: \
                         actor {actor_h} not in a building"
                    );
                    0
                }
            }
            CleanFromScriptZoneBeforeTeleport => {
                let loc_h = stack.pop_i32();
                let actor_h = stack.pop_i32();
                if loc_h == 0 {
                    return 0;
                }
                let actor_id = self.actor_id(actor_h);
                let zone_idx = self.zone_index(loc_h);
                if let (Some(actor_id), Some(zone)) = (
                    actor_id,
                    zone_idx.and_then(|idx| self.script_domains.zones.scripts.get_mut(idx)),
                ) {
                    if zone.is_inside(actor_id) {
                        zone.leave(actor_id);
                        1
                    } else {
                        tracing::warn!(
                            "Script error: CleanFromScriptZoneBeforeTeleport: \
                             actor {actor_h} not in zone {loc_h}"
                        );
                        0
                    }
                } else {
                    tracing::warn!(
                        "Script error: CleanFromScriptZoneBeforeTeleport: \
                         invalid zone {loc_h}"
                    );
                    0
                }
            }
            AddToScriptZoneAfterTeleport => {
                let loc_h = stack.pop_i32();
                let actor_h = stack.pop_i32();
                if loc_h == 0 {
                    return 0;
                }
                let Some(actor_id) = self.actor_id(actor_h) else {
                    return 0;
                };
                let Some(zone_idx) = self.zone_index(loc_h) else {
                    return 0;
                };
                let Some(zone) = self.script_domains.zones.scripts.get_mut(zone_idx) else {
                    return 0;
                };
                zone.enter(actor_id);
                1
            }
            SetCorpseExistsInBuilding => {
                let _actor = stack.pop_i32();
                // Asserts false — "DESPERADOS STUFF", unused in
                // this game.
                0
            }
            PutActorInBuilding => {
                let bld_h = stack.pop_i32();
                let actor_h = stack.pop_i32();
                if let Some(idx) = Self::building_index(bld_h) {
                    if idx >= self.script_domains.buildings.occupants.len() {
                        self.script_domains
                            .buildings
                            .occupants
                            .resize(idx + 1, Vec::new());
                    }
                    self.script_domains.buildings.occupants[idx].push(actor_h);
                    self.script_domains
                        .buildings
                        .actor_building
                        .insert(actor_h, bld_h);
                }
                // EngineInner applies positioning (inactive + special layer +
                // building sector + gate point_in + DisableAllActionsTemp
                // for PCs) after the script step.
                self.emit_barrier(DeferredCommand::PutActorInBuilding {
                    actor: actor_h,
                    building: bld_h,
                });
                0
            }
            SetBuildingActive => {
                let val = stack.pop_i32();
                let bld_h = stack.pop_i32();
                let active = val != 0;
                if let Some(idx) = Self::building_index(bld_h) {
                    if idx < self.script_domains.buildings.active.len() {
                        self.script_domains.buildings.active[idx] = active;
                    }
                    // Activate/deactivate all gates for this building
                    if let Some(gates) = self.script_domains.buildings.gates.get(idx).cloned() {
                        for &gate_h in &gates {
                            if let Some(door) = self.get_door_mut(gate_h) {
                                door.set_active(active);
                            }
                        }
                    }
                }
                0
            }
            GetAnyActorInsideBuilding => {
                // The original declared the parameter as a
                // building handle at the SCB API level but then
                // cast it to a script-sector type and probed for
                // OBJECT_SCRIPT_SECTOR.  Building sectors do not
                // derive from script-objects, so the cast was UB
                // and the type check almost always failed,
                // routing real callers through an error path
                // (effective return: 0).  We follow the declared
                // API intent and query the building occupant
                // list; if any mission script actually depended
                // on the "always 0" behaviour, this would start
                // returning real occupants.  No shipped SCB
                // appears to rely on it.
                let bld_h = stack.pop_i32();
                Self::building_index(bld_h)
                    .and_then(|idx| self.script_domains.buildings.occupants.get(idx))
                    .and_then(|occ| occ.first().copied())
                    .unwrap_or(0)
            }
            AreAllPCsInside => {
                let loc_h = stack.pop_i32();
                if loc_h == 0 {
                    return 0;
                }
                let all_inside = self.pc_handles().iter().all(|&pc| {
                    self.zone_occupant_handles(loc_h)
                        .is_some_and(|occ| occ.contains(&pc))
                });
                i32::from(all_inside)
            }
            AreAllEnemiesInsideHS => {
                // Returns false if any active Lacklandist
                // soldier inside the zone is still alive,
                // conscious, untied, and not carried.
                let loc_h = stack.pop_i32();
                if loc_h == 0 {
                    return 0;
                }
                let has_living_enemy = self.zone_occupant_handles(loc_h).is_some_and(|occupants| {
                    occupants
                        .iter()
                        .any(|&handle| match self.get_entity(handle) {
                            Some(Entity::Soldier(s)) => {
                                s.element.active
                                    && s.soldier.cached_camp == Camp::Lacklandists
                                    && s.npc.life_points > 0
                                    && !s.human.unconscious
                                    && s.element.posture != Posture::Tied
                                    && s.human.carrier.is_none()
                            }
                            _ => false,
                        })
                });
                i32::from(!has_living_enemy)
            }
            AreAllPCsAliveInside => {
                let loc_h = stack.pop_i32();
                if loc_h == 0 {
                    return 0;
                }
                let all_alive_inside = self.pc_handles().iter().all(|&pc| {
                    // Dead PCs are exempt (the check is
                    // `!is_dead before is_inside`).  Use the
                    // life-points-based `is_dead` (= life_points
                    // <= 0) rather than the posture-derived
                    // check, which only flips true once the
                    // death animation has begun.
                    let is_dead = self.get_entity(pc).is_some_and(|e| e.is_dead());
                    if is_dead {
                        true
                    } else {
                        self.zone_occupant_handles(loc_h)
                            .is_some_and(|occ| occ.contains(&pc))
                    }
                });
                i32::from(all_alive_inside)
            }

            // --- door ---
            IsDoorLockedPC => {
                let h = stack.pop_i32();
                self.get_door(h).map_or(0, |d| i32::from(d.is_locked_pc()))
            }
            IsDoorUnlockable => {
                let h = stack.pop_i32();
                self.get_door(h).map_or(0, |d| i32::from(d.is_unlockable()))
            }
            IsDoorLockedNPCCivilian => {
                let h = stack.pop_i32();
                self.get_door(h)
                    .map_or(0, |d| i32::from(d.is_locked_npc_civilian()))
            }
            IsDoorLockedNPCVillain => {
                let h = stack.pop_i32();
                self.get_door(h)
                    .map_or(0, |d| i32::from(d.is_locked_npc_villain()))
            }
            SetDoorLockedPC => {
                let val = stack.pop_i32();
                let h = stack.pop_i32();
                if let Some(door) = self.get_door_mut(h) {
                    let locked = val != 0;
                    door.set_locked_pc(locked);
                    // Unlocking also activates the door.
                    if !locked {
                        door.set_active(true);
                    }
                }
                0
            }
            SetDoorUnlockable => {
                let val = stack.pop_i32();
                let h = stack.pop_i32();
                if let Some(door) = self.get_door_mut(h) {
                    door.set_unlockable(val != 0);
                }
                0
            }
            SetDoorLockedNPCCivilian => {
                let val = stack.pop_i32();
                let h = stack.pop_i32();
                if let Some(door) = self.get_door_mut(h) {
                    let locked = val != 0;
                    door.set_locked_npc_civilian(locked);
                    if !locked {
                        door.set_active(true);
                    }
                }
                0
            }
            SetDoorLockedNPCVillain => {
                let val = stack.pop_i32();
                let h = stack.pop_i32();
                if let Some(door) = self.get_door_mut(h) {
                    let locked = val != 0;
                    door.set_locked_npc_villain(locked);
                    if !locked {
                        door.set_active(true);
                    }
                }
                0
            }
            SetDoorSpecialAutorisation => {
                let direct = stack.pop_i32();
                let actor_h = stack.pop_i32();
                let door_h = stack.pop_i32();
                let pc_bit = self.pc_authorisation_bit(actor_h);
                if let Some(door) = self.get_door_mut(door_h) {
                    door.grant_special_authorisation(pc_bit, direct != 0);
                }
                0
            }
            ActivateDoorMouseSector => {
                let door_h = stack.pop_i32();
                let active = stack.pop_i32();
                if self.get_door(door_h).is_none() {
                    tracing::warn!(
                        "Script Error: ActivateDoorMouseSector: door {door_h} not found"
                    );
                    return 0;
                }
                let door_idx = Self::door_index(door_h)
                    .expect("validated door handle must retain its door index")
                    as u32;
                let sector_idx = self
                    .fast_grid
                    .level
                    .sectors
                    .iter()
                    .position(|sector| sector.door_index == Some(door_idx))
                    .unwrap_or_else(|| {
                        panic!(
                            "ActivateDoorMouseSector: no grid sector registered for door {door_h}"
                        )
                    });
                self.fast_grid
                    .set_sector_active(sector_idx as u32, active != 0);
                0
            }

            // --- scroll ---
            ThisScroll => self.call_frame.current_scroll(),
            GetScrollStatus => {
                // Null → warn + 0; non-object or non-scroll →
                // "not a scroll" warn + 0; scroll → its status.
                let scroll_h = stack.pop_i32();
                if scroll_h == 0 {
                    tracing::warn!("Script Error: GetScrollStatus with null element");
                    0
                } else {
                    let is_scroll = self
                        .get_entity(scroll_h)
                        .is_some_and(|e| e.kind() == ElementKind::ObjectScroll);
                    if !is_scroll {
                        tracing::warn!(
                            "Script Error: GetScrollStatus on non-scroll element {scroll_h}"
                        );
                        return 0;
                    }
                    self.script_domains
                        .scrolls
                        .status
                        .get(&scroll_h)
                        .copied()
                        .unwrap_or(0)
                }
            }
            SetScrollStatus => {
                // Null/non-object/non-scroll → warn + return;
                // status outside [0, MaxStatus) → warn + return.
                // The setter stores the status, forces the
                // BonusThree animation on Opened, and refreshes
                // the minimap dot.
                let status = stack.pop_i32();
                let scroll_h = stack.pop_i32();
                if scroll_h == 0 {
                    tracing::warn!("Script Error: SetScrollStatus with null element");
                    return 0;
                }
                let is_scroll = self
                    .get_entity(scroll_h)
                    .is_some_and(|e| e.kind() == ElementKind::ObjectScroll);
                if !is_scroll {
                    tracing::warn!(
                        "Script Error: SetScrollStatus on non-scroll element {scroll_h}"
                    );
                    return 0;
                }
                if !(0..=3).contains(&status) {
                    tracing::warn!(
                        "Script Error: SetScrollStatus status {status} out of range (must be 0..=3)"
                    );
                    return 0;
                }
                self.script_domains.scrolls.status.insert(scroll_h, status);
                self.emit_engine(EngineCommand::SetScrollStatus {
                    scroll_handle: scroll_h,
                    status,
                });
                0
            }
            AttachScrollToNPC => {
                // Four branches:
                //   1. !ActorExists || !IsNPC -> warn + early return
                //   2. scroll == NULL          -> attach NULL (detach)
                //   3. !IsObject || GetObjectType() != SCROLL
                //                              -> warn but FALL THROUGH (legacy bug)
                //   4. valid                   -> attach scroll
                // `attach_scroll` strips the previous SPEAK
                // titbit and installs a fresh one whenever the
                // attached scroll pointer differs (relevant for
                // any titbit-index-bound consumer).
                let scroll_h = stack.pop_i32();
                let npc_h = stack.pop_i32();
                // Branch 1: bad NPC handle.
                let npc_is_npc = self.get_entity(npc_h).is_some_and(|e| e.is_npc());
                if !npc_is_npc {
                    tracing::warn!(
                        "Script Error: AttachScrollToNPC with non-NPC actor handle {npc_h}"
                    );
                    return 0;
                }
                if scroll_h == 0 {
                    // Branch 2: detach.
                    if self
                        .script_domains
                        .scrolls
                        .attachments
                        .remove(&npc_h)
                        .is_some()
                    {
                        self.script_domains.scrolls.attachment_dirty.insert(npc_h);
                    }
                } else {
                    // Branch 3: log if not an object/scroll, but match the
                    // Match the legacy fall-through and still
                    // record the attachment.
                    let scroll_ok = self
                        .get_entity(scroll_h)
                        .is_some_and(|e| e.kind() == ElementKind::ObjectScroll);
                    if !scroll_ok {
                        tracing::warn!(
                            "Script Error: AttachScrollToNPC element {scroll_h} is not a scroll object"
                        );
                    }
                    // Branch 4: replace-or-insert; mark dirty when the value
                    // changes so the SPEAK titbit gets re-installed.
                    let prev = self
                        .script_domains
                        .scrolls
                        .attachments
                        .insert(npc_h, scroll_h);
                    if prev != Some(scroll_h) {
                        self.script_domains.scrolls.attachment_dirty.insert(npc_h);
                    }
                }
                0
            }

            _ => self.dispatch_campaign(native, stack),
        }
    }
}
