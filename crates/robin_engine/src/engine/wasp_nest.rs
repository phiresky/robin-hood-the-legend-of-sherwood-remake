//! Wasp nest / wasp lifecycle on the engine side.
//!
//! Drives the per-frame nest and wasp behaviour:
//!
//! 1. **Nest trajectory advancement** until impact, then burst —
//!    spawns `NUMBER_OF_WASPS` wasp swarmers around the impact point,
//!    switches the nest to `ObjectBursting`, and seeds the nest's
//!    `flying_wasp_count`.
//!
//! 2. **Nest Hourglass**: while `flying_wasp_count > 0`, emit the 507
//!    buzz FX at the nest position each tick.
//!
//! 3. **Wasp AI**: each spawned wasp runs the full chase-and-sting
//!    behaviour — victim selection (enemy soldiers, smelling-apple
//!    soldiers prioritised, VIPs excluded), tethered random-walk
//!    flight with nest attraction, charge-to-victim, sting-timeout
//!    state machine.  On sting the wasp launches the
//!    `RECEIVE_WASP_STING` sequence on its victim and then dies,
//!    decrementing the nest's `flying_wasp_count`.

use super::{EngineInner, LevelAssets};
use crate::bow_shot::{self, NUMBER_OF_WASPS};
use crate::coordinates::{MapPoint, WorldPoint3D, WorldVec3D};
use crate::element::{Animation, Camp, Entity, EntityId, ObjectType};

/// Buzz FX id played at the nest position each frame while wasps are
/// in the air.
const FX_WASP_BUZZ: u32 = 507;

// ── Wasp-AI tuning constants ─────────────────────────────────────

/// Apple-smelling soldiers are detected / charged / forgotten at 3×
/// the baseline distance.
const APPLE_ATTRACTION: u32 = 3;
/// Above this range the wasp returns to its nest.
const MAX_NEST_DISTANCE: f32 = 50.0;
/// Keep wasps at least this high above the nest's elevation.
const MIN_GROUND_DISTANCE: f32 = 20.0;
/// Base speed, per frame, of the wasp's movement vector.
const WASP_SPEED: f32 = 5.0;
/// Weight of the nest-direction return pull.
const NEST_ATTRACTION: f32 = 0.08;
/// Base frames between direction changes; jittered +0..3.
const DIRECTION_CHANGE_TIMEOUT: u16 = 7;
// Kept-justified: paired with STINGING_MAX_TIMEOUT below for parity
// documentation; unused at runtime due to the parens bug.
#[allow(dead_code)]
const STINGING_MIN_TIMEOUT: u16 = 10;
/// Sting-delay ceiling in frames.
const STINGING_MAX_TIMEOUT: u16 = 60;
/// Starting range for victim search.  Multiplied by `APPLE_ATTRACTION`
/// for apple-smelling soldiers.
const VICTIM_DETECTION_DISTANCE: f32 = crate::gameplay_config::CLASSIC_WASP_ACQUISITION_RADIUS;
/// Above this range the wasp drops its current victim.
const VICTIM_FORGET_DISTANCE: f32 = 70.0;
/// Inside this range the wasp goes straight-line at the victim
/// instead of random-walking with attraction.
const VICTIM_CHARGE_DISTANCE: f32 = 25.0;
/// Weight of the victim-direction pull while outside the charge range.
const VICTIM_ATTRACTION: f32 = 0.08;
/// Range at which the wasp stops and commits to stinging.
const STING_DISTANCE: f32 = 10.0;
/// Number of `ChangeDirection` retries before the wasp gives up and
/// kills itself.
const CHANGE_DIRECTION_TRIES: u32 = 10;

fn victim_detection_distance(
    config: crate::gameplay_config::ItemGameplayConfig,
    smelling_apple: bool,
) -> u32 {
    let baseline = if config.wasp_reliable_acquisition {
        crate::gameplay_config::REBALANCED_WASP_ACQUISITION_RADIUS
    } else {
        VICTIM_DETECTION_DISTANCE
    };
    let attraction = if smelling_apple { APPLE_ATTRACTION } else { 1 };
    (baseline * attraction as f32) as u32
}

impl EngineInner {
    /// Per-frame tick for wasp nests and their spawned wasps.
    #[cfg(test)]
    pub(super) fn tick_wasp_nests(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
    ) {
        let mut slot = 0;
        while slot < self.world.entities.len() {
            if let Some(id) = self.world.entities.id_at_legacy_slot(slot as u32) {
                self.tick_wasp_nest_or_wasp(sim, assets, id);
            }
            slot += 1;
        }
    }

    /// Advance one wasp nest or wasp at its creation-order position.
    pub(super) fn tick_wasp_nest_or_wasp(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        id: EntityId,
    ) {
        let object_type = match self.get_entity(id) {
            Some(Entity::Projectile(projectile)) if projectile.element.active => {
                projectile.object.object_type
            }
            _ => return,
        };
        if object_type == ObjectType::Wasp {
            self.tick_single_wasp(sim, assets, id);
            return;
        }
        if !matches!(
            object_type,
            ObjectType::WaspNest | ObjectType::BonusWaspNest
        ) {
            return;
        }

        // The nest's projectile base runs first. HitObstacle may append the
        // wasps; the live outer entity loop reaches those new slots later.
        let impact = match self.get_entity_mut(id) {
            Some(Entity::Projectile(projectile)) if projectile.projectile.flying => projectile
                .advance_trajectory_one_frame()
                .then(|| (projectile.element.position(), projectile.element.layer())),
            _ => None,
        };
        if let Some((pos, layer)) = impact {
            let resolution = self.apply_projectile_landing_resolution(assets, id);
            let landed_layer = resolution
                .filter(|r| !r.blocked_by_motion_obstacle)
                .map(|r| r.layer)
                .unwrap_or(layer);
            self.burst_wasp_nest(id, pos, landed_layer);
        }

        // Nest-specific tail follows its projectile base Hourglass.
        let buzz_pos = match self.get_entity(id) {
            Some(Entity::Projectile(p))
                if p.element.active
                    && p.projectile.wasp.burst
                    && p.projectile.wasp.flying_wasp_count > 0 =>
            {
                Some(p.element.position())
            }
            _ => None,
        };
        if let Some(pos) = buzz_pos {
            self.feedback
                .pending_side_effects
                .sounds
                .push(super::SoundCommand::Fx {
                    fx_id: FX_WASP_BUZZ,
                    position: MapPoint::new(pos.x, pos.y),
                    material: None,
                });
        }
    }

    /// Burst a landed wasp nest into [`NUMBER_OF_WASPS`] wasp swarmers.
    fn burst_wasp_nest(&mut self, nest_id: EntityId, impact_pos: WorldPoint3D, layer: u16) {
        let already_burst = self
            .get_entity(nest_id)
            .map(|e| matches!(e, Entity::Projectile(p) if p.projectile.wasp.burst))
            .unwrap_or(false);
        if already_burst {
            return;
        }

        if let Some(Entity::Projectile(nest)) = self.world.entities.get_mut(nest_id) {
            nest.object.animation = Animation::ObjectBursting;
            nest.projectile.flying = false;
            nest.projectile.wasp.burst = true;
            nest.projectile.wasp.flying_wasp_count = u32::from(NUMBER_OF_WASPS);
        }

        for _ in 0..NUMBER_OF_WASPS {
            let wasp = bow_shot::spawn_wasp(nest_id, impact_pos, layer);
            self.add_entity(wasp);
        }

        tracing::debug!(
            ?nest_id,
            x = impact_pos.x,
            y = impact_pos.y,
            wasps = NUMBER_OF_WASPS,
            "WaspNest: burst on impact, spawned wasp swarm"
        );
    }

    /// Advance a single wasp by one frame.
    fn tick_single_wasp(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        wasp_id: EntityId,
    ) {
        // Load current state into locals.  Touch only fields that
        // exist on every `Projectile`, so later `get_mut` calls don't
        // need the object-type re-check.
        let (active, timeout, stinging, victim) = match self.get_entity(wasp_id) {
            Some(Entity::Projectile(p)) => (
                p.element.active,
                p.projectile.wasp.timeout,
                p.projectile.wasp.stinging,
                p.projectile.wasp.victim,
            ),
            _ => return,
        };
        if !active {
            return;
        }

        // ── Timeout == 0: either change direction / victim, or fire
        //    the sting.
        if timeout == 0 {
            if !stinging {
                self.wasp_change_victim(sim, assets, wasp_id);
                self.wasp_change_direction(sim, assets, wasp_id);
                // Reset timeout — `DIRECTION_CHANGE_TIMEOUT` plus a
                // 0..3 jitter.
                let jitter =
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::WaspDirectionTimer, 0..3);
                if let Some(Entity::Projectile(p)) = self.world.entities.get_mut(wasp_id) {
                    p.projectile.wasp.timeout = u32::from(DIRECTION_CHANGE_TIMEOUT) + jitter;
                }
            } else {
                // Stinging: if the victim is still a valid target,
                // launch `RECEIVE_WASP_STING`; either way, the wasp
                // dies on the same tick.
                if let Some(victim_id) = victim {
                    let victim_ok = self
                        .get_entity(victim_id)
                        .map(|v| {
                            !v.is_dead()
                                && !v.human_data().is_some_and(|h| h.unconscious)
                                && v.actor_data()
                                    .map(|a| !a.action_state.is_sword())
                                    .unwrap_or(true)
                        })
                        .unwrap_or(false);
                    if victim_ok {
                        // Use the engine wrapper so movement-element
                        // transitions are rewritten and any in-flight
                        // path request is cancelled (the bare
                        // `SequenceManager::stop_owner` skips both).
                        self.stop_owner(victim_id, crate::sequence::SequencePriority::Injury);
                        let sting = crate::sequence::SequenceElement::new(
                            1,
                            crate::element::Command::ReceiveWaspSting,
                            Some(victim_id),
                        );
                        self.launch_element(sting);
                    }
                }
                self.kill_wasp(wasp_id);
                return;
            }
        } else if let Some(Entity::Projectile(p)) = self.world.entities.get_mut(wasp_id) {
            p.projectile.wasp.timeout -= 1;
        }

        // ── Position update: non-stinging wasps apply their movement
        //    vector to their position each tick.  Stinging wasps
        //    hover at their sting point.
        let (stinging_now, movement) = match self.get_entity(wasp_id) {
            Some(Entity::Projectile(p)) => (p.projectile.wasp.stinging, p.projectile.wasp.movement),
            _ => return,
        };
        if !stinging_now && let Some(Entity::Projectile(p)) = self.world.entities.get_mut(wasp_id) {
            let mut pos = p.element.position();
            pos.x += movement.x;
            pos.y += movement.y;
            pos.z += movement.z;
            p.element.set_position(pos);
            p.element
                .set_position_map(crate::coordinates::MapPoint::from_world_xyz(
                    pos.x, pos.y, pos.z,
                ));
        }

        // ── Sting commit: when a non-stinging wasp with a victim
        //    gets within STING_DISTANCE of the victim's eyes, flip
        //    into stinging and roll a random sting delay.
        if !stinging_now && let Some(victim_id) = victim {
            // Guard: victim might have been removed between phases.
            let victim_eyes = self
                .get_entity(victim_id)
                .and_then(|v| v.compute_eyes_point(None));
            if let Some(eyes) = victim_eyes {
                let cur = match self.get_entity(wasp_id) {
                    Some(Entity::Projectile(p)) => p.element.position(),
                    _ => return,
                };
                let dx = cur.x - eyes.x;
                let dy = cur.y - eyes.y;
                let dz = cur.z - eyes.z;
                if (dx * dx + dy * dy + dz * dz).sqrt() <= STING_DISTANCE {
                    // The original sting-delay formula has a
                    // precedence bug:
                    //   `( rand() % STINGING_MAX_TIMEOUT - STINGING_MIN_TIMEOUT + 1 ) + STINGING_MIN_TIMEOUT`
                    // `%` binds tighter than `-`, so the parens are
                    // misplaced and the MIN floor cancels itself out:
                    //   `(rand()%MAX) - MIN + 1 + MIN` == `(rand()%MAX) + 1`
                    // i.e. the actual sting delay is 1..=STINGING_MAX_TIMEOUT,
                    // not the intended STINGING_MIN..=STINGING_MAX range.
                    // Preserved verbatim for parity.
                    let delay = crate::sim_rng::u32(
                        sim,
                        crate::sim_rng::RngSite::WaspStingTimer,
                        0..STINGING_MAX_TIMEOUT as u32,
                    ) + 1;
                    if let Some(Entity::Projectile(p)) = self.world.entities.get_mut(wasp_id) {
                        p.projectile.wasp.stinging = true;
                        p.projectile.wasp.timeout = delay;
                    }
                }
            }
        }
    }

    /// Pick / drop the wasp's victim.
    fn wasp_change_victim(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        wasp_id: EntityId,
    ) {
        let (cur_victim, wasp_pos) = match self.get_entity(wasp_id) {
            Some(Entity::Projectile(p)) => (p.projectile.wasp.victim, p.element.position()),
            _ => return,
        };

        if let Some(victim_id) = cur_victim {
            // Already have a victim — drop it if too far away.
            let (victim_pos, smelling_apple) = match self.get_entity(victim_id) {
                Some(v) => (
                    v.element_data().position(),
                    v.soldier_data().map(|s| s.apple_smell > 0).unwrap_or(false),
                ),
                None => (wasp_pos, false),
            };
            let forget = if smelling_apple {
                VICTIM_FORGET_DISTANCE * APPLE_ATTRACTION as f32
            } else {
                VICTIM_FORGET_DISTANCE
            };
            let dx = wasp_pos.x - victim_pos.x;
            let dy = wasp_pos.y - victim_pos.y;
            let dz = wasp_pos.z - victim_pos.z;
            let dist2 = dx * dx + dy * dy + dz * dz;
            if self.get_entity(victim_id).is_none() || dist2 >= forget * forget {
                self.clear_wasp_victim_flag(victim_id);
                if let Some(Entity::Projectile(p)) = self.world.entities.get_mut(wasp_id) {
                    p.projectile.wasp.victim = None;
                }
            }
            return;
        }

        // No victim — pick one.
        let new_victim = self.wasp_choose_victim(sim, assets, wasp_id);
        if let Some(vid) = new_victim {
            if let Some(Entity::Projectile(p)) = self.world.entities.get_mut(wasp_id) {
                p.projectile.wasp.victim = Some(vid);
            }
            if let Some(v) = self.world.entities.get_mut(vid)
                && let Some(npc) = v.npc_data_mut()
            {
                npc.wasp_victim = true;
            }
        }
    }

    /// Scan every active enemy soldier and pick the best victim.
    ///
    /// Victim priority:
    ///   1. Nearest apple-smelling soldier (3× detection range).
    ///   2. Nearest non-smelling soldier.
    ///
    /// VIPs are filtered out and trigger `VipWaspsNo`.
    fn wasp_choose_victim(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        wasp_id: EntityId,
    ) -> Option<EntityId> {
        let wasp_pos = match self.get_entity(wasp_id) {
            Some(Entity::Projectile(p)) => p.element.position(),
            _ => return None,
        };

        // The wasp↔soldier distance is stored as a u32 (truncated)
        // for both the detection-range gate and the candidate-set
        // ordering.  The integer truncation matters at the boundary
        // (e.g. 50.7 truncates to 50, passes a `<= 50` gate) and for
        // tie ordering.  The reverse iteration over `npc_ids` then
        // makes the highest NPC index win on integer-distance ties
        // (a set insert rejects equal keys, so the first-inserted
        // candidate sticks).
        let mut smelling: Vec<(u32, EntityId)> = Vec::new();
        let mut clean: Vec<(u32, EntityId)> = Vec::new();
        let mut vip_remarks: Vec<EntityId> = Vec::new();

        let soldier_ids: Vec<EntityId> = self
            .world
            .entities
            .soldier_ids()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        for soldier_id in soldier_ids {
            let entity = match self.get_entity(soldier_id) {
                Some(e) => e,
                None => continue,
            };
            if !entity.is_active() || !entity.camp().is_hostile_to(Camp::Royalists) {
                continue;
            }
            // Exclude already-swordfighting soldiers and existing
            // wasp victims.
            let in_swordfight = entity
                .actor_data()
                .map(|a| a.action_state.is_sword())
                .unwrap_or(false);
            if in_swordfight {
                continue;
            }
            let already_wasp_victim = entity.npc_data().map(|n| n.wasp_victim).unwrap_or(false);
            if already_wasp_victim {
                continue;
            }

            // Distance check against the soldier's eyes.
            let Some(eyes) = entity.compute_eyes_point(None) else {
                continue;
            };
            let dx = eyes.x - wasp_pos.x;
            let dy = eyes.y - wasp_pos.y;
            let dz = eyes.z - wasp_pos.z;
            // Truncate to a u32 to mirror the original integer-distance gate.
            let dist = (dx * dx + dy * dy + dz * dz).sqrt() as u32;
            let smelling_apple = entity
                .soldier_data()
                .map(|s| s.apple_smell > 0)
                .unwrap_or(false);
            let detect = victim_detection_distance(sim.config().item_gameplay, smelling_apple);
            if dist > detect {
                continue;
            }

            // VIP filter — VIPs get the VipWaspsNo remark instead of
            // becoming a victim.
            if super::melee::is_vip_from_profile(entity, &assets.profile_manager) {
                vip_remarks.push(soldier_id);
                continue;
            }

            if smelling_apple {
                smelling.push((dist, soldier_id));
            } else {
                clean.push((dist, soldier_id));
            }
        }

        // VIPs say `VipWaspsNo` instead of being targeted.
        for vid in vip_remarks {
            self.world
                .entities
                .get_mut(vid)
                .unwrap_or_else(|| panic!("wasp speech owner {} disappeared", vid.index()))
                .ai_controller_mut()
                .unwrap_or_else(|| panic!("wasp speech owner {} has no AI", vid.index()))
                .say(crate::ai::Remark::VipWaspsNo);
            self.drain_ai_owner_work_for(sim, assets, vid);
        }

        // Priority: smelling-apple first, nearest within that group.
        // Use stable sort on the integer key so the reverse-iteration
        // tie ordering above is preserved on equal distances (a set
        // rejects duplicates, leaving the first-inserted candidate).
        smelling.sort_by_key(|&(d, _)| d);
        if let Some(&(_, id)) = smelling.first() {
            return Some(id);
        }
        clean.sort_by_key(|&(d, _)| d);
        clean.first().map(|&(_, id)| id)
    }

    /// Re-roll the wasp's movement vector.
    ///
    /// Up to `CHANGE_DIRECTION_TRIES` random candidates; accept the
    /// first one whose short-horizon trajectory is clear of SOLID
    /// obstacles.  On exhaustion the wasp gives up and kills itself.
    fn wasp_change_direction(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        wasp_id: EntityId,
    ) {
        let (wasp_pos, nest_id) = match self.get_entity(wasp_id) {
            Some(Entity::Projectile(p)) => (p.element.position(), p.projectile.wasp.source_nest),
            _ => return,
        };
        let nest_pos =
            nest_id.and_then(|nid| self.get_entity(nid).map(|n| n.element_data().position()));

        let victim_info = match self.get_entity(wasp_id) {
            Some(Entity::Projectile(p)) => p.projectile.wasp.victim,
            _ => None,
        }
        .and_then(|vid| {
            self.get_entity(vid).map(|v| {
                let eyes = v
                    .compute_eyes_point(None)
                    .unwrap_or(v.element_data().position());
                let smelling = v.soldier_data().map(|s| s.apple_smell > 0).unwrap_or(false);
                (eyes, smelling)
            })
        });

        let mut tries = CHANGE_DIRECTION_TRIES;
        loop {
            // Random 3D movement vector with each component in -6..=4.
            let rx = (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::WaspMovement, 0..11) as i32
                - 6) as f32;
            let ry = (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::WaspMovement, 0..11) as i32
                - 6) as f32;
            let rz = (crate::sim_rng::u32(sim, crate::sim_rng::RngSite::WaspMovement, 0..11) as i32
                - 6) as f32;
            let mut mv = WorldVec3D {
                x: rx,
                y: ry,
                z: rz,
            };
            let norm = (mv.x * mv.x + mv.y * mv.y + mv.z * mv.z).sqrt();
            if norm == 0.0 {
                // Degenerate roll: zero movement and bail.
                if let Some(Entity::Projectile(p)) = self.world.entities.get_mut(wasp_id) {
                    p.projectile.wasp.movement = WorldVec3D {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    };
                }
                return;
            }
            let scale = WASP_SPEED / norm;
            mv.x *= scale;
            mv.y *= scale;
            mv.z *= scale;

            // Ground clearance.
            if let Some(np) = nest_pos
                && (wasp_pos.z - np.z).abs() < MIN_GROUND_DISTANCE
            {
                mv.z = mv.z.abs();
            }

            // Nest tether.
            if victim_info.is_none()
                && let Some(np) = nest_pos
            {
                let to_nest = WorldPoint3D {
                    x: np.x - wasp_pos.x,
                    y: np.y - wasp_pos.y,
                    z: np.z - wasp_pos.z,
                };
                let d =
                    (to_nest.x * to_nest.x + to_nest.y * to_nest.y + to_nest.z * to_nest.z).sqrt();
                if d > MAX_NEST_DISTANCE {
                    mv.x += to_nest.x * NEST_ATTRACTION;
                    mv.y += to_nest.y * NEST_ATTRACTION;
                    mv.z += to_nest.z * NEST_ATTRACTION;
                }
            }

            // Victim charge / attraction.
            if let Some((eyes, smelling)) = victim_info {
                let to_victim = WorldPoint3D {
                    x: eyes.x - wasp_pos.x,
                    y: eyes.y - wasp_pos.y,
                    z: eyes.z - wasp_pos.z,
                };
                let d = (to_victim.x * to_victim.x
                    + to_victim.y * to_victim.y
                    + to_victim.z * to_victim.z)
                    .sqrt();
                let charge = if smelling {
                    VICTIM_CHARGE_DISTANCE * APPLE_ATTRACTION as f32
                } else {
                    VICTIM_CHARGE_DISTANCE
                };
                if d <= charge {
                    mv = crate::coordinates::WorldVec3D {
                        x: to_victim.x,
                        y: to_victim.y,
                        z: to_victim.z,
                    };
                } else {
                    mv = WorldVec3D {
                        x: to_victim.x * VICTIM_ATTRACTION,
                        y: to_victim.y * VICTIM_ATTRACTION,
                        z: to_victim.z * VICTIM_ATTRACTION,
                    };
                }
            }

            // Renormalise to WASP_SPEED.
            let n2 = (mv.x * mv.x + mv.y * mv.y + mv.z * mv.z).sqrt();
            if n2 > 0.0 {
                let s = WASP_SPEED / n2;
                mv.x *= s;
                mv.y *= s;
                mv.z *= s;
            }

            // Short-horizon reachability probe.
            let estimated = WorldPoint3D {
                x: wasp_pos.x + mv.x * DIRECTION_CHANGE_TIMEOUT as f32,
                y: wasp_pos.y + mv.y * DIRECTION_CHANGE_TIMEOUT as f32,
                z: wasp_pos.z + mv.z * DIRECTION_CHANGE_TIMEOUT as f32,
            };
            let origin = [wasp_pos.x, wasp_pos.y, wasp_pos.z];
            let dest = [estimated.x, estimated.y, estimated.z];
            let clear = {
                let obstacles = self.sight_obstacles(assets);
                crate::sight_obstacle::is_reachable_3d(
                    obstacles,
                    origin,
                    dest,
                    crate::sight_obstacle::SIGHTOBSTACLE_SOLID,
                )
            };
            if clear {
                if let Some(Entity::Projectile(p)) = self.world.entities.get_mut(wasp_id) {
                    p.projectile.wasp.movement = mv;
                }
                return;
            }

            tries -= 1;
            if tries == 0 {
                // Exhausted — kill the wasp and release its victim.
                self.kill_wasp(wasp_id);
                return;
            }
        }
    }

    /// Remove a wasp entity, decrement the parent nest's
    /// `flying_wasp_count`, and clear any victim's `wasp_victim` flag
    /// (the released-victim cleanup folded in from the
    /// `wasp_change_direction` failure path).
    fn kill_wasp(&mut self, wasp_id: EntityId) {
        let (nest_id, victim_id) = match self.get_entity(wasp_id) {
            Some(Entity::Projectile(p)) => {
                (p.projectile.wasp.source_nest, p.projectile.wasp.victim)
            }
            _ => return,
        };

        if let Some(victim) = victim_id {
            self.clear_wasp_victim_flag(victim);
        }
        self.remove_entity(wasp_id);
        if let Some(nest) = nest_id {
            self.wasp_killed(nest);
        }
    }

    /// Decrement a nest's `flying_wasp_count`.
    fn wasp_killed(&mut self, nest_id: EntityId) {
        if let Some(Entity::Projectile(nest)) = self.world.entities.get_mut(nest_id)
            && nest.projectile.wasp.flying_wasp_count > 0
        {
            nest.projectile.wasp.flying_wasp_count -= 1;
        }
    }

    /// Clear a soldier's `wasp_victim` flag.
    fn clear_wasp_victim_flag(&mut self, victim_id: EntityId) {
        if let Some(entity) = self.world.entities.get_mut(victim_id)
            && let Some(npc) = entity.npc_data_mut()
        {
            npc.wasp_victim = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::MapPoint;
    use crate::element::{
        ActorData, ActorSoldier, ElementData, ElementKind, ElementProjectile, HumanData, NpcData,
        ObjectData, Posture, ProjectileData, SoldierData,
    };
    use crate::profiles::{ProfileManager, SoldierProfile};

    fn make_nest_at(engine: &mut EngineInner) -> EntityId {
        let mut element = ElementData {
            kind: ElementKind::ObjectProjectile,
            active: true,
            ..ElementData::default()
        };
        element.set_layer(0);
        let nest = ElementProjectile {
            element,
            object: ObjectData {
                object_type: ObjectType::BonusWaspNest,
                animation: Animation::ObjectFlying,
                ..ObjectData::default()
            },
            projectile: ProjectileData {
                flying: true,
                trajectory: Vec::new(),
                trajectory_frame_count: 0,
                ..ProjectileData::default()
            },
        };
        engine.add_entity(Entity::Projectile(nest))
    }

    fn empty_assets() -> LevelAssets {
        LevelAssets::default()
    }

    /// Assets with a single non-VIP Lacklandist soldier profile at index 0.
    fn assets_with_plain_soldier() -> LevelAssets {
        let mut pm = ProfileManager::new();
        pm.soldiers.push(SoldierProfile::default());
        LevelAssets {
            profile_manager: std::sync::Arc::new(pm),
            ..LevelAssets::default()
        }
    }

    fn make_soldier(pos: WorldPoint3D) -> Entity {
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
                soldier_profile_index: crate::profiles::SoldierProfileIdx(0),
                cached_camp: Camp::Lacklandists,
                ..SoldierData::default()
            },
        })
    }

    /// A wasp nest whose trajectory is exhausted should burst into 20
    /// wasps, flip to `ObjectBursting`, and seed the
    /// `flying_wasp_count`.  The nest itself stays alive so the buzz
    /// FX can keep emitting.
    #[test]
    fn wasp_nest_bursts_into_twenty_wasps() {
        crate::sim_rng::with_seed(0, |sim| {
            let mut engine = EngineInner::new();
            let nest_id = make_nest_at(&mut engine);
            let assets = empty_assets();

            engine.tick_wasp_nests(sim, &assets);

            let nest = match engine.get_entity(nest_id).unwrap() {
                Entity::Projectile(p) => p,
                _ => panic!("nest entity lost"),
            };
            assert!(nest.projectile.wasp.burst);
            // No hostile soldiers: the 20 wasps should survive Phase 3
            // because ChangeDirection's reachability check passes on an
            // empty obstacle list, so flying_wasp_count remains 20.
            assert_eq!(
                nest.projectile.wasp.flying_wasp_count,
                u32::from(NUMBER_OF_WASPS)
            );
            assert_eq!(nest.object.animation, Animation::ObjectBursting);
            assert!(!nest.projectile.flying);

            let wasp_count = engine
                .world
                .entities
                .projectiles()
                .filter(|(_, p)| {
                    p.object.object_type == ObjectType::Wasp
                        && p.projectile.wasp.source_nest == Some(nest_id)
                })
                .count();
            assert_eq!(wasp_count as u16, NUMBER_OF_WASPS);
        });
    }

    #[test]
    fn live_nest_pass_reaches_newly_appended_wasps_in_the_same_frame() {
        crate::sim_rng::with_seed(0x5A5A, |sim| {
            let mut parent_only = EngineInner::new();
            let nest_id = make_nest_at(&mut parent_only);
            let mut live_pass = parent_only.clone();
            let assets = empty_assets();

            parent_only.tick_wasp_nest_or_wasp(sim, &assets, nest_id);
            live_pass.tick_wasp_nests(sim, &assets);

            let parent_wasps: Vec<_> = parent_only
                .world
                .entities
                .projectiles()
                .filter(|(_, projectile)| projectile.object.object_type == ObjectType::Wasp)
                .map(|(id, projectile)| (EntityId::from(id), projectile.projectile.wasp.timeout))
                .collect();
            assert_eq!(parent_wasps.len(), NUMBER_OF_WASPS as usize);
            assert!(
                parent_wasps.iter().all(|(_, timeout)| *timeout == 0),
                "dispatching only the nest must leave appended wasps untouched"
            );
            for (wasp_id, _) in parent_wasps {
                let timeout = match live_pass.get_entity(wasp_id) {
                    Some(Entity::Projectile(wasp)) => wasp.projectile.wasp.timeout,
                    _ => panic!("same-frame wasp {wasp_id:?} missing"),
                };
                assert!(
                    (u32::from(DIRECTION_CHANGE_TIMEOUT)..=u32::from(DIRECTION_CHANGE_TIMEOUT) + 2)
                        .contains(&timeout),
                    "same-frame wasp {wasp_id:?} did not run its first Hourglass"
                );
            }
        });
    }

    /// While `flying_wasp_count > 0`, `tick_wasp_nests` emits the 507
    /// buzz FX every tick.
    #[test]
    fn burst_nest_emits_buzz_sound_while_wasps_fly() {
        crate::sim_rng::with_seed(0, |sim| {
            let mut engine = EngineInner::new();
            make_nest_at(&mut engine);
            let assets = empty_assets();

            engine.feedback.pending_side_effects.sounds.clear();
            engine.tick_wasp_nests(sim, &assets);
            let buzzes = engine.feedback.pending_side_effects
                .sounds
                .iter()
                .filter(|s| matches!(s, super::super::SoundCommand::Fx { fx_id, .. } if fx_id == &FX_WASP_BUZZ))
                .count();
            assert_eq!(
                buzzes, 1,
                "nest should emit buzz FX once per tick while wasps fly"
            );
        });
    }

    /// Once every wasp has been killed, the nest stops emitting the
    /// buzz FX.
    #[test]
    fn nest_stops_buzzing_when_all_wasps_despawn() {
        crate::sim_rng::with_seed(0, |sim| {
            let mut engine = EngineInner::new();
            let nest_id = make_nest_at(&mut engine);
            let assets = empty_assets();

            engine.tick_wasp_nests(sim, &assets);

            // Force-kill every wasp (mirrors what the per-wasp AI does
            // once each sting fires or each retry budget is exhausted).
            let wasp_ids: Vec<EntityId> = engine
                .world
                .entities
                .projectiles()
                .filter_map(|(id, projectile)| {
                    (projectile.object.object_type == ObjectType::Wasp).then_some(id.into())
                })
                .collect();
            for w in wasp_ids {
                engine.kill_wasp(w);
            }

            let nest = match engine.get_entity(nest_id).unwrap() {
                Entity::Projectile(p) => p,
                _ => panic!("nest entity lost"),
            };
            assert_eq!(nest.projectile.wasp.flying_wasp_count, 0);

            engine.feedback.pending_side_effects.sounds.clear();
            engine.tick_wasp_nests(sim, &assets);
            let buzzes = engine.feedback.pending_side_effects
                .sounds
                .iter()
                .filter(|s| matches!(s, super::super::SoundCommand::Fx { fx_id, .. } if fx_id == &FX_WASP_BUZZ))
                .count();
            assert_eq!(buzzes, 0, "nest should fall silent after wasps despawn");
        });
    }

    /// A wasp burst near an enemy soldier should select that soldier
    /// as its victim on the first tick that rolls a direction change,
    /// and flag the soldier's `wasp_victim`.  Exercises the full
    /// `ChangeVictim` / `ChooseVictim` port.
    #[test]
    fn wasp_targets_nearby_enemy_soldier() {
        crate::sim_rng::with_seed(42, |sim| {
            let mut engine = EngineInner::new();
            let assets = assets_with_plain_soldier();

            // Place a soldier 20 units from the nest — inside the
            // VICTIM_DETECTION_DISTANCE of 50.
            let soldier_pos = WorldPoint3D {
                x: 20.0,
                y: 0.0,
                z: 0.0,
            };
            let soldier_id = engine.add_entity(make_soldier(soldier_pos));

            // Pre-burst nest (same pattern as the other tests).
            make_nest_at(&mut engine);

            // Tick once to burst + run the first wasp AI pass.  The
            // freshly-spawned wasps have `timeout = 0`, so every wasp
            // runs ChangeVictim on this very tick.
            engine.tick_wasp_nests(sim, &assets);

            let soldier = engine.get_entity(soldier_id).unwrap();
            let Some(npc) = soldier.npc_data() else {
                panic!("soldier lost");
            };
            assert!(
                npc.wasp_victim,
                "nearby Lacklandist soldier should be flagged as a wasp victim"
            );

            // At least one wasp should point back at that soldier.
            let targeting = engine
                .world
                .entities
                .projectiles()
                .filter(|(_, p)| {
                    p.object.object_type == ObjectType::Wasp
                        && p.projectile.wasp.victim == Some(soldier_id)
                })
                .count();
            assert!(
                targeting >= 1,
                "at least one wasp should have the soldier selected as its victim"
            );
        });
    }

    #[test]
    fn reliable_acquisition_changes_only_the_documented_radius() {
        let classic = crate::gameplay_config::ItemGameplayConfig::classic();
        let mut reliable = classic;
        reliable.wasp_reliable_acquisition = true;

        assert_eq!(victim_detection_distance(classic, false), 50);
        assert_eq!(victim_detection_distance(reliable, false), 75);
        assert_eq!(victim_detection_distance(classic, true), 150);
        assert_eq!(victim_detection_distance(reliable, true), 225);
        assert_eq!(VICTIM_FORGET_DISTANCE, 70.0);
        assert_eq!(VICTIM_CHARGE_DISTANCE, 25.0);
        assert_eq!(STING_DISTANCE, 10.0);
    }
}
