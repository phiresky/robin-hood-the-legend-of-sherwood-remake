//! Bow shots and arrow projectile ticking.

use super::input::BowTarget;
use super::*;
use crate::bow_shot::{self};
use crate::coordinates::{GroundPoint, MapPoint};
use crate::element::{Command, Entity, EntityId};

#[cfg(test)]
thread_local! {
    static RECEIVE_PURSE_REVEAL_OBSERVER: std::cell::RefCell<Option<Box<dyn FnMut(&EngineInner, EntityId)>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_receive_purse_reveal_observer(
    observer: Option<Box<dyn FnMut(&EngineInner, EntityId)>>,
) {
    RECEIVE_PURSE_REVEAL_OBSERVER.with(|slot| *slot.borrow_mut() = observer);
}

/// Frames of apple-smell AI state after a soldier is hit by an apple.
pub const APPLE_SMELL_DURATION: u32 = 1500;

/// Piercing damage applied by a stone hit on an unprotected victim.
pub const STONE_DAMAGE: u16 = 10;

/// Concussion applied by a stone hit on an unprotected victim.  Heavy
/// KO potential relative to damage.
pub const STONE_CONCUSSION: u16 = 100;

/// Outcome of testing an arrow-candidate-victim impact.  See
/// [`EngineInner::classify_arrow_hit`] for the full control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArrowHitOutcome {
    /// Apply piercing damage to the victim.
    Damage,
    /// Arrow flies through silently — friendly-fire filter or VIP NPC.
    /// Hit flag and impact sound are both suppressed.
    PassThrough,
    /// Arrow ricochets off the victim's armor.  C++ puts the arrow into
    /// falling state before impact-FX lookup, so this path is silent.
    Ricochet,
}

impl EngineInner {
    // ─── Bow shots & arrow projectiles ───────────────────────────

    /// Rebuild Rust's derived active-shot latch after loading an Original
    /// save in the middle of a shooting order. Original needs no parallel
    /// latch: the selected sequence element and current RHOrder are sufficient
    /// for `Execute` to call `ShootWithBowAt` on the action-done pulse.
    pub(crate) fn restore_loaded_active_shots(&mut self) {
        let owners = self
            .world
            .entities
            .actors()
            .map(|(id, _)| EntityId::from(id))
            .collect::<Vec<_>>();
        let active = owners
            .into_iter()
            .filter_map(|owner| {
                let (sequence_id, element_index, _order) = self
                    .orders
                    .sequence_manager
                    .current_order_for_actor(owner)?;
                let element = self
                    .orders
                    .sequence_manager
                    .get_element(sequence_id, element_index)?;
                if !matches!(element.command, Command::ShootBow | Command::ShootBowOnce) {
                    return None;
                }
                let (shoot_mode, shoot_order_id) = element.orders.iter().find_map(|order| {
                    crate::bow_shot::shoot_mode_for_order(order.order_type)
                        .map(|mode| (mode, order.order_id))
                })?;
                let target = match &element.data {
                    crate::sequence::SequenceElementData::Interaction { antagonist } => {
                        (*antagonist)?
                    }
                    _ => return None,
                };
                Some((
                    owner,
                    crate::movement::ActiveShot {
                        sequence_id: Some(sequence_id),
                        element_index,
                        target: Some(target),
                        order_id: Some(shoot_order_id),
                        released: false,
                        shoot_mode: Some(shoot_mode),
                    },
                ))
            })
            .collect::<Vec<_>>();

        for (owner, shot) in active {
            self.world
                .entities
                .get_mut(owner)
                .and_then(Entity::actor_data_mut)
                .unwrap_or_else(|| {
                    panic!("loaded bow-shot owner {owner:?} is missing required actor state")
                })
                .active_shot = shot;
        }
    }

    pub(super) fn apply_projectile_landing_resolution(
        &mut self,
        assets: &LevelAssets,
        projectile_id: EntityId,
    ) -> Option<crate::fast_find_grid::ProjectileLandingResolution> {
        let landing_map = {
            let entity = self.get_entity(projectile_id)?;
            let pos = entity.element_data().position();
            pos.to_map()
        };
        let resolution = self
            .world
            .fast_grid
            .resolve_projectile_landing(landing_map, self.sight_obstacles(assets));
        if let Some(entity) = self.world.entities.get_mut(projectile_id) {
            let obstacle_plane = crate::position_interface::PlaneZCoeffs::resolve_for_obstacle(
                resolution.obstacle_index,
                assets.static_sight_obstacles.as_slice(),
            );
            bow_shot::apply_projectile_landing_resolution(
                entity.element_data_mut(),
                resolution,
                obstacle_plane,
            );
        }
        Some(resolution)
    }

    /// Public entry point for "player pressed the bow button on a
    /// target".  Launches a `Command::ShootBow` sequence element on the
    /// shooter and returns its sequence id.
    pub fn shoot_bow_at(
        &mut self,
        assets: &LevelAssets,
        shooter: EntityId,
        target: EntityId,
    ) -> Option<crate::sequence::SequenceId> {
        let Some(shooter_entity) = self.get_entity(shooter) else {
            tracing::warn!(
                shooter = ?shooter,
                target = ?target,
                "shoot_bow_at: missing shooter"
            );
            return None;
        };
        if !shooter_entity.is_human() {
            tracing::warn!(
                shooter = ?shooter,
                target = ?target,
                shooter_kind = ?shooter_entity.kind(),
                "shoot_bow_at: non-human shooter"
            );
            return None;
        }
        if shooter_entity.is_dead() {
            tracing::warn!(
                shooter = ?shooter,
                target = ?target,
                "shoot_bow_at: dead shooter"
            );
            return None;
        }
        let Some((bow_profile_idx, _)) = self.bow_profile_and_ability(assets, shooter) else {
            tracing::warn!(
                shooter = ?shooter,
                target = ?target,
                "shoot_bow_at: shooter has no bow profile"
            );
            return None;
        };
        let Some(bow_profile) = assets.profile_manager.get_bow(bow_profile_idx) else {
            tracing::warn!(
                shooter = ?shooter,
                target = ?target,
                bow_profile_idx,
                "shoot_bow_at: missing bow profile"
            );
            return None;
        };
        if bow_profile.normal_shoot.range == 0 {
            tracing::warn!(
                shooter = ?shooter,
                target = ?target,
                bow_profile_idx,
                "shoot_bow_at: shooter bow profile has no range"
            );
            return None;
        }

        // Both humans and FX targets are valid bow-shot targets.  FX
        // targets with the ARROW action filter are the hunting/puzzle
        // targets in forest levels.
        let Some(target_entity) = self.get_entity(target) else {
            tracing::warn!(
                shooter = ?shooter,
                target = ?target,
                "shoot_bow_at: missing target"
            );
            return None;
        };
        match target_entity {
            Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_) if target_entity.is_dead() => {
                tracing::warn!(
                    shooter = ?shooter,
                    target = ?target,
                    "shoot_bow_at: dead target"
                );
                return None;
            }
            Entity::Pc(_) | Entity::Soldier(_) | Entity::Civilian(_) => {}
            Entity::Target(t)
                if t.target
                    .action_filter
                    .contains(crate::element::TargetFilter::ARROW) => {}
            Entity::Target(_) => {
                tracing::warn!(
                    shooter = ?shooter,
                    target = ?target,
                    "shoot_bow_at: target does not accept arrows"
                );
                return None;
            }
            other => {
                tracing::warn!(
                    shooter = ?shooter,
                    target = ?target,
                    target_kind = ?other.kind(),
                    "shoot_bow_at: unsupported target kind"
                );
                return None;
            }
        }

        Some(self.launch_element(bow_shot::build_shoot_bow_element(shooter, target)))
    }

    /// Look up the bow profile index and shooting ability for an entity.
    ///
    /// Returns `(bow_profile_index, shooting_ability)` or `None` if the
    /// entity has no bow data.
    pub(super) fn bow_profile_and_ability(
        &self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) -> Option<(u32, u32)> {
        let entity = self.get_entity(entity_id)?;
        match entity {
            Entity::Pc(pc) => {
                let idx = usize::from(pc.pc.profile_index);
                let profile = assets.profile_manager.characters.get(idx)?;
                if profile.shooting_weapon_id == 0 {
                    return None;
                }
                Some((profile.shooting_weapon_id, profile.shooting as u32))
            }
            Entity::Soldier(s) => {
                let idx = usize::from(s.soldier.soldier_profile_index);
                let profile = assets.profile_manager.soldiers.get(idx)?;
                if profile.shooting_weapon_id == 0 {
                    return None;
                }
                // The shooting-ability lookup applies FIGHTING modifiers
                // (not SHOOTING — appears to be an upstream bug preserved
                // for accuracy).
                let mut shooting = if s.soldier.cached_camp == crate::element::Camp::Lacklandists {
                    let diff = self.control.sim_config.difficulty;
                    diff.modify_capacity(
                        profile.shooting,
                        crate::player_profile::difficulty_params::EASY_ENEMY_FIGHTING,
                        crate::player_profile::difficulty_params::HARD_ENEMY_FIGHTING,
                        100,
                    ) as u32
                } else {
                    profile.shooting as u32
                };
                // Apply blood_alcohol penalty:
                // result = result * (1.0 - 0.01 * bloodAlcohol)
                let blood_alcohol = s.npc.ai_brain.base().map_or(0, |a| a.blood_alcohol);
                if blood_alcohol > 0 {
                    shooting =
                        ((shooting as f32) * (1.0 - 0.01 * blood_alcohol as f32)).max(0.0) as u32;
                }
                Some((profile.shooting_weapon_id, shooting))
            }
            _ => None,
        }
    }

    pub(super) fn selected_bow_order(
        &self,
        owner: EntityId,
    ) -> Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)> {
        let shot = self.get_entity(owner)?.actor_data()?.active_shot;
        let (seq_id, elem_idx, order) = self
            .orders
            .sequence_manager
            .current_order_for_actor(owner)?;
        (shot.is_active()
            && shot.sequence_id == Some(seq_id)
            && shot.element_index == elem_idx
            && bow_shot::is_active_bow_order(order.order_type))
        .then_some((seq_id, elem_idx, order.order_id))
    }

    /// Advance one exact selected bow arm without detaching or restoring any
    /// other actor state.
    pub(super) fn tick_bow_shot_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        shooter_id: EntityId,
        expected_order_id: std::num::NonZeroU32,
    ) -> Vec<EntityId> {
        let mut spawned_projectiles = Vec::new();
        if self
            .get_entity(shooter_id)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.execution_frozen)
        {
            return spawned_projectiles;
        }
        let sprite_frozen = self.actors_frozen();
        let events = bow_shot::tick_bow_shot_for_owner(
            sim,
            &mut self.world.entities,
            &mut self.orders.sequence_manager,
            shooter_id,
            expected_order_id,
            sprite_frozen,
        );
        for result in events.fired {
            let Some(shooter_entity) = self.get_entity(result.shooter) else {
                tracing::warn!(
                    shooter = ?result.shooter,
                    target = ?result.target,
                    "Bow shot release skipped: shooter entity missing"
                );
                continue;
            };
            let layer = shooter_entity.element_data().layer();
            let shooter_is_pc = shooter_entity.kind().is_pc();

            let Some(target_entity) = self.get_entity(result.target) else {
                tracing::warn!(
                    shooter = ?result.shooter,
                    target = ?result.target,
                    "Bow shot release skipped: target entity missing"
                );
                continue;
            };
            let target_is_fx_target = target_entity.kind().is_fx_target();
            let target_is_human = target_entity.is_human();
            let target_posture = target_entity.element_data().posture;

            // ── Determine shoot mode from action state ───────────
            let shoot_mode = result.shoot_mode;
            let flat_shot = bow_shot::is_flat_shot(shoot_mode);
            let mass = bow_shot::arrow_mass(shoot_mode);

            // ── Look up bow profile for damage and hit chance ────
            let Some((bow_profile_idx, shooting_ability)) =
                self.bow_profile_and_ability(assets, result.shooter)
            else {
                tracing::warn!(
                    shooter = ?result.shooter,
                    "Bow shot release skipped: shooter has no bow profile data"
                );
                continue;
            };

            let Some(bow_profile) = assets.profile_manager.get_bow(bow_profile_idx) else {
                tracing::warn!(
                    shooter = ?result.shooter,
                    bow_profile_idx,
                    "Bow shot release skipped: missing bow profile"
                );
                continue;
            };

            use crate::weapons::{BowState, ShootMode};
            // Create a temporary BowState just for the lookup.
            let bow = BowState::new(bow_profile_idx, bow_profile, 1);
            // Down maps to Normal for damage lookup (flat shots use Normal,
            // arced shots use Long).
            let lookup_mode = match shoot_mode {
                ShootMode::Down => ShootMode::Normal,
                other => other,
            };
            let damage = bow.get_damage(bow_profile, lookup_mode);

            // ── Compute bow point (hand position) ────────────────
            let bow_point = bow_shot::compute_bow_point(
                result.shooter_position,
                shoot_mode,
                result.shooter_direction,
                result.sprite_hand_point,
            );

            // ── Target belt point (resolved in tick_bow_shots) ───
            // For LEANING_OUT targets the belt can be obstructed by
            // the parapet/crenel, so we fall back to the eyes point if the
            // belt aim would only be reachable as a long shot (or not at
            // all).  Non-leaning targets always aim at belt.
            //
            // Only applies when the planned shoot mode is Normal: re-run
            // `can_shoot_with_bow_at_point` against the belt and swap to
            // eyes when that re-check fails or upgrades to a long shot.
            // `can_shoot_with_bow_at_point` folds in range / posture-
            // override / ammo semantics.
            let mut target_point = result.target_point;
            if target_posture == crate::element::Posture::LeaningOut
                && shoot_mode == crate::weapons::ShootMode::Normal
            {
                let (belt_status, belt_mode) =
                    self.can_shoot_with_bow_at_point(assets, result.shooter, target_point, false);
                let belt_failed =
                    belt_status != BowTarget::Valid || belt_mode == crate::weapons::ShootMode::Long;
                if belt_failed
                    && let Some(eyes) = self
                        .get_entity(result.target)
                        .and_then(|e| e.compute_eyes_point(None))
                {
                    target_point = eyes;
                }
            }

            // ── Lead a moving target ─────────────────────────────
            // For human targets, read their forecasted movement so the
            // shot leads them; FX targets pass None.
            //
            // PositionInterface returns canonical world XYZ data; projectile
            // code still carries the older element-local 3D type for now.
            let target_movement = target_is_human.then_some(result.target_forecasted_movement);

            // ── Compute velocity ─────────────────────────────────
            // `compute_shot_velocity_params` forwards `target_movement`
            // into `compute_initial_throw_velocity`, which adds
            // `movement * 0.5 * TIME_FLYSEGMENT` to lead a moving target.
            // Adding the lead a second time here would double-correct,
            // so we trust the helper.
            let (mut velocity, _flight_time, _apex) = bow_shot::compute_shot_velocity_params(
                bow_point,
                target_point,
                shoot_mode,
                target_movement,
            );

            // Warn shield-bearing target soldiers that they're being shot
            // at before the hit roll and projectile insertion, matching
            // C++ `ShootWithBowAt`.
            let target_is_shield_soldier = match self.get_entity(result.target) {
                Some(Entity::Soldier(s)) => match assets
                    .profile_manager
                    .get_soldier(s.soldier.soldier_profile_index)
                    .and_then(|p| assets.profile_manager.get_hth_weapon(p.hth_weapon_id))
                {
                    Some(w) => w.shield,
                    None => {
                        tracing::warn!(
                            target = ?result.target,
                            "Bow shot shield warning skipped: missing soldier HtH weapon profile"
                        );
                        false
                    }
                },
                _ => false,
            };
            if target_is_shield_soldier {
                // "Detecting" means the shooter is visible *this frame*
                // (cone + LOS).  The NPC tick caches that live result as
                // `det.seen_now`, so checking that flag is equivalent to
                // a fresh visibility query without rebuilding the full
                // `VisibilityQuery`.  Using `detectable_lists` membership
                // alone would be wrong: the entry persists forever, so a
                // soldier whose LOS of the archer is now occluded by a
                // wall would still raise his shield — the audit-flagged
                // "soldier cheats" case.
                let target_detects_shooter =
                    match self.get_entity(result.target).and_then(|e| e.npc_data()) {
                        Some(npc) => npc.detectable_lists.iter().any(|list| {
                            list.iter()
                                .any(|d| d.element == Some(result.shooter) && d.seen_now)
                        }),
                        None => {
                            tracing::warn!(
                                target = ?result.target,
                                shooter = ?result.shooter,
                                "Bow shot shield warning skipped: shield soldier missing NPC data"
                            );
                            false
                        }
                    };
                if target_detects_shooter {
                    self.dispatch_ai_stimulus(
                        result.target,
                        crate::ai::Stimulus::with_human(
                            crate::ai::StimulusType::EventArrowLaunched,
                            result.shooter.index(),
                        ),
                    );
                }
            }

            // ── Hit chance roll ──────────────────────────────────
            // C++ only applies `mpBow->GetHitChance(...)` in the
            // `pTarget->IsHuman()` branch of `ShootWithBowAt`.
            // Scripted FX targets use the exact center-point trajectory.
            let hit_distance = {
                let dx = target_point.x - bow_point.x;
                let dy = target_point.y - bow_point.y;
                let dz = target_point.z - bow_point.z;
                (dx * dx + dy * dy + dz * dz).sqrt()
            };

            let hit_chance = if target_is_human {
                let bow = crate::weapons::BowState::new(bow_profile_idx, bow_profile, 1);
                bow.get_hit_chance(bow_profile, shooting_ability, hit_distance as u32)
            } else {
                100
            };

            // Bow skill capacity for bias scaling.
            let bow_skill_capacity = shooting_ability;

            if target_is_human
                && let Some(bias) =
                    bow_shot::roll_hit_and_compute_bias(sim, hit_chance, bow_skill_capacity)
            {
                // Miss — deflect the velocity.
                velocity.x += bias.x;
                velocity.y += bias.y;
                velocity.z += bias.z;
                tracing::debug!(
                    shooter = ?result.shooter,
                    ?hit_chance,
                    ?bias,
                    "Bow shot missed (bias applied)"
                );
            }

            // ── Bloodseeker-oil check ────────────────────────────
            // When a PC shoots an FX target in a forest level the
            // arrow gets magic-bullet mode, bypassing obstacle collision so
            // it can pass through trees to reach the target.
            let magic_bullet =
                target_is_fx_target && shooter_is_pc && self.world.weather.is_forest_level;

            // ── Compute ballistic trajectory ─────────────────────
            let obstacle_list = self.sight_obstacles(assets);
            let obstacle_check = bow_shot::TrajectoryObstacleCheck {
                fast_find_grid: &self.world.fast_grid,
                layer,
                sight_obstacles: obstacle_list,
                water_zones: Some(&assets.water_zones),
            };
            let (trajectory, terminal_obstacle) =
                bow_shot::compute_trajectory_ballistic_with_terminal_obstacle(
                    bow_point,
                    velocity,
                    mass,
                    flat_shot,
                    // Magic-bullet short-circuit: skip the obstacle check entirely.
                    if magic_bullet {
                        None
                    } else {
                        Some(&obstacle_check)
                    },
                );
            let trajectory_end = trajectory.last().map(|tp| tp.position);
            // ComputeTrajectory resolves and stores the eventual impact
            // membership before the projectile's explicit pre-add
            // Hourglass. It is therefore observable throughout flight, not
            // only after the projectile lands.
            let initial_landing_resolution = trajectory_end.map(|end| {
                self.world
                    .fast_grid
                    .resolve_projectile_landing_with_obstacle(
                        end.to_map(),
                        terminal_obstacle,
                        obstacle_list,
                    )
            });
            tracing::debug!(
                shooter = ?result.shooter,
                target = ?result.target,
                ?shoot_mode,
                ?bow_point,
                ?target_point,
                ?trajectory_end,
                trajectory_len = trajectory.len(),
                magic_bullet,
                predicted_hit = bow_shot::will_hit_target(&trajectory, bow_point, target_point),
                "Bow shot trajectory computed"
            );

            // ── Spawn the arrow ──────────────────────────────────
            // Pre-flag `disappear` when the trajectory's final approach
            // lies inside a hole polygon.  The extension inside
            // `compute_trajectory_ballistic_impl` may have already slid
            // the last waypoint to the hole's far edge, so we inspect
            // the last two points — one of them is the original
            // pre-extension landing.
            let lands_in_hole = trajectory
                .iter()
                .rev()
                .take(2)
                .any(|tp| assets.water_zones.landing_is_in_hole(tp.position.to_map()));
            let arrow = bow_shot::spawn_arrow(bow_shot::SpawnArrowParams {
                shooter: result.shooter,
                bow_point,
                trajectory_origin: crate::coordinates::MapPoint {
                    x: result.shooter_position.x,
                    y: result.shooter_position.y,
                },
                target: result.target,
                target_pos: result.target_pos,
                trajectory,
                damage,
                layer,
                lands_in_hole,
                initial_velocity: velocity,
            });
            let arrow_id = self.add_entity(arrow);
            if let Some(resolution) = initial_landing_resolution {
                let entity = self
                    .world
                    .entities
                    .get_mut(arrow_id)
                    .expect("newly added arrow vanished before landing-state initialization");
                let element = entity.element_data_mut();
                element.set_sector(resolution.sector);
                if resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle {
                    element.set_layer(resolution.layer);
                }
            }
            // Hydrate the arrow's sprite from the accessory registry so
            // the flying arrow renders its proper sprite instead of the
            // colored-rect fallback.
            self.attach_accessory_sprite(assets, arrow_id);
            self.tick_new_projectile_once(sim, assets, arrow_id);
            spawned_projectiles.push(arrow_id);

            tracing::debug!(
                shooter = ?result.shooter,
                target = ?result.target,
                arrow = ?arrow_id,
                ?shoot_mode,
                damage,
                ?hit_chance,
                "Arrow spawned from bow shot"
            );

            // ── Decrement bow ammo after shot ───────────────────
            // Decrement ammo by 1; disable the bow action if ammo hits 0.
            self.decrement_bow_ammo(assets, result.shooter);

            // The sequence element stays in progress after release so
            // the shoot animation and reload/unequip orders can finish.
            // `tick_bow_shots` emits completion when the final bow order
            // terminates.
        }
        for (seq_id, elem_idx) in events.completed {
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
        }
        spawned_projectiles
    }

    /// Put an arrow into non-shield falling state — the "armor ricochet"
    /// branch: inverse sector (xor 8), `y * 10`, z velocity zero.  Used
    /// when a PC/Soldier is hit but not hurtable (same-camp friendly fire
    /// or a successful piercing-protection roll).
    fn start_arrow_ricochet(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        arrow_id: EntityId,
    ) {
        let Some(entity) = self.world.entities.get_mut(arrow_id) else {
            return;
        };
        let Entity::Projectile(proj) = entity else {
            return;
        };

        bow_shot::make_arrow_falling_down(sim, proj, false);
    }

    /// Classify an arrow impact on a candidate victim.
    ///
    /// Folds together two distinct concerns whose outcomes differ on a
    /// "miss":
    ///   * Find-victim filter match (forest royalist vs royalist,
    ///     soldier→civilian, soldier→same-camp, PC→PC-with-shield) →
    ///     target is invisible to the search, the arrow sails past
    ///     silently.
    ///   * VIP-NPC / civilian-non-hurtable branch → no hit, no impact
    ///     sound: the arrow also passes through silently.
    ///   * PC/Soldier non-hurtable branch → falling state with impact
    ///     sound — armor ricochet.
    ///   * Piercing-protection roll for PC / Soldier targets rolls
    ///     `rand() % 101 <= protection`; if it passes the target is
    ///     flagged non-hurtable, funnelling into the PC/Soldier ricochet
    ///     branch.
    ///
    /// `PassThrough` replays the silent miss, `Ricochet` plays the
    /// falling-state transition, and `Damage` launches the damage
    /// sequence element.
    fn classify_arrow_hit(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        shooter_id: EntityId,
    ) -> ArrowHitOutcome {
        let victim = match self.get_entity(victim_id) {
            Some(e) => e,
            None => return ArrowHitOutcome::PassThrough,
        };

        // ── (A) VIP NPC — early-out ───────────────────────────────
        // Arrow sails past silently, no impact sound.
        if victim.is_npc() {
            let is_vip = match victim {
                Entity::Soldier(s) => match assets
                    .profile_manager
                    .soldiers
                    .get(usize::from(s.soldier.soldier_profile_index))
                {
                    Some(profile) => profile.vip,
                    None => {
                        tracing::warn!(
                            ?victim_id,
                            profile = ?s.soldier.soldier_profile_index,
                            "arrow hit classification missing soldier profile; treating victim as protected"
                        );
                        return ArrowHitOutcome::PassThrough;
                    }
                },
                _ => false,
            };
            if is_vip {
                return ArrowHitOutcome::PassThrough;
            }
        }

        // ── (B) Gather shooter / victim camp + kind info ────────────
        let Some(shooter) = self.get_entity(shooter_id) else {
            tracing::warn!(
                ?victim_id,
                ?shooter_id,
                "arrow hit classification missing shooter entity; skipping hit"
            );
            return ArrowHitOutcome::PassThrough;
        };
        let shooter_is_npc = shooter.is_npc();
        let shooter_is_pc = shooter.is_pc();
        let shooter_is_soldier = shooter.is_soldier();
        let shooter_camp = Some(match shooter {
            Entity::Pc(_) => crate::element::Camp::Royalists,
            Entity::Soldier(s) => s.soldier.cached_camp,
            Entity::Civilian(c) => c.civilian.cached_camp,
            _ => crate::element::Camp::Royalists,
        });
        let victim_camp = match victim {
            Entity::Pc(_) => Some(crate::element::Camp::Royalists),
            Entity::Soldier(s) => Some(s.soldier.cached_camp),
            Entity::Civilian(c) => Some(c.civilian.cached_camp),
            _ => None,
        };
        let same_camp = matches!(
            (shooter_camp, victim_camp),
            (Some(sc), Some(vc)) if sc == vc,
        );
        let victim_is_pc_with_shield = if victim.is_pc() {
            match victim.actor_data() {
                Some(actor) => actor.action_state.is_shield(),
                None => {
                    tracing::warn!(
                        ?victim_id,
                        "arrow hit classification PC victim missing actor data"
                    );
                    return ArrowHitOutcome::PassThrough;
                }
            }
        } else {
            false
        };
        let victim_is_pc_or_soldier = victim.is_pc() || victim.is_soldier();

        // ── (C) Find-victim pre-filter ──────────────────────────────
        // When one of these fires, the candidate is invisible to the
        // arrow's victim search — no impact sound, no ricochet. Maps to
        // PassThrough.
        //
        // Note: rule (1) "forest + both GoodSoldier" is strictly a
        // subset of rule (3) "Soldier shooter + same camp" (both
        // GoodSoldier ⇒ both Royalists ⇒ same camp), so testing rule
        // (3) alone covers it.
        //
        // Rule (2) Soldier → Civilian.
        // Rule (3) Soldier → same camp.
        // Rule (4) PC → PC with shield.
        if shooter_is_soldier && (victim.is_civilian() || same_camp) {
            return ArrowHitOutcome::PassThrough;
        }
        if shooter_is_pc && victim_is_pc_with_shield {
            return ArrowHitOutcome::PassThrough;
        }

        // ── (D) Base hurtable filter ────────────────────────────────
        // For NPC shooters (or PC shooters on non-Hard difficulty),
        // civilian victims and same-camp victims are flagged
        // non-hurtable. Hard-difficulty PC shooters skip this filter
        // entirely (civilian friendly fire allowed).
        let apply_hurtable_filter = if shooter_is_npc {
            true
        } else if shooter_is_pc {
            sim.config().difficulty != crate::player_profile::DifficultyLevel::Hard
        } else {
            false
        };
        let hurtable_base = if apply_hurtable_filter {
            !(victim.is_civilian() || same_camp)
        } else {
            true
        };

        // ── (E) Piercing-protection roll ─────────────────────────────
        // Applies to PC and Soldier victims, only when the base filter
        // already flagged the victim hurtable.
        let piercing_protection = match victim {
            Entity::Pc(pc) => assets
                .profile_manager
                .get_character(pc.pc.profile_index)
                .and_then(|p| assets.profile_manager.get_hth_weapon(p.hth_weapon_id))
                .map(|w| w.piercing_protection),
            Entity::Soldier(s) => assets
                .profile_manager
                .get_soldier(s.soldier.soldier_profile_index)
                .and_then(|p| assets.profile_manager.get_hth_weapon(p.hth_weapon_id))
                .map(|w| w.piercing_protection),
            _ => None,
        };
        let hurtable = if hurtable_base {
            // `(rand() % 101) > protection` runs even when protection is
            // 0 — gives a 1/101 ricochet for the exact `roll == 0` case,
            // and keeps RNG consumption consistent with the
            // piercing-protection > 0 path. Missing weapon profile data
            // is invalid actor state; C++ calls through `mpSword`, so do
            // not invent an unconditional-damage fallback.
            match piercing_protection {
                Some(protection) => {
                    let roll = crate::sim_rng::u32(
                        sim,
                        crate::sim_rng::RngSite::ArrowPiercingProtection,
                        0..101,
                    );
                    roll > protection as u32
                }
                None => {
                    tracing::warn!(
                        ?victim_id,
                        "IsHurtableByArrow: missing HtH weapon profile; treating victim as protected",
                    );
                    false
                }
            }
        } else {
            false
        };

        // ── (F) Outcome dispatch ────────────────────────────────────
        // Hurtable → Damage.
        // !Hurtable + victim is PC or Soldier → Ricochet. This is
        // silent for arrows because `MakeFallingDown` sets `mbFalling`
        // before `GetImpactFx()`.
        // !Hurtable + civilian → silent miss (PassThrough).
        if hurtable {
            ArrowHitOutcome::Damage
        } else if victim_is_pc_or_soldier {
            ArrowHitOutcome::Ricochet
        } else {
            ArrowHitOutcome::PassThrough
        }
    }

    /// Check if the shooter has bow ammo available.
    ///
    /// Returns `true` if the shooter has at least one arrow. PCs read
    /// campaign-side status; NPC soldiers read their live
    /// `number_of_arrows` counter.
    pub fn check_bow_ammo(&self, shooter_id: EntityId) -> bool {
        match self.get_entity(shooter_id) {
            Some(Entity::Pc(pc)) => match self.pc_description_for_pc_data(&pc.pc) {
                Some(pc_desc) => pc_desc.status.get_ammo(crate::profiles::Action::Bow) > 0,
                None => {
                    tracing::warn!(
                        shooter = ?shooter_id,
                        "check_bow_ammo: PC has no campaign status"
                    );
                    false
                }
            },
            Some(Entity::Soldier(s)) => s.npc.number_of_arrows > 0,
            Some(_) => true,
            None => {
                tracing::warn!(
                    shooter = ?shooter_id,
                    "check_bow_ammo: shooter entity missing"
                );
                false
            }
        }
    }

    /// Get the number of bow arrows the shooter has.
    ///
    /// Returns `u32::MAX` only for non-human object/civilian callers
    /// that do not track bow ammo.
    pub fn get_bow_ammo_count(&self, shooter_id: EntityId) -> u32 {
        match self.get_entity(shooter_id) {
            Some(Entity::Pc(pc)) => match self.pc_description_for_pc_data(&pc.pc) {
                Some(pc_desc) => pc_desc.status.get_ammo(crate::profiles::Action::Bow) as u32,
                None => {
                    tracing::warn!(
                        shooter = ?shooter_id,
                        "get_bow_ammo_count: PC has no campaign status"
                    );
                    0
                }
            },
            Some(Entity::Soldier(s)) => u32::from(s.npc.number_of_arrows),
            Some(_) => u32::MAX,
            None => {
                tracing::warn!(
                    shooter = ?shooter_id,
                    "get_bow_ammo_count: shooter entity missing"
                );
                0
            }
        }
    }

    /// Return one PC's authoritative campaign-side ammunition counter.
    ///
    /// PC entities intentionally do not duplicate these counters: the
    /// campaign character status is the live source, as in the Original.
    /// Debug/parity consumers need the same lookup rather than a stale
    /// entity-local mirror.
    pub fn get_pc_ammo_count(&self, pc_id: EntityId, action: crate::profiles::Action) -> u16 {
        let pc = match self.get_entity(pc_id) {
            Some(Entity::Pc(pc)) => pc,
            Some(entity) => panic!(
                "get_pc_ammo_count expected PC {pc_id:?}, found {:?}",
                entity.kind()
            ),
            None => panic!("get_pc_ammo_count PC {pc_id:?} is missing"),
        };
        self.pc_description_for_pc_data(&pc.pc)
            .unwrap_or_else(|| {
                panic!(
                    "get_pc_ammo_count PC {pc_id:?} profile {} has no campaign character status",
                    pc.pc.profile_index
                )
            })
            .status
            .get_ammo(action)
    }

    /// Decrement the shooter's bow ammo by 1 after a shot.
    ///
    /// PCs hit the campaign-side PcStatus; NPC soldiers
    /// saturate-decrement `npc.number_of_arrows` so the
    /// `FleeingRunForArrowReserves` refill loop has a chance to trigger
    /// (the AI gates on `ctx.remaining_arrows > 0` and
    /// `pending_refill_bow_ammo` restocks).
    fn decrement_bow_ammo(&mut self, assets: &LevelAssets, shooter_id: EntityId) {
        // Soldier branch — saturating sub on the live NPC field.
        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(shooter_id) {
            s.npc.number_of_arrows = s.npc.number_of_arrows.saturating_sub(1);
            tracing::debug!(
                shooter = ?shooter_id,
                remaining = s.npc.number_of_arrows,
                "NPC bow ammo decremented"
            );
            return;
        }

        let status_idx = match self.get_entity(shooter_id) {
            Some(Entity::Pc(pc)) => self.pc_description_index_for_pc_data(&pc.pc),
            _ => None,
        };
        let Some(status_idx) = status_idx else {
            return; // Civilians / props don't track ammo
        };

        let remaining = if let Some(campaign) = Some(&mut self.mission_domain.campaign)
            && let Some(pc_desc) = campaign.characters.get_mut(status_idx)
        {
            let removed = pc_desc
                .status
                .decrease_ammo(crate::profiles::Action::Bow, 1);
            let remaining = pc_desc.status.get_ammo(crate::profiles::Action::Bow);
            tracing::debug!(
                shooter = ?shooter_id,
                removed,
                remaining,
                "Bow ammo decremented"
            );
            remaining
        } else {
            return;
        };
        // When ammo hits 0, disable the Bow action and speak
        // HERO_OUT_OF_AMMO if the level isn't Sherwood.
        if remaining == 0 {
            self.disable_pc_action(assets, shooter_id, crate::profiles::Action::Bow);
            if !self.is_sherwood(&assets.profile_manager) {
                self.hero_speaking(assets, shooter_id, crate::engine::melee::HERO_OUT_OF_AMMO);
            }
        }
    }

    /// Decrement ammo for a generic ability (heal, net, wasp-nest, etc.)
    /// and disable the action in the UI when ammo reaches 0.
    ///
    /// Check if a PC has ammo for a given action (via campaign PcStatus).
    /// Returns `false` for non-PCs or if campaign isn't loaded.
    pub(super) fn has_ammo(&self, actor_id: EntityId, action: crate::profiles::Action) -> bool {
        match self.get_entity(actor_id) {
            Some(Entity::Pc(pc)) => match self.pc_description_for_pc_data(&pc.pc) {
                Some(pc_desc) => pc_desc.status.get_ammo(action) > 0,
                None => {
                    tracing::warn!(
                        actor = ?actor_id,
                        ?action,
                        "has_ammo: PC has no campaign status"
                    );
                    false
                }
            },
            _ => true, // non-PCs don't track ammo
        }
    }

    /// Decrement ability ammo by 1; disable the action slot when ammo
    /// hits 0.
    fn decrement_ability_ammo(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        action: crate::profiles::Action,
    ) {
        let status_idx = match self.get_entity(actor_id) {
            Some(Entity::Pc(pc)) => self.pc_description_index_for_pc_data(&pc.pc),
            _ => None,
        };
        let Some(status_idx) = status_idx else {
            return; // Only PCs track ammo
        };

        let remaining = if let Some(campaign) = Some(&mut self.mission_domain.campaign) {
            if let Some(pc_desc) = campaign.characters.get_mut(status_idx) {
                let removed = pc_desc.status.decrease_ammo(action, 1);
                let remaining = pc_desc.status.get_ammo(action);
                tracing::debug!(
                    actor = ?actor_id,
                    ?action,
                    removed,
                    remaining,
                    "Ability ammo decremented"
                );
                remaining
            } else {
                return;
            }
        } else {
            return;
        };

        // Disable the action and speak HERO_OUT_OF_AMMO.  Every ability
        // call site (Heal/Ale/Apple/Stone/Purse/WaspNest/Net) wants the
        // speech, so we always speak here; the speech is gated on
        // `!IsSherwood()` to suppress it on the hub map.
        if remaining == 0 {
            self.disable_pc_action(assets, actor_id, action);
            if !self.is_sherwood(&assets.profile_manager) {
                self.hero_speaking(assets, actor_id, crate::engine::melee::HERO_OUT_OF_AMMO);
            }
        }
    }

    /// Consume one Stoeckel ration through Original's `SetAmmoAmount` path.
    ///
    /// Eating is deliberately different from every `DecreaseAmmoAmount`
    /// ability: `RHElementActorPC::RHANIMATION_EATING` computes the remaining
    /// count and calls `SetAmmoAmount(RHACTION_EAT/GUZZLE, remaining)`.  That
    /// disables an emptied action slot but does not say `HERO_OUT_OF_AMMO`.
    pub(super) fn consume_ration_without_speech(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        action: crate::profiles::Action,
    ) {
        debug_assert!(matches!(
            action,
            crate::profiles::Action::Eat | crate::profiles::Action::Guzzle
        ));
        let status_idx = match self.get_entity(actor_id) {
            Some(Entity::Pc(pc)) => self.pc_description_index_for_pc_data(&pc.pc),
            _ => None,
        };
        let status_idx = status_idx
            .unwrap_or_else(|| panic!("ration consumer {actor_id:?} has no campaign status"));
        let remaining = {
            let pc_desc = self
                .mission_domain
                .campaign
                .characters
                .get_mut(status_idx)
                .unwrap_or_else(|| {
                    panic!("ration consumer {actor_id:?} campaign index {status_idx} is missing")
                });
            let removed = pc_desc.status.decrease_ammo(action, 1);
            assert_eq!(
                removed, 1,
                "ration consumer {actor_id:?} completed Eat without available ammo"
            );
            pc_desc.status.get_ammo(action)
        };
        if remaining == 0 {
            self.disable_pc_action(assets, actor_id, action);
        } else {
            // `SetAmmoAmount` also re-enables a non-empty slot. This matters
            // for a restored or temporarily reconciled status whose widget
            // mask was stale when the eating animation completed.
            self.enable_pc_action(assets, actor_id, action);
        }
    }

    /// Spawn an apple / stone projectile at the end of the throw
    /// animation.  Take the thrower's hand point and the victim's eyes
    /// point (or FX-target centre), compute a ballistic trajectory, and
    /// register the projectile.
    fn on_throw_projectile_done(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        target: Option<EntityId>,
        action: crate::profiles::Action,
        object_type: crate::element::ObjectType,
    ) {
        let target_id = target.expect("apple/stone throw selected without its required target");
        let (throw_pos, layer) = self.projectile_throw_origin(actor_id, "on_throw_projectile_done");
        // Lead the victim's forecasted motion only when it's an NPC
        // (Soldier/Civilian); FX targets and fellow-PC victims fall
        // through to the centre branch with no movement lead.
        let (target_pos, target_forecasted_movement) = match self.get_entity(target_id) {
            Some(e) => {
                if e.is_human() {
                    let pos = e.compute_eyes_point(None).unwrap_or_else(|| {
                        panic!("projectile human target {target_id:?} missing eyes hotspot")
                    });
                    let movement = if e.is_npc() {
                        Some(e.position_iface().get_forecasted_movement())
                    } else {
                        None
                    };
                    (pos, movement)
                } else if e.is_fx_target() {
                    let pos = e.compute_target_center().unwrap_or_else(|| {
                        panic!("projectile FX target {target_id:?} missing center hotspot")
                    });
                    (pos, None)
                } else {
                    panic!(
                        "projectile target {target_id:?} has unsupported kind {:?}",
                        e.kind()
                    );
                }
            }
            None => panic!("projectile target {target_id:?} disappeared before Done"),
        };
        let obstacle_check = crate::bow_shot::TrajectoryObstacleCheck {
            fast_find_grid: &self.world.fast_grid,
            layer,
            sight_obstacles: self.sight_obstacles(assets),
            water_zones: Some(&assets.water_zones),
        };
        let projectile = match object_type {
            crate::element::ObjectType::Apple => crate::bow_shot::spawn_apple(
                actor_id,
                throw_pos,
                target_pos,
                Some(target_id),
                target_forecasted_movement,
                layer,
                Some(&obstacle_check),
            ),
            crate::element::ObjectType::Stone => crate::bow_shot::spawn_stone(
                actor_id,
                throw_pos,
                target_pos,
                Some(target_id),
                target_forecasted_movement,
                layer,
                Some(&obstacle_check),
            ),
            _ => return,
        };
        let proj_id = self.add_entity(projectile);
        // Hydrate the accessory sprite (apple/stone) on demand.
        self.attach_accessory_sprite(assets, proj_id);
        tracing::debug!(
            actor = ?actor_id,
            target = ?target_id,
            ?action,
            ?object_type,
            "Throw projectile spawned"
        );
        self.decrement_ability_ammo(assets, actor_id, action);
    }

    fn projectile_throw_origin(
        &self,
        actor_id: EntityId,
        context: &'static str,
    ) -> (crate::coordinates::WorldPoint3D, u16) {
        let entity = self
            .get_entity(actor_id)
            .unwrap_or_else(|| panic!("{context}: projectile throw actor {actor_id:?} missing"));
        let hand = entity.compute_hand_point(None).unwrap_or_else(|| {
            panic!("{context}: projectile throw actor {actor_id:?} missing hand hotspot")
        });
        (hand, entity.element_data().layer())
    }

    /// Disable a PC action slot and deselect if it's the current action.
    ///
    ///   1. if `current_action == action`, set `current_action = NoAction`
    ///      (note this is unconditional `NoAction`, not first-available;
    ///      the HUD slot clears and the user must manually re-pick).
    ///   2. if `saved_action == action`, set `saved_action = NoAction`.
    ///   3. set `disabled_actions[idx] = true`.
    ///
    /// No widget messaging side-effect — the HUD reads `disabled_actions`
    /// directly each frame.
    pub(super) fn disable_pc_action(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        action: crate::profiles::Action,
    ) {
        let action_idx = self.pc_action_slot(assets, pc_id, action);
        if let Some(entity) = self.get_entity_mut(pc_id)
            && let Some(pc) = entity.pc_data_mut()
        {
            // Deselect if this was the current action.
            if pc.current_action == action {
                pc.current_action = crate::profiles::Action::NoAction;
            }
            // Clear `saved_action` if it matched, so a later ctrl-release
            // / EnableAllActionsTemp restore can't bring back a
            // now-disabled slot.
            if pc.saved_action == action {
                pc.saved_action = crate::profiles::Action::NoAction;
            }
            if let Some(action_idx) = action_idx
                && action_idx < pc.disabled_actions.len()
            {
                pc.disabled_actions[action_idx] = true;
            }
            tracing::trace!(
                pc = ?pc_id,
                ?action,
                "Action disabled"
            );
        }
    }

    /// Enable a PC action slot, respecting temp-disables.
    ///
    ///   1. unconditionally clear `disabled_actions[idx]`.
    ///   2. only emit the widget-enable side-effect when
    ///      `disabled_actions_temp[idx] == false`.
    ///
    /// No widget messaging because the HUD reads `disabled_actions` /
    /// `disabled_actions_temp` directly each frame, but the
    /// unconditional permanent-mask clear is load-bearing — without it,
    /// a slot left both perm-disabled and temp-disabled would stay
    /// perm-disabled after the temp mask later clears, leaving the
    /// action permanently unavailable.
    pub(super) fn enable_pc_action(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        action: crate::profiles::Action,
    ) {
        let action_idx = self.pc_action_slot(assets, pc_id, action);
        if let Some(entity) = self.get_entity_mut(pc_id)
            && let Some(pc) = entity.pc_data_mut()
            && let Some(action_idx) = action_idx
            && action_idx < pc.disabled_actions.len()
        {
            // Unconditional clear, BEFORE the temp-disable gate (which
            // only guards the widget side-effect).
            pc.disabled_actions[action_idx] = false;
            tracing::debug!(
                pc = ?pc_id,
                ?action,
                "Action re-enabled"
            );
        }
    }

    fn pc_action_slot(
        &self,
        assets: &LevelAssets,
        pc_id: EntityId,
        action: crate::profiles::Action,
    ) -> Option<usize> {
        let profile_idx = self
            .get_entity(pc_id)
            .and_then(|e| e.pc_data())
            .map(|pc| pc.profile_index)?;
        let profile = assets.profile_manager.get_character(profile_idx)?;
        crate::inventory::find_action_slot(profile, action)
    }

    /// Per-tick refresh of the Purse-action disable flag based on
    /// campaign ransom and each PC's purse ammo.
    ///
    /// The Purse button is disabled when either the PC's
    /// `num_purses == 0` or the campaign's ransom drops below
    /// `COINS_PER_PURSE * COIN_VALUE`, and re-enables when both pass.
    /// We piggyback on the per-tick sweep instead of hooking every
    /// ransom mutation.
    pub(super) fn tick_refresh_purse_disable(&mut self, assets: &LevelAssets) {
        use crate::profiles::Action;
        let ransom = Some(&self.mission_domain.campaign)
            .map(|c| c.get_value(crate::campaign::CampaignValue::Ransom))
            .unwrap_or(0);
        let threshold =
            crate::inventory::COINS_PER_PURSE as i32 * crate::inventory::COIN_VALUE as i32;
        let ransom_ok = ransom >= threshold;
        let pcs: Vec<EntityId> = self.world.entities.pcs().map(|(id, _)| id.into()).collect();
        for pc_id in pcs {
            // Only PCs that have the Purse action in their profile
            // participate in the gate — Robin/Stuteley don't have Purse
            // at all, and their slot array should stay untouched.
            let has_purse = self
                .get_entity(pc_id)
                .and_then(|e| match e {
                    Entity::Pc(pc) => {
                        let idx = usize::from(pc.pc.profile_index);
                        assets.profile_manager.characters.get(idx)
                    }
                    _ => None,
                })
                .map(|profile| profile.actions.contains(&Action::Purse))
                .unwrap_or(false);
            if !has_purse {
                continue;
            }
            // Disable if `num_purses == 0` OR ransom below threshold;
            // enable otherwise.  Purse ammo lives on the selected PC's
            // campaign status block, matching the C++ mpStatus pointer.
            let num_purses = self
                .get_entity(pc_id)
                .and_then(|e| match e {
                    Entity::Pc(pc) => self.pc_description_for_pc_data(&pc.pc),
                    _ => None,
                })
                .map(|desc| desc.status.get_ammo(Action::Purse))
                .unwrap_or(0);
            if num_purses == 0 || !ransom_ok {
                self.disable_pc_action(assets, pc_id, Action::Purse);
            } else {
                self.enable_pc_action(assets, pc_id, Action::Purse);
            }
        }
    }

    /// Increase ammo for a PC and re-enable the action if it was disabled.
    ///
    /// After adding ammo, if the new count is > 0, the action slot is
    /// re-enabled.  This is the counterpart of `decrement_bow_ammo` /
    /// `decrement_ability_ammo` which disable the slot when ammo reaches
    /// 0.
    pub fn increase_ammo_and_enable(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        action: crate::profiles::Action,
        amount: u16,
    ) {
        let (profile_idx, status_idx) = match self.get_entity(pc_id) {
            Some(Entity::Pc(pc)) => (
                pc.pc.profile_index,
                self.pc_description_index_for_pc_data(&pc.pc),
            ),
            None => return,
            _ => return,
        };
        let Some(status_idx) = status_idx else { return };

        // Look up the profile to get max ammo for clamping.
        let max_ammo = assets
            .profile_manager
            .characters
            .get(usize::from(profile_idx))
            .map(|cp| {
                let difficulty = self.control.sim_config.difficulty;
                crate::inventory::max_ammo_for_action(cp, action, difficulty)
            })
            .unwrap_or(u16::MAX);

        let new_ammo = if let Some(campaign) = Some(&mut self.mission_domain.campaign) {
            if let Some(pc_desc) = campaign.characters.get_mut(status_idx) {
                let added = pc_desc.status.increase_ammo(action, amount, max_ammo);
                let new_count = pc_desc.status.get_ammo(action);
                tracing::debug!(
                    pc = ?pc_id,
                    ?action,
                    added,
                    new_count,
                    "Ammo increased"
                );
                new_count
            } else {
                return;
            }
        } else {
            return;
        };

        // If ammo > 0, re-enable the action.
        if new_ammo > 0 {
            self.enable_pc_action(assets, pc_id, action);
        }
    }

    /// Handle a PC picking up a bonus item (arrows, plants, food, etc.).
    ///
    /// Increases ammo, re-enables the action if it was disabled, and
    /// returns the full [`PickupResult`] so callers can implement the
    /// three-way split (full pickup → remove / partial pickup → leave
    /// in world with reduced quantity / nothing taken → leave alone).
    pub fn handle_bonus_pickup(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        action: crate::profiles::Action,
        quantity: u16,
    ) -> Option<crate::inventory::PickupResult> {
        let (profile_idx, status_idx) = match self.get_entity(pc_id) {
            Some(Entity::Pc(pc)) => (
                pc.pc.profile_index,
                self.pc_description_index_for_pc_data(&pc.pc)?,
            ),
            _ => return None,
        };

        let profile = assets
            .profile_manager
            .characters
            .get(usize::from(profile_idx))
            .cloned()?;

        let difficulty = self.control.sim_config.difficulty;

        // Use the pure-function pickup logic from inventory module.
        let result = if let Some(campaign) = Some(&mut self.mission_domain.campaign) {
            if let Some(pc_desc) = campaign.characters.get_mut(status_idx) {
                crate::inventory::take_object(
                    &mut pc_desc.status,
                    &profile,
                    difficulty,
                    action,
                    quantity,
                )
            } else {
                None
            }
        } else {
            None
        };

        let result = result?;

        if result.taken > 0 {
            self.enable_pc_action(assets, pc_id, action);
        }

        Some(result)
    }

    /// Apply the take-object completion for a PC picking up an object.
    /// Handles every `ObjectType` branch — amulet, purse, coin, ransom,
    /// relics, and the default ammo-bonus fall-through.
    ///
    /// Called by the `Command::Take` DONE handler in [`super::tick`],
    /// after the explicit seek-and-take sequence finishes its `Taking`
    /// animation.
    ///
    /// When the take is fully consumed the object is deactivated;
    /// otherwise it stays in world with `taken = true` set.  Returns
    /// `true` iff the PC consumed the object (inventory-full ammo
    /// bonuses return `false` so the caller can skip the taken-flip).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_pc_take_object(
        &mut self,
        assets: &LevelAssets,
        pc_id: EntityId,
        bonus_id: EntityId,
        obj_type: crate::element::ObjectType,
        assoc_action: crate::profiles::Action,
        quantity: u16,
        bx: f32,
        by: f32,
        blayer: u16,
    ) -> bool {
        use crate::element::ObjectType;
        let pos = crate::coordinates::WorldPoint3D {
            x: bx,
            y: by,
            z: 2.0,
        };
        let mut remove = false;
        let mut consumed = true;

        match obj_type {
            // ── Amulet (clover): adds to amulet pool, no counter titbit ──
            ObjectType::BonusAmulet => {
                if let Some(c) = Some(&mut self.mission_domain.campaign) {
                    c.add_value(crate::campaign::CampaignValue::Amulets, quantity as i32);
                }
                remove = true;
            }

            // ── Purse: COINS_PER_PURSE * COIN_VALUE to ransom + counter ──
            // For a fresh world purse we always credit the full value.
            ObjectType::Purse => {
                let value = crate::inventory::COINS_PER_PURSE as u32 * crate::inventory::COIN_VALUE;
                self.add_campaign_value(crate::campaign::CampaignValue::Ransom, value as i32);
                self.spawn_take_counter(pos, blayer, value as u16);
                remove = true;
            }

            // ── Coin: VALUE_COIN to ransom + counter ──
            //
            // Walking near any coin from a burst takes every still-active
            // sibling coin from the source purse in one call.  Loose
            // coins (no source purse) take individually.
            ObjectType::Coin => {
                let source_purse = self.get_entity(bonus_id).and_then(|e| match e {
                    Entity::Projectile(p) => p.projectile.purse.source_purse,
                    _ => None,
                });
                let value = if let Some(purse_id) = source_purse {
                    // `take_purse` deactivates the picked-up coin
                    // along with every active sibling and returns
                    // the cumulative ransom value.
                    self.take_purse(purse_id)
                } else {
                    crate::inventory::COIN_VALUE
                };
                self.add_campaign_value(crate::campaign::CampaignValue::Ransom, value as i32);
                self.spawn_take_counter(pos, blayer, value as u16);
                remove = true;
            }

            // ── Ransom bonus (gold bag): quantity -> ransom + score + counter ──
            ObjectType::BonusRansom => {
                const SCORE_STOLEN_MONEY_HUNDRED: i32 = 10;
                self.add_campaign_value(crate::campaign::CampaignValue::Ransom, quantity as i32);
                self.add_campaign_value(
                    crate::campaign::CampaignValue::Score,
                    SCORE_STOLEN_MONEY_HUNDRED * (quantity as i32) / 100,
                );
                self.spawn_take_counter(pos, blayer, quantity);
                // HERO_GET_MONEY speech cue.
                self.hero_speaking(assets, pc_id, crate::engine::melee::HERO_GET_MONEY);
                remove = true;
            }

            // ── Relics: added to collection + fixed score ──
            ObjectType::BonusAmpulla
            | ObjectType::BonusCoronationSpoon
            | ObjectType::BonusRichardsCrown
            | ObjectType::BonusRoyalSeal
            | ObjectType::BonusRoyalSceptre
            | ObjectType::BonusDomesdayBook
            | ObjectType::BonusSwordOfTheState => {
                const SCORE_COLLECTED_RELIC: i32 = 1000;
                if let Some(c) = Some(&mut self.mission_domain.campaign) {
                    c.add_relic(relic_object_type_index(obj_type));
                }
                self.add_campaign_value(
                    crate::campaign::CampaignValue::Score,
                    SCORE_COLLECTED_RELIC,
                );
                remove = true;
            }

            // ── Default: ammo bonus (arrows, plants, food, stones, …) ──
            _ => {
                if assoc_action == crate::profiles::Action::NoAction {
                    // Unhandled pickup type — leave it in world.
                    return false;
                }
                match self.handle_bonus_pickup(assets, pc_id, assoc_action, quantity) {
                    None => {
                        consumed = false;
                    }
                    Some(result) if result.taken == 0 => {
                        consumed = false;
                    }
                    Some(result) if result.remove_from_world => {
                        remove = true;
                    }
                    Some(result) => {
                        // Partial pickup — write the residual quantity
                        // back to the world bonus and leave it active.
                        match self.world.entities.get_mut(bonus_id) {
                            Some(Entity::Bonus(b)) => {
                                b.object.quantity = result.remainder;
                            }
                            Some(Entity::Projectile(p)) => {
                                p.object.quantity = result.remainder;
                            }
                            _ => {}
                        }
                        consumed = false;
                    }
                }
            }
        }

        if consumed {
            // Note: burst-coin pickups already routed through
            // `take_purse` above, which deactivates this coin and
            // every active sibling and clears the purse's child
            // list.  The match below is therefore a no-op for
            // those (active already false), but it still flips
            // `taken` for non-purse projectile pickups (e.g. loose
            // coins or non-burst purses).
            match self.world.entities.get_mut(bonus_id) {
                Some(Entity::Bonus(bonus)) => {
                    bonus.object.taken = true;
                    if remove {
                        bonus.element.active = false;
                    }
                }
                Some(Entity::Projectile(proj)) => {
                    proj.object.taken = true;
                    if remove {
                        proj.element.active = false;
                    }
                }
                _ => {}
            }
            tracing::debug!(?pc_id, ?bonus_id, ?obj_type, "PC took object");
        }

        consumed
    }

    /// Spawn a floating `+N` counter titbit at `pos` / `layer` (no
    /// element supplier — stays at creation point and rises).
    pub(super) fn spawn_take_counter(
        &mut self,
        pos: crate::coordinates::WorldPoint3D,
        layer: u16,
        value: u16,
    ) {
        if value == 0 {
            return;
        }
        self.feedback.titbit_manager.add_titbit(
            pos,
            layer,
            crate::titbit::TitbitKind::Counter,
            crate::titbit::ElementHandle::INVALID,
            value,
            crate::titbit::ElementHandle::INVALID,
            false,
            crate::titbit::INVALID_ID,
            true,
            Some(pos.y),
            Some(layer),
        );
    }

    /// Compute a walkable drop position near the PC's hand.
    ///
    /// Computes the hand point, offsets the PC's `MoveBox` by the hand
    /// xy, snaps to a walkable cell via `find_authorized_position_toward`,
    /// and returns the resulting box centre.  Returns `None` when no
    /// walkable cell exists near the hand (e.g. against a wall), which
    /// causes the drop sequence to be refused.
    pub fn try_get_drop_position(&self, entity_id: crate::element::EntityId) -> Option<MapPoint> {
        let entity = self.get_entity(entity_id)?;
        let hand = entity.compute_hand_point(None)?;
        let move_box = *entity.position_iface().get_move_box();
        if !move_box.is_somewhere() {
            return None;
        }
        let layer = entity.element_data().layer();
        let hand_xy = crate::coordinates::MapPoint::new(hand.x, hand.y);
        let mut bbox = move_box.translated(hand_xy);
        if self
            .world
            .fast_grid
            .find_authorized_position_toward(&mut bbox, hand_xy, layer)
        {
            Some(bbox.center())
        } else {
            None
        }
    }
}

fn soldier_piercing_protection(
    profile_manager: &crate::profiles::ProfileManager,
    profile_index: crate::profiles::SoldierProfileIdx,
) -> Option<u16> {
    profile_manager
        .get_soldier(profile_index)
        .and_then(|p| profile_manager.get_hth_weapon(p.hth_weapon_id))
        .map(|w| w.piercing_protection)
}

fn soldier_shield_dimensions(
    profile_manager: &crate::profiles::ProfileManager,
    profile_index: crate::profiles::SoldierProfileIdx,
) -> Option<(u16, u16)> {
    profile_manager
        .get_soldier(profile_index)
        .and_then(|p| profile_manager.get_hth_weapon(p.hth_weapon_id))
        .map(|w| (w.shield_width, w.shield_height))
}

fn projectile_trajectory_origin(entity: &Entity) -> Option<crate::coordinates::MapPoint> {
    match entity {
        Entity::Projectile(p) => Some(crate::coordinates::MapPoint {
            x: p.projectile.start_of_trajectory_x,
            y: p.projectile.start_of_trajectory_y,
        }),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{soldier_piercing_protection, soldier_shield_dimensions};
    use crate::element::{
        ActionState, ActorData, ActorPc, ElementData, ElementKind, Entity, HumanData, Posture,
    };
    use crate::engine::{EngineInner, LevelAssets};
    use crate::order::OrderType;
    use crate::profiles::{HtHWeaponProfile, ProfileManager, SoldierProfile, SoldierProfileIdx};
    use crate::sequence::{SequenceElementData, SequenceState};
    use crate::sight_obstacle::{ObstaclePoint, SightObstacle};
    use std::sync::Arc;

    fn make_pc(posture: Posture) -> Entity {
        Entity::Pc(ActorPc {
            element: ElementData {
                kind: ElementKind::ActorPc,
                posture,
                ..Default::default()
            },
            actor: ActorData {
                action_state: ActionState::Waiting,
                ..Default::default()
            },
            human: HumanData::default(),
            pc: Default::default(),
        })
    }

    fn blocked_shoulder_pair() -> (
        EngineInner,
        LevelAssets,
        crate::element::EntityId,
        crate::element::EntityId,
    ) {
        let mut engine = EngineInner::new();
        let victim_id = engine.add_entity(make_pc(Posture::OnShoulders));
        let mut carrier = make_pc(Posture::CarryingOnShoulders);
        let Entity::Pc(carrier_pc) = &mut carrier else {
            unreachable!()
        };
        carrier_pc.pc.carried = Some(victim_id);
        let carrier_id = engine.add_entity(carrier);
        engine
            .get_entity_mut(victim_id)
            .unwrap()
            .human_data_mut()
            .unwrap()
            .carrier = Some(carrier_id);

        // A flat solid slab from z=60 through z=70 intersects the exact
        // CanCarryOnShoulders vertical segment (z=50..90) at the default
        // actor position (0, 0).
        let mut ceiling = SightObstacle::new_default(0);
        ceiling.obstacle_points = vec![
            ObstaclePoint {
                x: -10.0,
                y: -10.0,
                z_top: 70.0,
                z_bottom: 60.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: -10.0,
                z_top: 70.0,
                z_bottom: 60.0,
            },
            ObstaclePoint {
                x: 10.0,
                y: 10.0,
                z_top: 70.0,
                z_bottom: 60.0,
            },
            ObstaclePoint {
                x: -10.0,
                y: 10.0,
                z_top: 70.0,
                z_bottom: 60.0,
            },
        ];
        ceiling.top_plane_points = [
            [-10.0, -10.0, 70.0],
            [10.0, -10.0, 70.0],
            [-10.0, 10.0, 70.0],
        ];
        ceiling.bottom_plane_points = [
            [-10.0, -10.0, 60.0],
            [10.0, -10.0, 60.0],
            [-10.0, 10.0, 60.0],
        ];
        ceiling.rebuild_geometry();

        let mut assets = LevelAssets::new();
        assets.static_sight_obstacles = Arc::new(vec![ceiling]);
        (engine, assets, carrier_id, victim_id)
    }

    fn shoulder_drop_elements(engine: &EngineInner) -> Vec<&crate::sequence::SequenceElement> {
        engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .filter(|element| element.command == crate::element::Command::ReceiveDamage)
            .collect()
    }

    #[test]
    fn stone_soldier_protection_requires_real_weapon_profile() {
        let mut profiles = ProfileManager::new();
        profiles.soldiers.push(SoldierProfile {
            hth_weapon_id: 1,
            ..SoldierProfile::default()
        });

        assert_eq!(
            soldier_piercing_protection(&profiles, SoldierProfileIdx(0)),
            None
        );

        profiles.hth_weapons.push(HtHWeaponProfile {
            piercing_protection: 35,
            ..HtHWeaponProfile::default()
        });

        assert_eq!(
            soldier_piercing_protection(&profiles, SoldierProfileIdx(0)),
            Some(35)
        );
    }

    #[test]
    fn soldier_shield_dimensions_require_real_weapon_profile() {
        let mut profiles = ProfileManager::new();
        profiles.soldiers.push(SoldierProfile {
            hth_weapon_id: 1,
            ..SoldierProfile::default()
        });

        assert_eq!(
            soldier_shield_dimensions(&profiles, SoldierProfileIdx(0)),
            None
        );

        profiles.hth_weapons.push(HtHWeaponProfile {
            shield_width: 22,
            shield_height: 44,
            ..HtHWeaponProfile::default()
        });

        assert_eq!(
            soldier_shield_dimensions(&profiles, SoldierProfileIdx(0)),
            Some((22, 44))
        );
    }

    #[test]
    fn carrying_posture_waiting_action_does_not_run_ceiling_check() {
        let (mut engine, assets, carrier_id, _) = blocked_shoulder_pair();

        engine.tick_shouldered_carry_ceiling(
            &assets,
            &[(carrier_id, OrderType::WaitingCarryingOnShoulders)],
        );

        assert!(shoulder_drop_elements(&engine).is_empty());
    }

    #[test]
    fn walking_carry_action_launches_drop_on_that_action_frame() {
        let (mut engine, assets, carrier_id, victim_id) = blocked_shoulder_pair();
        assert!(shoulder_drop_elements(&engine).is_empty());

        engine.tick_shouldered_carry_ceiling(
            &assets,
            &[(carrier_id, OrderType::WalkingCarryingOnShoulders)],
        );

        let drops = shoulder_drop_elements(&engine);
        assert_eq!(drops.len(), 1);
        let drop = drops[0];
        assert_eq!(drop.owner, Some(victim_id));
        assert_eq!(drop.state, SequenceState::Todo);
        assert!(matches!(
            drop.data,
            SequenceElementData::Damage {
                origin: Some(origin),
                projectile: None,
                damage: 0,
                concussion: 0,
                sword_strike: None,
                sword_profile_idx: None,
                is_harder_hit: false,
            } if origin == victim_id
        ));
    }

    #[test]
    fn projectile_damage_waits_for_sequence_manager_dispatch() {
        let mut engine = EngineInner::new();
        let shooter = engine.add_entity(make_pc(Posture::Upright));
        let mut victim = make_pc(Posture::Upright);
        let Entity::Pc(victim_pc) = &mut victim else {
            unreachable!()
        };
        victim_pc.pc.life_points = 100;
        let victim = engine.add_entity(victim);

        engine.queue_projectile_damage(
            victim,
            shooter,
            crate::element::Command::ReceiveArrowDamage,
            40,
            40,
            Some(shooter),
        );

        assert_eq!(
            engine
                .get_entity(victim)
                .and_then(|entity| entity.pc_data())
                .map(|pc| pc.life_points),
            Some(100),
            "projectile collision must not apply damage before SequenceManager::Hourglass"
        );
        let damage = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| sequence.elements.iter())
            .find(|element| {
                element.owner == Some(victim)
                    && element.command == crate::element::Command::ReceiveArrowDamage
            })
            .expect("queued arrow damage element");
        assert_eq!(damage.state, SequenceState::Todo);
        assert!(matches!(
            damage.data,
            SequenceElementData::Damage {
                origin: Some(origin),
                projectile: Some(projectile),
                damage: 40,
                concussion: 40,
                ..
            } if origin == shooter && projectile == shooter
        ));
    }
}

/// Index used by relic-collection bookkeeping — the BonusType ordinal
/// for each relic.
fn relic_object_type_index(obj: crate::element::ObjectType) -> u32 {
    use crate::element::ObjectType as O;
    match obj {
        O::BonusAmpulla => 12,
        O::BonusCoronationSpoon => 13,
        O::BonusRichardsCrown => 14,
        O::BonusRoyalSeal => 15,
        O::BonusRoyalSceptre => 16,
        O::BonusDomesdayBook => 17,
        O::BonusSwordOfTheState => 18,
        _ => panic!("relic_object_type_index: not a relic: {obj:?}"),
    }
}

// Re-open the impl block for any methods that follow.
impl EngineInner {
    /// Award bow kill experience points to a PC shooter.
    ///
    /// Awards `BOW_KILL_EXPERIENCE_POINTS` to the shooter's Bow skill
    /// via the campaign's `PcStatus`.
    pub(super) fn award_bow_kill_xp(&mut self, shooter_id: EntityId) {
        let profile_idx = self.get_entity(shooter_id).and_then(|e| match e {
            Entity::Pc(pc) => Some(pc.pc.profile_index),
            _ => None,
        });
        let profile_idx = match profile_idx {
            Some(idx) => idx,
            None => return, // Only PCs get XP
        };

        if let Some(campaign) = Some(&mut self.mission_domain.campaign) {
            // The PC AddExperience path also awards a
            // `PC_ADDITIONAL_CAPACITY_POINTS` campaign-score bonus
            // whenever the call crosses a 100-XP boundary.
            campaign.add_pc_experience(
                usize::from(profile_idx),
                crate::pc_status::SkillName::Bow,
                bow_shot::BOW_KILL_EXPERIENCE_POINTS,
            );
            tracing::debug!(
                shooter = ?shooter_id,
                xp = bow_shot::BOW_KILL_EXPERIENCE_POINTS,
                "Bow kill XP awarded"
            );
        }
    }

    /// Advance one pre-existing projectile at its creation-order position in
    /// the virtual entity pass.
    pub(super) fn tick_existing_projectile(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        projectile_id: EntityId,
    ) {
        self.update_shield_obstacles(assets);
        let sight_obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let results = bow_shot::tick_existing_projectile(
            sim,
            &mut self.world.entities,
            sight_obstacles,
            projectile_id,
        );
        self.process_projectile_tick_results(sim, assets, results);
    }

    fn tick_new_projectile_once(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        arrow_id: EntityId,
    ) {
        self.update_shield_obstacles(assets);
        let sight_obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.world.dynamic_sight_obstacles,
            static_active: &self.world.static_sight_obstacle_active,
        };
        let results =
            bow_shot::tick_arrow(sim, &mut self.world.entities, sight_obstacles, arrow_id);
        self.process_projectile_tick_results(sim, assets, results);
    }

    fn process_projectile_tick_results(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        results: Vec<bow_shot::ArrowTickResult>,
    ) {
        for result in results {
            // ── Shield hit — trigger parry ───────────────────────
            // Runs for every projectile type.  The per-type impact FX
            // and the ParryShield sequence launch both fire at the
            // shield holder's map position.
            //
            // Arrow: impact FX is suppressed because the falling-state
            // transition runs before the impact sound check, and the
            // sound gate excludes already-falling projectiles. So
            // arrow shield hits leave `impact_fx = None`.
            //
            // Apple (509) and stone (508): impact_fx populated, played
            // at the holder's map position.
            if let Some(holder) = result.shield_hit {
                tracing::debug!(
                    arrow = ?result.arrow,
                    shield_holder = ?holder,
                    "Projectile blocked by shield"
                );
                if let Some(fx_id) = result.impact_fx
                    && let Some(entity) = self.get_entity(holder)
                {
                    let p = entity.element_data().position_map();
                    self.feedback
                        .pending_side_effects
                        .sounds
                        .push(super::SoundCommand::Fx {
                            fx_id,
                            position: p,
                            material: None,
                        });
                }
                // Trigger parry-shield animation if not already parrying.
                // The gate is on the current combat_anim order type, not
                // the action state (they can diverge by a frame).
                let already_parrying = self
                    .orders
                    .sequence_manager
                    .current_order_for_actor(holder)
                    .map(|(_, _, o)| o.order_type == crate::order::OrderType::ParryingShield)
                    .unwrap_or(false);
                if !already_parrying {
                    let seq_elem = crate::sequence::SequenceElement::new(
                        1,
                        Command::ParryShield,
                        Some(holder),
                    );
                    self.launch_element(seq_elem);
                }
                if result.despawn {
                    self.deactivate_projectile_tombstone(result.arrow);
                }
                continue;
            }

            // FX-target hit — launch the projectile's activation command
            // (ActivateArrow / ActivateApple) as an interaction element
            // on the target with the shooter as antagonist. `tick_arrows`
            // selects the command based on the projectile's object type.
            if let Some((target_id, activation_cmd)) = result.fx_target_hit {
                let shooter = self.get_entity(result.arrow).and_then(|e| match e {
                    Entity::Projectile(p) => p.projectile.shooter,
                    _ => None,
                });
                let mut seq_elem =
                    crate::sequence::SequenceElement::new(1, activation_cmd, Some(target_id));
                seq_elem.data = crate::sequence::SequenceElementData::Interaction {
                    antagonist: shooter,
                };
                self.launch_element(seq_elem);
                tracing::debug!(
                    projectile = ?result.arrow,
                    target = ?target_id,
                    ?shooter,
                    command = ?activation_cmd,
                    "FX target activated by projectile"
                );
                if let Some(fx_id) = result.impact_fx {
                    self.feedback
                        .pending_side_effects
                        .sounds
                        .push(super::SoundCommand::Fx {
                            fx_id,
                            position: MapPoint::new(result.impact_pos.x, result.impact_pos.y),
                            material: None,
                        });
                }
                if result.despawn {
                    self.deactivate_projectile_tombstone(result.arrow);
                }
                continue;
            }

            if let Some(victim) = result.hit_target {
                // Identify the shooter and projectile type.
                let Some((shooter, projectile_kind)) =
                    self.get_entity(result.arrow).and_then(|e| match e {
                        Entity::Projectile(p) => Some((p.projectile.shooter, p.object.object_type)),
                        _ => None,
                    })
                else {
                    tracing::warn!(
                        projectile = ?result.arrow,
                        ?victim,
                        "projectile human hit missing projectile entity; skipping hit"
                    );
                    continue;
                };
                let Some(shooter) = shooter else {
                    tracing::warn!(
                        projectile = ?result.arrow,
                        ?victim,
                        ?projectile_kind,
                        "projectile human hit missing shooter; skipping hit"
                    );
                    continue;
                };

                match projectile_kind {
                    crate::element::ObjectType::Apple => {
                        if let Some(old_pos) = result.human_hit_old_position {
                            self.rewind_projectile_to_human_hit_old_position(result.arrow, old_pos);
                        }
                        // No damage; if the victim is a soldier, set
                        // apple-smell and dispatch EventApple.
                        self.on_apple_hit_human(result.arrow, victim);
                    }
                    crate::element::ObjectType::Stone => {
                        if let Some(old_pos) = result.human_hit_old_position {
                            self.rewind_projectile_to_human_hit_old_position(result.arrow, old_pos);
                        }
                        // Non-VIP and (non-soldier OR piercing-protection
                        // roll failed) → piercing damage.  NPCs that
                        // dodge (VIP or protected soldier) trigger an
                        // EventApple stimulus instead.
                        self.on_stone_hit_human(sim, assets, result.arrow, victim, shooter);
                    }
                    _ => {
                        // ── Arrow path (default) — the 3-way classifier
                        // folds in the friendly-fire / shielded-PC
                        // pre-filter.  Each outcome is a distinct
                        // side-effect:
                        //   * `PassThrough`  — arrow keeps flying, no sound.
                        //   * `Ricochet`     — falling state, silent.
                        //   * `Damage`       — launch damage sequence element.
                        match self.classify_arrow_hit(sim, assets, victim, shooter) {
                            ArrowHitOutcome::PassThrough => {
                                // Friendly-fire / VIP-NPC / civilian-protected
                                // / PC-with-shield: arrow sails past.
                                // `tick_arrows` has already flagged the
                                // projectile for despawn; flip it back to
                                // flying and skip the sound / despawn
                                // sections below.
                                if let Some(Entity::Projectile(p)) =
                                    self.world.entities.get_mut(result.arrow)
                                {
                                    p.projectile.flying = true;
                                }
                                continue;
                            }
                            ArrowHitOutcome::Ricochet => {
                                // Piercing-protection deflected.  Arrow
                                // tumbles to the ground.  The
                                // ricochet-falling transition runs
                                // before the impact-sound check, and the
                                // sound gate excludes already-falling
                                // projectiles, so the ricochet impact
                                // sound is intentionally silent.
                                tracing::debug!(
                                    arrow = ?result.arrow,
                                    victim = ?victim,
                                    "Arrow ricocheted from armor"
                                );
                                self.start_arrow_ricochet(sim, result.arrow);
                                continue;
                            }
                            ArrowHitOutcome::Damage => {}
                        }

                        if let Some(old_pos) = result.human_hit_old_position {
                            self.rewind_projectile_to_human_hit_old_position(result.arrow, old_pos);
                        }

                        let damage = result.damage;
                        // Civilian-with-attached-scroll immunity
                        // (scroll-reveal beggar).  Consume the arrow but
                        // don't apply damage.
                        if self.is_scroll_protected_civilian(victim) {
                            tracing::debug!(
                                arrow = ?result.arrow,
                                ?victim,
                                "arrow hit blocked: civilian carrying unrevealed scroll"
                            );
                            continue;
                        }
                        self.queue_projectile_damage(
                            victim,
                            shooter,
                            Command::ReceiveArrowDamage,
                            damage,
                            damage,
                            Some(result.arrow),
                        );
                        tracing::debug!(
                            arrow = ?result.arrow,
                            victim = ?victim,
                            damage,
                            "Arrow damage queued"
                        );

                        // After launching the damage sequence, if the
                        // victim is an NPC, dispatch EventGetArrow at the
                        // arrow's trajectory origin so the surviving
                        // target wakes up and searches toward the shot
                        // origin.
                        let Some(victim_is_npc) = self.get_entity(victim).map(|e| e.is_npc())
                        else {
                            tracing::warn!(
                                ?victim,
                                arrow = ?result.arrow,
                                "arrow hit follow-up skipped: victim missing before EventGetArrow"
                            );
                            continue;
                        };
                        if victim_is_npc {
                            let trajectory_origin = self
                                .get_entity(result.arrow)
                                .and_then(projectile_trajectory_origin);
                            if let Some(origin) = trajectory_origin {
                                self.dispatch_event_get_arrow(sim, assets, victim, origin);
                            } else {
                                tracing::warn!(
                                    arrow = ?result.arrow,
                                    victim = ?victim,
                                    "arrow hit NPC missing trajectory origin; skipping EventGetArrow"
                                );
                            }
                        }
                    }
                }
            }

            // Impact sound: apple 509, stone 508.  The arrow's 510
            // plays only on shield deflection (handled above), so
            // non-shield arrow impacts stay silent.
            if let Some(fx_id) = result.impact_fx {
                self.feedback
                    .pending_side_effects
                    .sounds
                    .push(super::SoundCommand::Fx {
                        fx_id,
                        position: MapPoint::new(result.impact_pos.x, result.impact_pos.y),
                        material: None,
                    });
            }

            if result.despawn && result.hit_target.is_none() {
                self.apply_projectile_landing_resolution(assets, result.arrow);
            }

            // Water/hole splash — arrow landed in a water or hole zone
            // with no victim/shield/target.  Add the plouf titbit,
            // broadcast the PLOUF noise, and play impact sound ID 470.
            if result.despawn && result.hit_target.is_none() {
                self.maybe_splash_on_landing(sim, assets, result.arrow);
            }

            if result.despawn {
                self.deactivate_projectile_tombstone(result.arrow);
            }
        }
    }

    fn deactivate_projectile_tombstone(&mut self, projectile_id: EntityId) {
        let entity = self
            .get_entity_mut(projectile_id)
            .unwrap_or_else(|| panic!("despawning projectile {projectile_id:?} vanished"));
        assert!(
            matches!(entity, Entity::Projectile(_) | Entity::Net(_)),
            "projectile despawn targeted non-projectile {projectile_id:?}"
        );
        if let Entity::Projectile(projectile) = entity
            && projectile.object.object_type == crate::element::ObjectType::Arrow
            && !projectile.projectile.disappear
        {
            // A normal arrow impact/exhaustion ends flight but remains active
            // until RHElementArrow::Refresh has observed its later stationary
            // frame. Hole/water disappearance is immediate.
            projectile.projectile.flying = false;
            return;
        }
        entity.element_data_mut().active = false;
    }

    pub(super) fn rewind_projectile_to_human_hit_old_position(
        &mut self,
        projectile: EntityId,
        old_pos: crate::coordinates::WorldPoint3D,
    ) {
        let Some(Entity::Projectile(p)) = self.world.entities.get_mut(projectile) else {
            tracing::warn!(
                ?projectile,
                "projectile human-hit rewind skipped: projectile entity missing"
            );
            return;
        };
        // Successful RHElementProjectile::HitHuman handling rewinds to the
        // position snapshotted by NewMove, stops flight, and immediately
        // DeleteTrajectory()s.  Settling both position representations is
        // observable by the following parity snapshot and lets the subsequent
        // RHElementArrow::Refresh retire the stationary arrow.
        p.element.set_position(old_pos);
        p.element.sprite.position_iface.new_move();
        p.projectile.trajectory.clear();
    }

    /// Apple lands on a human.  Apples deal no damage; they only
    /// affect soldiers via the apple-smell AI hook.
    fn on_apple_hit_human(&mut self, apple: EntityId, victim: EntityId) {
        // Use the shooter's original position (trajectory origin) as
        // the EventApple stimulus anchor.
        let Some(trajectory_origin) = self
            .get_entity(apple)
            .and_then(projectile_trajectory_origin)
        else {
            tracing::warn!(
                ?apple,
                ?victim,
                "apple hit human missing trajectory origin; skipping EventApple"
            );
            return;
        };
        let Some(victim_is_soldier) = self.get_entity(victim).map(|e| e.is_soldier()) else {
            tracing::warn!(
                ?apple,
                ?victim,
                "apple hit follow-up skipped: victim missing before EventApple"
            );
            return;
        };
        if !victim_is_soldier {
            return;
        }
        self.set_soldier_apple_smell(victim);
        self.dispatch_event_apple(victim, trajectory_origin);
    }

    /// Stone lands on a human.  Non-VIPs that fail the
    /// piercing-protection roll take `STONE_DAMAGE`; NPCs that dodge
    /// (VIP or armored soldier) receive an EventApple stimulus.
    fn on_stone_hit_human(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        stone: EntityId,
        victim: EntityId,
        _shooter: EntityId,
    ) {
        let victim_entity = match self.get_entity(victim) {
            Some(e) => e,
            None => return,
        };
        let is_vip =
            crate::engine::melee::is_vip_from_profile(victim_entity, &assets.profile_manager);
        let is_npc = victim_entity.is_npc();

        // Piercing-protection roll for soldiers only:
        // `(!is_soldier) || (rand() % 100) >= protection`
        let protected = if let Entity::Soldier(s) = victim_entity {
            match soldier_piercing_protection(
                &assets.profile_manager,
                s.soldier.soldier_profile_index,
            ) {
                Some(protection) => {
                    let roll = crate::sim_rng::u32(
                        sim,
                        crate::sim_rng::RngSite::StonePiercingProtection,
                        0..100,
                    );
                    roll < protection as u32
                }
                None => panic!(
                    "stone hit: missing soldier HtH weapon profile for victim={victim:?} profile_index={:?}",
                    s.soldier.soldier_profile_index
                ),
            }
        } else {
            false
        };

        // Civilian-with-attached-scroll immunity.  The scroll-protected
        // check belongs *inside* the damage branch, not on the gate:
        // a scroll-carrying civilian enters the damage branch and the
        // damage is silently cancelled downstream by the civilian's
        // wound handler.  If we gated the branch on `!scroll_protected`,
        // the civilian would fall through to the `else if is_npc` arm
        // and erroneously dispatch EventApple.
        let scroll_protected = self.is_scroll_protected_civilian(victim);

        if !is_vip && !protected {
            if scroll_protected {
                // Damage cancelled, with no EventApple fall-through —
                // the civilian wound handler returns without applying
                // damage.
                tracing::debug!(
                    stone = ?stone,
                    ?victim,
                    "stone hit blocked: civilian carrying unrevealed scroll"
                );
                return;
            }
            self.queue_projectile_damage(
                victim,
                _shooter,
                Command::ReceiveStoneDamage,
                STONE_DAMAGE,
                STONE_CONCUSSION,
                None,
            );
        } else if is_npc {
            // VIP / armored-soldier dodge: treated similarly to an
            // apple hit.
            let Some(trajectory_origin) = self
                .get_entity(stone)
                .and_then(projectile_trajectory_origin)
            else {
                tracing::warn!(
                    ?stone,
                    ?victim,
                    "stone hit NPC missing trajectory origin; skipping EventApple"
                );
                return;
            };
            self.dispatch_event_apple(victim, trajectory_origin);
        }
    }

    /// Set the 1500-frame apple-smell counter on a soldier.  Titbit
    /// creation is driven event-free by `sync_apple_smell_titbits`.
    fn set_soldier_apple_smell(&mut self, victim: EntityId) {
        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(victim) {
            s.soldier.apple_smell = APPLE_SMELL_DURATION;
        }
    }

    /// Per-frame decrement of the apple-smell counter on all soldiers.
    /// The associated titbit is auto-removed by
    /// `sync_apple_smell_titbits` once the counter reaches 0.
    pub(super) fn tick_apple_smell_for(&mut self, soldier_id: EntityId) {
        let Entity::Soldier(soldier) =
            self.world.entities.get_mut(soldier_id).unwrap_or_else(|| {
                panic!(
                    "apple-smell owner {} disappeared from its legacy slot",
                    soldier_id.index()
                )
            })
        else {
            panic!("apple-smell owner {} is not a soldier", soldier_id.index());
        };
        if soldier.soldier.apple_smell > 0 {
            soldier.soldier.apple_smell -= 1;
        }
    }

    /// Per-frame body-direction re-snap for soldiers in reactiontime /
    /// bow substates.  While the soldier is in
    /// `AttackingReactiontimeTurning`, `AttackingReactiontime`,
    /// `AttackingBowLoading`, `AttackingBowAiming`, or
    /// `AttackingBowShooting`, re-orient the body to face the
    /// `primary_target`'s ground position every tick so a bowman keeps
    /// tracking a moving PC between Think stimuli.
    pub(super) fn tick_soldier_track_primary_target_for(&mut self, npc_id: EntityId) {
        use crate::ai::Substate;
        let target_handle = {
            let Some(Entity::Soldier(s)) = self.world.entities.get(npc_id) else {
                panic!("tracking soldier {} disappeared", npc_id.index());
            };
            let Some(ai) = s.npc.ai_brain.base() else {
                return;
            };
            let tracks = matches!(
                ai.current_substate,
                Substate::AttackingReactiontimeTurning
                    | Substate::AttackingReactiontime
                    | Substate::AttackingBowLoading
                    | Substate::AttackingBowAiming
                    | Substate::AttackingBowShooting
            );
            if !tracks || ai.primary_target == 0 {
                return;
            }
            ai.primary_target
        };
        let my_pos = match self.get_entity(npc_id) {
            Some(e) => e.ground_position(),
            None => panic!("tracking soldier {} disappeared", npc_id.index()),
        };
        let target_pos = match self.get_entity(
            self.expect_entity_id_for_index(target_handle, "update_bow_defense target handle"),
        ) {
            Some(e) => e.ground_position(),
            None => return,
        };
        let dx = target_pos.x - my_pos.x;
        let dy = target_pos.y - my_pos.y;
        let sector = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(npc_id) {
            s.element.set_direction_instantly(sector);
        }
    }

    /// Per-frame PC life-point auto-heal.
    ///
    /// * If the PC is immortal and below the max, bump HP by 1
    ///   (snapping up to 75 first if below that floor).
    /// * Otherwise on Easy difficulty, once every `TIME_AUTO_HEAL`
    ///   frames and while the PC is neither sword-fighting nor in
    ///   coma, bump HP by 1.
    ///
    /// The shared human prelude (concussion decrement, tiredness
    /// recovery, produced-noise refresh) is handled by
    /// [`Self::tick_concussion_healing`], [`Self::tick_tiredness`],
    /// and the PC noise bookkeeping in `engine/ai.rs`; this tick only
    /// covers the PC-specific heal branches.
    /// Apply the PC-specific tail of `RHElementActorPC::Hourglass` to one PC.
    pub(super) fn tick_pc_auto_heal_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        pc_id: EntityId,
    ) {
        /// Auto-heal cadence in frames.
        const TIME_AUTO_HEAL: u32 = 100;

        let tick_easy = sim.config().difficulty == crate::player_profile::DifficultyLevel::Easy
            && self.control.frame_counter.is_multiple_of(TIME_AUTO_HEAL);

        let (lp, immortal, swordfighting, in_coma) = {
            let Some(Entity::Pc(pc)) = self.get_entity(pc_id) else {
                return;
            };
            // Fried-psykokwack PCs short-circuit the whole hourglass
            // tick; skip heals too.  Also skip inactive / dead /
            // already-maxed PCs.
            if !pc.element.active
                || pc.pc.fried_psykokwack
                || pc.pc.life_points <= 0
                || pc.pc.life_points >= crate::pc_status::LIFEPOINTS_PC
            {
                return;
            }
            let in_coma = self
                .pc_description_for_pc_data(&pc.pc)
                .map(|d| d.status.in_coma)
                .unwrap_or(false);
            (
                pc.pc.life_points,
                pc.pc.immortal,
                !pc.human.opponents.is_empty(),
                in_coma,
            )
        };

        let new_lp = if immortal {
            // Snap up to a 75 floor before incrementing.
            if lp < 75 { 75 } else { lp + 1 }
        } else if tick_easy && !swordfighting {
            if in_coma {
                return;
            }
            lp + 1
        } else {
            return;
        };
        let new_lp = new_lp.min(crate::pc_status::LIFEPOINTS_PC);

        if let Some(Entity::Pc(pc)) = self.get_entity_mut(pc_id) {
            pc.pc.life_points = new_lp;
        }
    }

    /// Dispatch an EventApple stimulus at the origin of the thrown
    /// projectile.  Used by both apple and stone impacts on NPCs.
    fn dispatch_event_apple(&mut self, victim: EntityId, origin: crate::coordinates::MapPoint) {
        let Some(layer) = self.get_entity(victim).map(|e| e.element_data().layer()) else {
            tracing::warn!(
                ?victim,
                "dispatch_event_apple: victim missing for layer lookup"
            );
            return;
        };
        let pos = crate::ai::Position {
            x: origin.x,
            y: origin.y,
            sector: None,
            level: layer,
        };
        self.dispatch_ai_stimulus(
            victim,
            crate::ai::Stimulus::with_position(crate::ai::StimulusType::EventApple, pos),
        );
    }

    /// Dispatch an EventGetArrow stimulus at the arrow's trajectory
    /// origin — wakes the struck NPC and seeds the search toward the
    /// shot origin.
    fn dispatch_event_get_arrow(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim: EntityId,
        origin: crate::coordinates::MapPoint,
    ) {
        let Some(layer) = self.get_entity(victim).map(|e| e.element_data().layer()) else {
            tracing::warn!(
                ?victim,
                "dispatch_event_get_arrow: victim missing for layer lookup"
            );
            return;
        };
        let pos = crate::ai::Position {
            x: origin.x,
            y: origin.y,
            sector: None,
            level: layer,
        };
        // RHElementArrow::HitHuman calls the NPC's Think directly after
        // LaunchSequenceElement and before returning to the projectile
        // Hourglass.  Merely appending this to the deferred detection FIFO
        // makes the outcome depend on whether the NPC's creation-order slot
        // is before or after the projectile.  Run the one Think inline while
        // retaining older deferred stimuli ahead of work emitted here.
        self.dispatch_synchronous_ai_think_preserving_detection_fifo(
            sim,
            victim,
            assets,
            crate::ai::Stimulus::with_position(crate::ai::StimulusType::EventGetArrow, pos),
        );
    }

    /// If the arrow's landing position is inside a water or hole zone,
    /// spawn the splash titbit, broadcast a PLOUF noise, and play the
    /// plouf impact sound (FX 470).
    fn maybe_splash_on_landing(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        arrow: EntityId,
    ) {
        let proj_entity = match self.get_entity(arrow) {
            Some(e) => e,
            None => return,
        };
        let elem = proj_entity.element_data();
        let position = elem.position();
        let position_map = elem.position_map();
        let layer = elem.layer();
        let (object_type, pre_flagged_disappear) = match proj_entity {
            Entity::Projectile(p) => (p.object.object_type, p.projectile.disappear),
            _ => return,
        };

        // Pre-flagged hole landing: the trajectory builder identified
        // the terminal waypoint as inside a hole polygon.  Skip the
        // water-zone lookup (which can miss when the extended final
        // point sits on the polygon boundary) and drop into the silent
        // hole-disappear branch directly.
        if pre_flagged_disappear {
            return;
        }

        // Water-hole determination has three branches: a standalone
        // sector-sound scan (branch 1) and two obstacle-anchored
        // sub-sector iterations (branches 2 & 3, lakes / holes carved
        // into a roof).  The obstacle is the impact-bounce target.
        //
        // Note: `apply_projectile_landing_resolution` does write
        // `element.set_obstacle_index(...)` before this runs, so we
        // *could* read the projectile's stored obstacle. We
        // deliberately don't, because `resolve_projectile_landing`
        // picks the first projection-area obstacle covering the
        // landing in screen coords, while splash detection wants the
        // topmost obstacle by `compute_top_z`.  For overlapping
        // obstacles — a roof above a water polygon — the topmost rule
        // is the correct one.  If none of the projection-area obstacles
        // cover the landing, fall through to the standalone water-zone
        // scan (branch 1).
        let landing_map = position_map;
        let landing_ground = GroundPoint::new(position.x, position.y);
        let landing_obstacle = self.find_landing_water_obstacle(assets, landing_ground);
        let resolved_material = if let Some(obs) = landing_obstacle {
            crate::water_zones::determine_water_hole_with_obstacle(obs, landing_map)
        } else {
            assets.water_zones.determine_water_hole(landing_map)
        };

        let material = match resolved_material {
            Some(m) => m,
            None => {
                // Dry landing — broadcast a ZONK noise for arrows so
                // nearby NPCs hear the thud.  Apples/stones use their
                // own FX sound instead and don't emit the noise.
                if matches!(object_type, crate::element::ObjectType::Arrow) {
                    self.broadcast_noise_synchronously(
                        sim,
                        assets,
                        crate::ai::NoiseType::Zonk,
                        position_map,
                        layer,
                        crate::parameters_ai::NOISE_VOLUME_ZONK as u16,
                        position.z.max(0.0) as u16,
                        Some(arrow),
                    );
                }
                return;
            }
        };

        // `disappear` fires only for HOLE material; the splash titbit
        // and Plouf sound for water are emitted inline below.  Water
        // doesn't need a stored flag because the side-effects fire in
        // the same tick the landing is detected.
        let is_water = matches!(material, crate::sound_cache::Material::Water);
        if !is_water && let Some(Entity::Projectile(p)) = self.world.entities.get_mut(arrow) {
            p.projectile.disappear = true;
        }

        if !is_water {
            return;
        }

        // Plouf titbit at the landing position.
        use crate::titbit::{ElementHandle, INVALID_ID, TitbitKind};
        self.feedback.titbit_manager.add_titbit(
            crate::coordinates::WorldPoint3D {
                x: position.x,
                y: position.y,
                z: position.z,
            },
            layer,
            TitbitKind::Plouf,
            ElementHandle::INVALID,
            0,
            ElementHandle::INVALID,
            false,
            INVALID_ID,
            true,
            None,
            None,
        );

        // Plouf impact sound (FX 470).
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Fx {
                fx_id: 470,
                position: position_map,
                material: None,
            });

        // Broadcast PLOUF noise so nearby NPCs react. Volume from
        // `parameters_ai::NOISE_VOLUME_PLOUF` (300).
        self.broadcast_noise_synchronously(
            sim,
            assets,
            crate::ai::NoiseType::Plouf,
            position_map,
            layer,
            crate::parameters_ai::NOISE_VOLUME_PLOUF as u16,
            position.z.max(0.0) as u16,
            Some(arrow),
        );
    }

    /// Pick the topmost sight obstacle whose ground polygon contains the
    /// landing impact and whose configuration could yield a water/hole
    /// hit — i.e. either the obstacle's own material is WATER (branch 2)
    /// or it carries a non-empty material sub-sector list (branch 3).
    /// Implements the "highest projection-area" disambiguation.
    ///
    /// Selection-rule note: the projectile carries a stored
    /// `obstacle_index()` by this point (set in
    /// `apply_projectile_landing_resolution`), but that index comes
    /// from `FastFindGrid::resolve_projectile_landing` which picks the
    /// *first* projection-area obstacle covering the landing in screen
    /// coords. Splash detection wants the *topmost* by `compute_top_z`,
    /// matching the projection-area disambiguation, so this scan
    /// recovers the correct obstacle independently rather than reading
    /// `obstacle_index()`. Pre-filtering on
    /// `material == WATER || !material_sectors.is_empty()` keeps the
    /// scan cheap on levels with many non-water obstacles.
    fn find_landing_water_obstacle<'a>(
        &'a self,
        assets: &'a LevelAssets,
        landing: GroundPoint,
    ) -> Option<&'a crate::sight_obstacle::SightObstacle> {
        use crate::geo2d::polygon_contains_point;
        const WATER_MATERIAL_CODE: u8 = 5;
        let obstacles = self.sight_obstacles(assets);
        let mut best: Option<(&crate::sight_obstacle::SightObstacle, f32)> = None;
        for (idx, obs) in obstacles.iter_indexed() {
            if !obstacles.is_active(idx as usize) {
                continue;
            }
            if obs.material != WATER_MATERIAL_CODE && obs.material_sectors.is_empty() {
                continue;
            }
            if !obs.box_ground.contains_point(landing) {
                continue;
            }
            if !polygon_contains_point(obs.polygon.as_geo(), landing.to_geo()) {
                continue;
            }
            let height = obs.compute_top_z(landing.x, landing.y);
            match best {
                None => best = Some((obs, height)),
                Some((_, best_h)) if height > best_h => best = Some((obs, height)),
                _ => {}
            }
        }
        best.map(|(o, _)| o)
    }

    // ─── Shield obstacle update ─────────────────────────────────

    /// Recompute shield obstacles for all actors currently holding a shield.
    ///
    /// Called every frame before `tick_arrows` so the arrow-blocking
    /// geometry is always up-to-date with the actor's current position
    /// and facing direction.
    ///
    /// Shield obstacles are stored in two places:
    /// 1. `ActorData::shield_obstacle` — used by `tick_arrows` for the
    ///    per-arrow directional check + 3D intersection.
    /// 2. `EngineInner::sight_obstacles` (appended after static obstacles) —
    ///    makes shields visible to all systems that query the global
    ///    obstacle list (AI vision filters them out via `is_opaque()`,
    ///    but reachability checks and trajectory checks will see them).
    fn update_shield_obstacles(&mut self, assets: &LevelAssets) {
        use crate::bow_shot::{
            compute_shield_obstacle, shield_params_for_pc, shield_params_for_soldier,
        };

        // Remove previous frame's dynamic shield obstacles.
        self.world.dynamic_sight_obstacles.clear();

        let mut clear_obstacles = Vec::new();
        let mut set_obstacles = Vec::new();
        let mut dynamic_sight_obstacles = Vec::new();

        for (id, entity) in self.world.entities.humans() {
            let id: EntityId = id.into();
            if !entity.is_active() || entity.is_dead() {
                continue;
            }
            let actor = match entity.actor_data() {
                Some(a) => a,
                None => continue,
            };

            if !actor.action_state.is_shield() {
                // Not holding shield — clear any stale obstacle.
                if actor.shield_obstacle.is_some() {
                    clear_obstacles.push(id);
                }
                continue;
            }

            // Compute shield dimensions based on entity type.
            let params = match entity {
                Entity::Pc(pc) => {
                    // Check BigShield via character profile.
                    let has_big_shield = assets
                        .profile_manager
                        .get_character(pc.pc.profile_index)
                        .map(|p| p.has_action(crate::profiles::Action::BigShield))
                        .unwrap_or(false);
                    shield_params_for_pc(has_big_shield)
                }
                Entity::Soldier(s) => {
                    // Look up HtH weapon profile for shield dimensions.
                    let Some((sw, sh)) = soldier_shield_dimensions(
                        &assets.profile_manager,
                        s.soldier.soldier_profile_index,
                    ) else {
                        tracing::warn!(
                            soldier = ?id,
                            profile_index = ?s.soldier.soldier_profile_index,
                            "shield update: missing soldier HtH weapon profile; clearing shield obstacle"
                        );
                        if actor.shield_obstacle.is_some() {
                            clear_obstacles.push(id);
                        }
                        continue;
                    };
                    shield_params_for_soldier(sw, sh)
                }
                _ => continue,
            };

            let elem = entity.element_data();
            let obstacle = compute_shield_obstacle(
                elem.position_map(),
                elem.position().z,
                elem.direction(),
                &params,
            );

            // Store on the entity (for tick_arrows per-arrow directional check)
            // and append to the global obstacle list (for all other systems).
            dynamic_sight_obstacles.push(obstacle.clone());
            set_obstacles.push((id, obstacle));
        }

        self.world.dynamic_sight_obstacles = dynamic_sight_obstacles;

        for id in clear_obstacles {
            if let Some(actor) = self.get_entity_mut(id).and_then(|e| e.actor_data_mut()) {
                actor.shield_obstacle = None;
            }
        }
        for (id, obstacle) in set_obstacles {
            if let Some(actor) = self.get_entity_mut(id).and_then(|e| e.actor_data_mut()) {
                actor.shield_obstacle = Some(obstacle);
            }
        }
    }

    // ─── Hero ability tick ──────────────────────────────────────

    /// Drive one actor's ability and apply its completion effects inline at
    /// that actor's creation-order position.
    #[allow(unused_variables)] // Done results retain owner identity; Terminated consumes it.
    pub(super) fn tick_ability_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        display: &mut super::HostDisplayState,
        assets: &LevelAssets,
        actor_id: EntityId,
    ) {
        let pending_hit_init = self
            .get_entity(actor_id)
            .and_then(Entity::actor_data)
            .and_then(|actor| {
                let ability = &actor.active_ability;
                (ability.kind == Some(crate::movement::AbilityKind::Hit)
                    && actor.execute_order_initialising)
                    .then(|| {
                        (
                            ability
                                .sequence_id
                                .expect("pending Hit initialization lost sequence identity"),
                            ability.element_index,
                            ability
                                .target
                                .expect("pending Hit initialization lost antagonist identity"),
                            ability
                                .order_id
                                .expect("pending Hit initialization lost order identity"),
                        )
                    })
            });
        if let Some((seq_id, elem_idx, victim_id, order_id)) = pending_hit_init {
            let attacker_ground = self
                .get_entity(actor_id)
                .expect("Hit owner vanished during initialization")
                .ground_position();
            let victim_ground = self
                .get_entity(victim_id)
                .unwrap_or_else(|| {
                    panic!("Hit victim {victim_id:?} vanished during initialization")
                })
                .ground_position();
            let facing = crate::position_interface::vector_to_sector_0_to_15(
                victim_ground.x - attacker_ground.x,
                victim_ground.y - attacker_ground.y,
            );
            // Original HITTING initialization changes only the progressive
            // direction goal, before checking whether the interaction remains
            // valid. The later Turn call owns the current-direction change.
            self.get_entity_mut(actor_id)
                .expect("Hit owner vanished before direction initialization")
                .element_data_mut()
                .set_direction_goal(facing);

            let valid = {
                let element = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .unwrap_or_else(|| {
                        panic!("pending Hit owner {actor_id:?} lost element {seq_id:?}/{elem_idx}")
                    });
                assert_eq!(element.owner, Some(actor_id));
                assert_eq!(element.command, crate::element::Command::HitCmd);
                let order = element.current_order().unwrap_or_else(|| {
                    panic!("pending Hit element {seq_id:?}/{elem_idx} lost its selected order")
                });
                assert_eq!(order.order_id, order_id);
                assert_eq!(order.target_actor, Some(victim_id.index()));
                self.check_sequence_element_validity(assets, actor_id, element, true)
            };
            if !valid {
                self.cleanup_aborted_ability(
                    actor_id,
                    crate::movement::AbilityKind::Hit,
                    seq_id,
                    elem_idx,
                    Some(order_id),
                );
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                self.dispatch_condolations_for_owner_boundary(sim, actor_id, assets);
                return;
            }
        }

        let pending_tie_init = self
            .get_entity(actor_id)
            .and_then(Entity::actor_data)
            .and_then(|actor| {
                let ability = &actor.active_ability;
                (ability.kind == Some(crate::movement::AbilityKind::Tie)
                    && actor.execute_order_initialising)
                    .then(|| {
                        (
                            ability
                                .sequence_id
                                .expect("pending Tie initialization lost sequence identity"),
                            ability.element_index,
                            ability
                                .target
                                .expect("pending Tie initialization lost antagonist identity"),
                            ability
                                .order_id
                                .expect("pending Tie initialization lost order identity"),
                        )
                    })
            });
        if let Some((seq_id, elem_idx, target_id, order_id)) = pending_tie_init {
            let valid = {
                let element = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .unwrap_or_else(|| {
                        panic!("pending Tie owner {actor_id:?} lost element {seq_id:?}/{elem_idx}")
                    });
                assert_eq!(element.owner, Some(actor_id));
                assert_eq!(element.command, crate::element::Command::TieCmd);
                let order = element.current_order().unwrap_or_else(|| {
                    panic!("pending Tie element {seq_id:?}/{elem_idx} lost its selected order")
                });
                assert_eq!(order.order_id, order_id);
                assert_eq!(order.target_actor, Some(target_id.index()));
                self.check_sequence_element_validity(assets, actor_id, element, true)
            };
            if !valid {
                self.cleanup_aborted_ability(
                    actor_id,
                    crate::movement::AbilityKind::Tie,
                    seq_id,
                    elem_idx,
                    Some(order_id),
                );
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                self.dispatch_condolations_for_owner_boundary(sim, actor_id, assets);
                return;
            }

            let actor_pos = self
                .get_entity(actor_id)
                .expect("validated Tie owner vanished during initialization")
                .element_data()
                .position_map();
            let target_pos = self
                .get_entity(target_id)
                .expect("validated Tie target vanished during initialization")
                .element_data()
                .position_map();
            let facing = crate::position_interface::vector_to_sector_0_to_15_iso(
                target_pos.x - actor_pos.x,
                target_pos.y - actor_pos.y,
            );
            // RHElementActorPC::Execute(TYING) installs only the progressive
            // direction goal during the order's first Execute. Translation
            // itself must not rotate or stop a moving PC: the Tie can still
            // be interrupted by an earlier manager-FIFO continuation before
            // its animation ever owns an actor slot.
            self.get_entity_mut(actor_id)
                .expect("validated Tie owner vanished before direction initialization")
                .element_data_mut()
                .set_direction_goal(facing);
        }

        let pending_heal_facing = self
            .get_entity(actor_id)
            .and_then(Entity::actor_data)
            .and_then(|actor| {
                let ability = &actor.active_ability;
                (ability.kind == Some(crate::movement::AbilityKind::Heal)
                    && actor.execute_order_initialising)
                    .then_some(ability.target)
                    .flatten()
                    .filter(|target| *target != actor_id)
            });
        if let Some(target_id) = pending_heal_facing {
            let healer_pos = self
                .get_entity(actor_id)
                .expect("Heal owner vanished during initialization")
                .element_data()
                .position_map();
            let target_pos = self
                .get_entity(target_id)
                .unwrap_or_else(|| {
                    panic!("Heal target {target_id:?} vanished during initialization")
                })
                .element_data()
                .position_map();
            let facing = crate::position_interface::vector_to_sector_0_to_15_iso(
                target_pos.x - healer_pos.x,
                target_pos.y - healer_pos.y,
            );
            // RHANIMATION_HEALING computes the goal in its first Execute,
            // then calls Turn before advancing the animation. Selection of
            // the interaction alone must not rotate the actor a frame early.
            self.get_entity_mut(actor_id)
                .expect("Heal owner vanished before direction initialization")
                .element_data_mut()
                .set_direction_goal(facing);
        }

        let pending_strangle_init = self
            .get_entity(actor_id)
            .and_then(Entity::actor_data)
            .and_then(|actor| {
                let ability = &actor.active_ability;
                (ability.kind == Some(crate::movement::AbilityKind::Strangle)
                    && !ability.strangle_initialized)
                    .then(|| {
                        (
                            ability
                                .sequence_id
                                .expect("pending Strangle initialization lost sequence identity"),
                            ability.element_index,
                            ability
                                .target
                                .expect("pending Strangle initialization lost antagonist identity"),
                            ability
                                .order_id
                                .expect("pending Strangle initialization lost order identity"),
                        )
                    })
            });
        if let Some((seq_id, elem_idx, victim_id, order_id)) = pending_strangle_init {
            let valid = {
                let element = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .unwrap_or_else(|| {
                        panic!(
                            "pending Strangle owner {actor_id:?} lost element {seq_id:?}/{elem_idx}"
                        )
                    });
                assert_eq!(element.owner, Some(actor_id));
                assert_eq!(element.command, crate::element::Command::StrangleCmd);
                let order = element.current_order().unwrap_or_else(|| {
                    panic!("pending Strangle element {seq_id:?}/{elem_idx} lost its selected order")
                });
                assert_eq!(order.order_id, order_id);
                assert_eq!(order.target_actor, Some(victim_id.index()));
                self.check_sequence_element_validity(assets, actor_id, element, true)
            };
            if !valid {
                self.cleanup_aborted_ability(
                    actor_id,
                    crate::movement::AbilityKind::Strangle,
                    seq_id,
                    elem_idx,
                    Some(order_id),
                );
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                self.dispatch_condolations_for_owner_boundary(sim, actor_id, assets);
                return;
            }

            let attacker_pos = self
                .get_entity(actor_id)
                .expect("validated strangler vanished during initialization")
                .element_data()
                .position_map();
            let victim_pos = self
                .get_entity(victim_id)
                .expect("validated Strangle victim vanished during initialization")
                .element_data()
                .position_map();
            let facing = crate::position_interface::vector_to_sector_0_to_15_iso(
                victim_pos.x - attacker_pos.x,
                victim_pos.y - attacker_pos.y,
            );
            self.get_entity_mut(victim_id)
                .expect("validated Strangle victim vanished before FREEZE")
                .ai_controller_mut()
                .expect("validated Strangle victim lost AI before FREEZE")
                .non_script_lock(crate::ai::AiLockFlags::FREEZE);
            self.get_entity_mut(actor_id)
                .expect("validated strangler vanished before direction initialization")
                .element_data_mut()
                .set_direction_goal(facing);
            self.get_entity_mut(victim_id)
                .expect("validated Strangle victim vanished before direction initialization")
                .element_data_mut()
                .set_direction_goal(facing);
            self.get_entity_mut(actor_id)
                .expect("validated strangler vanished before initialization latch")
                .actor_data_mut()
                .expect("validated strangler lost actor state before initialization latch")
                .active_ability
                .strangle_initialized = true;
        }

        let strangle_victim_after_attacker = self
            .get_entity(actor_id)
            .and_then(Entity::actor_data)
            .and_then(|actor| {
                (actor.active_ability.kind == Some(crate::movement::AbilityKind::Strangle)
                    && actor.active_ability.done_effect_applied)
                    .then_some(actor.active_ability.target)
                    .flatten()
            });
        let sprite_frozen = self.actors_frozen();
        let results = crate::abilities::tick_ability(
            sim,
            &mut self.world.entities,
            &self.orders.sequence_manager,
            actor_id,
            sprite_frozen,
        );
        if let Some(victim_id) = strangle_victim_after_attacker
            && !sprite_frozen
        {
            self.get_entity_mut(victim_id)
                .unwrap_or_else(|| panic!("strangle victim {victim_id:?} vanished"))
                .element_data_mut()
                .sprite
                .perform_virgin_increment(sim, crate::sprite::FrameProgression::Default);
        }
        for result in results {
            use crate::abilities::AbilityTickResult;
            match result {
                AbilityTickResult::Terminated {
                    actor_id,
                    kind,
                    seq_id,
                    elem_idx,
                } => {
                    self.do_next_order(seq_id, elem_idx);
                    // `DoNextOrder` terminates the exhausted element, and
                    // Original immediately runs SendCondolationCard before
                    // returning from that SetState stack. This is required
                    // for every ability, not only Strangle: it clears the
                    // actor's selected element (so GetCommand becomes Wait)
                    // and may synchronously instruct a successor.
                    self.dispatch_condolations_for_owner_boundary(sim, actor_id, assets);
                    let next = self
                        .orders
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .and_then(|element| element.current_order())
                        .map(|order| (order.order_id, order.order_type));
                    if let Some(actor) = self
                        .get_entity_mut(actor_id)
                        .and_then(Entity::actor_data_mut)
                        && actor.active_ability.kind == Some(kind)
                        && actor.active_ability.sequence_id == Some(seq_id)
                        && actor.active_ability.element_index == elem_idx
                    {
                        match (kind, next) {
                            (crate::movement::AbilityKind::Listen, Some((order_id, crate::order::OrderType::Listening))) => {
                                actor.listen_phase = crate::element::ListenPhase::CountingDown;
                                actor.active_ability.order_id = Some(order_id);
                                actor.active_ability.done_effect_applied = false;
                            }
                            (crate::movement::AbilityKind::Listen, Some((order_id, crate::order::OrderType::TransitionListeningWaitingUpright))) => {
                                actor.listen_phase = crate::element::ListenPhase::ExitTransition;
                                actor.active_ability.order_id = Some(order_id);
                                actor.active_ability.done_effect_applied = false;
                            }
                            (crate::movement::AbilityKind::ReceivePurse, Some((order_id, crate::order::OrderType::WaitingWithPurse))) => {
                                actor.receive_purse_phase = crate::element::ReceivePursePhase::Waiting;
                                actor.active_ability.order_id = Some(order_id);
                                actor.active_ability.done_effect_applied = false;
                            }
                            (crate::movement::AbilityKind::ReceivePurse, Some((order_id, crate::order::OrderType::TransitionWaitingWithPurseWaitingUpright))) => {
                                actor.receive_purse_phase = crate::element::ReceivePursePhase::Transition;
                                actor.active_ability.order_id = Some(order_id);
                                actor.active_ability.done_effect_applied = false;
                            }
                            _ => {
                                if kind == crate::movement::AbilityKind::Listen {
                                    actor.listen_phase = crate::element::ListenPhase::Inactive;
                                    actor.listen_wait_time = 0;
                                } else if kind == crate::movement::AbilityKind::ReceivePurse {
                                    actor.receive_purse_phase = crate::element::ReceivePursePhase::Inactive;
                                }
                                actor.active_ability.clear();
                                actor.action_state = crate::element::ActionState::Waiting;
                            }
                        }
                    }
                }
                AbilityTickResult::Aborted {
                    actor_id,
                    kind,
                    seq_id,
                    elem_idx,
                    order_id,
                } => {
                    self.cleanup_aborted_ability(actor_id, kind, seq_id, elem_idx, order_id);
                    self.orders
                        .sequence_manager
                        .element_impossible(seq_id, elem_idx);
                    if kind == crate::movement::AbilityKind::Strangle {
                        self.dispatch_condolations_for_owner_boundary(sim, actor_id, assets);
                    }
                }
                AbilityTickResult::CarryDone {
                    carrier_id,
                    target_id,
                    carried_posture: _,
                    seq_id,
                    elem_idx,
                } => {
                    // Set PcData.carried and target posture.
                    if let Some(carrier) = self.get_entity_mut(carrier_id)
                        && let Some(pc) = carrier.pc_data_mut()
                    {
                        pc.carried = Some(target_id);
                    }
                    if let Some(target) = self.get_entity_mut(target_id) {
                        target.set_posture(crate::element::Posture::Carried);
                        if let Some(actor) = target.actor_data_mut() {
                            actor.action_state = crate::element::ActionState::Waiting;
                        }
                    }
                    tracing::debug!(
                        carrier = ?carrier_id,
                        target = ?target_id,
                        "Carry: picked up body"
                    );
                }
                AbilityTickResult::DropDone {
                    carrier_id,
                    target_id,
                    drop_posture,
                    carrier_pos,
                    carrier_direction,
                    seq_id,
                    elem_idx,
                } => {
                    // Clear PcData.carried and restore target posture.
                    // Force the carrier back to UPRIGHT + WAITING
                    // synchronously.  In the normal path the transition
                    // animation already lands the carrier in Upright
                    // before this fires, but a future non-transition
                    // instant-drop path would otherwise leave the
                    // carrier in CarryingCorpse — apply defensively.
                    if let Some(carrier) = self.get_entity_mut(carrier_id) {
                        if let Some(pc) = carrier.pc_data_mut() {
                            pc.carried = None;
                        }
                        carrier.set_posture(crate::element::Posture::Upright);
                        if let Some(actor) = carrier.actor_data_mut() {
                            actor.action_state = crate::element::ActionState::Waiting;
                        }
                    }

                    // Resolve the carrier's sector/layer. Both the drop
                    // position logic and the post-drop hulk flash need
                    // the sector's building flag (drives the
                    // instant-drop shortcut and the visibility flash).
                    let (carrier_sector, carrier_layer) = self
                        .get_entity(carrier_id)
                        .map(|e| (e.element_data().sector(), e.element_data().layer()))
                        .unwrap_or((None, 0));
                    let in_building = carrier_sector
                        .and_then(|s| {
                            self.grid_sector_by_number(crate::sector::SectorNumber::new(i16::from(
                                s,
                            )))
                        })
                        .map(|gs| gs.sector_type.is_building())
                        .unwrap_or(false);

                    // Choose the drop position.  In instant-drop mode
                    // the corpse drops under the carrier's feet;
                    // otherwise the target's move box is translated to
                    // the carrier's position and nudged off any motion
                    // lines with `find_authorized_position_toward`,
                    // using the resulting box centre. Falls back to
                    // `carrier_pos` when no authorised spot is found
                    // or the target has no move-box geometry.
                    let drop_pos = if in_building {
                        carrier_pos
                    } else {
                        let target_box = self
                            .get_entity(target_id)
                            .map(|e| e.position_iface())
                            .map(|pi| *pi.get_move_box())
                            .filter(|b| b.is_somewhere());
                        match target_box {
                            Some(b) => {
                                let mut bbox = b.translated(carrier_pos);
                                if self.world.fast_grid.find_authorized_position_toward(
                                    &mut bbox,
                                    carrier_pos,
                                    carrier_layer,
                                ) {
                                    bbox.center()
                                } else {
                                    carrier_pos
                                }
                            }
                            None => carrier_pos,
                        }
                    };

                    if let Some(target) = self.get_entity_mut(target_id) {
                        target.set_posture(drop_posture);
                        target.element_data_mut().set_position_map(drop_pos);
                        // direction = (carrier_dir + 12) & 15
                        // (12/16 * 360° = 270° offset from carrier facing).
                        target.element_data_mut().set_direction_instantly(
                            ((carrier_direction.wrapping_add(12)) & 15) as i16,
                        );
                        // Clearing the carrier link sets the direction
                        // *goal* to the carrier's direction, overwriting
                        // the +12 offset's goal (the immediate facing
                        // keeps the +12 offset; the goal slowly turns
                        // toward the carrier).
                        target
                            .element_data_mut()
                            .set_direction_goal(carrier_direction as i16);
                        // Unfreeze execution and clear the carrier
                        // back-reference.
                        if let Some(human) = target.human_data_mut() {
                            human.carrier = None;
                        }
                        if let Some(actor) = target.actor_data_mut() {
                            actor.execution_frozen = false;
                            actor.action_state = crate::element::ActionState::Waiting;
                        }
                        // Stop tracking the carrier's display_order now
                        // that they're separate sprites again.
                        let sprite = &mut target.element_data_mut().sprite;
                        sprite.display_order_ref = None;
                        sprite.behind_display_order_ref = false;
                    }
                    // Launch a low-priority Wait on the target so its
                    // AI re-enters an idle state instead of staying in
                    // whatever command it was running when it was picked
                    // up.
                    self.actor_wait(target_id);
                    // Post-drop hulk flash: when the carrier is inside a
                    // building and the dropped body is dead or
                    // unconscious, start the hulk effect on the body and
                    // unhide it so it stays visible through walls.
                    if in_building && let Some(target) = self.get_entity_mut(target_id) {
                        let is_dead = target.is_dead();
                        let is_unconscious = target.human_data().is_some_and(|h| h.unconscious);
                        if is_dead || is_unconscious {
                            crate::engine::door_pass::start_hulk_on(target, 1.0);
                            let elem = target.element_data_mut();
                            elem.hidden_in_building = false;
                            elem.active = true;
                        }
                    }
                    tracing::debug!(
                        carrier = ?carrier_id,
                        target = ?target_id,
                        "Drop: put down body"
                    );
                }
                AbilityTickResult::TieDone {
                    actor_id,
                    target_id,
                    seq_id,
                    elem_idx,
                } => {
                    let target = self
                        .get_entity_mut(target_id)
                        .unwrap_or_else(|| panic!("tie target {target_id:?} vanished at Done"));
                    target.set_posture(crate::element::Posture::Tied);
                    if target.is_soldier() {
                        target
                            .ai_controller_mut()
                            .expect("tied soldier must have AI")
                            .say(crate::ai::Remark::TiedUp);
                        // Original invokes Say directly from the tying PC's
                        // owner tick, so expose the request at this same
                        // creation-order boundary.
                        self.drain_ai_owner_work_for(sim, assets, target_id);
                    }
                    // RHElementActorPC::PerformAbility refreshes the victim
                    // with Wait after applying the tied posture and remark.
                    self.actor_wait(target_id);
                    tracing::debug!(
                        actor = ?actor_id,
                        target = ?target_id,
                        "Tie: enemy tied up"
                    );
                }
                AbilityTickResult::ClimbOnShouldersDone {
                    climber_id,
                    helper_id,
                    seq_id,
                    elem_idx,
                } => {
                    // Postures were latched on init by
                    // `begin_climb_on_shoulders`.  Terminate the
                    // climber's sequence element so the post-seek
                    // sequence advances and park the helper on a
                    // low-priority Wait so its frozen-execution can
                    // re-enter the idle loop while still
                    // `CarryingOnShoulders`.
                    self.actor_wait(helper_id);
                    tracing::debug!(
                        climber = ?climber_id,
                        helper = ?helper_id,
                        "ClimbOnShoulders: PC mounted helper's shoulders"
                    );
                }
                AbilityTickResult::ClimbDownFromShouldersDone {
                    climber_id,
                    helper_id,
                    seq_id,
                    elem_idx,
                } => {
                    // On the climbing-down completion: reset paired
                    // postures, sever the carrier ↔ carried link, copy
                    // the carrier's plane/sector/material onto the
                    // climber (so the dismount happens on the helper's
                    // surface) and snap the climber to an authorised
                    // landing slot adjacent to the helper.
                    let helper_snapshot = self.get_entity(helper_id).map(|e| {
                        (
                            e.element_data().position_map(),
                            e.element_data().layer(),
                            e.element_data().sector(),
                            e.element_data().material(),
                            e.element_data().obstacle_index(),
                            e.position_iface().get_plane().copied(),
                            e.element_data().direction(),
                        )
                    });

                    if let Some((
                        helper_pos,
                        helper_layer,
                        helper_sector,
                        helper_material,
                        helper_obstacle,
                        helper_plane,
                        helper_dir,
                    )) = helper_snapshot
                    {
                        // Resolve a landing slot using the climber's
                        // upright move-box translated to the helper's
                        // position.  We use the climber's current
                        // move-box rather than re-deriving the upright
                        // variant — the upright move-box was set when
                        // the PC was last upright and isn't overwritten
                        // while OnShoulders.
                        let landing_pos = {
                            let climber_box = self
                                .get_entity(climber_id)
                                .map(|e| e.position_iface())
                                .map(|pi| *pi.get_move_box())
                                .filter(|b| b.is_somewhere());
                            match climber_box {
                                Some(b) => {
                                    let mut bbox = b.translated(helper_pos);
                                    if self
                                        .world
                                        .fast_grid
                                        .find_authorized_position(&mut bbox, helper_layer)
                                    {
                                        bbox.center()
                                    } else {
                                        helper_pos
                                    }
                                }
                                None => helper_pos,
                            }
                        };

                        if let Some(climber) = self.get_entity_mut(climber_id) {
                            climber.set_posture(crate::element::Posture::Upright);
                            // Sever climber → carrier back-reference.
                            if let Some(human) = climber.human_data_mut() {
                                human.carrier = None;
                            }
                            if let Some(actor) = climber.actor_data_mut() {
                                actor.execution_frozen = false;
                                actor.action_state = crate::element::ActionState::Waiting;
                            }
                            // Copy plane/sector/material/obstacle from
                            // helper so the climber's reprojection lands
                            // on the helper's surface.
                            {
                                let elem = climber.element_data_mut();
                                elem.set_layer(helper_layer);
                                elem.set_sector(helper_sector);
                                elem.set_material(helper_material);
                            }
                            {
                                let pi = climber.position_iface_mut();
                                pi.set_obstacle(helper_obstacle, helper_plane);
                                pi.set_material(helper_material);
                            }
                            // Preserve the climber's facing through the
                            // copy.  The helper's direction was set to
                            // the opposite of the climber's at climb
                            // start, so adding 8 (180°) recovers the
                            // climber's original facing.
                            let preserved_dir = (helper_dir + 8) & 15;
                            climber
                                .element_data_mut()
                                .set_direction_instantly(preserved_dir);
                            // Snap to landing slot.
                            climber.element_data_mut().set_position_map(landing_pos);
                            // The climber is no longer carried so its
                            // draw order detaches from the helper.
                            let sprite = &mut climber.element_data_mut().sprite;
                            sprite.display_order_ref = None;
                            sprite.behind_display_order_ref = false;
                        }
                    }

                    // Reset the helper to HelpingToClimb / Waiting and
                    // sever the carrier-side link.
                    if let Some(helper) = self.get_entity_mut(helper_id) {
                        helper.set_posture(crate::element::Posture::HelpingToClimb);
                        if let Some(actor) = helper.actor_data_mut() {
                            actor.execution_frozen = false;
                            actor.action_state = crate::element::ActionState::Waiting;
                        }
                        if let Some(pc) = helper.pc_data_mut() {
                            pc.carried = None;
                            pc.set_live_carried_posture(crate::element::Posture::Lying);
                        }
                    }

                    // Park the helper on a low-priority idle so it
                    // doesn't immediately re-acquire its previous
                    // element.
                    self.actor_wait(helper_id);

                    tracing::debug!(
                        climber = ?climber_id,
                        helper = ?helper_id,
                        "ClimbDownFromShoulders: PC dismounted"
                    );
                }
                AbilityTickResult::HealDone {
                    healer_id,
                    target_id,
                    seq_id,
                    elem_idx,
                } => {
                    // Heal effect depends on the antagonist's type.
                    let target_is_fx_target = self
                        .get_entity(target_id)
                        .is_some_and(|e| e.kind().is_fx_target());
                    if target_is_fx_target {
                        // FX target — launch `Command::ActivateHeal` so
                        // the target's bound script's `ActivatedByHeal`
                        // hook fires.
                        let mut activation = crate::sequence::SequenceElement::new(
                            1,
                            crate::element::Command::ActivateHeal,
                            Some(target_id),
                        );
                        activation.data = crate::sequence::SequenceElementData::Interaction {
                            antagonist: Some(healer_id),
                        };
                        self.launch_element(activation);
                    } else if let Some(target) = self.get_entity_mut(target_id) {
                        // Heal the target PC via the shared helper that
                        // applies the heal + life-point clamp guards.
                        if let Some(pc) = target.pc_data_mut() {
                            crate::pc_status::heal(
                                &mut pc.life_points,
                                crate::abilities::HEAL_AMOUNT,
                                false, // invulnerable cheat unimplemented
                            );
                        }
                        // Clear concussion.
                        if let Some(human) = target.human_data_mut() {
                            human.concussion_of_the_brain = 0;
                        }
                        // "Sexual healing" speech cue on the healed PC.
                        self.hero_speaking(assets, target_id, crate::engine::melee::HERO_HEALED);
                    }
                    // Decrease healer's bandage ammo.
                    self.decrement_ability_ammo(assets, healer_id, crate::profiles::Action::Heal);
                    tracing::debug!(
                        healer = ?healer_id,
                        target = ?target_id,
                        "Heal: restored HP"
                    );
                }
                AbilityTickResult::EatDone {
                    actor_id,
                    seq_id,
                    elem_idx,
                } => {
                    // Re-check sequence-element validity by verifying
                    // the actor still has Eat ammo — if it dropped to 0
                    // mid-animation, skip the heal.
                    //
                    // Eat and Guzzle share the `num_rations` counter, so
                    // the Guzzle branch only changes the heal amount
                    // (80 vs 40); both end up decrementing the same
                    // underlying field.
                    let pc_status = self.get_entity(actor_id).and_then(|e| match e {
                        Entity::Pc(pc) => {
                            Some((pc.pc.profile_index, self.pc_description_for_pc_data(&pc.pc)))
                        }
                        _ => None,
                    });
                    if let Some((profile_idx, Some(pc_desc))) = pc_status {
                        let still_has_ammo =
                            pc_desc.status.get_ammo(crate::profiles::Action::Eat) > 0;
                        if still_has_ammo {
                            // Determine heal amount based on whether the
                            // PC has the Guzzle action (gluttons heal
                            // more).
                            let has_guzzle = assets
                                .profile_manager
                                .get_character(profile_idx)
                                .map(|p| p.has_action(crate::profiles::Action::Guzzle))
                                .unwrap_or(false);
                            let heal_amount: i16 = if has_guzzle { 80 } else { 40 };
                            // Original uses SetAmmoAmount here rather than
                            // DecreaseAmmoAmount. That distinction suppresses
                            // HERO_OUT_OF_AMMO for the last ration. Gluttons
                            // also address their Guzzle action slot even
                            // though Eat and Guzzle share one counter.
                            let ration_action = if has_guzzle {
                                crate::profiles::Action::Guzzle
                            } else {
                                crate::profiles::Action::Eat
                            };
                            self.consume_ration_without_speech(assets, actor_id, ration_action);
                            // Apply heal capped at LIFEPOINTS_PC.
                            if let Some(target) = self.get_entity_mut(actor_id)
                                && let Some(pc) = target.pc_data_mut()
                            {
                                crate::pc_status::heal(&mut pc.life_points, heal_amount, false);
                            }
                            tracing::debug!(
                                actor = ?actor_id,
                                heal_amount,
                                has_guzzle,
                                "Eat: ration consumed and HP restored"
                            );
                        }
                    }
                }
                AbilityTickResult::WhistleDone {
                    actor_id,
                    position,
                    seq_id,
                    elem_idx,
                } => {
                    // Emit a PFIIIT noise at the whistle position with
                    // radius NOISE_VOLUME_PFIIIT (400).
                    let (layer, elevation) = self
                        .get_entity(actor_id)
                        .map(|e| {
                            (
                                e.element_data().layer(),
                                e.element_data().position().z.max(0.0) as u16,
                            )
                        })
                        .unwrap_or((0, 0));
                    self.broadcast_noise_synchronously(
                        sim,
                        assets,
                        crate::ai::NoiseType::Pfiiit,
                        crate::coordinates::MapPoint::new(position.x, position.y),
                        layer,
                        crate::abilities::NOISE_VOLUME_WHISTLE,
                        elevation,
                        Some(actor_id),
                    );
                    tracing::debug!(
                        actor = ?actor_id,
                        x = position.x,
                        y = position.y,
                        "Whistle: noise emitted to attract NPCs"
                    );
                }
                AbilityTickResult::ListenEntered { actor_id } => {
                    // Entry transition animation just finished; the
                    // PC is now in ActionState::Listening /
                    // ListenPhase::CountingDown.  Forward
                    // PcMessage::SelectAction(Listen) so HUD/UI
                    // reflects the active listen.
                    self.orders
                        .messenger
                        .send(crate::messenger::Message::pc_with_value(
                            crate::messenger::PcMessage::SelectAction,
                            Some(actor_id),
                            crate::profiles::Action::Listen as u32,
                        ));
                    tracing::debug!(
                        actor = ?actor_id,
                        "Listen: entry transition done → CountingDown, MSG_SELECT_ACTION sent"
                    );
                }
                AbilityTickResult::ListenDone {
                    actor_id,
                    seq_id,
                    elem_idx,
                } => {
                    // Exit transition animation finished.  Forward
                    // PcMessage::UnselectAction(Listen) to clear the
                    // HUD active state.
                    self.orders
                        .messenger
                        .send(crate::messenger::Message::pc_with_value(
                            crate::messenger::PcMessage::UnselectAction,
                            Some(actor_id),
                            crate::profiles::Action::Listen as u32,
                        ));
                    tracing::debug!(
                        actor = ?actor_id,
                        "Listen: exit transition done → Inactive, MSG_UNSELECT_ACTION sent"
                    );
                }
                AbilityTickResult::ThrowNetDone {
                    actor_id,
                    target_pos,
                    seq_id,
                    elem_idx,
                } => {
                    // Spawn a net projectile entity with ballistic
                    // trajectory.  Launch origin is the thrower's hand
                    // point, not their feet.
                    let (throw_pos, layer) = self.projectile_throw_origin(actor_id, "ThrowNetDone");
                    let target_3d = crate::coordinates::WorldPoint3D {
                        x: target_pos.x,
                        y: target_pos.y,
                        z: 0.0,
                    };
                    let obstacle_check = crate::bow_shot::TrajectoryObstacleCheck {
                        fast_find_grid: &self.world.fast_grid,
                        layer,
                        sight_obstacles: self.sight_obstacles(assets),
                        water_zones: Some(&assets.water_zones),
                    };
                    let net_entity = crate::bow_shot::spawn_net(
                        actor_id,
                        throw_pos,
                        target_3d,
                        layer,
                        Some(&obstacle_check),
                    );
                    let net_id = self.add_entity(net_entity);
                    self.attach_accessory_sprite(assets, net_id);
                    // Run the landing-site crumple test at spawn time.
                    // We keep the ballistic trajectory inside
                    // `spawn_net` and run the crumple check here, where
                    // we have engine access to obstacles +
                    // fast_find_grid.
                    self.detect_initial_net_crumple(assets, net_id);
                    tracing::debug!(
                        actor = ?actor_id,
                        x = target_pos.x,
                        y = target_pos.y,
                        "ThrowNet: spawned net projectile"
                    );
                    self.decrement_ability_ammo(assets, actor_id, crate::profiles::Action::Net);
                }
                AbilityTickResult::ThrowPurseDone {
                    actor_id,
                    target_pos,
                    seq_id,
                    elem_idx,
                } => {
                    // Spawn the purse projectile.  The trajectory is
                    // computed against the current sight obstacles so
                    // the purse arcs over walls / falls onto roofs the
                    // same way other ground-targeted throwables do.
                    // Launch origin is the thrower's hand point.
                    let (throw_pos, layer) =
                        self.projectile_throw_origin(actor_id, "ThrowPurseDone");
                    let target_3d = crate::coordinates::WorldPoint3D {
                        x: target_pos.x,
                        y: target_pos.y,
                        z: 0.0,
                    };
                    let obstacle_check = crate::bow_shot::TrajectoryObstacleCheck {
                        fast_find_grid: &self.world.fast_grid,
                        layer,
                        sight_obstacles: self.sight_obstacles(assets),
                        water_zones: Some(&assets.water_zones),
                    };
                    let purse_entity = crate::bow_shot::spawn_purse(
                        actor_id,
                        throw_pos,
                        target_3d,
                        layer,
                        Some(&obstacle_check),
                    );
                    let purse_id = self.add_entity(purse_entity);
                    self.attach_accessory_sprite(assets, purse_id);
                    tracing::debug!(
                        actor = ?actor_id,
                        x = target_pos.x,
                        y = target_pos.y,
                        "ThrowPurse: spawned purse projectile"
                    );
                    self.decrement_ability_ammo(assets, actor_id, crate::profiles::Action::Purse);
                    // Deduct the thrown purse's face value from the
                    // campaign ransom pool on throw.  Coin pickup later
                    // credits `COIN_VALUE` per recovered coin, so
                    // conservation holds: uncollected coins are a real
                    // loss and fully-recovered purses wash out.
                    let face_value = crate::inventory::COINS_PER_PURSE as i32
                        * crate::inventory::COIN_VALUE as i32;
                    self.add_campaign_value(crate::campaign::CampaignValue::Ransom, -face_value);
                }
                AbilityTickResult::ThrowWaspNestDone {
                    actor_id,
                    target_pos,
                    seq_id,
                    elem_idx,
                } => {
                    // Spawn a wasp nest projectile entity with ballistic
                    // trajectory.  Launch origin is the thrower's hand
                    // point.
                    let (throw_pos, layer) =
                        self.projectile_throw_origin(actor_id, "ThrowWaspNestDone");
                    let target_3d = crate::coordinates::WorldPoint3D {
                        x: target_pos.x,
                        y: target_pos.y,
                        z: 0.0,
                    };
                    let obstacle_check = crate::bow_shot::TrajectoryObstacleCheck {
                        fast_find_grid: &self.world.fast_grid,
                        layer,
                        sight_obstacles: self.sight_obstacles(assets),
                        water_zones: Some(&assets.water_zones),
                    };
                    let wasp_entity = crate::bow_shot::spawn_wasp_nest(
                        actor_id,
                        throw_pos,
                        target_3d,
                        layer,
                        Some(&obstacle_check),
                    );
                    let wasp_id = self.add_entity(wasp_entity);
                    self.attach_accessory_sprite(assets, wasp_id);
                    tracing::debug!(
                        actor = ?actor_id,
                        x = target_pos.x,
                        y = target_pos.y,
                        "ThrowWaspNest: spawned wasp nest projectile"
                    );
                    self.decrement_ability_ammo(
                        assets,
                        actor_id,
                        crate::profiles::Action::WaspNest,
                    );
                }
                AbilityTickResult::ThrowAppleDone {
                    actor_id,
                    target,
                    seq_id,
                    elem_idx,
                } => {
                    self.on_throw_projectile_done(
                        assets,
                        actor_id,
                        target,
                        crate::profiles::Action::Apple,
                        crate::element::ObjectType::Apple,
                    );
                }
                AbilityTickResult::ThrowStoneDone {
                    actor_id,
                    target,
                    seq_id,
                    elem_idx,
                } => {
                    self.on_throw_projectile_done(
                        assets,
                        actor_id,
                        target,
                        crate::profiles::Action::Stone,
                        crate::element::ObjectType::Stone,
                    );
                }
                AbilityTickResult::PayDone {
                    pc_id,
                    beggar_id,
                    seq_id,
                    elem_idx,
                } => {
                    // On Paying-animation completion: deduct
                    // BEGGAR_SALARY from the ransom, and either launch
                    // `Command::ActivateMoney` on an FX-target antagonist
                    // or a `Command::ReceivePurse` sequence element on a
                    // beggar NPC.
                    self.add_campaign_value(
                        crate::campaign::CampaignValue::Ransom,
                        -crate::engine::BEGGAR_SALARY,
                    );
                    let antagonist_is_fx_target = self
                        .get_entity(beggar_id)
                        .is_some_and(|e| e.kind().is_fx_target());
                    if antagonist_is_fx_target {
                        // FX target — fire the script's ActivatedByMoney
                        // hook via the central `Command::Activate*`
                        // dispatch.
                        let mut activation = crate::sequence::SequenceElement::new(
                            1,
                            crate::element::Command::ActivateMoney,
                            Some(beggar_id),
                        );
                        activation.data = crate::sequence::SequenceElementData::Interaction {
                            antagonist: Some(pc_id),
                        };
                        self.launch_element(activation);
                    } else {
                        let mut receive = crate::sequence::SequenceElement::new(
                            1,
                            crate::element::Command::ReceivePurse,
                            Some(beggar_id),
                        );
                        receive.priority = crate::sequence::SequencePriority::Normal;
                        self.launch_element(receive);
                    }
                    tracing::debug!(
                        pc = ?pc_id,
                        beggar = ?beggar_id,
                        "Pay: salary deducted, ACTIVATE_MONEY / RECEIVE_PURSE launched"
                    );
                }
                AbilityTickResult::ReceivePurseRevealing { beggar_id } => {
                    // Middle of the receive-purse chain — the beggar is
                    // waving the purse.  `reveal_scrolls` runs on
                    // WaitingWithPurse termination, driving the
                    // delayed-highlight display flow.  The beggar's
                    // CIV_REMARK_BEGGAR_* speech cue is queued inside
                    // `reveal_scrolls` and later dispatched by
                    // the owner-local speech drain.
                    match self.reveal_scrolls(sim, display, assets, beggar_id) {
                        Some(remark) => tracing::debug!(
                            beggar = ?beggar_id,
                            ?remark,
                            "ReceivePurse: reveal_scrolls fired",
                        ),
                        None => tracing::debug!(
                            beggar = ?beggar_id,
                            "ReceivePurse: reveal_scrolls returned None \
                             (non-beggar?), ignoring"
                        ),
                    }
                    #[cfg(test)]
                    RECEIVE_PURSE_REVEAL_OBSERVER.with(|observer| {
                        if let Some(observer) = observer.borrow_mut().as_mut() {
                            observer(self, beggar_id);
                        }
                    });
                }
                AbilityTickResult::ReceivePurseDone {
                    beggar_id,
                    seq_id,
                    elem_idx,
                } => {
                    tracing::debug!(
                        beggar = ?beggar_id,
                        "ReceivePurse: animation chain complete"
                    );
                }
                AbilityTickResult::HitDone {
                    actor_id,
                    target_id,
                    seq_id,
                    elem_idx,
                } => {
                    // Resolve base concussion from attacker profile:
                    //   if PC has HitHard action → 150,
                    //   else PC → 80,
                    //   else NPC hitter → 40.
                    // The Hard-difficulty scaling is re-applied
                    // consumer-side in `apply_hit_damage` via
                    // `combat::receive_hit_damage` when the victim is a
                    // Lacklandist, so we pass the un-scaled base here.
                    let (concussion, is_harder_hit) = {
                        let attacker = self.get_entity(actor_id);
                        if attacker.is_some_and(|e| e.kind().is_pc()) {
                            let has_hit_hard = attacker
                                .and_then(|e| e.pc_data())
                                .map(|pc| pc.profile_index)
                                .and_then(|idx| assets.profile_manager.get_character(idx))
                                .is_some_and(|cp| cp.has_action(crate::profiles::Action::HitHard));
                            if has_hit_hard {
                                (150u16, true)
                            } else {
                                (80u16, false)
                            }
                        } else {
                            (40u16, false)
                        }
                    };

                    // Launch a damage element on the target carrying the
                    // attacker as antagonist and the resolved
                    // concussion.
                    let mut dmg = crate::sequence::SequenceElement::new_damage(
                        1,
                        crate::element::Command::ReceiveHitDamage,
                        Some(target_id),
                        Some(actor_id),
                        0,
                        concussion,
                    );
                    if let crate::sequence::SequenceElementData::Damage {
                        is_harder_hit: ih, ..
                    } = &mut dmg.data
                    {
                        *ih = is_harder_hit;
                    }
                    self.launch_element(dmg);

                    tracing::debug!(
                        attacker = ?actor_id,
                        target = ?target_id,
                        concussion,
                        is_harder_hit,
                        "Hit: launched RECEIVE_HIT_DAMAGE"
                    );
                }
                AbilityTickResult::StrangleDone {
                    actor_id,
                    target_id,
                    seq_id,
                    elem_idx,
                } => {
                    self.find_place_to_die(target_id);
                    // A soldier flagged not-stranglable in their profile
                    // survives the strangle — the AI lock is released
                    // and the soldier gets an EventGotHit stimulus so
                    // it retaliates.
                    let stranglable = match self.get_entity(target_id) {
                        Some(crate::element::Entity::Soldier(s)) => assets
                            .profile_manager
                            .get_soldier(s.soldier.soldier_profile_index)
                            .unwrap_or_else(|| {
                                panic!(
                                    "strangle victim {target_id:?} has missing soldier profile {}",
                                    s.soldier.soldier_profile_index
                                )
                            })
                            .strangle,
                        Some(crate::element::Entity::Civilian(_)) => true,
                        Some(_) => panic!("strangle victim {target_id:?} is not an NPC human"),
                        None => panic!("strangle victim {target_id:?} disappeared at termination"),
                    };

                    if !stranglable {
                        self.get_entity_mut(target_id)
                            .expect("validated non-stranglable victim disappeared")
                            .ai_controller_mut()
                            .expect("non-stranglable victim must have AI")
                            .non_script_unlock(crate::ai::AiLockFlags::FREEZE);
                        let stimulus = crate::ai::Stimulus::with_human(
                            crate::ai::StimulusType::EventGotHit,
                            actor_id.index(),
                        );
                        self.dispatch_synchronous_ai_think_preserving_detection_fifo(
                            sim, target_id, assets, stimulus,
                        );
                        #[cfg(test)]
                        crate::engine::soldier_helpers::observe_strangle_condolation_step(
                            "TerminalEventGotHit",
                        );
                        tracing::debug!(
                            attacker = ?actor_id,
                            target = ?target_id,
                            "Strangle: target not stranglable, completed EVENT_GOTHIT Think"
                        );
                        continue;
                    }

                    // Full-life-points kill — launch ReceiveDamage on
                    // the victim with damage = current life and
                    // concussion = 0 and no damage origin.
                    let life = match self.get_entity(target_id) {
                        Some(crate::element::Entity::Soldier(s)) => s.npc.life_points,
                        Some(crate::element::Entity::Civilian(c)) => c.npc.life_points,
                        Some(_) => unreachable!("strangle victim kind validated above"),
                        None => panic!("strangle victim {target_id:?} disappeared before damage"),
                    };
                    let life = u16::try_from(life).unwrap_or_else(|_| {
                        panic!("strangle victim {target_id:?} has invalid life {life}")
                    });
                    let dmg = crate::sequence::SequenceElement::new_damage(
                        1,
                        crate::element::Command::ReceiveDamage,
                        Some(target_id),
                        None,
                        life,
                        0,
                    );
                    self.launch_element(dmg);

                    tracing::debug!(
                        attacker = ?actor_id,
                        target = ?target_id,
                        life,
                        "Strangle: launched RECEIVE_DAMAGE for kill"
                    );
                }
                AbilityTickResult::StrangleSetupDone {
                    actor_id,
                    target_id,
                    seq_id,
                    elem_idx,
                } => {
                    let (position, action_point, direction, layer, sector, obstacle, plane) = {
                        let attacker = self
                            .get_entity(actor_id)
                            .unwrap_or_else(|| panic!("strangler {actor_id:?} vanished at Done"));
                        let position = attacker.element_data().position_map();
                        let hotspot = attacker
                            .sprite()
                            .current_hotspot()
                            .expect("strangler current animation has no action point");
                        let sprite_pos = attacker.cxx_position_sprite();
                        (
                            position,
                            crate::coordinates::MapPoint::new(
                                sprite_pos.x + hotspot.x,
                                sprite_pos.y + hotspot.y,
                            ),
                            u16::try_from(attacker.element_data().direction()).expect(
                                "strangler direction must be in the canonical 0..=15 range",
                            ),
                            attacker.element_data().layer(),
                            attacker.element_data().sector(),
                            attacker.element_data().obstacle_index(),
                            attacker.position_iface().get_plane().copied(),
                        )
                    };
                    {
                        let victim = self.get_entity_mut(target_id).unwrap_or_else(|| {
                            panic!("strangle victim {target_id:?} vanished at Done")
                        });
                        victim
                            .element_data_mut()
                            .set_obstacle_index(obstacle, plane);
                        victim.element_data_mut().set_layer(layer);
                        victim.element_data_mut().set_sector(sector);
                        victim.element_data_mut().set_position_map(action_point);
                        victim
                            .element_data_mut()
                            .set_direction_instantly(direction as i16);
                    }
                    let victim_move_box = {
                        let victim = self.get_entity(target_id).unwrap_or_else(|| {
                            panic!("strangle victim {target_id:?} vanished at Done")
                        });
                        *victim.position_iface().get_move_box()
                    };
                    let mut victim_box = victim_move_box.translated(action_point);
                    if !victim_move_box.is_somewhere()
                        || !self.world.fast_grid.find_authorized_position_toward(
                            &mut victim_box,
                            position,
                            layer,
                        )
                    {
                        self.cleanup_aborted_ability(
                            actor_id,
                            crate::movement::AbilityKind::Strangle,
                            seq_id,
                            elem_idx,
                            self.get_entity(actor_id)
                                .and_then(Entity::actor_data)
                                .and_then(|actor| actor.active_ability.order_id),
                        );
                        self.orders
                            .sequence_manager
                            .element_impossible(seq_id, elem_idx);
                        self.dispatch_condolations_for_owner_boundary(sim, actor_id, assets);
                        continue;
                    }
                    let authorized_position = victim_box.center();
                    {
                        let victim = self.get_entity_mut(target_id).unwrap_or_else(|| {
                            panic!("strangle victim {target_id:?} vanished at Done")
                        });
                        victim
                            .element_data_mut()
                            .set_position_map(authorized_position);
                        victim.element_data_mut().sprite.display_order_ref = None;
                        victim.element_data_mut().sprite.behind_display_order_ref = false;
                    }
                    self.actor_freeze_execution(target_id);
                    let victim = self.get_entity_mut(target_id).unwrap_or_else(|| {
                        panic!("strangle victim {target_id:?} vanished at Done")
                    });
                    victim
                        .element_data_mut()
                        .sprite
                        .force_animation(crate::order::OrderType::BeingStrangled, direction);
                    let remark = if matches!(victim, crate::element::Entity::Civilian(_)) {
                        crate::ai::Remark::CivDies
                    } else {
                        crate::ai::Remark::Strangled
                    };
                    victim
                        .ai_controller_mut()
                        .expect("strangle victim must have AI")
                        .say_with_flags(remark, crate::ai::SpeechFlags::EMERGENCY);
                    self.drain_ai_owner_work_for(sim, assets, target_id);
                    if !sprite_frozen {
                        self.get_entity_mut(target_id)
                            .expect("strangle victim disappeared after speech")
                            .element_data_mut()
                            .sprite
                            .perform_virgin_increment(
                                sim,
                                crate::sprite::FrameProgression::Default,
                            );
                    }
                }
            }
        }
    }

    pub(super) fn cleanup_aborted_ability(
        &mut self,
        actor_id: EntityId,
        kind: crate::movement::AbilityKind,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        order_id: Option<std::num::NonZeroU32>,
    ) {
        if let Some(actor) = self
            .get_entity_mut(actor_id)
            .and_then(Entity::actor_data_mut)
            && actor.active_ability.kind == Some(kind)
            && actor.active_ability.sequence_id == Some(seq_id)
            && actor.active_ability.element_index == elem_idx
            && actor.active_ability.order_id == order_id
        {
            actor.active_ability.clear();
            if kind == crate::movement::AbilityKind::Listen {
                actor.listen_phase = crate::element::ListenPhase::Inactive;
                actor.listen_wait_time = 0;
            } else if kind == crate::movement::AbilityKind::ReceivePurse {
                actor.receive_purse_phase = crate::element::ReceivePursePhase::Inactive;
            }
            actor.action_state = crate::element::ActionState::Waiting;
        }
    }

    // ─── Shouldered-carry ceiling check ─────────────────────────────

    /// Check PCs whose movement action executed this frame for the original
    /// `RHANIMATION_WALKING_CARRYING_ON_SHOULDERS` ceiling collision.
    ///
    /// The original calls `CanCarryOnShoulders` only from that action's
    /// `RHElementActorPC::Execute` arm, after `PerformMotion`.  In particular,
    /// the persistent `RHPOSTURE_CARRYING_ON_SHOULDERS` waiting state does not
    /// run this check.
    pub(super) fn tick_shouldered_carry_ceiling(
        &mut self,
        assets: &LevelAssets,
        executed_actions: &[(EntityId, crate::order::OrderType)],
    ) {
        if self.actors_frozen() {
            return;
        }

        // Collect (carrier_id, victim_id) pairs first to avoid
        // overlapping borrows with launch_element.
        let mut drops: Vec<(crate::element::EntityId, crate::element::EntityId)> = Vec::new();
        for &(carrier_id, action) in executed_actions {
            if action != crate::order::OrderType::WalkingCarryingOnShoulders {
                continue;
            }
            let Some(entity) = self.get_entity(carrier_id) else {
                tracing::warn!(
                    ?carrier_id,
                    "shouldered-carry ceiling check lost executing carrier"
                );
                continue;
            };
            let elem = entity.element_data();
            let carrier_pos = elem.position();

            let obstacles = self.sight_obstacles(assets);
            if crate::abilities::can_carry_on_shoulders(carrier_pos, obstacles) {
                continue;
            }

            // Find the shouldered victim — the human whose `carrier`
            // back-pointer references this carrier.  We track the
            // relationship from the victim side only.
            let victim_id = self.world.entities.humans().find_map(|(victim_id, v)| {
                let victim_id: EntityId = victim_id.into();
                let hd = v.human_data()?;
                if hd.carrier == Some(carrier_id) {
                    Some(victim_id)
                } else {
                    None
                }
            });
            let Some(victim_id) = victim_id else {
                tracing::warn!(
                    ?carrier_id,
                    "shouldered-carry ceiling check: executing carrier has no \
                     shouldered victim — skipping"
                );
                continue;
            };
            drops.push((carrier_id, victim_id));
        }

        for (carrier_id, victim_id) in drops {
            // Launch ReceiveDamage on the victim with damage = 0 and
            // concussion = 0; the victim-side
            // `translate_shoulder_damage` path uses the event as a
            // trigger to drop off the shoulders rather than to apply HP
            // loss.  Origin is the victim itself (no antagonist).
            let dmg = crate::sequence::SequenceElement::new_damage(
                1,
                crate::element::Command::ReceiveDamage,
                Some(victim_id),
                Some(victim_id),
                0,
                0,
            );
            self.launch_element(dmg);
            tracing::debug!(
                ?carrier_id,
                ?victim_id,
                "CarryOnShoulders: ceiling blocked → launched drop damage"
            );
        }
    }
}
