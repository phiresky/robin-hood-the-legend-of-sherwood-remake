//! Falling-net capture sweep, per-victim release, and per-tick driver.
//!
//! - [`EngineInner::apply_net_falling_effect`]: sweeps every active human
//!   inside `SQUARE_RADIUS_NET_CAPTURE` of the net's landing point,
//!   classifies them as VIP/Rider/Stuteley → "crumple" or normal →
//!   "stick", and launches a `Command::ReceiveNet` damage element per
//!   non-crumple victim.
//!
//! - [`EngineInner::unapply_net_effect`]: per-victim, decrement the
//!   stuck-under-nets counter, snap `StuckUnderNet` posture back to
//!   `Lying`, abort lower-priority sequences, queue a wait, dispatch
//!   `EventNetAway` (NPCs only), and remove the victim from every
//!   NPC's `Body` detectable list.
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

/// Cosine threshold for the landing-slope crumple test in
/// [`EngineInner::detect_initial_net_crumple`]: any obstacle with a
/// top-plane normal tilted more than ~30° from vertical (cos ≈ 0.87)
/// is too steep, so the net crumples on landing.
const NET_LANDING_NORMAL_Z_THRESHOLD: f32 = 0.87;

/// Test-radius for the 8-point reach-ring crumple check.
const TEST_RADIUS_NET_CRUMPLED: f32 = 40.0;

/// Marker layer value meaning "no associated obstacle layer".
const INVALID_LAYER: u16 = u16::MAX;

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
    pub(crate) fn apply_net_falling_effect(&mut self, assets: &LevelAssets, net_id: EntityId) {
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
            .entities_iter_with_id()
            .filter_map(|(id, e)| {
                if e.is_active() && e.is_human() {
                    Some(id)
                } else {
                    None
                }
            })
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
                        .map(|p| p.vip)
                        .unwrap_or(false);
                    (vip, s.soldier.rider, false, vip)
                }
                Entity::Civilian(c) => {
                    let vip = assets
                        .profile_manager
                        .civilians
                        .get(usize::from(c.civilian.civilian_profile_index))
                        .map(|p| p.civilian_type == crate::profiles::CivilianType::Vip)
                        .unwrap_or(false);
                    (vip, false, false, false)
                }
                Entity::Pc(pc) => {
                    // In the shipping campaigns only Stuteley has the
                    // Net action in his main action slots, so the
                    // action check doubles as a Stuteley check.
                    let stuteley = assets
                        .profile_manager
                        .get_character(pc.pc.profile_index)
                        .is_some_and(|cp| cp.has_action(crate::profiles::Action::Net));
                    (false, false, stuteley, false)
                }
                _ => (false, false, false, false),
            };

            if is_vip || is_rider || is_stuteley {
                // VIP soldiers play the VipNetNo remark on the crumple
                // path; this only fires for VIPs, not riders/Stuteley.
                if is_soldier_vip
                    && let Some(Some(entity)) = self.entities.get_mut(actor_id.index() as usize)
                    && let Some(npc) = entity.npc_data_mut()
                    && let Some(base) = npc.ai_brain.base_mut()
                {
                    base.say(crate::ai::Remark::VipNetNo);
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
            if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize)
                && let Some(human) = entity.human_data_mut()
            {
                crate::combat::increment_stuck_under_net(human);
            }

            self.quit_swordfight(assets, victim_id);

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
            if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize) {
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
    pub(crate) fn unapply_net_effect(&mut self, net_id: EntityId) {
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
            self.ai_global
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
            if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize) {
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
        let npc_ids = self.npc_ids.clone();
        for friend_id in npc_ids {
            if friend_id == body_id {
                continue;
            }
            if let Some(Some(Entity::Soldier(s))) =
                self.entities.get_mut(friend_id.index() as usize)
                && det_idx < s.npc.detectable_lists.len()
            {
                s.npc.detectable_lists[det_idx].retain(|d| d.element != Some(body_id));
            } else if let Some(Some(Entity::Civilian(c))) =
                self.entities.get_mut(friend_id.index() as usize)
                && det_idx < c.npc.detectable_lists.len()
            {
                c.npc.detectable_lists[det_idx].retain(|d| d.element != Some(body_id));
            }
        }
    }

    // ════════════════════════════════════════════════════════════════
    //  Per-frame net driver
    // ════════════════════════════════════════════════════════════════

    /// Advance every active net by one frame.
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
    ///   transition (`NetUnfolding` → `ObjectLying`/`NetMoving` per
    ///   `any_victim_is_moving`, `NetUnfoldingCrumpled` →
    ///   `NetLyingCrumpled`); toggle between `NetMoving` and
    ///   `ObjectLying` based on victim wriggle each frame. Release
    ///   happens via `Command::Take` pickup — the `TakingNet`
    ///   animation-Done handler in `engine/animation.rs` queues a
    ///   net-antagonist pickup, and the pickup branch in
    ///   `engine/tick.rs` calls [`EngineInner::unapply_net_effect`] +
    ///   despawns the net.
    pub(crate) fn tick_nets(&mut self, assets: &LevelAssets) {
        if self.freeze_all {
            return;
        }

        // Phase 1: advance trajectory + classify each net into
        // (descending-near-landing, just-landed) and stamp the
        // in-flight animation transitions on it directly.
        let mut applies: Vec<EntityId> = Vec::new();
        let mut just_landed: Vec<EntityId> = Vec::new();

        for (net_id, entity) in crate::engine::occupied_entity_slots_mut(&mut self.entities) {
            let net = match entity {
                Entity::Net(n) if n.element.active => n,
                _ => continue,
            };

            if net.projectile.flying {
                advance_net_trajectory(net);

                // `time_till_unfolding` countdown — when it hits 0,
                // switch animation to NetUnfolding (or _Crumpled if the
                // spawn-time crumple test flagged it). Subsequent
                // frames leave the animation alone; the sprite plays
                // out until the landing transition.
                if net.net.time_till_unfolding > 0 {
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
                    applies.push(net_id);
                }

                if !net.projectile.flying && net.net.was_flying {
                    // Just landed this frame — queue the landing-time
                    // work for phase 2 (which holds `&mut self` so it
                    // can register repulsive points + look up obstacles).
                    applies.push(net_id);
                    just_landed.push(net_id);
                    net.net.was_flying = false;
                }
            } else {
                // ── Ground state ───────────────────────────────────
                // Resolve the one-shot animation transition first.
                if !net.net.landed_animation_resolved {
                    let next = match net.object.animation {
                        crate::element::Animation::NetUnfolding => {
                            if net.net.victims.is_empty() {
                                crate::element::Animation::ObjectLying
                            } else {
                                crate::element::Animation::NetMoving
                            }
                        }
                        crate::element::Animation::NetUnfoldingCrumpled => {
                            crate::element::Animation::NetLyingCrumpled
                        }
                        _ => net.object.animation,
                    };
                    if next != net.object.animation {
                        net.object.animation = next;
                        net.net.landed_animation_resolved = true;
                    }
                }
            }
        }

        // Phase 1b — `NetMoving` ↔ `ObjectLying` toggle. While the
        // net's animation is `ObjectLying` or `NetMoving`, swap based
        // on whether any victim is currently in `WriggleUnderNet`.
        // Done in a separate read pass so we can borrow victim
        // entities.
        let mut wriggle_updates: Vec<(EntityId, crate::element::Animation)> = Vec::new();
        for (id, entity) in self.entities_iter_with_id() {
            let net = match entity {
                Entity::Net(n) if n.element.active && !n.projectile.flying => n,
                _ => continue,
            };
            if !matches!(
                net.object.animation,
                crate::element::Animation::NetMoving | crate::element::Animation::ObjectLying
            ) {
                continue;
            }
            let any_moving = self.any_victim_is_moving(&net.net.victims);
            let desired = if any_moving {
                crate::element::Animation::NetMoving
            } else {
                crate::element::Animation::ObjectLying
            };
            if desired != net.object.animation {
                wriggle_updates.push((id, desired));
            }
        }
        for (id, anim) in wriggle_updates {
            if let Some(Entity::Net(n)) = self.get_entity_mut(id) {
                n.object.animation = anim;
            }
        }

        // Phase 2: apply effects (mutable engine borrow released above).
        for net_id in applies {
            self.apply_net_falling_effect(assets, net_id);
        }
        for net_id in just_landed {
            self.apply_projectile_landing_resolution(assets, net_id);
            self.snap_net_to_landing_obstacle(assets, net_id);
            self.register_net_repulsive_points(net_id);
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
    fn snap_net_to_landing_obstacle(&mut self, assets: &LevelAssets, net_id: EntityId) {
        let (landing_xy, layer) = match self.get_entity(net_id) {
            Some(Entity::Net(n)) => (
                (n.element.position().x, n.element.position().y),
                n.element.layer(),
            ),
            _ => return,
        };

        // Branch on `(layer, find_landing_obstacle)`:
        //   - no obstacle           → 0.001 (we should already have
        //                             arrived with elevation ≈ 0)
        //   - obstacle, layer valid → snap to top plane + 0.001
        //   - obstacle, layer = INVALID_LAYER → keep current elevation,
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
            (INVALID_LAYER, Some(_)) => None, // keep current elevation
            (INVALID_LAYER, None) => Some(0.001),
            (_, Some(idx)) => Some(
                assets
                    .static_sight_obstacles
                    .get(idx)
                    .or_else(|| {
                        self.dynamic_sight_obstacles
                            .get(idx - assets.static_sight_obstacles.len())
                    })
                    .map(|o| o.compute_top_z(landing_xy.0, landing_xy.1) + 0.001)
                    .unwrap_or(0.001),
            ),
            (_, None) => Some(0.001),
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
        let origin = crate::geo2d::pt(landing_xy.0, landing_xy.1);
        let layer_u16 = if layer == INVALID_LAYER { 0 } else { layer };
        self.broadcast_noise(
            crate::ai::NoiseType::Bonk,
            origin,
            layer_u16,
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
    /// anti-collision loop that queries these is not yet ported, so
    /// no landed-purse/ale repulsion is wired up — once that loop is
    /// ported, it should follow this same persistent-registration
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
            let id = self.ai_global.next_repulsive_point_id;
            self.ai_global.next_repulsive_point_id += 1;
            self.ai_global
                .repulsive_points
                .push(crate::ai::RepulsivePoint {
                    id,
                    position: crate::ai::Position {
                        x: pos.x,
                        y: pos.y,
                        ..Default::default()
                    },
                    radius,
                    action_radius,
                    flags: 0,
                });
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
            if let Some((_, _, order)) = self.sequence_manager.current_order_for_actor(v)
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
    /// 1. **Layer marker**: layer == [`INVALID_LAYER`] means the net
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
            Some(Entity::Net(n)) => (n.element.layer(), n.projectile.end),
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
        layer: u16,
    ) -> bool {
        // No valid landing layer at all → crumple.
        if layer == INVALID_LAYER {
            return true;
        }

        let obstacle_idx = self.find_landing_obstacle(assets, landing);

        // Slope check.
        if let Some(idx) = obstacle_idx {
            let nz = assets
                .static_sight_obstacles
                .get(idx)
                .or_else(|| {
                    self.dynamic_sight_obstacles
                        .get(idx - assets.static_sight_obstacles.len())
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
                    .static_sight_obstacles
                    .get(idx)
                    .or_else(|| {
                        self.dynamic_sight_obstacles
                            .get(idx - assets.static_sight_obstacles.len())
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
            if !self.is_reachable_solid(assets, p_test, p_centre_high, layer) {
                return true;
            }

            let p_drop = crate::coordinates::WorldPoint3D {
                x: test_x,
                y: test_proj_y,
                z: test_proj_z - 40.0,
            };
            if self.is_reachable_solid(assets, p_test, p_drop, layer) {
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
        for (i, o) in assets.static_sight_obstacles.iter().enumerate() {
            if obstacle_bbox_contains(o, landing.x, landing.y) {
                return Some(i);
            }
        }
        let base = assets.static_sight_obstacles.len();
        for (i, o) in self.dynamic_sight_obstacles.iter().enumerate() {
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
        self.fast_grid.is_reachable_3d(
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
    o.box_ground.contains_point(crate::geo2d::pt(x, y))
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
mod tests {
    use super::*;
    use crate::coordinates::WorldPoint3D;
    use crate::element::{
        ActorData, ActorPc, ActorSoldier, ElementData, ElementKind, ElementNet, HumanData, NetData,
        NpcData, ObjectData, PcData, Posture, ProjectileData, SoldierData,
    };
    use crate::profiles::{Action, CharacterProfile, ProfileManager, SoldierProfile};

    /// Square root of [`SQUARE_RADIUS_NET_CAPTURE`] is 40, so any human
    /// within 40 isometric units of the landing point qualifies (with Y
    /// stretched).  Tests place victims well inside the radius (Δ = 5)
    /// to avoid borderline arithmetic.
    const LAND_X: f32 = 100.0;
    const LAND_Y: f32 = 100.0;
    const LAND_Z: f32 = 0.0;

    fn make_engine() -> EngineInner {
        EngineInner::new()
    }

    fn make_net(landing: WorldPoint3D) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ObjectNet,
            active: true,
            ..ElementData::default()
        };
        element.set_position(landing);
        element.set_position_map(MapPoint::from_world_xyz(landing.x, landing.y, landing.z));
        Entity::Net(ElementNet {
            element,
            object: ObjectData::default(),
            projectile: ProjectileData {
                end: landing,
                flying: false,
                ..ProjectileData::default()
            },
            net: NetData::default(),
        })
    }

    fn make_soldier(pos: WorldPoint3D, profile_idx: u32, rider: bool) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorSoldier,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(pos);
        element.set_position_map(MapPoint::from_world_xyz(pos.x, pos.y, pos.z));
        Entity::Soldier(ActorSoldier {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            npc: NpcData {
                life_points: 50,
                ..NpcData::default()
            },
            soldier: SoldierData {
                soldier_profile_index: crate::profiles::SoldierProfileIdx(profile_idx),
                cached_camp: crate::element::Camp::Lacklandists,
                rider,
                ..SoldierData::default()
            },
        })
    }

    fn make_pc(pos: WorldPoint3D, profile_idx: u32) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ActorPc,
            active: true,
            posture: Posture::Upright,
            ..ElementData::default()
        };
        element.set_position(pos);
        element.set_position_map(MapPoint::from_world_xyz(pos.x, pos.y, pos.z));
        Entity::Pc(ActorPc {
            element,
            actor: ActorData::default(),
            human: HumanData::default(),
            pc: PcData {
                profile_index: crate::profiles::CharacterProfileIdx(profile_idx),
                life_points: 50,
                ..PcData::default()
            },
        })
    }

    /// Build a [`LevelAssets`] with three character/soldier profiles set
    /// up so:
    ///   * soldier 0 — plain Royalist (vip = false)
    ///   * soldier 1 — VIP Royalist
    ///   * character 0 — plain PC (no Net action)
    ///   * character 1 — Stuteley (Net action present)
    fn assets_with_profiles() -> LevelAssets {
        let mut pm = ProfileManager::new();
        pm.soldiers.push(SoldierProfile::default());
        pm.soldiers.push(SoldierProfile {
            vip: true,
            ..SoldierProfile::default()
        });
        pm.characters.push(CharacterProfile::default());
        pm.characters.push(CharacterProfile {
            actions: [Action::Net, Action::NoAction, Action::NoAction],
            ..CharacterProfile::default()
        });
        let mut assets = LevelAssets::new();
        assets.profile_manager = std::sync::Arc::new(pm);
        assets
    }

    fn count_receive_net_for(engine: &EngineInner, victim_id: EntityId) -> usize {
        engine
            .sequence_manager
            .sequences_iter()
            .flat_map(|s| s.elements.iter())
            .filter(|e| e.owner == Some(victim_id) && e.command == Command::ReceiveNet)
            .count()
    }

    #[test]
    fn net_captures_three_normal_soldiers() {
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        let soldiers: Vec<EntityId> = (0..3)
            .map(|i| {
                engine.add_entity(make_soldier(
                    WorldPoint3D {
                        x: LAND_X + i as f32 * 5.0,
                        y: LAND_Y,
                        z: 0.0,
                    },
                    0, // plain non-VIP profile
                    false,
                ))
            })
            .collect();

        engine.apply_net_falling_effect(&assets, net_id);

        let net = match engine.get_entity(net_id).unwrap() {
            Entity::Net(n) => n,
            _ => panic!("not a net"),
        };
        assert!(
            !net.net.crumpled,
            "net should not crumple on plain soldiers"
        );
        assert_eq!(
            net.net.victims.len(),
            3,
            "all three soldiers in range should be captured"
        );
        for s in &soldiers {
            assert!(net.net.victims.contains(s));
            assert_eq!(count_receive_net_for(&engine, *s), 1);
            // Counter is bumped synchronously even though the posture
            // snap waits for the ReceiveNet damage element next frame.
            assert_eq!(
                engine
                    .get_entity(*s)
                    .unwrap()
                    .human_data()
                    .unwrap()
                    .stuck_under_nets_counter,
                1,
                "counter should be incremented eagerly on capture"
            );
            assert_eq!(
                engine.get_entity(*s).unwrap().element_data().posture,
                Posture::Upright,
                "posture stays Upright until the ReceiveNet handler runs"
            );
        }
    }

    #[test]
    fn second_apply_pass_does_not_double_capture() {
        // The capture sweep runs every frame the net is descending
        // within the apply threshold of landing — the dedup guard
        // ensures each victim is only captured once.
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        let victim_id = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: LAND_X,
                y: LAND_Y,
                z: 0.0,
            },
            0,
            false,
        ));

        engine.apply_net_falling_effect(&assets, net_id);
        engine.apply_net_falling_effect(&assets, net_id);
        engine.apply_net_falling_effect(&assets, net_id);

        let net = match engine.get_entity(net_id).unwrap() {
            Entity::Net(n) => n,
            _ => panic!("not a net"),
        };
        assert_eq!(net.net.victims, vec![victim_id]);
        assert_eq!(count_receive_net_for(&engine, victim_id), 1);
        assert_eq!(
            engine
                .get_entity(victim_id)
                .unwrap()
                .human_data()
                .unwrap()
                .stuck_under_nets_counter,
            1,
            "dedup guard prevents counter double-bump on repeat ApplyEffect"
        );
    }

    #[test]
    fn net_crumples_when_only_rider_in_range() {
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        let rider_id = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: LAND_X + 5.0,
                y: LAND_Y,
                z: 0.0,
            },
            0,
            true, // rider
        ));

        engine.apply_net_falling_effect(&assets, net_id);

        let net = match engine.get_entity(net_id).unwrap() {
            Entity::Net(n) => n,
            _ => panic!("not a net"),
        };
        assert!(
            net.net.crumpled,
            "net should crumple when only a rider is in range"
        );
        assert!(net.net.victims.is_empty(), "no victims when crumpled");
        assert_eq!(count_receive_net_for(&engine, rider_id), 0);
    }

    #[test]
    fn net_crumples_on_vip_soldier_alone() {
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        // Profile 1 = VIP soldier
        let vip_id = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: LAND_X,
                y: LAND_Y,
                z: 0.0,
            },
            1,
            false,
        ));

        engine.apply_net_falling_effect(&assets, net_id);

        let net = match engine.get_entity(net_id).unwrap() {
            Entity::Net(n) => n,
            _ => panic!("not a net"),
        };
        assert!(net.net.crumpled);
        assert!(net.net.victims.is_empty());
        assert_eq!(count_receive_net_for(&engine, vip_id), 0);
    }

    #[test]
    fn net_with_existing_victim_ignores_new_rider() {
        // Pre-existing victim simulates a previous capture-sweep
        // call's captures; a Rider seen on a subsequent sweep triggers
        // the "new arrivants won't be caught" branch — the net does
        // NOT crumple, and the existing victim list is kept.
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        let existing_id = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: LAND_X,
                y: LAND_Y,
                z: 0.0,
            },
            0,
            false,
        ));
        let rider_id = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: LAND_X + 5.0,
                y: LAND_Y,
                z: 0.0,
            },
            0,
            true,
        ));
        // Seed the net with an already-captured victim so the
        // crumple guard sees a non-empty list.
        if let Some(Some(Entity::Net(n))) = engine.entities.get_mut(net_id.index() as usize) {
            n.net.victims.push(existing_id);
        }

        engine.apply_net_falling_effect(&assets, net_id);

        let net = match engine.get_entity(net_id).unwrap() {
            Entity::Net(n) => n,
            _ => panic!("not a net"),
        };
        assert!(
            !net.net.crumpled,
            "rider with existing victim must not crumple"
        );
        // No new captures were added (rider triggered the early return
        // before the existing victim could be re-processed; existing
        // entry is dedup'd against itself).
        assert_eq!(net.net.victims, vec![existing_id]);
        assert_eq!(count_receive_net_for(&engine, rider_id), 0);
    }

    #[test]
    fn net_skips_humans_outside_radius() {
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        let near_id = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: LAND_X + 5.0,
                y: LAND_Y,
                z: 0.0,
            },
            0,
            false,
        ));
        // 200 units away in X — way outside SQUARE_RADIUS_NET_CAPTURE.
        let far_id = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: LAND_X + 200.0,
                y: LAND_Y,
                z: 0.0,
            },
            0,
            false,
        ));

        engine.apply_net_falling_effect(&assets, net_id);

        let net = match engine.get_entity(net_id).unwrap() {
            Entity::Net(n) => n,
            _ => panic!("not a net"),
        };
        assert_eq!(net.net.victims, vec![near_id]);
        assert_eq!(count_receive_net_for(&engine, near_id), 1);
        assert_eq!(count_receive_net_for(&engine, far_id), 0);
    }

    #[test]
    fn net_crumples_on_stuteley_pc() {
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        // Character profile 1 = Stuteley (Action::Net present).
        let _ = engine.add_entity(make_pc(
            WorldPoint3D {
                x: LAND_X,
                y: LAND_Y,
                z: 0.0,
            },
            1,
        ));

        engine.apply_net_falling_effect(&assets, net_id);

        let net = match engine.get_entity(net_id).unwrap() {
            Entity::Net(n) => n,
            _ => panic!("not a net"),
        };
        assert!(net.net.crumpled);
        assert!(net.net.victims.is_empty());
    }

    #[test]
    fn unapply_clears_victims_and_releases_counters() {
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        let victim_id = engine.add_entity(make_soldier(
            WorldPoint3D {
                x: LAND_X,
                y: LAND_Y,
                z: 0.0,
            },
            0,
            false,
        ));

        // First, fire the apply sweep so the victim is registered.
        // The sweep eagerly increments stuck_under_nets_counter; the
        // posture snap to StuckUnderNet would normally happen the
        // next frame inside `EngineInner::apply_net` (the ReceiveNet
        // damage handler). We don't run the per-tick dispatcher in
        // this unit test, so set the posture by hand to simulate the
        // post-dispatch state.
        engine.apply_net_falling_effect(&assets, net_id);
        if let Some(Some(entity)) = engine.entities.get_mut(victim_id.index() as usize) {
            entity.set_posture_stuck_under_net_for_human();
        }
        assert_eq!(
            engine
                .get_entity(victim_id)
                .unwrap()
                .human_data()
                .unwrap()
                .stuck_under_nets_counter,
            1,
            "apply_net_falling_effect should eagerly increment counter to 1"
        );
        assert_eq!(
            engine.get_entity(victim_id).unwrap().element_data().posture,
            Posture::StuckUnderNet
        );

        engine.unapply_net_effect(net_id);

        let net = match engine.get_entity(net_id).unwrap() {
            Entity::Net(n) => n,
            _ => panic!("not a net"),
        };
        assert!(net.net.victims.is_empty(), "victims drained");
        let v = engine.get_entity(victim_id).unwrap();
        assert_eq!(v.human_data().unwrap().stuck_under_nets_counter, 0);
        assert_eq!(v.element_data().posture, Posture::Lying);
    }

    #[test]
    fn vip_soldier_says_vip_net_no_remark() {
        // The VipNetNo remark fires for VIP soldiers in the crumple
        // radius. We populate an EnemyAi brain so the say() call has
        // somewhere to land.
        use crate::ai::AiController;
        use crate::ai_enemy::EnemyAi;
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        let mut vip = make_soldier(
            WorldPoint3D {
                x: LAND_X,
                y: LAND_Y,
                z: 0.0,
            },
            1, // VIP profile
            false,
        );
        if let Entity::Soldier(ref mut s) = vip {
            s.npc.ai_brain = crate::element::AiBrain::Enemy(Box::new(EnemyAi {
                base: AiController::default(),
                ..Default::default()
            }));
        }
        let vip_id = engine.add_entity(vip);

        engine.apply_net_falling_effect(&assets, net_id);

        let entity = engine.get_entity(vip_id).unwrap();
        let remark = entity
            .npc_data()
            .and_then(|n| n.ai_brain.base())
            .map(|b| b.current_remark)
            .unwrap();
        assert_eq!(remark, crate::ai::Remark::VipNetNo);
    }

    #[test]
    fn capture_sets_victim_display_order_behind_net() {
        // Capture should mark the victim's sprite as
        // `display_order_ref = Some(net_id)` + `behind = true`.
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        let victim = make_soldier(
            WorldPoint3D {
                x: LAND_X,
                y: LAND_Y,
                z: 0.0,
            },
            0,
            false,
        );
        // The victim already has a default Sprite (non-Option).
        let victim_id = engine.add_entity(victim);

        engine.apply_net_falling_effect(&assets, net_id);

        let sprite = engine.get_entity(victim_id).unwrap().sprite();
        assert_eq!(sprite.display_order_ref, Some(net_id));
        assert!(sprite.behind_display_order_ref);

        engine.unapply_net_effect(net_id);
        let sprite = engine.get_entity(victim_id).unwrap().sprite();
        assert_eq!(
            sprite.display_order_ref, None,
            "unapply should clear the behind-net reference"
        );
        assert!(!sprite.behind_display_order_ref);
    }

    #[test]
    fn landing_registers_repulsive_points() {
        let mut engine = make_engine();
        let _assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));

        // Manually fire the landing-time helper (no flight ticking).
        engine.register_net_repulsive_points(net_id);

        let net = match engine.get_entity(net_id).unwrap() {
            Entity::Net(n) => n,
            _ => panic!("not a net"),
        };
        assert_eq!(net.net.repulsive_point_ids.len(), 2);
        // Two RepulsivePoints should be registered on AiGlobalState.
        let registered_ids: Vec<i32> = engine
            .ai_global
            .repulsive_points
            .iter()
            .map(|p| p.id)
            .collect();
        for id in &net.net.repulsive_point_ids {
            assert!(registered_ids.contains(id));
        }

        engine.unapply_net_effect(net_id);
        // After unapply: zero repulsive points left.
        assert!(engine.ai_global.repulsive_points.is_empty());
    }

    #[test]
    fn taking_net_animation_dispatched_for_pc() {
        // PC taking a net should pick `OrderType::TakingNet` in the
        // dispatcher. Soldiers picking purses still get `Taking`.
        // We verify by populating PC + net + manually launching a
        // Take element, then asserting the active_ai_anim type.
        use crate::sequence::SequenceElement;
        let mut engine = make_engine();
        let assets = assets_with_profiles();
        let landing = WorldPoint3D {
            x: LAND_X,
            y: LAND_Y,
            z: LAND_Z,
        };
        let net_id = engine.add_entity(make_net(landing));
        let pc_id = engine.add_entity(make_pc(
            WorldPoint3D {
                x: LAND_X,
                y: LAND_Y,
                z: 0.0,
            },
            1, // Stuteley (has Action::Net)
        ));

        // Fire the landing path so the net is actually on the ground.
        engine.snap_net_to_landing_obstacle(&assets, net_id);

        // Launch Take(antagonist=net) targeting the PC.
        let elem = SequenceElement::new_interaction(1, Command::Take, Some(pc_id), Some(net_id));
        engine.launch_element(elem);
        // Process the pending element so the dispatcher runs.
        let mut dev = crate::engine::DevState::default();
        let mut display = crate::engine::HostDisplayState::default();
        engine.perform_hourglass(&mut display, &assets, &mut dev);

        let active_anim = engine
            .sequence_manager
            .current_order_for_actor(pc_id)
            .map(|(_, _, o)| o.order_type);
        assert_eq!(
            active_anim,
            Some(crate::order::OrderType::TakingNet),
            "PC picking up a net should play TakingNet, not generic Taking"
        );
    }
}
