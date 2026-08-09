//! Archery: `shoot_arrow_at`, `propose_shot_target`,
//! `archer_is_too_near_to_enemy`.

use crate::ai::*;
use crate::position_interface::{ASPECT_RATIO, INVERSE_ASPECT_RATIO};

use super::util::{det2, dot2, pos_diff, sector_to_vector, square_norm, vec_to_sector_ar};
use super::{EnemyAi, archer};

/// `SBGeoVector2D::Angle` from the Original geometry library.
///
/// This is deliberately not `det.atan2(dot)`: the Original handles a zero
/// determinant first and returns PI when the dot product is non-positive.
/// Consequently, the angle from a nonzero facing vector to a coincident point
/// is PI, not the zero returned by Rust's `atan2(0, 0)`.
fn legacy_vector_angle(from: (f32, f32), to: (f32, f32)) -> f32 {
    let dot = dot2(from, to);
    let det = det2(from, to);
    if det == 0.0 {
        if dot > 0.0 { 0.0 } else { std::f32::consts::PI }
    } else {
        let angle = (det / dot).atan();
        if dot >= 0.0 {
            angle
        } else if det > 0.0 {
            angle + std::f32::consts::PI
        } else {
            angle - std::f32::consts::PI
        }
    }
}

impl EnemyAi {
    // -----------------------------------------------------------------------
    // Archery
    // -----------------------------------------------------------------------

    /// ShootArrowAt.
    ///
    /// Stops current actions and sets a pending flag for the engine to launch
    /// a `Command::ShootBow` sequence element on the next post-think drain.
    pub fn shoot_arrow_at(&mut self, enemy: HumanHandle, ctx: &AiContext, _tick: &AiPerTickData) {
        // Asserts: is_archer() && remaining_arrows > 0.
        debug_assert!(self.is_archer_unit, "shoot_arrow_at called on non-archer");
        debug_assert!(
            ctx.remaining_arrows > 0,
            "shoot_arrow_at called with 0 arrows"
        );

        self.base.stop_all();

        // Set pending flag — the engine drains this after think() and
        // calls EngineInner::shoot_bow_at to launch the sequence element.
        self.base.outbox.actor.shoot_target = Some(enemy);
    }

    /// ProposeShotTarget.
    ///
    /// Picks the best enemy to shoot at, considering:
    /// - Bow range (must be within max range)
    /// - Isometric Y-axis stretching (`INVERSE_ASPECT_RATIO`)
    /// - Friendly-fire avoidance (angle check against nearby allies)
    /// - Already-targeted penalty (10000 per existing attacker)
    /// - Minimum distance check (enemies < 100px get no random scatter)
    /// - Merry-man-forest randomisation
    pub fn propose_shot_target(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> HumanHandle {
        let my_pos = &ctx.position;
        // Nose direction vector (not Y-stretched).
        let nose = sector_to_vector(ctx.direction);

        // Get my bow max range from the fighter snapshot.
        let my_bow_max_range = self
            .find_fighter(self.base.me, tick)
            .map(|f| f.bow_max_range as f32)
            .unwrap_or(0.0);
        let sqr_range = my_bow_max_range * my_bow_max_range;
        // Pre-compute angle and squared distance for each friendly
        // fighter (excluding self). The reference scans the `mlistUs` rebuilt
        // by `BattleDecisions`, not every same-camp fighter in the broader
        // 500px engine snapshot. In particular, friends outside the owner's
        // 360-degree detection set must not spuriously block a shot.
        struct FriendInfo {
            sq_distance: f32,
            angle: f32,
            is_pc: bool,
            is_shield: bool,
        }
        let mut friends: Vec<FriendInfo> = Vec::new();
        for &friend_handle in &self.base.list_us {
            if friend_handle == self.base.me {
                continue;
            }
            let f = self.find_fighter(friend_handle, tick).unwrap_or_else(|| {
                panic!("friend {friend_handle} in list_us is absent from fighter snapshot")
            });
            let dx = f.position.x - my_pos.x;
            let dy = (f.position.y - my_pos.y) * INVERSE_ASPECT_RATIO;
            let to_friend = (dx, dy);
            let sq_dist = square_norm(to_friend);
            let angle = legacy_vector_angle(nose, to_friend);
            friends.push(FriendInfo {
                sq_distance: sq_dist,
                angle,
                is_pc: f.is_pc,
                is_shield: f.action_state.is_shield(),
            });
        }

        // Compute bow-specific primary-target multiplicity.
        // Only friends in bow substates (shooting/loading/aiming) count — the
        // generic `tick.primary_target_multiplicity` also includes melee fighters.
        let mut bow_multiplicity: Vec<(HumanHandle, u32)> = Vec::new();
        for &friend_handle in &self.base.list_us {
            if friend_handle == self.base.me {
                continue;
            }
            let f = self.find_fighter(friend_handle, tick).unwrap_or_else(|| {
                panic!("friend {friend_handle} in list_us is absent from fighter snapshot")
            });
            if !f.is_soldier {
                continue;
            }
            let sub = f.current_substate;
            if sub == Substate::AttackingBowShooting as u32
                || sub == Substate::AttackingBowLoading as u32
                || sub == Substate::AttackingBowAiming as u32
            {
                let target = f.primary_target;
                if target != 0 {
                    if let Some(entry) = bow_multiplicity.iter_mut().find(|e| e.0 == target) {
                        entry.1 += 1;
                    } else {
                        bow_multiplicity.push((target, 1));
                    }
                }
            }
        }

        let is_forest = self.is_merry_man_forest(ctx);

        // Scan all enemies, pick nearest valid target.
        let mut best: HumanHandle = 0;
        let mut min_sq_distance = f32::INFINITY;

        for &enemy_handle in &self.list_them {
            if enemy_handle == 0 {
                continue;
            }
            if !self.is_allowed_to_attack(enemy_handle, ctx, tick) {
                continue;
            }
            let Some(enemy) = self.find_fighter(enemy_handle, tick) else {
                continue;
            };

            // Isometric-stretched vector and distance to enemy.
            let dx = enemy.position.x - my_pos.x;
            let dy = (enemy.position.y - my_pos.y) * INVERSE_ASPECT_RATIO;
            let mut sq_distance = dx * dx + dy * dy;

            // Within bow range?
            if sq_distance > sqr_range {
                continue;
            }

            // If not too near, optionally scatter for forest.
            if sq_distance > 100.0 * 100.0 && is_forest {
                sq_distance +=
                    crate::sim_rng::u32(sim, crate::sim_rng::RngSite::ArcherForestTarget, 0..10000)
                        as f32;
            }

            // Penalise targets already being shot at.
            let multiplicity = bow_multiplicity
                .iter()
                .find(|&&(h, _)| h == enemy_handle)
                .map(|&(_, m)| m)
                .unwrap_or(0);
            sq_distance += 10000.0 * multiplicity as f32;

            // New record?
            if sq_distance > min_sq_distance {
                continue;
            }

            // Check all friends for friendly fire.
            let angle_to_enemy = legacy_vector_angle(nose, (dx, dy));
            let friend_in_the_way = friends.iter().any(|fri| {
                // Friend must be closer than enemy.
                if fri.sq_distance > sq_distance {
                    return false;
                }
                // In merry-man forest, only PC friends block.
                if is_forest && !fri.is_pc {
                    return false;
                }
                // Skip shield bearers.
                if fri.is_shield {
                    return false;
                }
                // Compare angles: too similar means friend is in the line of fire.
                let mut angle_diff = (angle_to_enemy - fri.angle).abs();
                if angle_diff > std::f32::consts::TAU {
                    angle_diff -= std::f32::consts::TAU;
                }
                angle_diff < archer::MIN_TARGET_FRIEND_ANGLE
            });

            if !friend_in_the_way {
                best = enemy_handle;
                min_sq_distance = sq_distance;
            }
        }

        best
    }

    /// ArcherIsToNearToEnemy —
    /// per-enemy "is this archer dangerously close to that enemy"
    /// predicate.  The threshold depends on the relative facing of the
    /// enemy (head-on vs. quartering vs. passing vs. leaving) and on
    /// the enemy's action state (fast-running attackers inflate the
    /// bound).
    ///
    /// Returns `false` early if a shield bearer is covering us
    /// (`shield_bearer_before_me != 0`) or if we are a Royalist on a
    /// Sherwood Forest level (`IsMerryManForest`).  Returns `false` if
    /// Panics if the required primary target is absent from the live fighter
    /// registry, matching the original's unconditional pointer dereference.
    pub fn archer_is_too_near_to_enemy(
        &self,
        pos_me: &Position,
        enemy_handle: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        // Shield bearer in front of us — he will
        // protect us, don't flinch.
        if self.shield_bearer_before_me != 0 {
            return false;
        }

        // Merry-man-forest royalist — different fear
        // logic applies via MerryManForestCassos.
        if self.is_merry_man_forest(ctx) {
            return false;
        }

        let enemy = self.find_fighter(enemy_handle, tick).unwrap_or_else(|| {
            panic!(
                "archer {} requires missing primary-target fighter {}",
                self.base.me, enemy_handle
            )
        });

        // Vector from enemy to me (note the original name
        // `vEnemyToMe = posMe - Position(pEnemy)` is the vector pointing
        // from the enemy toward me).
        let v_enemy_to_me = pos_diff(pos_me, &enemy.position);
        let sq_norm =
            crate::position_interface::vector_square_norm_iso(v_enemy_to_me.0, v_enemy_to_me.1);

        // Sector of enemy→me, expressed relative to the
        // enemy's facing direction.  Sector 0 = directly in front of
        // the enemy.
        let sector = vec_to_sector_ar(v_enemy_to_me.0, v_enemy_to_me.1, ASPECT_RATIO);
        let relative = (sector as i32 - enemy.direction as i32).rem_euclid(16) as u16;

        let action_state = enemy.action_state;
        let is_fast = matches!(
            action_state,
            crate::element::ActionState::MovingFast | crate::element::ActionState::MovingFastSword
        );
        let is_approaching = matches!(
            action_state,
            crate::element::ActionState::Moving
                | crate::element::ActionState::MovingShield
                | crate::element::ActionState::MovingSword
        );

        // 7-arm switch.
        let critical_distance = match relative {
            0 => {
                if is_fast {
                    archer::MIN_DISTANCE_ENEMY_HEAD_ON_ATTACK
                } else if is_approaching {
                    archer::MIN_DISTANCE_ENEMY_APPROACHING_FAST
                } else {
                    archer::MIN_DISTANCE_ENEMY_APPROACHING_SLOWLY
                }
            }
            1 | 15 => {
                if is_fast {
                    archer::MIN_DISTANCE_ENEMY_APPROACHING_FAST
                } else if is_approaching {
                    archer::MIN_DISTANCE_ENEMY_APPROACHING_SLOWLY
                } else {
                    archer::MIN_DISTANCE_ENEMY_PASSING
                }
            }
            2 | 14 => {
                if is_fast {
                    archer::MIN_DISTANCE_ENEMY_APPROACHING
                } else if is_approaching {
                    archer::MIN_DISTANCE_ENEMY_APPROACHING_SLOWLY
                } else {
                    archer::MIN_DISTANCE_ENEMY_PASSING
                }
            }
            3 | 4 | 12 | 13 => archer::MIN_DISTANCE_ENEMY_PASSING,
            // 5 | 6 | 7 | 8 | 9 | 10 | 11
            _ => archer::MIN_DISTANCE_ENEMY_LEAVING,
        };

        let cd = critical_distance as f32;
        sq_norm < cd * cd
    }
}

#[cfg(test)]
mod tests {
    use crate::ai_enemy::archer;

    use super::legacy_vector_angle;

    #[test]
    fn legacy_angle_treats_a_coincident_point_as_opposite() {
        assert_eq!(
            legacy_vector_angle((1.0, 0.0), (0.0, 0.0)),
            std::f32::consts::PI
        );
    }

    #[test]
    fn coincident_friend_does_not_block_a_near_forward_archery_target() {
        let friend_angle = legacy_vector_angle((1.0, 0.0), (0.0, 0.0));
        let target_angle = legacy_vector_angle((1.0, 0.0), (100.0, -3.0));
        assert!((target_angle - friend_angle).abs() >= archer::MIN_TARGET_FRIEND_ANGLE);
    }
}
