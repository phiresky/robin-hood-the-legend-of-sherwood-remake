//! Bow shots and arrow projectile ticking.

use super::input::BowTarget;
use super::*;
use crate::bow_shot::{self};
use crate::coordinates::MapPoint;
use crate::element::{Command, Entity, EntityId};
fn arrow_publication_debug_gate() -> &'static super::diagnostics::ParityGate<3> {
    static GATE: std::sync::OnceLock<super::diagnostics::ParityGate<3>> =
        std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        super::diagnostics::ParityGate::from_env(
            "PARITY_DEBUG_ARROW_PUBLICATION",
            [
                "PARITY_DEBUG_ARROW_PUBLICATION_FRAME_AFTER",
                "PARITY_DEBUG_ARROW_PUBLICATION_SHOOTER_CREATION_ORDER",
                "PARITY_DEBUG_ARROW_PUBLICATION_PROJECTILE_CREATION_ORDER",
            ],
        )
    })
}

fn record_arrow_publication_debug(
    stage: &str,
    frame_after: u32,
    shooter_creation_order: u32,
    projectile_creation_order: Option<u32>,
    entity: &Entity,
) {
    if !arrow_publication_debug_gate().matches([
        Some(frame_after),
        Some(shooter_creation_order),
        projectile_creation_order,
    ]) {
        return;
    }
    let Entity::Projectile(arrow) = entity else {
        panic!("arrow publication diagnostic received a non-projectile entity");
    };
    let sprite = &arrow.element.sprite;
    let position = sprite.position_iface.get_position();
    let old_position = sprite.position_iface.v48_serialized_state().old_position;
    eprintln!(
        "PARITY_ARROW_PUBLICATION_RUST stage={stage} frame_after={frame_after} \
         projectile_creation_order={projectile_creation_order:?} \
         shooter_creation_order={shooter_creation_order} active={} flying={} falling={} \
         trajectory_size={} row={} frame={} frame_count={} \
         position_bits=[{:08x},{:08x},{:08x}] old_position_bits=[{:08x},{:08x},{:08x}]",
        arrow.element.active,
        arrow.projectile.flying,
        arrow.projectile.falling,
        arrow.projectile.trajectory.len(),
        sprite.current_row,
        sprite.current_frame,
        sprite.frame_count,
        position.x.to_bits(),
        position.y.to_bits(),
        position.z.to_bits(),
        old_position.x.to_bits(),
        old_position.y.to_bits(),
        old_position.z.to_bits(),
    );
}

fn projectile_landing_debug_matches(frame: u32, shooter: EntityId, projectile: EntityId) -> bool {
    static GATE: std::sync::OnceLock<super::diagnostics::ParityGate<3>> =
        std::sync::OnceLock::new();
    GATE.get_or_init(|| {
        super::diagnostics::ParityGate::from_env(
            "PARITY_DEBUG_PROJECTILE_LANDING",
            [
                "PARITY_DEBUG_PROJECTILE_LANDING_FRAME",
                "PARITY_DEBUG_PROJECTILE_LANDING_SHOOTER",
                "PARITY_DEBUG_PROJECTILE_LANDING_PROJECTILE",
            ],
        )
    })
    .matches([Some(frame), Some(shooter.index()), Some(projectile.index())])
}

/// State seen right after a ReceivePurse termination revealed the beggar's
/// scrolls.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReceivePurseRevealObservation {
    pub owner: EntityId,
    /// Identity and action of the beggar's current order at that point.
    pub current_order: Option<(std::num::NonZeroU32, crate::order::OrderType)>,
}

#[cfg(test)]
thread_local! {
    pub(super) static RECEIVE_PURSE_REVEALS: super::test_support::Probe<ReceivePurseRevealObservation> =
        const { super::test_support::Probe::new() };
}

#[cfg(test)]
pub(crate) fn capture_receive_purse_reveals<T>(
    f: impl FnOnce() -> T,
) -> (T, Vec<ReceivePurseRevealObservation>) {
    RECEIVE_PURSE_REVEALS.with(|reveals| reveals.capture(f))
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
pub(super) enum ArrowHitOutcome {
    /// Apply piercing damage to the victim.
    Damage,
    /// Arrow flies through silently — friendly-fire filter or VIP NPC.
    /// Hit flag and impact sound are both suppressed.
    PassThrough,
    /// Arrow ricochets off the victim's armor. The original game puts the arrow into
    /// falling state before impact-FX lookup, so this path is silent.
    Ricochet,
}

/// Finish the original game's arrow-vulnerability decision after its civilian/camp
/// filter. Only PCs and soldiers own a sword whose piercing protection can
/// reject an otherwise hurtable arrow. A civilian on Hard difficulty is
/// immediately hurtable and must not be treated as if a missing sword meant
/// full protection.
fn resolve_arrow_hurtable(
    hurtable_base: bool,
    victim_is_pc_or_soldier: bool,
    piercing_roll_passed: Option<bool>,
) -> bool {
    if !hurtable_base {
        false
    } else if !victim_is_pc_or_soldier {
        true
    } else {
        piercing_roll_passed.expect("PC/soldier arrow classification requires a piercing result")
    }
}

#[cfg(test)]
mod arrow_hurtable_tests {
    use super::resolve_arrow_hurtable;

    #[test]
    fn hard_difficulty_civilian_does_not_require_a_piercing_weapon_profile() {
        assert!(resolve_arrow_hurtable(true, false, None));
    }

    #[test]
    fn protected_or_filtered_human_remains_unhurtable() {
        assert!(!resolve_arrow_hurtable(true, true, Some(false)));
        assert!(!resolve_arrow_hurtable(false, false, None));
    }
}

impl EngineInner {
    // ─── Bow shots & arrow projectiles ───────────────────────────

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
                assets.environment.static_sight_obstacles.as_slice(),
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
    pub(crate) fn shoot_bow_at(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
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
            // Arrow shooting targets the retained actor
            // without rechecking whether it is dead. A target may die after an
            // archer selected it but before the aiming timer expires.
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

        Some(self.launch_element(
            sim,
            assets,
            bow_shot::build_shoot_bow_element(shooter, target),
        ))
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
                let mut shooting = if self.is_hostile_to_player_camp(s.soldier.cached_camp) {
                    let diff = self.control.sim_config.difficulty;
                    diff.rules().enemy_shooting(profile.shooting, 100) as u32
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

    /// Return the live bow-skill capacity used to scale a
    /// missed arrow's random bias.
    ///
    /// This is deliberately distinct from [`Self::bow_profile_and_ability`].
    /// The original game's bow shot uses the actor's shooting ability, including
    /// difficulty and drunkenness modifiers for soldiers) to the hit-chance
    /// lookup, but scales the miss vector with the unmodified capacity stored
    /// in the actor's human status. Soldier status is initialized from the
    /// raw profile; PC status aliases its campaign description.
    fn bow_skill_capacity(&self, assets: &LevelAssets, entity_id: EntityId) -> Option<u32> {
        match self.get_entity(entity_id)? {
            Entity::Pc(pc) => self
                .pc_description_for_pc_data(&pc.pc)
                .map(|description| description.status.human_status.bow.capacity),
            Entity::Soldier(soldier) => assets
                .profile_manager
                .soldiers
                .get(usize::from(soldier.soldier.soldier_profile_index))
                .map(|profile| u32::from(profile.shooting)),
            _ => None,
        }
    }

    pub(super) fn selected_bow_order(
        &self,
        owner: EntityId,
    ) -> Option<(crate::sequence::SequenceId, usize, std::num::NonZeroU32)> {
        let (seq_id, elem_idx, order) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, owner)?;
        (matches!(
            self.orders
                .sequence_manager
                .get_element(seq_id, elem_idx)?
                .command,
            Command::ShootBow | Command::ShootBowOnce
        ) && bow_shot::is_active_bow_order(order.order_type))
        .then_some((seq_id, elem_idx, order.order_id))
    }

    /// Deliver the original game's bow-shot shield warning at the release
    /// callsite. The arrow-launched AI event is
    /// synchronous there, before both the hit roll and arrow insertion.
    fn warn_shield_target_of_arrow(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        shooter: EntityId,
        target: EntityId,
    ) {
        // Shield-bearer admission is two-gated: the HtH weapon must be a
        // shield weapon *and* the soldier's sprite profile must carry a
        // `WaitingShield` animation row.
        let target_is_shield_soldier = match self.get_entity(target) {
            Some(Entity::Soldier(s)) => {
                let soldier_profile = assets
                    .profile_manager
                    .get_soldier(s.soldier.soldier_profile_index)
                    .unwrap_or_else(|| {
                        panic!(
                            "bow target {} requires missing soldier profile {}",
                            target.index(),
                            s.soldier.soldier_profile_index
                        )
                    });
                let weapon = assets
                    .profile_manager
                    .get_hth_weapon(soldier_profile.hth_weapon_id)
                    .unwrap_or_else(|| {
                        panic!(
                            "bow target {} soldier profile {} requires missing HtH weapon {}",
                            target.index(),
                            s.soldier.soldier_profile_index,
                            soldier_profile.hth_weapon_id
                        )
                    });
                weapon.shield
                    && s.element
                        .sprite
                        .has_animation(crate::order::OrderType::WaitingShield)
            }
            _ => false,
        };
        if !target_is_shield_soldier {
            return;
        }

        // This is a live cone + LOS query, not the detection cadence's stale
        // `seen_now` snapshot, matching the original game's immediate detection.
        if self.npc_is_detecting_human(assets, target, shooter, self.control.frame_counter) {
            self.execute_ai_callback(
                sim,
                assets,
                target,
                &crate::ai::Stimulus::with_human(
                    crate::ai::StimulusType::EventArrowLaunched,
                    shooter.index(),
                ),
            );
        }
    }
    /// Execute one bow order and close its callbacks before returning to the actor loop.
    pub(crate) fn tick_bow_shot_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        shooter_id: EntityId,
        expected_order_id: std::num::NonZeroU32,
    ) -> Option<crate::sprite::MotionState> {
        use crate::element::{ActionState, Posture};
        use crate::sprite::MotionState;

        let shooter = self.expect_entity(shooter_id, "bow execution owner");
        let actor = shooter.actor_data().expect("bow owner must be an actor");
        if actor.execution_frozen {
            return Some(MotionState::InProgress);
        }
        let Some((sequence_id, element_index, order_id)) = self.selected_bow_order(shooter_id)
        else {
            return None;
        };
        if order_id != expected_order_id {
            return None;
        }
        let element = self
            .orders
            .sequence_manager
            .get_element(sequence_id, element_index)
            .expect("selected bow element must exist");
        let order_type = element
            .current_order()
            .expect("selected bow order must exist")
            .order_type;
        let target_id = match element.data {
            crate::sequence::SequenceElementData::Interaction { antagonist } => antagonist,
            _ => None,
        };
        let script_driven = element.script_driven;
        if bow_shot::is_shoot_order(order_type) && actor.execute_order_initialising {
            let target_id =
                target_id.expect("shooting initialization requires an interaction target");
            let target = self.expect_entity(target_id, "bow initialization target");
            let (target_position, shooter_position) =
                if order_type == OrderType::ShootingWithBowLeaningOut {
                    (
                        target.element_data().position_map(),
                        shooter.element_data().position_map(),
                    )
                } else {
                    let position = shooter.element_data().position();
                    (
                        bow_shot::bow_target_ground_position(target),
                        MapPoint::new(position.x, position.y),
                    )
                };
            self.expect_entity_mut(shooter_id, "bow initialization owner")
                .element_data_mut()
                .set_direction_goal(crate::position_interface::vector_to_sector_0_to_15_iso(
                    target_position.x - shooter_position.x,
                    target_position.y - shooter_position.y,
                ));
        }
        let frozen = self.actors_frozen();
        let shooter = self.expect_entity_mut(shooter_id, "bow animation owner");
        let progression = if bow_shot::is_shoot_order(order_type)
            && shooter.element_data_mut().sprite.position_iface.turn()
        {
            crate::sprite::FrameProgression::FrozenFirstFrame
        } else {
            crate::sprite::FrameProgression::Default
        };
        let direction = u16::try_from(shooter.element_data().direction())
            .expect("bow shooter direction must be nonnegative");
        let motion = if frozen {
            MotionState::InProgress
        } else {
            shooter.element_data_mut().sprite.perform_action(
                sim,
                Some(expected_order_id),
                order_type,
                direction,
                progression,
                false,
            )
        };
        if bow_shot::is_bow_transition_order(order_type) {
            bow_shot::apply_bow_transition_state_side_effect(shooter, order_type, motion);
            if shooter.is_pc()
                && !script_driven
                && motion == MotionState::Start
                && matches!(
                    order_type,
                    OrderType::TransitionEquipBow | OrderType::TransitionEquipBowAnonymous
                )
            {
                self.set_pc_action_from_message(
                    sim,
                    assets,
                    0,
                    shooter_id,
                    crate::profiles::Action::Bow,
                );
            }
        } else if motion == MotionState::Done {
            let target_id = target_id.expect("bow release requires an interaction target");
            let shoot_mode = match shooter.actor_data().unwrap().action_state {
                ActionState::AimingWithBow => crate::weapons::ShootMode::Normal,
                ActionState::AimingWithBowUp => crate::weapons::ShootMode::Long,
                ActionState::AimingWithBowDown => crate::weapons::ShootMode::Down,
                state => panic!("bow release requires an aiming action, got {state:?}"),
            };
            self.release_bow_arrow(sim, assets, shooter_id, target_id, shoot_mode);
            let shooter = self.expect_entity_mut(shooter_id, "bow owner after release");
            shooter.actor_data_mut().unwrap().action_state = ActionState::AimingWithBow;
            if order_type == OrderType::ShootingWithBowLeaningOut {
                shooter
                    .element_data_mut()
                    .publish_order_posture(Posture::LeaningOut);
            } else if shooter.element_data().posture() != Posture::AnonymousArcher {
                shooter
                    .element_data_mut()
                    .publish_order_posture(Posture::Upright);
            }
            return Some(motion);
        }
        Some(motion)
    }

    fn release_bow_arrow(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        shooter_id: EntityId,
        target_id: EntityId,
        shoot_mode: crate::weapons::ShootMode,
    ) -> Option<EntityId> {
        let shooter = self.expect_entity(shooter_id, "bow release shooter");
        let shooter_direction = shooter.element_data().direction();
        let position = shooter.element_data().position_map();
        let shooter_position = crate::coordinates::WorldPoint3D {
            x: position.x,
            y: position.y,
            z: shooter.position_iface().get_elevation(),
        };
        let Some(sprite_hand_point) =
            bow_shot::bow_sprite_hand_point(shooter, shoot_mode, shooter_direction)
        else {
            tracing::warn!(?shooter_id, "Bow release skipped: missing hand hotspot");
            return None;
        };
        let target = self.expect_entity(target_id, "bow release target");
        let target_pos = target.element_data().position_map();
        let target_point = if target.is_human() {
            target.compute_belt_point()
        } else if target.is_fx_target() {
            target.compute_target_center()
        } else {
            panic!("bow target {target_id:?} is neither human nor target");
        };
        let Some(target_point) = target_point else {
            tracing::warn!(?target_id, "Bow release skipped: missing target hotspot");
            return None;
        };
        let target_forecasted_movement = target.position_iface().get_forecasted_movement();
        let layer = shooter.element_data().layer();
        let trajectory_origin_sector =
            super::ai::ai_view_position_sector(self, shooter.element_data());
        let shooter_is_pc = shooter.kind().is_pc();
        let target_is_fx_target = target.kind().is_fx_target();
        let target_is_human = target.is_human();
        let target_posture = target.element_data().posture();

        // ── Determine shoot mode from action state ───────────
        let flat_shot = bow_shot::is_flat_shot(shoot_mode);
        let mass = bow_shot::arrow_mass(shoot_mode);

        // ── Look up bow profile for damage and hit chance ────
        let Some((bow_profile_idx, shooting_ability)) =
            self.bow_profile_and_ability(assets, shooter_id)
        else {
            tracing::warn!(
                shooter = ?shooter_id,
                "Bow shot release skipped: shooter has no bow profile data"
            );
            return None;
        };

        let Some(bow_profile) = assets.profile_manager.get_bow(bow_profile_idx) else {
            tracing::warn!(
                shooter = ?shooter_id,
                bow_profile_idx,
                "Bow shot release skipped: missing bow profile"
            );
            return None;
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
            shooter_position,
            shoot_mode,
            shooter_direction,
            sprite_hand_point,
        );

        // ── Target belt point ───────────────────────────────
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
        let mut target_point = target_point;
        if target_posture == crate::element::Posture::LeaningOut
            && shoot_mode == crate::weapons::ShootMode::Normal
        {
            let (belt_status, belt_mode) =
                self.can_shoot_with_bow_at_point(assets, shooter_id, target_point, false);
            let belt_failed =
                belt_status != BowTarget::Valid || belt_mode == crate::weapons::ShootMode::Long;
            if belt_failed
                && let Some(eyes) = self
                    .get_entity(target_id)
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
        let target_movement = target_is_human.then_some(target_forecasted_movement);

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

        self.warn_shield_target_of_arrow(sim, assets, shooter_id, target_id);

        // ── Hit chance roll ──────────────────────────────────
        // The original game only applies the bow's hit chance in the
        // human-target branch of bow shooting.
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

        // human-status capacity, not the difficulty-/alcohol-adjusted
        // shooting-ability value used by the hit-chance lookup.
        let bow_skill_capacity = self
            .bow_skill_capacity(assets, shooter_id)
            .unwrap_or_else(|| {
                panic!(
                    "bow shot shooter {:?} is missing its authoritative bow skill capacity",
                    shooter_id
                )
            });

        if target_is_human
            && let Some(bias) =
                bow_shot::roll_hit_and_compute_bias(sim, hit_chance, bow_skill_capacity)
        {
            // Miss — deflect the velocity.
            velocity.x += bias.x;
            velocity.y += bias.y;
            velocity.z += bias.z;
            tracing::debug!(
                shooter = ?shooter_id,
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
            sight_obstacles: obstacle_list,
            water_zones: Some(&assets.environment.water_zones),
        };
        let collision_debug_identity =
            crate::sight_obstacle::projectile_collision_debug_requested().then(|| {
                crate::sight_obstacle::ProjectileCollisionDebugIdentity {
                    frame: self.control.frame_counter,
                    shooter: shooter_id.index(),
                    projectile_creation_order: self.world.next_original_creation_order,
                }
            });
        let capture_collision_debug = collision_debug_identity
            .is_some_and(crate::sight_obstacle::projectile_collision_debug_matches);
        let compute_trajectory = || {
            bow_shot::compute_trajectory_ballistic_with_terminal_impact(
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
            )
        };
        let (
            trajectory,
            terminal_obstacle,
            terminal_impact,
            terminal_lands_in_hole,
            terminal_lands_in_water,
        ) = if capture_collision_debug {
            crate::sight_obstacle::with_projectile_collision_debug_identity(
                collision_debug_identity.expect("matched collision debug has no identity"),
                compute_trajectory,
            )
        } else {
            compute_trajectory()
        };
        let terminal_obstacle_plane =
            bow_shot::terminal_obstacle_plane(terminal_obstacle, obstacle_list);
        let trajectory_end = trajectory.last().map(|tp| tp.position);
        // Trajectory calculation resolves and stores the eventual impact
        // membership before the projectile's explicit pre-add
        // update. It is therefore observable throughout flight, not
        // only after the projectile lands.
        //
        // A terminal impact that classifies as water or hole returns from
        // trajectory calculation *before* the membership block
        // (setting the dive flag and returning; and
        // adding a fall-into-hole trajectory and returning, both
        // before clearing the layer when no obstacle is present).
        // Neither fall-into-hole trajectory creation, projectile-impact
        // handling, nor the dive flag touches
        // layer, sector or obstacle, so such a projectile keeps the
        // clearing the layer, sector, and obstacle that
        // installed for the whole of its fall.
        let terminal_membership =
            terminal_impact && !terminal_lands_in_hole && !terminal_lands_in_water;
        let initial_landing_resolution = terminal_membership.then(|| {
            let end = trajectory_end.expect("terminal impact has no trajectory endpoint");
            if let Some(obstacle) = terminal_obstacle {
                self.world
                    .fast_grid
                    .resolve_projectile_landing_with_obstacle(
                        end.to_map(),
                        Some(obstacle),
                        obstacle_list,
                    )
            } else {
                self.world
                    .fast_grid
                    .resolve_projectile_ground_landing(end.to_map())
            }
        });
        tracing::debug!(
            shooter = ?shooter_id,
            target = ?target_id,
            ?shoot_mode,
            ?bow_point,
            ?target_point,
            ?trajectory_end,
            trajectory_len = trajectory.len(),
            magic_bullet,
            predicted_hit = bow_shot::will_hit_target(&trajectory, bow_point, target_point),
            "Bow shot trajectory computed"
        );
        // Launch-parameter snapshot, on its own target so it can be
        // enabled without the rest of the combat module's chatter:
        // `RUST_LOG=arrow_launch=trace`.
        tracing::trace!(
            target: "arrow_launch",
            frame = self.control.frame_counter,
            shooter = shooter_id.index(),
            target_id = target_id.index(),
            ?shoot_mode,
            shooter_pos = ?shooter_position,
            shooter_dir = shooter_direction,
            hand = ?sprite_hand_point,
            ?bow_point,
            ?target_point,
            target_movement = ?target_movement,
            ?velocity,
            hit_chance,
            trajectory_len = trajectory.len(),
            first_waypoint = ?trajectory.first().map(|tp| tp.position),
            "arrow launch parameters"
        );

        // ── Spawn the arrow ──────────────────────────────────
        let mut arrow = bow_shot::spawn_arrow(bow_shot::SpawnArrowParams {
            shooter: shooter_id,
            bow_point,
            trajectory_origin: crate::coordinates::MapPoint {
                x: shooter_position.x,
                y: shooter_position.y,
            },
            target: target_id,
            target_pos: target_pos,
            trajectory,
            damage,
            layer,
            lands_in_hole: terminal_lands_in_hole,
            initial_velocity: velocity,
        });
        let diagnostic_identity = arrow_publication_debug_gate().enabled().then(|| {
            (
                self.control
                    .frame_counter
                    .checked_add(1)
                    .expect("frame counter overflow while recording arrow publication diagnostic"),
                self.world.original_creation_order(shooter_id),
            )
        });
        if let Some((frame_after, shooter_creation_order)) = diagnostic_identity {
            record_arrow_publication_debug(
                "after_spawn_arrow",
                frame_after,
                shooter_creation_order,
                None,
                &arrow,
            );
        }
        let Entity::Projectile(arrow_projectile) = &mut arrow else {
            panic!("spawn_arrow returned a non-projectile entity");
        };
        // Trajectory calculation retains the dive flag across a later ricochet
        // trajectory. Its terminal update must therefore still take
        // the water-return path even when the recomputed fall ends dry.
        arrow_projectile.projectile.dive = terminal_lands_in_water;
        set_projectile_trajectory_origin(
            &mut arrow_projectile.projectile,
            trajectory_origin_sector,
            layer,
        );
        let arrow_id = self.add_entity(arrow);
        if capture_collision_debug {
            crate::sight_obstacle::validate_projectile_collision_debug_spawn(
                arrow_id,
                self.world.original_creation_order(arrow_id),
            );
        }
        let diagnostic_projectile_creation_order =
            diagnostic_identity.map(|_| self.world.original_creation_order(arrow_id));
        if let Some(((frame_after, shooter_creation_order), projectile_creation_order)) =
            diagnostic_identity.zip(diagnostic_projectile_creation_order)
        {
            record_arrow_publication_debug(
                "after_add_entity",
                frame_after,
                shooter_creation_order,
                Some(projectile_creation_order),
                self.world
                    .entities
                    .get(arrow_id)
                    .expect("new arrow missing immediately after add_entity"),
            );
        }
        if let Some(resolution) = initial_landing_resolution {
            if projectile_landing_debug_matches(self.control.frame_counter, shooter_id, arrow_id) {
                let obstacle_list = self.sight_obstacles(assets);
                let obstacle = terminal_obstacle.map(|handle| {
                    let index = usize::from(handle);
                    let obstacle = obstacle_list.get(index).unwrap_or_else(|| {
                        panic!("diagnostic terminal obstacle {index} disappeared")
                    });
                    (
                        index,
                        obstacle_list.is_active(index),
                        obstacle.is_projection_area(),
                        obstacle.projection_area_ref(),
                        obstacle.contains_point_projection(
                            trajectory_end
                                .expect("terminal impact lost its endpoint")
                                .to_map(),
                        ),
                    )
                });
                let landing = trajectory_end
                    .expect("terminal impact lost its endpoint")
                    .to_map();
                let terminal_obstacle_ref = terminal_obstacle.map(|handle| {
                    let index = usize::from(handle);
                    obstacle_list.get(index).unwrap_or_else(|| {
                        panic!("diagnostic terminal obstacle {index} disappeared")
                    })
                });
                let material_inputs = terminal_obstacle_ref.map(|obstacle| {
                    let sectors = obstacle
                        .material_sectors
                        .iter()
                        .enumerate()
                        .map(|(index, sector)| {
                            (
                                index,
                                sector.material,
                                sector.bounding_box.contains_point(landing),
                                sector.contains(landing),
                            )
                        })
                        .collect::<Vec<_>>();
                    (obstacle.material, sectors)
                });
                let ground_material_inputs = assets
                    .environment
                    .water_zones
                    .zones
                    .iter()
                    .enumerate()
                    .map(|(index, zone)| {
                        (
                            index,
                            zone.material,
                            zone.bounding_box.contains_point(landing),
                            zone.contains(landing),
                        )
                    })
                    .collect::<Vec<_>>();
                let scoped_material = crate::water_zones::determine_water_hole_scoped(
                    &assets.environment.water_zones,
                    terminal_obstacle_ref,
                    landing,
                )
                .map(|resolved| (resolved.material, resolved.sector_points.map(<[_]>::len)));
                let candidate_layer = obstacle
                    .filter(|(_, active, projection, topology, _)| {
                        *active && *projection && topology.is_some()
                    })
                    .map_or(0, |(_, _, _, topology, _)| {
                        topology.expect("filtered projection topology").layer.get()
                    });
                let candidates = if self.world.fast_grid.is_inside_grid_point(landing) {
                    let block = self
                        .world
                        .fast_grid
                        .get_block_index(landing, candidate_layer);
                    self.world
                        .fast_grid
                        .get_sectors_at_block(block, crate::sector::SectorType::MOTION)
                        .into_iter()
                        .map(|(index, sector)| {
                            (
                                index,
                                i16::from(sector.sector_number),
                                sector.sector_type.is_area(),
                                sector.bounding_box.contains_point(landing),
                                sector.contains_point(landing),
                            )
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                eprintln!(
                    "PARITY_PROJECTILE_LANDING frame={} shooter={} projectile={} end_bits=[{:#010x},{:#010x},{:#010x}] landing_bits=[{:#010x},{:#010x}] terminal_impact={} lands_in_water={} lands_in_hole={} terminal_obstacle={obstacle:?} material_inputs={material_inputs:?} ground_material_inputs={ground_material_inputs:?} scoped_material={scoped_material:?} candidate_layer={} candidates={candidates:?} result={resolution:?}",
                    self.control.frame_counter,
                    shooter_id.index(),
                    arrow_id.index(),
                    trajectory_end
                        .expect("terminal impact lost its endpoint")
                        .x
                        .to_bits(),
                    trajectory_end
                        .expect("terminal impact lost its endpoint")
                        .y
                        .to_bits(),
                    trajectory_end
                        .expect("terminal impact lost its endpoint")
                        .z
                        .to_bits(),
                    landing.x.to_bits(),
                    landing.y.to_bits(),
                    terminal_impact,
                    terminal_lands_in_water,
                    terminal_lands_in_hole,
                    candidate_layer,
                );
            }
            let entity = self
                .world
                .entities
                .get_mut(arrow_id)
                .expect("newly added arrow vanished before landing-state initialization");
            let element = entity.element_data_mut();
            element.set_sector(resolution.sector);
            if resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle {
                element.set_layer(
                    resolution
                        .layer
                        .expect("authorized projectile landing has no resolved layer")
                        .get(),
                );
            }
        }
        if terminal_membership {
            // obstacle assignment lives inside the same membership
            // block the water/hole `return`s skip
            // for flying projectiles, so a projectile that ends in
            // water or a hole stays bound to no obstacle.
            let element = self
                .world
                .entities
                .get_mut(arrow_id)
                .expect("newly added arrow vanished before obstacle binding")
                .element_data_mut();
            bow_shot::bind_trajectory_obstacle(element, terminal_obstacle, terminal_obstacle_plane);
        }
        // Hydrate the arrow's sprite from the accessory registry so
        // the flying arrow renders its proper sprite instead of the
        // colored-rect fallback.
        self.attach_accessory_sprite(assets, arrow_id);
        if let Some(((frame_after, shooter_creation_order), projectile_creation_order)) =
            diagnostic_identity.zip(diagnostic_projectile_creation_order)
        {
            record_arrow_publication_debug(
                "after_attach_accessory_sprite",
                frame_after,
                shooter_creation_order,
                Some(projectile_creation_order),
                self.world
                    .entities
                    .get(arrow_id)
                    .expect("new arrow missing after accessory sprite attachment"),
            );
        }
        self.tick_new_projectile_once(sim, assets, arrow_id);
        if let Some(((frame_after, shooter_creation_order), projectile_creation_order)) =
            diagnostic_identity.zip(diagnostic_projectile_creation_order)
        {
            record_arrow_publication_debug(
                "after_first_hourglass",
                frame_after,
                shooter_creation_order,
                Some(projectile_creation_order),
                self.world
                    .entities
                    .get(arrow_id)
                    .expect("new arrow missing after its first hourglass"),
            );
        }

        tracing::debug!(
            shooter = ?shooter_id,
            target = ?target_id,
            arrow = ?arrow_id,
            ?shoot_mode,
            damage,
            ?hit_chance,
            "Arrow spawned from bow shot"
        );

        // ── Decrement bow ammo after shot ───────────────────
        // Decrement ammo by 1; disable the bow action if ammo hits 0.
        self.decrement_bow_ammo(assets, shooter_id);

        Some(arrow_id)
    }

    /// Put an arrow into non-shield falling state — the "armor ricochet"
    /// branch: inverse sector (xor 8), `y * 10`, z velocity zero.  Used
    /// when a PC/Soldier is hit but not hurtable (same-camp friendly fire
    /// or a successful piercing-protection roll).
    pub(super) fn start_arrow_ricochet(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        arrow_id: EntityId,
    ) {
        let (entities, sight_obstacles, fast_find_grid, _) =
            self.world.entities_mut_with_sight(assets);
        let obstacle_check = bow_shot::TrajectoryObstacleCheck {
            fast_find_grid,
            sight_obstacles,
            water_zones: Some(&assets.environment.water_zones),
        };
        let Some(entity) = entities.get_mut(arrow_id) else {
            return;
        };
        let Entity::Projectile(proj) = entity else {
            return;
        };

        if bow_shot::make_arrow_falling_down(proj, false, Some(&obstacle_check)) {
            // The nested update's retirement result does not retire its caller.
            self.finish_projectile_landing(sim, assets, arrow_id);
        }
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
    pub(super) fn classify_arrow_hit(
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
        let shooter_camp = shooter.is_human().then(|| shooter.camp());
        let victim_camp = victim.is_human().then(|| victim.camp());
        let same_camp = matches!(
            (shooter_camp, victim_camp),
            (Some(sc), Some(vc)) if self.camps_are_allied(sc, vc),
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
        // NPC shooters always keep the retail protection. PC shooters use the
        // resolved difficulty rule; Hard and Legendary disable it, preserving
        // the retail Hard civilian-friendly-fire behavior.
        let apply_hurtable_filter = if shooter_is_npc {
            true
        } else if shooter_is_pc {
            sim.config()
                .difficulty
                .rules()
                .protect_allies_from_pc_arrows
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
        let piercing_roll_passed = if hurtable_base && victim_is_pc_or_soldier {
            // `(rand() % 101) > protection` runs even when protection is
            // 0 — gives a 1/101 ricochet for the exact `roll == 0` case,
            // and keeps RNG consumption consistent with the
            // piercing-protection > 0 path. Missing weapon profile data
            // is invalid actor state; the original game requires a weapon, so do
            // not invent an unconditional-damage fallback.
            match piercing_protection {
                Some(protection) => {
                    let roll = crate::sim_rng::u32(
                        sim,
                        crate::sim_rng::RngSite::ArrowPiercingProtection,
                        0..101,
                    );
                    Some(roll > protection as u32)
                }
                None => {
                    tracing::warn!(
                        ?victim_id,
                        "arrow hurtability: missing HtH weapon profile; treating victim as protected",
                    );
                    Some(false)
                }
            }
        } else {
            None
        };
        let hurtable =
            resolve_arrow_hurtable(hurtable_base, victim_is_pc_or_soldier, piercing_roll_passed);

        // ── (F) Outcome dispatch ────────────────────────────────────
        // Hurtable → Damage.
        // !Hurtable + victim is PC or Soldier → Ricochet. This is
        // silent for arrows because falling setup marks them as falling
        // before impact-effect selection.
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
    /// when the live arrow count reaches zero.
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
    pub(super) fn decrement_ability_ammo(
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
        // speech, so we always speak here except on the Sherwood hub map.
        if remaining == 0 {
            self.disable_pc_action(assets, actor_id, action);
            if !self.is_sherwood(&assets.profile_manager) {
                self.hero_speaking(assets, actor_id, crate::engine::melee::HERO_OUT_OF_AMMO);
            }
        }
    }

    /// Consume one Stoeckel ration through the original game's ammunition update.
    ///
    /// Eating is deliberately different from every ammunition-decrement
    /// ability: the player-character eating action computes the remaining
    /// count and updates the remaining food or drink amount. That
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
            // Ammo-amount assignment also re-enables a non-empty slot. This matters
            // for a restored or temporarily reconciled status whose widget
            // mask was stale when the eating animation completed.
            self.enable_pc_action(assets, actor_id, action);
        }
    }

    /// Spawn an apple / stone projectile at the end of the throw
    /// animation.  Take the thrower's hand point and the victim's eyes
    /// point (or FX-target centre), compute a ballistic trajectory, and
    /// register the projectile.
    pub(super) fn on_throw_projectile_done(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        target: Option<EntityId>,
        action: crate::profiles::Action,
        object_type: crate::element::ObjectType,
    ) {
        let target_id = target.expect("apple/stone throw selected without its required target");
        let (throw_pos, layer) = self.projectile_throw_origin(actor_id, "on_throw_projectile_done");
        let thrower = self
            .get_entity(actor_id)
            .unwrap_or_else(|| panic!("projectile thrower {actor_id:?} disappeared before Done"));
        let trajectory_origin_sector =
            super::ai::ai_view_position_sector(self, thrower.element_data());
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
            sight_obstacles: self.sight_obstacles(assets),
            water_zones: Some(&assets.environment.water_zones),
        };
        let mut projectile = match object_type {
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
        let Entity::Projectile(projectile_data) = &mut projectile else {
            panic!("apple/stone spawn returned a non-projectile entity");
        };
        set_projectile_trajectory_origin(
            &mut projectile_data.projectile,
            trajectory_origin_sector,
            layer,
        );
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

    /// Spawn the ground-targeted stone extension after the original throw
    /// animation completes. The projectile carries the one-shot noise latch,
    /// so saving or rolling back mid-flight cannot lose or duplicate the
    /// impact stimulus.
    pub(super) fn on_throw_noise_distraction_done(
        &mut self,
        assets: &LevelAssets,
        actor_id: EntityId,
        target: crate::coordinates::WorldPoint3D,
    ) {
        // Admission was validated when the sequence began. Disabling the
        // option during the throw animation prevents future throws but does
        // not erase this already-authoritative command.
        let (throw_pos, layer) =
            self.projectile_throw_origin(actor_id, "ThrowNoiseDistractionDone");
        let thrower = self.get_entity(actor_id).unwrap_or_else(|| {
            panic!("noise-distraction thrower {actor_id:?} disappeared before Done")
        });
        let trajectory_origin_sector =
            super::ai::ai_view_position_sector(self, thrower.element_data());
        let obstacle_check = crate::bow_shot::TrajectoryObstacleCheck {
            fast_find_grid: &self.world.fast_grid,
            sight_obstacles: self.sight_obstacles(assets),
            water_zones: Some(&assets.environment.water_zones),
        };
        let mut projectile = crate::bow_shot::spawn_stone(
            actor_id,
            throw_pos,
            target,
            None,
            None,
            layer,
            Some(&obstacle_check),
        );
        let Entity::Projectile(projectile_data) = &mut projectile else {
            panic!("ground stone spawn returned a non-projectile entity");
        };
        projectile_data.projectile.noise_distraction = true;
        set_projectile_trajectory_origin(
            &mut projectile_data.projectile,
            trajectory_origin_sector,
            layer,
        );
        let projectile_id = self.add_entity(projectile);
        self.attach_accessory_sprite(assets, projectile_id);
        self.decrement_ability_ammo(assets, actor_id, crate::profiles::Action::Stone);
        tracing::debug!(
            actor = ?actor_id,
            projectile = ?projectile_id,
            x = target.x,
            y = target.y,
            "ground noise-distraction stone spawned"
        );
    }

    pub(super) fn projectile_throw_origin(
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
            // campaign status block, matching the original game's status reference.
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
    pub(crate) fn increase_ammo_and_enable(
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
    pub(crate) fn handle_bonus_pickup(
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

#[cfg(test)]
fn soldier_shield_dimensions(
    profile_manager: &crate::profiles::ProfileManager,
    profile_index: crate::profiles::SoldierProfileIdx,
) -> Option<(u16, u16)> {
    profile_manager
        .get_soldier(profile_index)
        .and_then(|p| profile_manager.get_hth_weapon(p.hth_weapon_id))
        .map(|w| (w.shield_width, w.shield_height))
}

fn set_projectile_trajectory_origin(
    projectile: &mut crate::element::ProjectileData,
    sector: Option<crate::position_interface::SectorHandle>,
    layer: u16,
) {
    projectile.trajectory_origin_sector = sector.map(crate::position_interface::SectorHandle::get);
    projectile.trajectory_origin_sector_index = sector.and_then(|sector| sector.arena_index());
    projectile.trajectory_origin_layer = crate::position_interface::Layer::new(layer);
}

fn projectile_trajectory_origin_sector(
    projectile: &crate::element::ProjectileData,
) -> Option<crate::position_interface::SectorHandle> {
    match (
        projectile.trajectory_origin_sector,
        projectile.trajectory_origin_sector_index,
    ) {
        (None, None) => None,
        (Some(public), index) => crate::position_interface::SectorHandle::new(public)
            .map(|sector| index.map_or(sector, |index| sector.with_arena_index(index))),
        (None, Some(index)) => panic!(
            "projectile trajectory origin retains exact sector index {index:?} without its public sector number"
        ),
    }
}

pub(super) fn projectile_trajectory_origin(entity: &Entity) -> Option<crate::ai::Position> {
    match entity {
        Entity::Projectile(p) => {
            let sector = projectile_trajectory_origin_sector(&p.projectile);
            let layer = p.projectile.trajectory_origin_layer?;
            Some(crate::ai::Position {
                x: p.projectile.start_of_trajectory_x,
                y: p.projectile.start_of_trajectory_y,
                sector,
                level: layer.get(),
            })
        }
        _ => None,
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
        let Some(entity) = self.get_entity(shooter_id) else {
            return;
        };
        let Entity::Pc(pc) = entity else {
            return; // Only PCs get XP
        };
        let character_idx = self
            .pc_description_index_for_pc_data(&pc.pc)
            .unwrap_or_else(|| {
                panic!(
                    "bow-kill XP shooter {shooter_id:?} has no valid campaign character identity"
                )
            });

        let capacity_increased = self.mission_domain.campaign.add_pc_experience(
            character_idx,
            crate::pc_status::SkillName::Bow,
            bow_shot::BOW_KILL_EXPERIENCE_POINTS,
        );
        if capacity_increased {
            self.add_campaign_value(
                crate::campaign::CampaignValue::Score,
                crate::pc_status::PC_ADDITIONAL_CAPACITY_POINTS,
            );
        }
        tracing::debug!(
            shooter = ?shooter_id,
            xp = bow_shot::BOW_KILL_EXPERIENCE_POINTS,
            capacity_increased,
            "Bow kill XP awarded"
        );
    }

    /// Consume a ground-stone's one-shot impact latch and synchronously feed
    /// the resulting authored noise into the existing AI hearing pipeline.
    /// Returns whether this impact was the distraction terminal, allowing the
    /// caller to apply the independently configurable feedback gate.
    pub(super) fn emit_noise_distraction_impact(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        projectile_id: EntityId,
        impact: MapPoint,
    ) -> bool {
        let (layer, elevation) = {
            let entity = self.get_entity_mut(projectile_id).unwrap_or_else(|| {
                panic!("noise-distraction impact projectile {projectile_id:?} is missing")
            });
            let Entity::Projectile(projectile) = entity else {
                panic!("noise-distraction impact id {projectile_id:?} is not a projectile");
            };
            if !projectile.projectile.noise_distraction {
                return false;
            }
            projectile.projectile.noise_distraction = false;
            (
                projectile.element.optional_layer(),
                projectile.element.sprite.position_iface.get_elevation() as u16,
            )
        };

        self.broadcast_noise_synchronously(
            sim,
            assets,
            crate::ai::NoiseType::Distraction,
            impact,
            layer,
            crate::parameters_ai::NOISE_VOLUME_DISTRACTION as u16,
            elevation,
            Some(projectile_id),
        );
        true
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
        // Successful projectile impact handling rewinds to the
        // position snapshotted at movement start, stops flight, and immediately
        // deletes the trajectory. Settling both position representations is
        // observable by the following parity snapshot and lets the subsequent
        // arrow refresh retire the stationary arrow.
        p.element.set_position(old_pos);
        p.element.sprite.position_iface.new_move();
        // The original game immediately recomputes all position projections after the rewind.
        // Retain the already-published projectile increment while restoring
        // all three position-cache validity bits.
        p.element
            .finish_projectile_position_update(p.projectile.velocity_increment);
        p.projectile.trajectory.clear();
    }

    /// Apple lands on a human.  Apples deal no damage; they only
    /// affect soldiers via the apple-smell AI hook.
    pub(super) fn on_apple_hit_human(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        apple: EntityId,
        victim: EntityId,
    ) {
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
        self.dispatch_event_apple(sim, assets, victim, trajectory_origin);
    }

    /// Stone lands on a human.  Non-VIPs that fail the
    /// piercing-protection roll take `STONE_DAMAGE`; NPCs that dodge
    /// (VIP or armored soldier) receive an EventApple stimulus.
    pub(super) fn on_stone_hit_human(
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
                sim,
                assets,
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
            self.dispatch_event_apple(sim, assets, victim, trajectory_origin);
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
            if !tracks {
                return;
            }
            let Some(target) = ai.primary_target else {
                return;
            };
            target
        };
        let my_pos = match self.get_entity(npc_id) {
            Some(e) => e.ground_position(),
            None => panic!("tracking soldier {} disappeared", npc_id.index()),
        };
        let target_pos =
            match self.get_entity(self.expect_entity_id_for_index(
                target_handle.get(),
                "update_bow_defense target handle",
            )) {
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
    /// * Otherwise, when the resolved difficulty enables auto-heal, use its
    ///   configured cadence while the PC is neither sword-fighting nor in
    ///   coma. The Easy preset remains exactly once every 100 frames.
    ///
    /// The shared human prelude (concussion decrement, tiredness
    /// recovery, produced-noise refresh) is handled by
    /// [`Self::tick_concussion_healing`], [`Self::tick_tiredness`],
    /// and the PC noise bookkeeping in `engine/ai.rs`; this tick only
    /// covers the PC-specific heal branches.
    /// Apply the PC-specific update tail to one PC.
    pub(super) fn tick_pc_auto_heal_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        pc_id: EntityId,
    ) {
        let auto_heal_interval = sim.config().difficulty.rules().pc_auto_heal_interval_frames;
        let tick_auto_heal = auto_heal_interval != 0
            && self
                .control
                .frame_counter
                .is_multiple_of(u32::from(auto_heal_interval));

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
        } else if tick_auto_heal && !swordfighting {
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
    fn dispatch_event_apple(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim: EntityId,
        origin: crate::ai::Position,
    ) {
        self.execute_ai_callback(
            sim,
            assets,
            victim,
            &crate::ai::Stimulus::with_position(crate::ai::StimulusType::EventApple, origin),
        );
    }

    /// Dispatch an EventGetArrow stimulus at the arrow's trajectory
    /// origin — wakes the struck NPC and seeds the search toward the
    /// shot origin.
    pub(super) fn dispatch_event_get_arrow(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim: EntityId,
        origin: crate::ai::Position,
    ) {
        // Arrow impact calls the NPC's AI directly after
        // sequence-element launch and before returning to the projectile
        // update. Merely appending this to the deferred detection FIFO
        // makes the outcome depend on whether the NPC's creation-order slot
        // is before or after the projectile.  Run the one Think inline while
        // retaining older deferred stimuli ahead of work emitted here.
        self.execute_ai_callback(
            sim,
            assets,
            victim,
            &crate::ai::Stimulus::with_position(crate::ai::StimulusType::EventGetArrow, origin),
        );
    }

    /// If the arrow's landing position is inside a water or hole zone,
    /// spawn the splash titbit, broadcast a PLOUF noise, and play the
    /// plouf impact sound (FX 470).
    pub(super) fn maybe_splash_on_landing(
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
        let layer = elem.optional_layer();
        // Trajectory calculation deliberately leaves water/hole projectiles at
        // Original's raw 0xffff "no layer" sentinel. Projectile ticking
        // still passes that value to the Plouf titbit before despawning it
        // during projectile impact handling.
        let raw_layer = layer.map_or(u16::MAX, crate::position_interface::Layer::get);
        let (object_type, pre_flagged_disappear, pre_flagged_dive) = match proj_entity {
            Entity::Projectile(p) => (
                p.object.object_type,
                p.projectile.disappear,
                p.projectile.dive,
            ),
            _ => return,
        };

        // Pre-flagged hole landing: the trajectory builder identified
        // the terminal waypoint as inside a hole polygon.  Skip the
        // water-zone lookup (which can miss when the extended final
        // point sits on the polygon boundary) and drop into the silent
        // hole-disappear branch directly.
        if pre_flagged_disappear && !pre_flagged_dive {
            return;
        }

        // Original scopes material lookup to the exact obstacle returned by
        // the terminal trajectory raycast. Only a bare-ground impact scans
        // global sound sectors. This prevents a raised dry platform from
        // inheriting a projected ground-level water/hole polygon.
        let landing_map = position_map;
        let obstacle_handle = elem.obstacle_index();
        let obstacles = self.sight_obstacles(assets);
        let landing_obstacle = obstacle_handle.map(|handle| {
            obstacles
                .get(usize::from(handle))
                .unwrap_or_else(|| panic!("projectile landing obstacle {handle} disappeared"))
        });
        // Trajectory calculation stores the dive flag at the trajectory that first found
        // water and does not clear it when falling motion later recomputes a
        // dry ricochet. The terminal update still emits the splash at the final
        // position in that case, so the retained flag outranks a fresh lookup.
        let resolved_material = if pre_flagged_dive {
            Some(crate::sound_cache::Material::Water)
        } else {
            crate::water_zones::determine_water_hole_scoped(
                &assets.environment.water_zones,
                landing_obstacle,
                landing_map,
            )
            .map(|resolution| resolution.material)
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
            raw_layer,
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

        // Plouf impact sound (FX 470).
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Fx {
                fx_id: 470,
                position: position_map,
                material: None,
            });
    }

    // ─── Shield obstacle update ─────────────────────────────────

    /// Apply one original-game shield update to the owner's retained box.
    pub(super) fn refresh_retained_shield_obstacle(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
    ) {
        let entity = self
            .get_entity_mut(owner)
            .unwrap_or_else(|| panic!("shield refresh owner {owner:?} disappeared"));
        crate::bow_shot::refresh_retained_shield_obstacle(entity, &assets.profile_manager);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        projectile_trajectory_origin, projectile_trajectory_origin_sector,
        set_projectile_trajectory_origin, soldier_piercing_protection, soldier_shield_dimensions,
    };
    use crate::element::{
        ActionState, ElementData, ElementKind, ElementProjectile, Entity, EntityId, ObjectData,
        Posture, ProjectileData,
    };
    use crate::engine::test_support::actors::TestActor;
    use crate::engine::{EngineInner, LevelAssets};
    use crate::order::OrderType;
    use crate::profiles::{HtHWeaponProfile, ProfileManager, SoldierProfile, SoldierProfileIdx};
    use crate::sequence::{SequenceElementData, SequenceState};
    use crate::sight_obstacle::{ObstaclePoint, SightObstacle};
    use std::sync::Arc;

    #[test]
    fn distraction_projectile_latch_survives_serialization_and_emits_once() {
        std::thread::Builder::new()
            .name("distraction-projectile-latch-roundtrip".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(distraction_projectile_latch_survives_serialization_and_emits_once_inner)
            .expect("spawn large-stack projectile round-trip regression")
            .join()
            .expect("large-stack projectile round-trip regression panicked");
    }

    fn distraction_projectile_latch_survives_serialization_and_emits_once_inner() {
        let mut engine = EngineInner::new();
        let mut projectile = Entity::Projectile(ElementProjectile {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectProjectile;
                initial_element
            },
            object: ObjectData {
                object_type: crate::element::ObjectType::Stone,
                ..Default::default()
            },
            projectile: ProjectileData {
                noise_distraction: true,
                ..Default::default()
            },
        });
        projectile.element_data_mut().set_layer(2);

        let encoded = bitcode::encode(&projectile);
        let restored: Entity = bitcode::decode(&encoded).expect("decode distraction projectile");
        assert!(matches!(
            &restored,
            Entity::Projectile(projectile) if projectile.projectile.noise_distraction
        ));

        let projectile_id = engine.add_test_entity(restored);
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();
        let impact = crate::coordinates::MapPoint::new(80.0, 120.0);
        assert!(engine.emit_noise_distraction_impact(&sim, &assets, projectile_id, impact));
        assert!(!engine.emit_noise_distraction_impact(&sim, &assets, projectile_id, impact));
    }

    #[test]
    fn water_splash_accepts_original_no_layer_sentinel() {
        let mut engine = EngineInner::new();
        let mut element = {
            let mut initial_element = ElementData::default();
            initial_element.active = true;
            initial_element.kind = ElementKind::ObjectProjectile;
            initial_element
        };
        element.clear_layer();
        element.set_position(crate::coordinates::WorldPoint3D::new(80.0, 120.0, 2.0));
        let projectile_id = engine.add_test_entity(Entity::Projectile(ElementProjectile {
            element,
            object: ObjectData {
                object_type: crate::element::ObjectType::Arrow,
                ..Default::default()
            },
            projectile: ProjectileData {
                dive: true,
                ..Default::default()
            },
        }));
        engine.elem_mut(projectile_id).clear_layer();

        engine.maybe_splash_on_landing(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            projectile_id,
        );

        let splash = engine
            .feedback
            .titbit_manager
            .titbits()
            .iter()
            .find(|titbit| titbit.kind == crate::titbit::TitbitKind::Plouf)
            .expect("water landing must emit a Plouf titbit");
        // The original game normalizes an unowned raw -1 hint layer to layer 0
        // during the effect update.
        assert_eq!(splash.layer, 0);
    }

    fn purse_publication_assets() -> LevelAssets {
        use crate::element::{Animation, ObjectType};
        use crate::sprite::Sprite;
        use crate::sprite_script::SpriteScript;

        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[Animation::ObjectFlying as usize] = 16;
        let script = SpriteScript {
            action_id: Animation::ObjectFlying as u16,
            action_done: 4,
            frame_ids: vec![1, 2, 3, 4, 5],
            delays: vec![0; 5],
            distances: vec![0; 5],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 5],
            sound_ids: vec![0; 5],
            ..Default::default()
        };
        let mut assets = LevelAssets::new();
        let prototype = Sprite::new(Arc::new(vec![script; 17]), Arc::new(conversion));
        assets
            .accessory_sprite_prototypes
            .insert(ObjectType::Purse, prototype.clone());
        assets
            .accessory_sprite_prototypes
            .insert(ObjectType::Coin, prototype);
        assets
    }

    #[test]
    fn purse_prepublication_hourglass_preserves_origin_material_and_next_tick_edge() {
        use crate::coordinates::{MapPoint, WorldPoint3D};
        use crate::element::{Entity, GameMaterial, TrajectoryPointRuntime};
        use crate::position_interface::SectorHandle;

        let sim = crate::sim_rng::test_context();
        let assets = purse_publication_assets();
        let mut engine = EngineInner::new();
        let mut thrower = TestActor::pc(Posture::Upright)
            .action_state(ActionState::Waiting)
            .build();
        thrower
            .element_data_mut()
            .set_position_map(MapPoint::new(400.0, 500.0));
        thrower.element_data_mut().set_layer(2);
        thrower.element_data_mut().set_sector(SectorHandle::new(7));
        let thrower = engine.add_test_entity(thrower);
        let start = WorldPoint3D::new(64.0, 64.0, 20.0);
        let target = WorldPoint3D::new(200.0, 64.0, 0.0);
        let mut entity = crate::bow_shot::spawn_purse(thrower, start, target, 2, None);
        let Entity::Projectile(purse) = &mut entity else {
            unreachable!()
        };
        purse.projectile.trajectory_runtime = vec![
            TrajectoryPointRuntime {
                bounce: false,
                material: GameMaterial::Stone.as_u32(),
            };
            purse.projectile.trajectory.len()
        ];

        let purse_id = engine.publish_new_purse(&sim, &assets, thrower, entity);
        let Some(Entity::Projectile(purse)) = engine.get_entity(purse_id) else {
            panic!("published purse disappeared")
        };
        assert_eq!(purse.element.direction(), 0);
        assert_eq!(purse.element.sprite.position_iface.old_position(), start);
        assert_eq!(purse.element.material(), GameMaterial::Stone);
        assert_eq!(purse.element.sprite.current_row, 16);
        assert_eq!(purse.element.sprite.current_frame, 2);
        assert_eq!(purse.projectile.frame_count, 1);
        assert_eq!(purse.projectile.start_of_trajectory_x, 400.0);
        assert_eq!(purse.projectile.start_of_trajectory_y, 500.0);
        assert_eq!(purse.projectile.trajectory_origin_sector, Some(7));
        assert_eq!(
            purse.projectile.trajectory_origin_layer,
            crate::position_interface::Layer::new(2)
        );
        let after_prime = purse.element.position();
        engine.tick_projectile_or_net_hourglass(&sim, &assets, purse_id);
        let Some(Entity::Projectile(purse)) = engine.get_entity(purse_id) else {
            panic!("published purse disappeared")
        };
        assert_eq!(purse.projectile.frame_count, 2);
        assert_eq!(
            purse.element.sprite.position_iface.old_position(),
            after_prime
        );
    }

    #[test]
    fn purse_prepublication_empty_and_one_step_trajectories_are_not_double_primed() {
        use crate::coordinates::WorldPoint3D;
        use crate::element::{Entity, TrajectoryPoint};

        let sim = crate::sim_rng::test_context();
        let assets = purse_publication_assets();
        let mut engine = EngineInner::new();
        let mut unplaced_thrower = TestActor::pc(Posture::Upright)
            .action_state(ActionState::Waiting)
            .build();
        unplaced_thrower.element_data_mut().clear_layer();
        unplaced_thrower.element_data_mut().set_sector(None);
        let thrower = engine.add_test_entity(unplaced_thrower);
        let start = WorldPoint3D::new(20.0, 30.0, 10.0);

        let mut empty = crate::bow_shot::spawn_purse(thrower, start, start, 0, None);
        let Entity::Projectile(empty_purse) = &mut empty else {
            unreachable!()
        };
        empty_purse.projectile.trajectory.clear();
        empty_purse.projectile.trajectory_runtime.clear();
        let empty_id = engine.publish_new_purse(&sim, &assets, thrower, empty);
        let Some(Entity::Projectile(empty_purse)) = engine.get_entity(empty_id) else {
            panic!("published empty purse disappeared")
        };
        assert!(!empty_purse.element.active);
        assert_eq!(
            empty_purse.projectile.purse.child_coins.len(),
            usize::from(crate::bow_shot::NUMBER_OF_COINS_IN_PURSE)
        );
        assert!(
            empty_purse
                .projectile
                .purse
                .child_coins
                .iter()
                .all(|child| child.index() < empty_id.index()),
            "Original adds every burst coin before the inactive purse"
        );
        let purse_creation = engine.original_creation_order(empty_id);
        for &child in &empty_purse.projectile.purse.child_coins {
            let Some(Entity::Projectile(coin)) = engine.get_entity(child) else {
                panic!("purse child {child} is not a coin projectile")
            };
            assert_eq!(coin.projectile.purse.source_purse, Some(empty_id));
            assert_eq!(
                coin.object.animation,
                crate::element::Animation::ObjectFlying
            );
            assert_eq!(coin.element.sprite.current_row, 16);
            assert_eq!(coin.element.sprite.current_frame, 2);
            assert_eq!(coin.element.sprite.position_iface.old_position(), start);
            assert_eq!(coin.projectile.start_of_trajectory_x, start.x);
            assert_eq!(coin.projectile.start_of_trajectory_y, start.y - start.z);
            assert_eq!(coin.projectile.trajectory_origin_sector, None);
            assert_eq!(coin.projectile.trajectory_origin_layer, None);
            assert_eq!(coin.element.sector(), None);
            assert_eq!(coin.element.optional_layer(), None);
            assert!(
                purse_creation < engine.original_creation_order(child),
                "purse constructor identity must precede child coin constructors"
            );
        }
        assert_eq!(empty_purse.element.optional_layer(), None);
        assert_eq!(empty_purse.element.sector(), None);
        assert_eq!(empty_purse.projectile.trajectory_origin_layer, None);
        assert_eq!(empty_purse.projectile.trajectory_origin_sector, None);

        let endpoint = WorldPoint3D::new(24.0, 36.0, 8.0);
        let mut one = crate::bow_shot::spawn_purse(thrower, start, endpoint, 0, None);
        let Entity::Projectile(one_purse) = &mut one else {
            unreachable!()
        };
        one_purse.projectile.trajectory = vec![TrajectoryPoint {
            position: endpoint,
            time: 1,
        }];
        one_purse.projectile.trajectory_runtime.clear();
        let one_id = engine.publish_new_purse(&sim, &assets, thrower, one);
        let Some(Entity::Projectile(one_purse)) = engine.get_entity(one_id) else {
            panic!("published one-step purse disappeared")
        };
        assert_eq!(one_purse.element.position(), endpoint);
        assert_eq!(one_purse.projectile.frame_count, 1);
        assert!(one_purse.projectile.trajectory.is_empty());
    }

    #[test]
    fn purse_prepublication_water_and_hole_exhaustion_do_not_burst() {
        use crate::coordinates::WorldPoint3D;
        use crate::element::{Entity, GameMaterial};

        for (material, dive, disappear) in [
            (GameMaterial::Water, true, false),
            (GameMaterial::Hole, false, true),
        ] {
            let sim = crate::sim_rng::test_context();
            let assets = purse_publication_assets();
            let mut engine = EngineInner::new();
            let thrower = engine.add_test_entity(
                TestActor::pc(Posture::Upright)
                    .action_state(ActionState::Waiting)
                    .build(),
            );
            let start = WorldPoint3D::new(20.0, 30.0, 10.0);
            let mut purse = crate::bow_shot::spawn_purse(thrower, start, start, 0, None);
            let Entity::Projectile(projectile) = &mut purse else {
                unreachable!()
            };
            projectile.projectile.trajectory.clear();
            projectile.projectile.trajectory_runtime.clear();
            projectile.projectile.dive = dive;
            projectile.projectile.disappear = disappear;
            projectile.element.set_material(material);

            let purse_id = engine.publish_new_purse(&sim, &assets, thrower, purse);
            let Some(Entity::Projectile(projectile)) = engine.get_entity(purse_id) else {
                panic!("published water/hole purse disappeared")
            };
            assert!(projectile.element.active);
            assert!(!projectile.projectile.purse.burst);
            assert!(projectile.projectile.purse.child_coins.is_empty());
            assert_eq!(
                projectile.projectile.trajectory_frame_count,
                if dive { 0 } else { u16::MAX }
            );
            assert_eq!(
                projectile.projectile.velocity_increment,
                crate::coordinates::WorldVec3D::ZERO
            );
        }
    }

    #[test]
    fn purse_prepublication_first_segment_obeys_base_shield_early_return() {
        use crate::coordinates::{MapPoint, WorldPoint3D};
        use crate::element::{ActionState, Entity, TrajectoryPoint};

        let sim = crate::sim_rng::test_context();
        let assets = purse_publication_assets();
        let mut engine = EngineInner::new();
        let thrower = engine.add_test_entity(
            TestActor::pc(Posture::Upright)
                .action_state(ActionState::Waiting)
                .build(),
        );
        let mut holder = make_arrow_warning_soldier();
        holder
            .element_data_mut()
            .set_position_map(MapPoint::new(50.0, 0.0));
        holder.element_data_mut().set_direction_instantly(4);
        {
            let actor = holder.actor_data_mut().unwrap();
            actor.action_state = ActionState::HoldingShield;
            actor.shield_obstacle = Some(Box::new(crate::bow_shot::compute_shield_obstacle(
                MapPoint::new(50.0, 0.0),
                0.0,
                4,
                &crate::bow_shot::ShieldParams {
                    pre_offset: 20.0,
                    width: 20.0,
                    depth: 5.0,
                    height: 40.0,
                    z_offset: 10.0,
                },
            )));
        }
        let holder = engine.add_test_entity(holder);

        let start = WorldPoint3D::new(100.0, 0.0, 40.0);
        let end = WorldPoint3D::new(50.0, 0.0, 40.0);
        let mut purse = crate::bow_shot::spawn_purse(thrower, start, end, 0, None);
        let Entity::Projectile(projectile) = &mut purse else {
            unreachable!()
        };
        projectile.projectile.trajectory = vec![TrajectoryPoint {
            position: end,
            time: 1,
        }];
        projectile.projectile.trajectory_runtime.clear();
        let purse_id = engine.publish_new_purse(&sim, &assets, thrower, purse);
        let Some(Entity::Projectile(projectile)) = engine.get_entity(purse_id) else {
            panic!("published shielded purse disappeared")
        };
        assert!(projectile.projectile.flying);
        assert!(projectile.projectile.trajectory.is_empty());
        assert_eq!(projectile.element.sprite.current_frame, 2);
        assert!(
            engine
                .orders
                .sequence_manager
                .sequences_iter()
                .flat_map(|sequence| &sequence.elements)
                .any(|element| element.owner == Some(holder)
                    && element.command == crate::element::Command::ParryShield)
        );
    }

    fn make_arrow_warning_soldier() -> Entity {
        let mut soldier = crate::element::ActorSoldier {
            element: {
                let mut initial_element = ElementData::from_initial_posture(Posture::Upright);
                initial_element.kind = ElementKind::ActorSoldier;
                initial_element.active = true;
                initial_element
            },
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        };
        soldier.soldier.cached_camp = crate::element::Camp::Lacklandists;
        soldier.npc.life_points = 100;
        soldier.npc.ai_brain = crate::element::AiBrain::Enemy(Box::default());
        Entity::Soldier(soldier)
    }

    fn bind_arrow_warning_sprite(entity: &mut Entity) {
        use crate::sprite_script::SpriteScript;

        let mut conversion = crate::engine::test_support::unmapped_conversion();
        conversion[OrderType::WaitingShield as usize] = 0;
        conversion[OrderType::LoweringShield as usize] = 0;
        let script = SpriteScript {
            action_id: OrderType::WaitingShield as u16,
            action_done: 1,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1, 2],
            delays: vec![0, 0],
            distances: vec![0, 0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO; 2],
            sound_ids: vec![0, 0],
        };
        entity.element_data_mut().sprite =
            crate::sprite::Sprite::new(Arc::new(vec![script; 16]), Arc::new(conversion));
    }

    fn arrow_warning_fixture(
        shield_weapon: bool,
        shooter_x: f32,
    ) -> (
        EngineInner,
        LevelAssets,
        EntityId,
        EntityId,
        crate::sequence::SequenceId,
    ) {
        use crate::ai::{AiState, Substate};
        use crate::coordinates::{MapPoint, WorldPoint3D};
        use crate::element::{Command, EyeStatus};
        use crate::sequence::SequenceElement;

        let mut engine = EngineInner::new();
        // Legacy human handles reserve zero as missing; production has a hidden
        // pre-level prefix, so keep the test shooter on a nonzero handle too.
        engine.add_test_entity(Entity::Target(crate::element::ElementTarget {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::Target;
                initial_element
            },
            fx: Default::default(),
            target: Default::default(),
        }));

        let mut shooter = TestActor::pc(Posture::Upright)
            .action_state(ActionState::Waiting)
            .build();
        shooter.element_data_mut().active = true;
        shooter
            .element_data_mut()
            .set_position(WorldPoint3D::new(shooter_x, 0.0, 0.0));
        shooter
            .element_data_mut()
            .set_position_map(MapPoint::new(shooter_x, 0.0));
        shooter.pc_data_mut().unwrap().life_points = 100;
        let shooter_id = engine.add_test_entity(shooter);

        let mut target = make_arrow_warning_soldier();
        bind_arrow_warning_sprite(&mut target);
        target
            .element_data_mut()
            .set_position(WorldPoint3D::new(0.0, 0.0, 0.0));
        target
            .element_data_mut()
            .set_position_map(MapPoint::new(0.0, 0.0));
        target.element_data_mut().set_direction_instantly(4);
        let target_id = engine.add_test_entity(target);
        assert!(shooter_id.index() < target_id.index());

        let mut assets = engine.test_runtime_assets();
        let profiles = Arc::make_mut(&mut assets.profile_manager);
        profiles.soldiers[0].hth_weapon_id = 1;
        profiles.hth_weapons[0].shield = shield_weapon;

        let Entity::Soldier(target) = engine.ent_mut(target_id) else {
            unreachable!()
        };
        target.actor.action_state = ActionState::HoldingShield;
        target.npc.view_direction = [1.0, 0.0];
        target.npc.view_radius = 135;
        target.npc.real_half_aperture = crate::ai_vision::NORMAL_HALF_APERTURE;
        target.npc.eye_status = EyeStatus::Stare;
        let ai = target.npc.ai_brain.enemy_mut().unwrap();
        ai.base.me = target_id.index();
        ai.base.current_state = AiState::Attacking;
        ai.base.current_substate = Substate::AttackingProtectingWithShield;

        let lower = engine
            .orders
            .sequence_manager
            .insert_element(SequenceElement::new(
                1,
                Command::LowerShield,
                Some(target_id),
            ));
        engine.orders.sequence_manager.start_sequence_level(lower);
        engine.t_instruct_owner(&assets, target_id, lower, 0);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .current_order_for_actor(&engine.world.entities, target_id)
                .map(|(_, _, order)| order.order_type),
            Some(OrderType::LoweringShield)
        );

        (engine, assets, shooter_id, target_id, lower)
    }

    #[test]
    fn arrow_warning_synchronously_interrupts_later_shield_target_and_preserves_fifo() {
        use crate::ai::{Stimulus, StimulusType};
        use crate::sequence::SequenceState;

        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, shooter, target, lower) = arrow_warning_fixture(true, 55.0);
        let optical_batch = vec![Stimulus::new(StimulusType::EventTimer)];

        engine.warn_shield_target_of_arrow(&sim, &assets, shooter, target);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(lower, 0)
                .unwrap()
                .state,
            SequenceState::Interrupted,
            "the release-site Think must interrupt LowerShield before the later target slot animates"
        );
        assert_eq!(
            engine
                .get_entity(target)
                .and_then(Entity::enemy_ai)
                .unwrap()
                .base
                .primary_target,
            Some(crate::ai::AiEntityHandle::new(shooter.index())),
            "the arrow reaction must run now, not remain queued for the target's later slot"
        );
        let ai = engine
            .get_entity(target)
            .and_then(Entity::ai_controller)
            .unwrap();
        assert!(
            !ai.ai_log
                .iter()
                .any(|line| line.line_type == crate::ai::LogLineType::Event
                    && line.info == StimulusType::EventTimer as u16)
        );
        engine.dispatch_optical_stimuli(&sim, target, &assets, optical_batch);
        let ai = engine
            .get_entity(target)
            .and_then(Entity::ai_controller)
            .unwrap();
        assert!(
            ai.ai_log
                .iter()
                .any(|line| line.line_type == crate::ai::LogLineType::Event
                    && line.info == StimulusType::EventTimer as u16),
            "the caller's local optical batch is delivered only at its scan boundary"
        );
    }

    #[test]
    fn arrow_warning_skips_nonshield_and_nonseeing_targets() {
        use crate::sequence::SequenceState;

        let sim = crate::sim_rng::test_context();
        for (shield_weapon, shooter_x) in [(false, 55.0), (true, 500.0)] {
            let (mut engine, assets, shooter, target, lower) =
                arrow_warning_fixture(shield_weapon, shooter_x);

            engine.warn_shield_target_of_arrow(&sim, &assets, shooter, target);

            assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .get_element(lower, 0)
                    .unwrap()
                    .state,
                SequenceState::InProgress
            );
            assert_eq!(
                engine
                    .get_entity(target)
                    .and_then(Entity::ai_controller)
                    .unwrap()
                    .primary_target,
                None,
                "a rejected warning must not select the shooter",
            );
        }
    }

    #[test]
    #[should_panic(expected = "bow target 2 requires missing soldier profile 9")]
    fn arrow_warning_rejects_missing_authoritative_soldier_profile() {
        let sim = crate::sim_rng::test_context();
        let (mut engine, assets, shooter, target, _) = arrow_warning_fixture(true, 55.0);
        let Entity::Soldier(soldier) = engine.ent_mut(target) else {
            unreachable!()
        };
        soldier.soldier.soldier_profile_index = SoldierProfileIdx(9);

        engine.warn_shield_target_of_arrow(&sim, &assets, shooter, target);
    }

    fn attach_drop_test_sprite(entity: &mut Entity) {
        use crate::sprite_script::{NONANIMATION_END, SpriteScript};

        let script = SpriteScript {
            action_id: 0,
            action_done: 0,
            average_speed: 0.0,
            hotspot: crate::coordinates::SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![0],
            distances: vec![0],
            offsets: vec![crate::coordinates::SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        entity.element_data_mut().sprite = crate::sprite::Sprite::new(
            Arc::new(vec![script; 16]),
            Arc::new(vec![0; NONANIMATION_END]),
        );
    }

    fn corpse_drop_pair(
        carrier_pos: crate::coordinates::MapPoint,
    ) -> (
        EngineInner,
        crate::element::EntityId,
        crate::element::EntityId,
    ) {
        let mut engine = EngineInner::new();
        let target_id = engine.add_test_entity(
            TestActor::pc(Posture::Carried)
                .action_state(ActionState::Waiting)
                .build(),
        );
        let mut carrier = TestActor::pc(Posture::CarryingCorpse)
            .action_state(ActionState::Waiting)
            .build();
        attach_drop_test_sprite(&mut carrier);
        carrier.pc_data_mut().unwrap().carried = Some(target_id);
        carrier.element_data_mut().set_position_map(carrier_pos);
        let carrier_id = engine.add_test_entity(carrier);
        engine.human_mut(target_id).carrier = Some(carrier_id);
        (engine, carrier_id, target_id)
    }

    fn install_corpse_drop_building_sector(engine: &mut EngineInner, raw_sector: u16) {
        let mut level = crate::fast_find_grid::LevelGrid::default();
        level
            .sector_number_map
            .insert(crate::sector::SectorNumber::new(raw_sector as i16), 0);
        level.sectors.push(crate::fast_find_grid::GridSector {
            points: Vec::new(),
            bounding_box: crate::coordinates::MapBBox::new(),
            sector_type: crate::sector::SectorType::BUILDING,
            layer: 0,
            sector_number: crate::sector::SectorNumber::new(raw_sector as i16),
            door_index: None,
            lift_type: None,
            lift_direction: 0,
            force_crouched: false,
            building_index: None,
            low_exit_point: None,
            high_exit_point: None,
            lowest_door_index: None,
            jump_line_indices: Vec::new(),
            gate_indices: Vec::new(),
            underlying_sector: None,
        });
        engine.world.fast_grid_mut().level = Arc::new(level);
    }

    #[test]
    fn delayed_corpse_drop_carries_sloped_surface_into_next_frame_position() {
        let carrier_pos = crate::coordinates::MapPoint::new(743.0, 1681.0);
        let plane = crate::position_interface::PlaneZCoeffs {
            az: -0.270_139,
            bz: -1.787_207,
            dz: 3_396.161_9,
        };
        let obstacle = crate::position_interface::ObstacleHandle::new(221).unwrap();
        let (mut engine, carrier_id, target_id) = corpse_drop_pair(carrier_pos);
        let cached_position = crate::coordinates::WorldPoint3D::new(700.0, 1906.001, 225.001);
        engine.place(target_id, cached_position);
        engine
            .elem_mut(target_id)
            .set_material(crate::element::GameMaterial::Grass);
        {
            let carrier = engine.ent_mut(carrier_id);
            let elem = carrier.element_data_mut();
            elem.set_layer(1);
            elem.set_obstacle_index(Some(obstacle), Some(plane));
        }

        let assets = engine.test_runtime_assets();
        engine.apply_completed_corpse_drop(
            &crate::sim_rng::test_context(),
            &assets,
            carrier_id,
            target_id,
            Posture::Lying,
            carrier_pos,
            15,
        );

        let target = engine.ent(target_id);
        assert!(target.element_data().position_map_delayed);
        assert_eq!(target.element_data().layer(), 1);
        assert_eq!(
            target.element_data().material(),
            crate::element::GameMaterial::Grass
        );
        assert_eq!(target.position_iface().get_obstacle(), Some(obstacle));
        assert_eq!(target.position_iface().get_plane(), Some(&plane));
        assert_eq!(target.element_data().position(), cached_position);

        engine
            .elem_mut(target_id)
            .apply_next_delayed_position()
            .expect("outdoor corpse drop must commit its delayed position next frame");
        let target = engine.ent(target_id);
        assert_eq!(target.element_data().position_map(), carrier_pos);
        assert_eq!(
            target.element_data().position().z.to_bits(),
            plane.compute_z(743.0, 1681.0).to_bits()
        );
        assert_ne!(
            target.element_data().position().z.to_bits(),
            0.0_f32.to_bits()
        );
    }

    #[test]
    fn outdoor_null_surface_corpse_drop_preserves_cached_elevation_until_delayed_commit() {
        let carrier_pos = crate::coordinates::MapPoint::new(3126.2605, 2149.9695);
        let carried_position = crate::coordinates::WorldPoint3D::new(3125.0, 2375.001, 225.001);
        let (mut engine, carrier_id, target_id) = corpse_drop_pair(carrier_pos);
        {
            let target = engine.ent_mut(target_id);
            target.element_data_mut().set_position(carried_position);
        }
        let carried_map = engine.map_pos_of(target_id);

        let assets = engine.test_runtime_assets();
        engine.apply_completed_corpse_drop(
            &crate::sim_rng::test_context(),
            &assets,
            carrier_id,
            target_id,
            Posture::DeadBack,
            carrier_pos,
            0,
        );

        let target = engine.ent(target_id);
        assert!(target.element_data().position_map_delayed);
        assert_eq!(target.element_data().position(), carried_position);
        assert_eq!(target.element_data().position_map(), carried_map);
        assert_eq!(target.position_iface().get_obstacle(), None);
        assert_eq!(target.position_iface().get_plane(), None);

        engine
            .elem_mut(target_id)
            .apply_next_delayed_position()
            .expect("outdoor corpse drop must commit its delayed position next frame");
        let target = engine.ent(target_id);
        assert_eq!(target.element_data().position_map(), carrier_pos);
        assert_eq!(
            target.element_data().position().z.to_bits(),
            0.0_f32.to_bits()
        );
    }

    #[test]
    fn delayed_corpse_drop_updates_intersections_at_old_current_position() {
        let carried_position = crate::coordinates::MapPoint::new(100.0, 100.0);
        let drop_position = crate::coordinates::MapPoint::new(300.0, 300.0);
        let (mut engine, carrier_id, target_id) = corpse_drop_pair(drop_position);
        {
            let target = engine.ent_mut(target_id);
            target.element_data_mut().set_position_map(carried_position);
            assert_eq!(
                target
                    .human_data()
                    .unwrap()
                    .last_is_lying_for_corpse_intersection,
                None,
                "freshly adopted carried bodies have no derived observer state"
            );
        }
        let mut neighbour = TestActor::pc(Posture::Tied)
            .action_state(ActionState::Waiting)
            .build();
        neighbour
            .element_data_mut()
            .set_position_map(crate::coordinates::MapPoint::new(110.0, 100.0));
        neighbour
            .human_data_mut()
            .unwrap()
            .last_is_lying_for_corpse_intersection = Some(true);
        let neighbour_id = engine.add_test_entity(neighbour);

        let assets = engine.test_runtime_assets();
        engine.apply_completed_corpse_drop(
            &crate::sim_rng::test_context(),
            &assets,
            carrier_id,
            target_id,
            Posture::Tied,
            drop_position,
            0,
        );

        let target = engine.ent(target_id);
        assert!(target.element_data().position_map_delayed);
        assert_eq!(target.element_data().position_map(), carried_position);
        assert!(target.human_data().unwrap().small_repulsive_radius);
        assert!(
            engine
                .get_entity(neighbour_id)
                .unwrap()
                .human_data()
                .unwrap()
                .small_repulsive_radius
        );

        engine
            .elem_mut(target_id)
            .apply_next_delayed_position()
            .expect("outdoor corpse drop must retain its delayed destination");
        assert_eq!(
            engine
                .get_entity(target_id)
                .unwrap()
                .element_data()
                .position_map(),
            drop_position
        );
    }

    #[test]
    fn instant_building_corpse_drop_keeps_carrier_surface_and_commits_immediately() {
        let carrier_pos = crate::coordinates::MapPoint::new(120.0, 240.0);
        let plane = crate::position_interface::PlaneZCoeffs {
            az: 0.125,
            bz: -0.25,
            dz: 45.0,
        };
        let obstacle = crate::position_interface::ObstacleHandle::new(17).unwrap();
        let sector = crate::position_interface::SectorHandle::new(7).unwrap();
        let (mut engine, carrier_id, target_id) = corpse_drop_pair(carrier_pos);
        install_corpse_drop_building_sector(&mut engine, 7);
        engine
            .elem_mut(target_id)
            .set_material(crate::element::GameMaterial::Leaves);
        {
            let carrier = engine.ent_mut(carrier_id);
            let elem = carrier.element_data_mut();
            elem.set_layer(3);
            elem.set_sector(Some(sector));
            elem.set_obstacle_index(Some(obstacle), Some(plane));
        }

        let assets = engine.test_runtime_assets();
        engine.apply_completed_corpse_drop(
            &crate::sim_rng::test_context(),
            &assets,
            carrier_id,
            target_id,
            Posture::Lying,
            carrier_pos,
            4,
        );

        let target = engine.ent(target_id);
        assert!(!target.element_data().position_map_delayed);
        assert_eq!(target.element_data().position_map(), carrier_pos);
        assert_eq!(target.element_data().layer(), 3);
        assert_eq!(target.element_data().sector(), Some(sector));
        assert_eq!(
            target.element_data().material(),
            crate::element::GameMaterial::Leaves
        );
        assert_eq!(target.position_iface().get_obstacle(), Some(obstacle));
        assert_eq!(target.position_iface().get_plane(), Some(&plane));
        assert_eq!(
            target.element_data().position().z.to_bits(),
            plane.compute_z(120.0, 240.0).to_bits()
        );
        assert_eq!(target.element_data().direction(), 0);
        assert_eq!(
            i16::from(target.position_iface().get_direction_goal()),
            4,
            "clearing the carrier must restore its facing as the dropped corpse's goal"
        );
    }

    #[test]
    fn task229_projectile_ai_origin_preserves_saved_sector_and_layer() {
        let exact_sector = crate::fast_find_grid::SectorIndex::new(41).unwrap();
        let mut projectile = Entity::Projectile(ElementProjectile {
            element: {
                let mut initial_element = ElementData::default();
                initial_element.kind = ElementKind::ObjectProjectile;
                initial_element
            },
            object: ObjectData::default(),
            projectile: ProjectileData {
                start_of_trajectory_x: 572.0,
                start_of_trajectory_y: 2360.0,
                ..Default::default()
            },
        });
        let Entity::Projectile(projectile_data) = &mut projectile else {
            unreachable!()
        };
        let exact_handle = crate::position_interface::SectorHandle::new(17)
            .unwrap()
            .with_arena_index(exact_sector);
        set_projectile_trajectory_origin(&mut projectile_data.projectile, Some(exact_handle), 11);
        assert_eq!(
            projectile_data.projectile.trajectory_origin_sector,
            Some(17)
        );
        assert_eq!(
            projectile_data.projectile.trajectory_origin_sector_index,
            Some(exact_sector),
            "the shared arrow/apple publication writer must retain exact origin topology"
        );

        let origin = projectile_trajectory_origin(&projectile).unwrap();
        assert_eq!(origin.x, 572.0);
        assert_eq!(origin.y, 2360.0);
        assert_eq!(origin.sector.map(|sector| sector.get()), Some(17));
        assert_eq!(
            origin.sector.and_then(|sector| sector.arena_index()),
            Some(exact_sector),
            "arrow-hit events must copy the exact trajectory-origin sector identity"
        );
        assert_eq!(origin.level, 11);

        // The task-229 boundary lies on opposite sides of a direction-sector
        // threshold depending on whether Face(Position) retains sector 17's
        // projection elevation. Dropping the sector changes the authored turn.
        let dx = 572.0 - 785.243_35;
        let dy = 2360.0 - 2_192.851_6;
        assert_eq!(
            crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy),
            10
        );
        assert_eq!(
            crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy + 105.001_01),
            9
        );
    }

    #[test]
    #[should_panic(expected = "without its public sector number")]
    fn projectile_origin_rejects_orphan_exact_sector_identity() {
        let projectile = ProjectileData {
            trajectory_origin_sector_index: crate::fast_find_grid::SectorIndex::new(41),
            ..Default::default()
        };
        let _ = projectile_trajectory_origin_sector(&projectile);
    }

    fn blocked_shoulder_pair() -> (
        EngineInner,
        LevelAssets,
        crate::element::EntityId,
        crate::element::EntityId,
    ) {
        let mut engine = EngineInner::new();
        let victim_id = engine.add_test_entity(
            TestActor::pc(Posture::OnShoulders)
                .action_state(ActionState::Waiting)
                .build(),
        );
        let mut carrier = TestActor::pc(Posture::CarryingOnShoulders)
            .action_state(ActionState::Waiting)
            .build();
        let Entity::Pc(carrier_pc) = &mut carrier else {
            unreachable!()
        };
        carrier_pc.pc.carried = Some(victim_id);
        let carrier_id = engine.add_test_entity(carrier);
        engine.human_mut(victim_id).carrier = Some(carrier_id);

        // A flat solid slab from z=60 through z=70 intersects the exact
        // Shoulder-carry eligibility's vertical segment (z=50..90) at the default
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
        assets.environment.static_sight_obstacles = Arc::new(vec![ceiling]);
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

        let mut element = crate::sequence::SequenceElement::new(
            1,
            crate::element::Command::Wait,
            Some(carrier_id),
        );
        element.orders.push_back(crate::order::Order::test_new(
            OrderType::WaitingCarryingOnShoulders,
            0.0,
            0.0,
        ));
        element.state = SequenceState::InProgress;
        let sequence = engine.orders.sequence_manager.insert_element(element);
        engine.actor_mut(carrier_id).selected_sequence_element =
            Some(crate::sequence::SequenceElementRef::new(sequence, 0));
        let executed =
            engine.tick_actor_animation_for(&crate::sim_rng::test_context(), &assets, carrier_id);
        assert!(executed.is_some());
        assert!(shoulder_drop_elements(&engine).is_empty());
    }

    #[test]
    fn carry_done_applies_effect_without_releasing_selected_ability() {
        let mut engine = EngineInner::new();
        let carrier = engine.add_test_entity(
            TestActor::pc(Posture::CarryingCorpse)
                .action_state(ActionState::Waiting)
                .build(),
        );
        let target = engine.add_test_entity(
            TestActor::pc(Posture::Lying)
                .action_state(ActionState::Waiting)
                .build(),
        );
        let mut element = crate::sequence::SequenceElement::new_interaction(
            1,
            crate::element::Command::TakeCorpse,
            Some(carrier),
            Some(target),
        );
        element.orders.push_back(crate::order::Order::test_new(
            OrderType::TransitionWaitingUprightCarryingCorpse,
            0.0,
            0.0,
        ));
        let sequence = engine.orders.sequence_manager.insert_element(element);
        let selected = crate::sequence::SequenceElementRef::new(sequence, 0);
        engine.actor_mut(carrier).selected_sequence_element = Some(selected);

        crate::abilities::initialize_carry_relationship(
            &mut engine.world.entities,
            carrier,
            target,
        );
        engine.apply_ability_carry_done(carrier, target);

        let carrier = engine.ent(carrier);
        assert_eq!(carrier.pc_data().unwrap().carried, Some(target));
        assert_eq!(
            carrier.actor_data().unwrap().selected_sequence_element,
            Some(selected)
        );
        let target = engine.ent(target);
        assert_eq!(target.posture(), Posture::Carried);
        assert_eq!(
            target.actor_data().unwrap().action_state,
            ActionState::Waiting
        );
        assert_eq!(engine.orders.sequence_manager.sequences_iter().count(), 1);
        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .unwrap()
                .current_order()
                .unwrap()
                .order_type,
            OrderType::TransitionWaitingUprightCarryingCorpse,
        );
    }

    #[test]
    fn shoulder_dismount_done_detaches_both_owners_before_helper_wait() {
        let (mut engine, assets, helper, climber) = blocked_shoulder_pair();
        let helper_position = crate::coordinates::MapPoint::new(80.0, 96.0);
        {
            let helper = engine.ent_mut(helper);
            helper.element_data_mut().set_position_map(helper_position);
            helper.element_data_mut().set_direction_instantly(6);
            helper.actor_data_mut().unwrap().execution_frozen = true;
        }
        {
            let climber_entity = engine.ent_mut(climber);
            climber_entity.actor_data_mut().unwrap().execution_frozen = true;
            climber_entity.element_data_mut().sprite.display_order_ref = Some(helper);
            climber_entity
                .element_data_mut()
                .sprite
                .behind_display_order_ref = true;
        }

        engine.apply_ability_climb_down_from_shoulders_done(
            &crate::sim_rng::test_context(),
            &assets,
            climber,
            helper,
        );

        let climber_entity = engine.ent(climber);
        assert_eq!(climber_entity.posture(), Posture::Upright);
        assert_eq!(climber_entity.human_data().unwrap().carrier, None);
        assert!(!climber_entity.actor_data().unwrap().execution_frozen);
        assert_eq!(
            climber_entity.element_data().position_map(),
            helper_position
        );
        assert_eq!(climber_entity.element_data().direction(), 14);
        assert_eq!(climber_entity.sprite().display_order_ref, Some(helper));
        assert!(climber_entity.sprite().behind_display_order_ref);
        assert_eq!(climber_entity.sprite().display_depth, 0.0);
        let helper_entity = engine.ent(helper);
        assert_eq!(helper_entity.posture(), Posture::HelpingToClimb);
        assert_eq!(helper_entity.pc_data().unwrap().carried, None);
        assert!(!helper_entity.actor_data().unwrap().execution_frozen);
        let waits = engine
            .orders
            .sequence_manager
            .sequences_iter()
            .flat_map(|sequence| &sequence.elements)
            .filter(|element| element.command == crate::element::Command::Wait)
            .collect::<Vec<_>>();
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].owner, Some(helper));
        assert_eq!(waits[0].priority, crate::sequence::SequencePriority::Wait);
    }

    #[test]
    fn walking_carry_action_launches_drop_on_that_action_frame() {
        let (mut engine, assets, carrier_id, victim_id) = blocked_shoulder_pair();
        assert!(shoulder_drop_elements(&engine).is_empty());

        assert!(!engine.check_walking_shoulder_clearance(
            &crate::sim_rng::test_context(),
            &assets,
            carrier_id,
        ));

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
        let shooter = engine.add_test_entity(
            TestActor::pc(Posture::Upright)
                .action_state(ActionState::Waiting)
                .build(),
        );
        let mut victim = TestActor::pc(Posture::Upright)
            .action_state(ActionState::Waiting)
            .build();
        let Entity::Pc(victim_pc) = &mut victim else {
            unreachable!()
        };
        victim_pc.pc.life_points = 100;
        let victim = engine.add_test_entity(victim);

        engine.queue_projectile_damage(
            &crate::sim_rng::test_context(),
            &LevelAssets::new(),
            victim,
            shooter,
            crate::element::Command::ReceiveArrowDamage,
            40,
            0,
            Some(shooter),
        );

        assert_eq!(
            engine
                .get_entity(victim)
                .and_then(|entity| entity.pc_data())
                .map(|pc| pc.life_points),
            Some(100),
            "projectile collision must not apply damage before the sequence-manager tick"
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
                concussion: 0,
                ..
            } if origin == shooter && projectile == shooter
        ));
    }
}
