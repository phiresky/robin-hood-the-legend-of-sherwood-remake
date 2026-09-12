//! Falling-net capture sweep, per-victim release, and per-tick driver.
//!
//! - [`EngineInner::apply_net_falling_effect`]: sweeps every active human
//!   inside `SQUARE_RADIUS_NET_CAPTURE` of the net's landing point,
//!   classifies them as VIP/Rider/Stuteley → "crumple" (Classic) or
//!   "skip" (selective immunity), and launches a `Command::ReceiveNet`
//!   damage element per ordinary victim.
//!
//! - [`EngineInner::unapply_net_effect`]: per-victim, decrement the
//!   stuck-under-nets counter, snap `StuckUnderNet` posture back to
//!   `Lying`, abort lower-priority sequences, queue a wait, dispatch
//!   `EventNetAway` (NPCs only), and remove the victim from every
//!   NPC's `Body` detectable list, including its own.
//!
//! - [`EngineInner::tick_nets`]: per-frame driver. Advances the net's
//!   ballistic trajectory (using the same waypoint loop as
//!   `tick_arrows`) and fires `apply_net_falling_effect` on landing.
//!   Release happens when a PC (or soldier) picks the net up via
//!   `Command::Take` — see the `TakingNet` animation-Done handler in
//!   [`engine/animation.rs`] and the `ObjectType::Net` pickup branch
//!   in [`engine/tick.rs`] that calls `unapply_net_effect` + despawns
//!   the net.

use super::*;
use crate::coordinates::MapPoint;
use crate::coordinates::WorldVec3D;
use crate::element::{Command, Entity, EntityId};

// ─── Constants ───────────────────────────────────────────────────────

/// Square radius (in isometric units) within which humans are caught
/// by a falling net.
const SQUARE_RADIUS_NET_CAPTURE: f32 = 1600.0;

/// Vertical distance below which a falling net starts firing the
/// capture sweep every frame, while still descending.
const NET_DESCENT_APPLY_THRESHOLD: f32 = 60.0;

#[cfg(test)]
thread_local! {
    static NET_SPRITE_PROGRESSIONS: super::test_support::Probe<(EntityId, crate::sprite::FrameProgression)> =
        const { super::test_support::Probe::new() };
}

#[cfg(test)]
fn observe_net_sprite_progression(net: EntityId, progression: crate::sprite::FrameProgression) {
    NET_SPRITE_PROGRESSIONS.with(|trace| trace.record((net, progression)));
}

#[cfg(test)]
fn capture_net_sprite_progressions<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<(EntityId, crate::sprite::FrameProgression)>) {
    NET_SPRITE_PROGRESSIONS.with(|trace| trace.capture(f))
}

/// Cosine threshold for the landing-slope crumple test in
/// [`EngineInner::detect_initial_net_crumple`]: any obstacle with a
/// top-plane normal tilted more than ~30° from vertical (cos ≈ 0.87)
/// is too steep, so the net crumples on landing.
const NET_LANDING_NORMAL_Z_THRESHOLD: f32 = 0.87;

/// Test-radius for the 8-point reach-ring crumple check.
const TEST_RADIUS_NET_CRUMPLED: f32 = 40.0;

impl EngineInner {
    // ════════════════════════════════════════════════════════════════
    //  Falling-net capture sweep
    // ════════════════════════════════════════════════════════════════

    /// Sweep every active human within [`SQUARE_RADIUS_NET_CAPTURE`]
    /// of the net's landing point and either capture them or crumple
    /// the net on a VIP/Rider/Stuteley.
    ///
    /// ## Behaviour summary
    ///
    /// 1. If the net is already crumpled, return immediately.
    /// 2. Iterate every `Entity::*` that `is_active() && is_human()`.
    /// 3. For each, test 3D distance to the net's `projectile.end`
    ///    landing point with Y stretched by [`INVERSE_ASPECT_RATIO`].
    /// 4. Classify in-range humans:
    ///    - **Soldier**: VIP from profile, Rider from `SoldierData`.
    ///    - **Civilian**: VIP from `CivilianType::Vip` profile flag.
    ///    - **PC**: "Stuteley" = has `Action::Net` slot (only Stuteley
    ///      has it in the shipping campaigns).
    /// 5. On a crumple-class victim:
    ///    - If no victims yet: set `crumpled = true`, clear list, stop.
    ///    - Otherwise: stop immediately ("new arrivants won't be
    ///      caught"), keeping the existing victims.
    /// 6. For every other victim not already in the list: append, call
    ///    [`EngineInner::quit_swordfight`], and launch a `Command::ReceiveNet`
    ///    damage element targeting them.
    ///
    /// The `stuck_under_nets_counter` is incremented **eagerly** here.
    /// The posture snap to `StuckUnderNet`, `DetectableType::Body`
    /// broadcast, and `EventNet` AI stimulus run on the next frame
    /// inside [`EngineInner::apply_net`] (`engine/melee.rs`) when the
    /// queued `Command::ReceiveNet` damage element dispatches.
    pub(crate) fn apply_net_falling_effect(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        net_id: EntityId,
    ) {
        // ── Snapshot the net's state up front ──────────────────────
        let (already_crumpled, landing_pos, mut victims_snapshot) = match self.get_entity(net_id) {
            Some(Entity::Net(n)) => (n.net.crumpled, n.projectile.end, n.net.victims.clone()),
            _ => {
                tracing::warn!(?net_id, "apply_net_falling_effect: not a net entity");
                return;
            }
        };
        if already_crumpled {
            return;
        }

        // ── Sweep candidates ───────────────────────────────────────
        // Build a snapshot of (id, position) for every active human.
        // 3D position with a Y-stretched isometric square-norm is used
        // for the proximity test.
        let candidates: Vec<EntityId> = self
            .world
            .entities
            .humans()
            .filter_map(|(id, e)| if e.is_active() { Some(id.into()) } else { None })
            .collect();

        let mut new_victims: Vec<EntityId> = Vec::new();
        let mut should_crumple = false;

        for actor_id in candidates {
            let entity = match self.get_entity(actor_id) {
                Some(e) => e,
                None => continue,
            };
            let pos = entity.element_data().position();
            let dx = pos.x - landing_pos.x;
            let dz = pos.z - landing_pos.z;
            let sq_xy =
                crate::position_interface::vector_square_norm_iso(dx, pos.y - landing_pos.y);
            if sq_xy + dz * dz >= SQUARE_RADIUS_NET_CAPTURE {
                continue;
            }

            // Classify: VIP / Rider / Stuteley → crumple; else stick.
            let (is_vip, is_rider, is_stuteley, is_soldier_vip) = match entity {
                Entity::Soldier(s) => {
                    let vip = assets
                        .profile_manager
                        .get_soldier(s.soldier.soldier_profile_index)
                        .unwrap_or_else(|| {
                            panic!(
                                "net sweep requires missing soldier profile {:?} for {actor_id:?}",
                                s.soldier.soldier_profile_index
                            )
                        })
                        .vip;
                    (vip, s.soldier.rider, false, vip)
                }
                Entity::Civilian(c) => {
                    let vip = assets
                        .profile_manager
                        .civilians
                        .get(usize::from(c.civilian.civilian_profile_index))
                        .unwrap_or_else(|| {
                            panic!(
                                "net sweep requires missing civilian profile {} for {actor_id:?}",
                                c.civilian.civilian_profile_index
                            )
                        })
                        .civilian_type
                        == crate::profiles::CivilianType::Vip;
                    (vip, false, false, false)
                }
                Entity::Pc(pc) => {
                    // In the shipping campaigns only Stuteley has the
                    // Net action in his main action slots, so the
                    // action check doubles as a Stuteley check.
                    let stuteley = assets
                        .profile_manager
                        .get_character(pc.pc.profile_index)
                        .unwrap_or_else(|| {
                            panic!(
                                "net sweep requires missing character profile {:?} for {actor_id:?}",
                                pc.pc.profile_index
                            )
                        })
                        .has_action(crate::profiles::Action::Net);
                    (false, false, stuteley, false)
                }
                _ => (false, false, false, false),
            };

            if is_vip || is_rider || is_stuteley {
                // VIP soldiers play the VipNetNo remark on the crumple
                // path; this only fires for VIPs, not riders/Stuteley.
                if is_soldier_vip
                    && let Some(entity) = self.world.entities.get_mut(actor_id)
                    && let Some(npc) = entity.npc_data_mut()
                    && let Some(base) = npc.ai_brain.base_mut()
                {
                    base.say(crate::ai::Remark::VipNetNo);
                }
                if is_soldier_vip {
                    self.drain_ai_owner_work_for(sim, assets, actor_id);
                }
                if self.control.sim_config.item_gameplay.net_selective_immunity {
                    // Rebalanced behavior: resistant actors remain immune but
                    // cannot invalidate ordinary captures elsewhere in the
                    // original strict 40-unit landing circle.
                    continue;
                }
                if victims_snapshot.is_empty() {
                    should_crumple = true;
                    break;
                } else {
                    // "New arrivants won't be caught": keep existing
                    // victims, leave crumpled = false. The sprite still
                    // flips to the crumple-unfold cycle unconditionally
                    // on the VIP/Rider/Stuteley path — even when
                    // existing victims prevent a full crumple.
                    if let Some(Entity::Net(n)) = self.get_entity_mut(net_id) {
                        n.object.animation = crate::element::Animation::NetUnfoldingCrumpled;
                    }
                    return;
                }
            } else {
                new_victims.push(actor_id);
            }
        }

        // ── Crumple branch ──────────────────────────────────────────
        if should_crumple {
            if let Some(Entity::Net(n)) = self.get_entity_mut(net_id) {
                n.net.crumpled = true;
                n.net.victims.clear();
                // Switch the sprite into its crumple-unfold cycle the
                // moment the crumple is decided.
                n.object.animation = crate::element::Animation::NetUnfoldingCrumpled;
            }
            tracing::debug!(
                ?net_id,
                "Net crumpled on landing (VIP/Rider/Stuteley in radius)"
            );
            return;
        }

        // ── Capture branch ──────────────────────────────────────────
        for victim_id in new_victims {
            if victims_snapshot.contains(&victim_id) {
                continue;
            }
            victims_snapshot.push(victim_id);

            // Append to the net's persistent list.
            if let Some(Entity::Net(n)) = self.get_entity_mut(net_id)
                && !n.net.victims.contains(&victim_id)
            {
                n.net.victims.push(victim_id);
            }

            // Eager counter bump — posture is left alone;
            // `EngineInner::apply_net` snaps it to StuckUnderNet next
            // frame when the ReceiveNet element dispatches.
            if let Some(entity) = self.world.entities.get_mut(victim_id)
                && let Some(human) = entity.human_data_mut()
            {
                crate::combat::increment_stuck_under_net(human);
            }

            self.quit_swordfight(sim, assets, victim_id);

            // Launch a ReceiveNet damage element (damage/concussion = 0
            // — the handler reads only the origin pointer).
            let elem = crate::sequence::SequenceElement::new_damage(
                1,
                Command::ReceiveNet,
                Some(victim_id),
                Some(net_id),
                0,
                0,
            );
            self.launch_element(elem);

            // Set the victim's sprite to draw behind the net so the
            // net visually covers them. The display-order pipeline is
            // sprite-driven and only needs the reference + flag set
            // once per capture.
            if let Some(entity) = self.world.entities.get_mut(victim_id) {
                let sprite = &mut entity.element_data_mut().sprite;
                sprite.display_order_ref = Some(net_id);
                sprite.behind_display_order_ref = true;
            }
        }

        tracing::debug!(
            ?net_id,
            victim_count = victims_snapshot.len(),
            "Net captured victims on landing"
        );
    }

    // ════════════════════════════════════════════════════════════════
    //  Per-victim release
    // ════════════════════════════════════════════════════════════════

    /// Release every human currently captured by `net_id`.
    ///
    /// Per victim:
    /// 1. Decrement the stuck-under-nets counter via
    ///    [`Entity::remove_net_from_human`] (which also clears
    ///    `Posture::StuckUnderNet` back to `Lying` if no other net is
    ///    still holding the victim down).
    /// 2. Stop in-progress sequences with `Injury` priority.
    /// 3. Launch a `Command::Wait` element so the actor parks idle.
    /// 4. For NPCs, dispatch `StimulusType::EventNetAway` (their AI
    ///    transitions out of the wondering-under-net substate) and
    ///    remove the victim from every other NPC's `Body` detectable
    ///    list.
    /// 5. Clear the net's `victims` list.
    pub(crate) fn unapply_net_effect(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        net_id: EntityId,
    ) {
        // Snapshot + drain the victim list and the repulsive-point IDs
        // so we can iterate without re-borrowing the net entity.
        let (victims, repulsive_ids): (Vec<EntityId>, Vec<i32>) = match self.get_entity_mut(net_id)
        {
            Some(Entity::Net(n)) => (
                std::mem::take(&mut n.net.victims),
                std::mem::take(&mut n.net.repulsive_point_ids),
            ),
            _ => {
                tracing::warn!(?net_id, "unapply_net_effect: not a net entity");
                return;
            }
        };

        // Tear down the net's pathfinding repulsion points so the
        // pathfinder stops seeing them next tick.
        if !repulsive_ids.is_empty() {
            self.ai
                .global
                .repulsive_points
                .retain(|p| !repulsive_ids.contains(&p.id));
        }

        for victim_id in victims {
            // ── 1. Decrement counter / unstick posture ─────────────
            // `Entity::remove_net_from_human` decrements the counter and snaps
            // posture out of StuckUnderNet atomically.
            let was_stuck = match self.get_entity_mut(victim_id) {
                Some(e) => e.remove_net_from_human(),
                None => continue,
            };

            // The remaining steps only run when this was the last net
            // holding the victim.
            if !was_stuck {
                continue;
            }

            // A netted human transitioning back from StuckUnderNet
            // must not be dead or unconscious. Use `debug_assert!` so
            // dev builds catch the violation but release builds
            // tolerate unusual scripted states.
            debug_assert!(
                self.get_entity(victim_id)
                    .map(|e| !e.is_dead() && !e.human_data().is_some_and(|h| h.unconscious))
                    .unwrap_or(true),
                "victim {victim_id:?} is dead or unconscious during net release"
            );

            // ── 2. Stop in-progress sequences (priority Injury) ─────
            // Use the engine wrapper so the movement-element transition
            // rewrite + path cancel runs (the bare
            // `SequenceManager::stop_owner` skips both).
            self.stop_owner(victim_id, crate::sequence::SequencePriority::Injury);

            // ── 3. Park the victim with a Wait element ──────────────
            self.actor_wait(victim_id);

            // Clear the "behind net" sprite reference so the victim
            // goes back to normal Y-sorting.
            if let Some(entity) = self.world.entities.get_mut(victim_id) {
                let sprite = &mut entity.element_data_mut().sprite;
                sprite.display_order_ref = None;
                sprite.behind_display_order_ref = false;
            }

            // ── 4. NPC-only AI + detectable cleanup ─────────────────
            let victim_is_npc = self
                .get_entity(victim_id)
                .map(|e| e.is_npc())
                .unwrap_or(false);
            if victim_is_npc {
                self.dispatch_ai_stimulus(
                    victim_id,
                    crate::ai::Stimulus::new(crate::ai::StimulusType::EventNetAway),
                );
                // Original-game net removal sends the net-away event
                // synchronously, even when the victim's creation slot has
                // already run this frame.
                self.tick_enemy_ai_drain_pending_stimuli_for_npc(
                    sim, victim_id, assets, None, None,
                );

                // Skip the body-detectable cleanup for dead/unconscious
                // victims — their body is genuinely a body to detect.
                let still_alive = self
                    .get_entity(victim_id)
                    .map(|e| !e.is_dead() && !e.human_data().is_some_and(|h| h.unconscious))
                    .unwrap_or(false);
                if still_alive {
                    self.delete_body_detectable_for_all_npc(victim_id);
                }
            }
        }

        tracing::debug!(?net_id, "Net effect unapplied; victims released");
    }

    /// Remove `body_id` from every NPC's `DetectableType::Body` list.
    ///
    /// This is the inverse of
    /// [`EngineInner::broadcast_body_detectable`] (`engine/ai.rs`).
    fn delete_body_detectable_for_all_npc(&mut self, body_id: EntityId) {
        use crate::element::DetectableType;
        let det_idx = DetectableType::Body as usize;
        let npc_ids: Vec<_> = self.world.entities.npc_ids().collect();
        for friend_id in npc_ids {
            if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(friend_id)
                && det_idx < s.npc.detectable_lists.len()
            {
                s.npc.delete_detectable(body_id, DetectableType::Body);
            } else if let Some(Entity::Civilian(c)) = self.world.entities.get_mut(friend_id)
                && det_idx < c.npc.detectable_lists.len()
            {
                c.npc.delete_detectable(body_id, DetectableType::Body);
            }
        }
    }

    // ════════════════════════════════════════════════════════════════
    //  Per-frame net driver
    // ════════════════════════════════════════════════════════════════

    /// Advance one active net by one frame at its creation-order position.
    ///
    /// * **In flight**: advance the ballistic trajectory; decrement
    ///   `time_till_unfolding` and switch the sprite animation to
    ///   `NetUnfolding`/`NetUnfoldingCrumpled` when it hits 0; fire
    ///   [`EngineInner::apply_net_falling_effect`] every frame the
    ///   net is within [`NET_DESCENT_APPLY_THRESHOLD`] of its landing
    ///   point and still descending.
    /// * **Landing transition** (`flying` → not flying with
    ///   `was_flying = true`): snap Z to the landing obstacle's top
    ///   plane and register the dual repulsive points so actors path
    ///   around the net.
    /// * **On the ground**: resolve the post-landing animation
    ///   transition (`NetUnfolding` → `ObjectLying`/`NetMoving`,
    ///   `NetUnfoldingCrumpled` → `NetLyingCrumpled`) without advancing the
    ///   new row that tick. Stationary `NetMoving` stays on that row and uses
    ///   frozen progression without transitioning back to a lying object. Release
    ///   happens via `Command::Take` pickup — the `TakingNet`
    ///   animation-Done handler in `engine/animation.rs` queues a
    ///   net-antagonist pickup, and the pickup branch in
    ///   `engine/tick.rs` calls [`EngineInner::unapply_net_effect`] +
    ///   despawns the net.
    pub(crate) fn tick_net(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        net_id: EntityId,
    ) {
        let was_flying = match self.get_entity(net_id) {
            Some(Entity::Net(net)) => net.projectile.flying,
            _ => return,
        };
        // Phase 1: advance trajectory + classify the net into
        // (descending-near-landing, just-landed) and stamp the
        // in-flight animation transitions on it directly.
        let (apply, just_landed, skip_sprite_this_tick) = {
            let Some(Entity::Net(net)) = self.world.entities.get_mut(net_id) else {
                return;
            };
            let mut apply = false;
            let mut just_landed = false;
            let mut skip_sprite_this_tick = false;
            if net.projectile.flying {
                // Net ticking ignores the base projectile tick's false
                // result and always continues/returns true. An inactive net
                // therefore skips only the base movement body.
                if net.element.active {
                    advance_net_trajectory(net);
                }

                // `time_till_unfolding` countdown — when it hits 0,
                // switch animation to NetUnfolding (or _Crumpled if the
                // spawn-time crumple test flagged it). Subsequent
                // frames leave the animation alone; the sprite plays
                // out until the landing transition.
                if net.net.time_till_unfolding > 0 {
                    // The original game's nonzero unfolding-timer branch never
                    // reaches either sprite increment, including when this
                    // decrement changes the counter to zero and selects the
                    // unfolding animation.
                    skip_sprite_this_tick = true;
                    net.net.time_till_unfolding -= 1;
                    if net.net.time_till_unfolding == 0 {
                        net.object.animation = if net.net.crumpled {
                            crate::element::Animation::NetUnfoldingCrumpled
                        } else {
                            crate::element::Animation::NetUnfolding
                        };
                    }
                }

                // Multi-frame descent apply — fire the capture sweep
                // each frame the net is within the descent threshold
                // of its landing point and still descending. The sweep
                // dedups against existing victims, so re-firing only
                // adds late-arrivers.
                let z_above_landing = net.element.position().z - net.projectile.end.z;
                let descending = net.projectile.velocity_increment.z < 0.0;
                if net.projectile.flying
                    && z_above_landing <= NET_DESCENT_APPLY_THRESHOLD
                    && descending
                {
                    apply = true;
                }

                if !net.projectile.flying && net.net.was_flying {
                    // Just landed this frame — queue the landing-time
                    // work for phase 2 (which holds `&mut self` so it
                    // can register repulsive points + look up obstacles).
                    apply = true;
                    just_landed = true;
                    net.net.was_flying = false;
                }
            } else {
                // The two transition cases only assign animation and return;
                // the newly selected row must not advance until next tick.
                match net.object.animation {
                    crate::element::Animation::NetUnfolding => {
                        net.object.animation = if net.net.victims.is_empty() {
                            crate::element::Animation::ObjectLying
                        } else {
                            crate::element::Animation::NetMoving
                        };
                        net.net.landed_animation_resolved = true;
                        skip_sprite_this_tick = true;
                    }
                    crate::element::Animation::NetUnfoldingCrumpled => {
                        net.object.animation = crate::element::Animation::NetLyingCrumpled;
                        net.net.landed_animation_resolved = true;
                        skip_sprite_this_tick = true;
                    }
                    _ => {}
                }
            }
            (apply, just_landed, skip_sprite_this_tick)
        };

        // Phase 2: apply effects (mutable engine borrow released above).
        if apply {
            self.apply_net_falling_effect(sim, assets, net_id);
        }
        if just_landed {
            self.apply_projectile_landing_resolution(assets, net_id);
            self.snap_net_to_landing_obstacle(sim, assets, net_id);
            self.register_net_repulsive_points(net_id);
        }

        // The net's sprite tail is inside its update. FreezeAll
        // suppresses only this sprite operation; trajectory, capture, landing,
        // and bookkeeping above continue.
        let progression = if skip_sprite_this_tick {
            None
        } else if was_flying {
            match self.get_entity(net_id) {
                Some(Entity::Net(net))
                    if net.object.animation == crate::element::Animation::ObjectFlying =>
                {
                    Some(crate::sprite::FrameProgression::SkipShadow)
                }
                Some(Entity::Net(_)) => {
                    Some(crate::sprite::FrameProgression::SkipShadowFreezeWhenTerminated)
                }
                _ => None,
            }
        } else {
            match self.get_entity(net_id) {
                Some(Entity::Net(net)) => match net.object.animation {
                    crate::element::Animation::ObjectLying
                    | crate::element::Animation::NetLyingCrumpled => {
                        Some(crate::sprite::FrameProgression::Default)
                    }
                    crate::element::Animation::NetMoving => {
                        if self.any_victim_is_moving(&net.net.victims) {
                            Some(crate::sprite::FrameProgression::Default)
                        } else {
                            Some(crate::sprite::FrameProgression::Frozen)
                        }
                    }
                    crate::element::Animation::NetBeingTaken => {
                        Some(crate::sprite::FrameProgression::FreezeWhenTerminated)
                    }
                    _ => None,
                },
                _ => None,
            }
        };
        if let Some(progression) = progression
            && !self.actors_frozen()
            && let Some(Entity::Net(net)) = self.get_entity_mut(net_id)
        {
            #[cfg(test)]
            observe_net_sprite_progression(net_id, progression);
            net.element
                .sprite
                .perform_virgin_increment(sim, progression);
        }
    }

    // ════════════════════════════════════════════════════════════════
    //  Landing-time helpers
    // ════════════════════════════════════════════════════════════════

    /// Snap the net's elevation to the top plane of the obstacle it
    /// lands on (with a small epsilon offset so it sits *on* rather
    /// than *in* the obstacle).
    ///
    /// When the net lands on bare ground (no obstacle at the landing
    /// 2D point) the elevation is also reset to a tiny positive
    /// epsilon to avoid Z-fighting — that's the `0.001` offset below.
    fn snap_net_to_landing_obstacle(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        net_id: EntityId,
    ) {
        let (landing_xy, layer) = match self.get_entity(net_id) {
            Some(Entity::Net(n)) => (
                (n.element.position().x, n.element.position().y),
                n.element.optional_layer(),
            ),
            _ => return,
        };

        // Branch on `(layer, find_landing_obstacle)`:
        //   - no obstacle           → 0.001 (we should already have
        //                             arrived with elevation ≈ 0)
        //   - obstacle, layer valid → snap to top plane + 0.001
        //   - obstacle, no layer → keep current elevation,
        //     so a crumpled-launched-no-layer net doesn't get clamped
        //     to 0.001.
        let obstacle_idx = self.find_landing_obstacle(
            assets,
            crate::coordinates::WorldPoint3D {
                x: landing_xy.0,
                y: landing_xy.1,
                z: 0.0,
            },
        );
        let new_z: Option<f32> = match (layer, obstacle_idx) {
            (None, Some(_)) => None, // keep current elevation
            (None, None) => Some(0.001),
            (Some(_), Some(idx)) => Some(
                assets
                    .environment
                    .static_sight_obstacles
                    .get(idx)
                    .or_else(|| {
                        self.world
                            .dynamic_sight_obstacles
                            .get(idx - assets.environment.static_sight_obstacles.len())
                    })
                    .map(|o| o.compute_top_z(landing_xy.0, landing_xy.1) + 0.001)
                    .unwrap_or(0.001),
            ),
            (Some(_), None) => Some(0.001),
        };

        if let Some(Entity::Net(n)) = self.get_entity_mut(net_id) {
            let mut p = n.element.position();
            if let Some(z) = new_z {
                p.z = z;
            }
            n.element.set_position(p);
            // Recompute the 2D map projection.
            n.element
                .set_position_map(MapPoint::from_world_xyz(p.x, p.y, p.z));
        }

        // Broadcast the BONK so nearby NPCs react to the thud of the
        // landed net.
        let origin = MapPoint::new(landing_xy.0, landing_xy.1);
        self.broadcast_noise_synchronously(
            sim,
            assets,
            crate::ai::NoiseType::Bonk,
            origin,
            layer,
            crate::parameters_ai::NOISE_VOLUME_BONK as u16,
            new_z.unwrap_or(0.001).max(0.0) as u16,
            Some(net_id),
        );
    }

    /// Register the two `RepulsivePoint`s that prevent NPCs from
    /// pathing through a landed net. Registers them once on landing
    /// and tears them down on `unapply_net_effect`.
    ///
    /// Two points at the same map position with `(radius,
    /// action_radius)` = `(40, 15)` and `(15, 30)`. Crumpled nets
    /// would have their own radii, but that branch is disabled in the
    /// reference, so we use the same dual-point setup regardless of
    /// crumple state.
    ///
    /// ## Other object-class entities
    ///
    /// Every non-Net object subclass either contributes nothing
    /// (Bonus, Scroll, base Projectile, Arrow, Stone, Apple, WaspNest,
    /// Cape, Wasp — all radius 0) or explicitly skips registration
    /// (Coin). The two subclasses that *would* contribute points are
    /// Purse (radius 7) and Ale (radius 5); both are projectile
    /// variants here (`ObjectType::Purse` / `ObjectType::Ale`). The
    /// anti-collision loop that queries these is not yet implemented, so
    /// no landed-purse/ale repulsion is wired up — once that loop is
    /// implemented, it should follow this same persistent-registration
    /// pattern.
    fn register_net_repulsive_points(&mut self, net_id: EntityId) {
        // Snapshot landing pos.
        let pos = match self.get_entity(net_id) {
            Some(Entity::Net(n)) => n.element.position_map(),
            _ => return,
        };
        let configs = [(40.0_f32, 15.0_f32), (15.0_f32, 30.0_f32)];
        let mut ids: Vec<i32> = Vec::with_capacity(2);
        for (radius, action_radius) in configs {
            let id = self.ai.global.next_repulsive_point_id;
            self.ai.global.next_repulsive_point_id += 1;
            self.ai
                .global
                .repulsive_points
                .push(crate::ai::RepulsivePoint::new(
                    id,
                    crate::ai::Position {
                        x: pos.x,
                        y: pos.y,
                        ..Default::default()
                    },
                    radius,
                    action_radius,
                    0,
                ));
            ids.push(id);
        }
        if let Some(Entity::Net(n)) = self.get_entity_mut(net_id) {
            n.net.repulsive_point_ids = ids;
        }
    }

    /// Returns `true` if any of the given victims is currently playing
    /// the wriggle-under-net animation.
    fn any_victim_is_moving(&self, victims: &[EntityId]) -> bool {
        for &v in victims {
            if self.get_entity(v).is_none() {
                continue;
            }
            // The victim's currently-active order animation on the
            // owning sequence element.
            if let Some((_, _, order)) = self.orders.sequence_manager.current_order_for_actor(v)
                && order.order_type == crate::order::OrderType::WriggleUnderNet
            {
                return true;
            }
        }
        false
    }

    // ════════════════════════════════════════════════════════════════
    //  Spawn-time crumple detection
    // ════════════════════════════════════════════════════════════════

    /// Decide at spawn time whether a freshly-thrown net will land
    /// crumpled (because it lands on too-steep terrain or its
    /// landing-area ring is blocked by obstacles).
    ///
    /// The `time_till_unfolding` initialization lives in `bow_shot.rs`
    /// where the net entity is constructed.
    ///
    /// Crumple signals:
    /// 1. **Missing layer**: `layer == None` means the net
    ///    had no valid landing surface at all → crumple.
    /// 2. **Slope**: the obstacle the net lands on has a top-plane
    ///    normal whose Z component ≤ [`NET_LANDING_NORMAL_Z_THRESHOLD`]
    ///    (~30° from vertical) → crumple.
    /// 3. **Ring blocked**: any of the 8 cardinal points around the
    ///    landing centre at radius [`TEST_RADIUS_NET_CRUMPLED`] either
    ///    can't be reached from the centre OR has a clear vertical
    ///    drop below it (the second test catches "net hangs over a
    ///    ledge" scenarios) → crumple.
    ///
    /// Caller should invoke this immediately after `spawn_net` adds
    /// the net entity to the engine.
    pub(crate) fn detect_initial_net_crumple(&mut self, assets: &LevelAssets, net_id: EntityId) {
        // ── Snapshot landing pos, layer; bail if not a net ─────────
        let (layer, landing) = match self.get_entity(net_id) {
            Some(Entity::Net(n)) => (n.element.optional_layer(), n.projectile.end),
            _ => {
                tracing::warn!(?net_id, "detect_initial_net_crumple: not a net entity");
                return;
            }
        };
        if self.predict_net_crumple_at(assets, landing, layer) {
            self.set_net_crumpled(net_id);
        }
    }

    /// Pure predicate form of [`detect_initial_net_crumple`] — takes a
    /// landing point + layer and returns `true` when a net dropped
    /// there would crumple. Used by the Easy-difficulty trajectory
    /// preview to tint the arc pink before the net is actually thrown.
    pub fn predict_net_crumple_at(
        &self,
        assets: &LevelAssets,
        landing: crate::coordinates::WorldPoint3D,
        layer: Option<crate::position_interface::Layer>,
    ) -> bool {
        // No valid landing layer at all → crumple.
        let Some(layer) = layer else { return true };

        let obstacle_idx = self.find_landing_obstacle(assets, landing);

        // Slope check.
        if let Some(idx) = obstacle_idx {
            let nz = assets
                .environment
                .static_sight_obstacles
                .get(idx)
                .or_else(|| {
                    self.world
                        .dynamic_sight_obstacles
                        .get(idx - assets.environment.static_sight_obstacles.len())
                })
                .map(top_plane_normal_z)
                .unwrap_or(1.0);
            if nz <= NET_LANDING_NORMAL_Z_THRESHOLD {
                return true;
            }
        }

        // 8-point reach-ring check.
        let centre_2d = (landing.x, landing.y);
        let centre_z = landing.z;
        let mut radius = (TEST_RADIUS_NET_CRUMPLED, 0.0_f32);
        let quarter_turn = std::f32::consts::FRAC_PI_4;

        for _ in 0..8 {
            radius = rotate_2d(radius, quarter_turn);
            let test_x = centre_2d.0 + radius.0;
            let test_y = centre_2d.1 + radius.1;

            // When there's an obstacle, project the ring sample onto
            // the obstacle's top plane along the screen-Y axis
            // (`y = y - z`); when there isn't, the projected point
            // keeps its world Y and the projected Z is 0.
            let (test_proj_y, test_proj_z) = if let Some(idx) = obstacle_idx {
                let proj_z = assets
                    .environment
                    .static_sight_obstacles
                    .get(idx)
                    .or_else(|| {
                        self.world
                            .dynamic_sight_obstacles
                            .get(idx - assets.environment.static_sight_obstacles.len())
                    })
                    .map(|o| o.compute_top_z(test_x, test_y))
                    .unwrap_or(0.0);
                // projected_y = (test_y - centre_z) + projected_z
                (test_y - centre_z + proj_z, proj_z)
            } else {
                (test_y, 0.0)
            };

            let p_test = crate::coordinates::WorldPoint3D {
                x: test_x,
                y: test_proj_y,
                z: test_proj_z + 20.0,
            };
            let p_centre_high = crate::coordinates::WorldPoint3D {
                x: landing.x,
                y: landing.y,
                z: centre_z + 20.0,
            };
            if !self.is_reachable_solid(assets, p_test, p_centre_high, layer.get()) {
                return true;
            }

            let p_drop = crate::coordinates::WorldPoint3D {
                x: test_x,
                y: test_proj_y,
                z: test_proj_z - 40.0,
            };
            if self.is_reachable_solid(assets, p_test, p_drop, layer.get()) {
                return true;
            }
        }

        false
    }

    /// Helper: flip the net's `crumpled` flag.  Defensive — if the
    /// entity is gone or no longer a net, do nothing.
    fn set_net_crumpled(&mut self, net_id: EntityId) {
        if let Some(Entity::Net(n)) = self.get_entity_mut(net_id) {
            n.net.crumpled = true;
            tracing::debug!(?net_id, "Net flagged crumpled at spawn");
        }
    }

    /// Find the first sight obstacle whose 2D bbox contains the
    /// landing point. Returns a flat index spanning
    /// `LevelAssets::static_sight_obstacles` first, then
    /// `dynamic_sight_obstacles`.
    ///
    /// We don't model a position-interface obstacle handle yet, so a
    /// direct point-in-bbox scan is the simplest faithful equivalent
    /// of the original projectile-obstacle lookup.
    fn find_landing_obstacle(
        &self,
        assets: &LevelAssets,
        landing: crate::coordinates::WorldPoint3D,
    ) -> Option<usize> {
        for (i, o) in assets.environment.static_sight_obstacles.iter().enumerate() {
            if obstacle_bbox_contains(o, landing.x, landing.y) {
                return Some(i);
            }
        }
        let base = assets.environment.static_sight_obstacles.len();
        for (i, o) in self.world.dynamic_sight_obstacles.iter().enumerate() {
            if obstacle_bbox_contains(o, landing.x, landing.y) {
                return Some(base + i);
            }
        }
        None
    }

    /// 3D ray reachability against `SIGHTOBSTACLE_SOLID` obstacles.
    /// Wrapper around [`FastFindGrid::is_reachable_3d`] that passes
    /// both static and dynamic sight obstacles in the
    /// `SIGHTOBSTACLE_SOLID` filter.
    fn is_reachable_solid(
        &self,
        assets: &LevelAssets,
        origin: crate::coordinates::WorldPoint3D,
        destination: crate::coordinates::WorldPoint3D,
        layer: u16,
    ) -> bool {
        let obstacles = self.sight_obstacles(assets);
        self.world.fast_grid.is_reachable_3d(
            origin,
            destination,
            layer,
            crate::sight_obstacle::SIGHTOBSTACLE_SOLID,
            obstacles,
        )
    }
}

/// Rotate a 2D vector by `angle` radians. Used by the
/// crumple-detection 8-point ring iteration.
fn rotate_2d((x, y): (f32, f32), angle: f32) -> (f32, f32) {
    let (s, c) = angle.sin_cos();
    (x * c - y * s, x * s + y * c)
}

/// Compute the Z component of an obstacle's top-plane normal.
/// Inline copy of `engine::melee::EngineInner::obstacle_top_normal`
/// (which is private to the melee module). Returns 1.0 (flat) for
/// degenerate obstacles.
fn top_plane_normal_z(obstacle: &crate::sight_obstacle::SightObstacle) -> f32 {
    let [p0, p1, p2] = obstacle.top_plane_points;
    let u = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let v = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
    let nz = u[0] * v[1] - u[1] * v[0];
    let nx = u[1] * v[2] - u[2] * v[1];
    let ny = u[2] * v[0] - u[0] * v[2];
    let len = (nx * nx + ny * ny + nz * nz).sqrt();
    if len < 1e-6 {
        return 1.0;
    }
    let normalized = nz / len;
    // Match `obstacle_top_normal`'s "ensure normal points up" flip.
    normalized.abs()
}

/// Point-in-bbox test for the 2D ground-plane bounding box of a
/// sight obstacle.
fn obstacle_bbox_contains(o: &crate::sight_obstacle::SightObstacle, x: f32, y: f32) -> bool {
    o.box_ground
        .contains_point(crate::coordinates::GroundPoint::new(x, y))
}

/// Advance a single net's ballistic trajectory by one frame.
///
/// This is the trajectory-pop / increment-apply / land-detection slice
/// of [`tick_arrows`]. Nets don't shield-block, hit FX targets, or
/// damage humans on flight, so all we need is the ballistic step + a
/// "trajectory exhausted → flying = false" landing signal.
fn advance_net_trajectory(net: &mut crate::element::ElementNet) {
    let proj = &mut net.projectile;

    if proj.trajectory_frame_count == 0 {
        if !proj.trajectory.is_empty() {
            let point = proj.trajectory.remove(0);
            let time = point.time.max(1);
            proj.trajectory_frame_count = time - 1;

            let current = net.element.position();
            let factor = 1.0 / time as f32;
            proj.velocity_increment = WorldVec3D {
                x: (point.position.x - current.x) * factor,
                y: (point.position.y - current.y) * factor,
                z: (point.position.z - current.z) * factor,
            };
            proj.end = point.position;
        } else {
            proj.flying = false;
            return;
        }
    } else {
        proj.trajectory_frame_count -= 1;
    }

    let mut p = net.element.position();
    p.x += proj.velocity_increment.x;
    p.y += proj.velocity_increment.y;
    p.z += proj.velocity_increment.z;
    net.element.set_position(p);
    net.element
        .set_position_map(MapPoint::from_world_xyz(p.x, p.y, p.z));
    let vx = proj.velocity_increment.x;
    let vy = proj.velocity_increment.y;
    if vx != 0.0 || vy != 0.0 {
        net.element
            .set_direction_instantly(crate::position_interface::vector_to_sector_0_to_15(vx, vy));
    }

    proj.frame_count = proj.frame_count.saturating_add(1);
}

#[cfg(test)]
#[path = "nets/tests.rs"]
mod tests;
