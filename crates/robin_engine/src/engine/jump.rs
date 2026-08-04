//! Line-jump sequence.
//!
//! A line-jump is a `Command::JumpCmd` sequence element that moves the PC
//! across a pair of jump lines.  The translator picks between three
//! branches based on the height delta between source and destination:
//!
//! * **Long jump** — roughly horizontal (`|Δh| < PC_HEIGHT` or the pair
//!   is force-long): the actor trots to the source edge, launches on a
//!   ballistic arc through N trajectory points, and lands on the far
//!   side.  Sword-fighting variant uses paired sword-specific orders.
//! * **Jump up** — destination is above source: crouch up → transition
//!   → single `JumpingUp` order to destination → land crouched →
//!   optional stand-up.
//! * **Jump down** — destination is below source: optional crouch down →
//!   transition → `JumpingDown` order to destination → land crouched →
//!   optional stand-up.
//!
//! Each branch pushes a list of [`JumpStep`]s onto `ActorData::active_jump`.
//! [`EngineInner::tick_active_jumps`] drains them one at a time, interpolating
//! position over the step's animation duration. Airborne target points
//! are absolute Spellbound 3D coordinates, matching the original C++
//! `SetPosition(pointDestination3D)` path.  When the last step terminates,
//! the owning sequence element is notified via
//! [`SequenceManager::element_terminated`].

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::coordinates::{MapPoint, MapVec, WorldPoint3D, WorldVec3D};
use crate::element::{ActionState, EntityId, Posture};
use crate::engine::{EngineInner, LevelAssets};
use crate::jump_line::JumpLine;
use crate::order::OrderType;
use crate::sequence::SequenceId;

/// PC's vertical reach.  Jumps with `|Δh|` under this threshold run the
/// long-jump branch; above it they split into `jump-up` / `jump-down`.
pub const TELEPORT_JUMPING_UP: f32 = 60.0;

/// Gravity constant.
const GRAVITY: f32 = -8.01;

/// PC mass for the jump trajectory.
const MASS_CHARACTER: f32 = 0.7;

/// Frames per trajectory segment.  Each airborne `JumpingLong` step
/// runs for this many frames before the next trajectory waypoint takes
/// over.
pub const TIME_FLYSEGMENT: u16 = 4;

/// A single step in a jump sequence.
///
/// Each step installs one `active_ai_anim` with completion
/// `AiAnimCompletion::NextJumpStep`.  If `target_3d` is `Some`, the
/// actor's absolute 3D position interpolates linearly from the start of
/// the step to the target across the animation's duration.
/// If `None`, the animation plays in place (transition crouch up/down,
/// waiting↔jumping transitions, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct JumpStep {
    /// The animation to play during this step.
    pub anim: OrderType,
    /// Optional 3D destination for the step.  `None` means the animation
    /// plays in place with no position change.
    pub target_3d: Option<WorldPoint3D>,
    /// Whether this step's animation places the actor airborne.  During
    /// airborne steps `target_3d` is an absolute world position; on a
    /// ground step it is a map-space target encoded with z=0.
    pub airborne: bool,
    /// Cap this step at `N` frames instead of the animation's full
    /// duration.  Used for `JumpingLong` trajectory segments where each
    /// segment runs for `TIME_FLYSEGMENT = 4` frames and rolls over to
    /// the next segment mid-animation.
    pub max_frames: Option<u16>,
}

/// Tracks the currently-executing step.  Stored inside [`ActiveJump`].
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct CurrentStepState {
    pub start_x: f32,
    pub start_y: f32,
    pub start_z: f32,
    pub total_frames: u16,
    pub frames_elapsed: u16,
    pub order_id: std::num::NonZeroU32,
    /// The step being executed — retained so `advance_jump_step` can
    /// snap position to the target and apply the posture transition
    /// when the animation completes.
    pub step: JumpStep,
}

/// Active jump state stored on an actor.
///
/// Created by [`EngineInner::start_jump`] from a `Command::JumpCmd` sequence
/// element and drained by [`EngineInner::tick_active_jumps`].
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ActiveJump {
    /// Remaining steps to execute.
    pub steps: VecDeque<JumpStep>,
    /// State of the currently-executing step, or `None` if the next
    /// frame should pop a fresh step off `steps`.
    pub current: Option<CurrentStepState>,
    /// Sequence that owns this jump.  Terminated once all steps run.
    pub sequence_id: SequenceId,
    pub element_index: usize,
    /// Destination sector of the jump (for the post-jump sector swap).
    pub dest_sector: Option<u16>,
    pub dest_layer: u16,
    /// Exact bare-map-space `GetSector0to15()` of the normalized source jump
    /// line normal. Every jump take-off Execute arm installs this facing.
    pub source_direction_goal: i16,
    /// Projection-area probe used at landing. C++ asks the destination
    /// motion sector for the projection area at the destination jump-line
    /// midpoint, not necessarily at the exact landing point.
    pub dest_projection_point: MapPoint,
}

/// Produces a polyline of 3D waypoints from `start` to `dest` under
/// gravity with the character-mass apex.
///
/// The loop iterates at most 50 steps with `TIME_FLYSEGMENT = 4`
/// frames per segment; each step advances position by `2 * velocity`
/// and decreases `vz` by `2 * g * mass`.  When
/// `(destination - newPosition) · direction > -0.1`, the final point is
/// snapped to the destination and the loop ends.
pub fn compute_trajectory_jump(start: WorldPoint3D, dest: WorldPoint3D) -> Vec<WorldPoint3D> {
    let mut trajectory = Vec::new();

    let direction = WorldVec3D {
        x: dest.x - start.x,
        y: dest.y - start.y,
        z: dest.z - start.z,
    };

    // Re-use the ballistic helper but inline the zero-apex case so we
    // don't pull a target actor-forecast.
    let velocity =
        crate::bow_shot::compute_initial_throw_velocity(direction, 0.5, MASS_CHARACTER, 0, None);

    let fg = GRAVITY * MASS_CHARACTER;

    let mut position = start;
    let mut vz = velocity.z;

    for _ in 0..50 {
        let new_vz = fg * 2.0 + vz;

        if position.z < 0.0 && new_vz <= 0.0 {
            break;
        }

        let new_position = WorldPoint3D {
            x: velocity.x * 2.0 + position.x,
            y: velocity.y * 2.0 + position.y,
            z: vz * 2.0 + position.z,
        };

        // Escape clause: `direction · (newPosition - dest) > -0.1` —
        // we've reached (or overshot) the destination plane.
        let to_dest_x = new_position.x - dest.x;
        let to_dest_y = new_position.y - dest.y;
        let to_dest_z = new_position.z - dest.z;
        let proj = direction.x * to_dest_x + direction.y * to_dest_y + direction.z * to_dest_z;
        if proj > -0.1 {
            trajectory.push(dest);
            return trajectory;
        }

        trajectory.push(new_position);

        vz = new_vz;
        position = new_position;
    }

    trajectory
}

/// Build the step list for a jump.
///
/// `dest_forces_crouched` comes from the destination sector's
/// `is_forcing_crouched()` and `posture_before` is the actor's posture
/// at the moment the Jump command is dispatched.
pub fn build_jump_steps(
    source: &JumpLine,
    destination: &JumpLine,
    pt_source: crate::coordinates::MapPoint,
    posture_before: Posture,
    is_swordfighting: bool,
    dest_forces_crouched: bool,
    jump_height: f32,
) -> Vec<JumpStep> {
    let v_line = source.vector();
    let v_line_norm = (v_line.x * v_line.x + v_line.y * v_line.y).sqrt().max(1e-6);
    let v_line_n = MapVec {
        x: v_line.x / v_line_norm,
        y: v_line.y / v_line_norm,
    };

    // Project current position onto the source line.
    let dot = v_line_n.x * (pt_source.x - source.point_a.x)
        + v_line_n.y * (pt_source.y - source.point_a.y);
    let f_dot = dot.clamp(0.0, v_line_norm);

    // Destination on the paired line at the same parametric offset:
    // `destination.point_b + dot * line_reference`.
    let pt_destination = MapPoint {
        x: destination.point_b.x + f_dot * v_line_n.x,
        y: destination.point_b.y + f_dot * v_line_n.y,
    };

    let ratio = f_dot / v_line_norm;

    let z_source = source.z_a + ratio * (source.z_b - source.z_a);
    let z_destination = destination.z_b + ratio * (destination.z_b - destination.z_a);

    let pc_height = TELEPORT_JUMPING_UP
        + if posture_before == Posture::OnShoulders {
            40.0
        } else {
            0.0
        };

    let mut steps: Vec<JumpStep> = Vec::new();

    // ── Straight long jump ────────────────────────────────────────
    // Forced long jump OR `|jump_height| < pc_height`.
    if source.long_jump_forced || jump_height.abs() < pc_height {
        // Normal to the source line, facing the destination side.
        // The normal must point toward the destination.
        let normal = MapVec {
            x: -v_line_n.y,
            y: v_line_n.x,
        };
        // Ensure the normal points toward the destination line's B point.
        let to_dest_x = destination.point_b.x - source.point_a.x;
        let to_dest_y = destination.point_b.y - source.point_a.y;
        let sign = if normal.x * to_dest_x + normal.y * to_dest_y >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let v_normal_src = MapVec {
            x: normal.x * sign,
            y: normal.y * sign,
        };

        // Launch point sits 15u inside the destination side of the line.
        let pt_source_jump = MapPoint {
            x: pt_source.x + 15.0 * v_normal_src.x,
            y: pt_source.y + 15.0 * v_normal_src.y,
        };

        // 3D positions are stored as (x, y + z, z) — the world Y that
        // the sprite renders at bakes in the elevation.  Keeping this
        // convention means linear interpolation of the trajectory
        // produces the correct visual.
        let src_3d = WorldPoint3D {
            x: pt_source_jump.x,
            y: pt_source_jump.y + z_source,
            z: z_source,
        };
        let dst_3d = WorldPoint3D {
            x: pt_destination.x,
            y: pt_destination.y + z_destination,
            z: z_destination,
        };

        let trajectory = compute_trajectory_jump(src_3d, dst_3d);

        if is_swordfighting {
            // Sword variant (3 orders).
            steps.push(JumpStep {
                anim: OrderType::TransitionWaitingSwordJumpingLongSword,
                target_3d: Some(WorldPoint3D {
                    x: pt_source_jump.x,
                    y: pt_source_jump.y,
                    z: 0.0,
                }),
                airborne: false,
                max_frames: None,
            });
            steps.push(JumpStep {
                anim: OrderType::JumpingLongSword,
                target_3d: Some(dst_3d),
                airborne: true,
                max_frames: None,
            });
            steps.push(JumpStep {
                anim: OrderType::TransitionJumpingLongSwordWaitingSword,
                target_3d: None,
                airborne: false,
                max_frames: None,
            });
            return steps;
        }

        // Non-sword variant.
        if posture_before == Posture::Crouched {
            steps.push(JumpStep {
                anim: OrderType::TransitionCrouchingUp,
                target_3d: None,
                airborne: false,
                max_frames: None,
            });
        }

        let init_anim = if posture_before == Posture::OnShoulders {
            OrderType::TransitionWaitingOnShouldersJumpingLong
        } else {
            OrderType::TransitionWaitingUprightJumpingLong
        };
        steps.push(JumpStep {
            anim: init_anim,
            target_3d: Some(WorldPoint3D {
                x: pt_source_jump.x,
                y: pt_source_jump.y,
                z: 0.0,
            }),
            airborne: false,
            max_frames: None,
        });

        // One JumpingLong order per trajectory point.
        for pt in &trajectory {
            steps.push(JumpStep {
                anim: OrderType::JumpingLong,
                target_3d: Some(*pt),
                airborne: true,
                // TIME_FLYSEGMENT shapes the projectile trajectory. Runtime
                // order duration is derived independently from the exact
                // actor-to-waypoint 3D distance in Execute.
                max_frames: None,
            });
        }

        steps.push(JumpStep {
            anim: OrderType::TransitionJumpingLongWaitingUpright,
            target_3d: None,
            airborne: false,
            max_frames: None,
        });

        if posture_before == Posture::Crouched || dest_forces_crouched {
            steps.push(JumpStep {
                anim: OrderType::TransitionCrouchingDown,
                target_3d: None,
                airborne: false,
                max_frames: None,
            });
        }

        return steps;
    }

    // ── Jump up ────────────────────────────────────────────────────
    if jump_height > 0.0 {
        let normal = MapVec {
            x: -v_line_n.y,
            y: v_line_n.x,
        };
        // For jump-up the asserted sign is negative:
        // `normal · (source.A - destination.A) < 0`.  Flip so the
        // landing offset moves *away from* the destination edge (into
        // the landing sector).
        let to_src_x = source.point_a.x - destination.point_a.x;
        let to_src_y = source.point_a.y - destination.point_a.y;
        let sign = if normal.x * to_src_x + normal.y * to_src_y < 0.0 {
            1.0
        } else {
            -1.0
        };
        let v_normal_src = MapVec {
            x: normal.x * sign,
            y: normal.y * sign,
        };

        let pt_destination_jump = MapPoint {
            x: pt_destination.x - 15.0 * v_normal_src.x,
            y: pt_destination.y - 15.0 * v_normal_src.y,
        };

        if posture_before == Posture::Crouched {
            steps.push(JumpStep {
                anim: OrderType::TransitionCrouchingUp,
                target_3d: None,
                airborne: false,
                max_frames: None,
            });
        }

        if posture_before == Posture::OnShoulders {
            if jump_height < pc_height {
                // Descend from shoulders first, then take off as upright.
                steps.push(JumpStep {
                    anim: OrderType::ClimbingDownFromShoulders,
                    target_3d: None,
                    airborne: false,
                    max_frames: None,
                });
                steps.push(JumpStep {
                    anim: OrderType::TransitionWaitingUprightJumpingUp,
                    target_3d: None,
                    airborne: false,
                    max_frames: None,
                });
            } else {
                steps.push(JumpStep {
                    anim: OrderType::TransitionWaitingOnShouldersJumpingUp,
                    target_3d: None,
                    airborne: false,
                    max_frames: None,
                });
            }
        } else {
            steps.push(JumpStep {
                anim: OrderType::TransitionWaitingUprightJumpingUp,
                target_3d: None,
                airborne: false,
                max_frames: None,
            });
        }

        // Landing point 3D is (landX, landY + zDest, zDest - TELEPORT_JUMPING_UP).
        // The z subtracted is the extra lift before the actor lands on
        // the raised platform — during the JUMPING_UP animation the
        // sprite rises an additional TELEPORT_JUMPING_UP units before
        // clearing the edge, then settles onto the top.  We emit the
        // animation with target_3d at the apex (above the landing
        // pad), and the closing transition with target_3d at the
        // landing pad — so the sprite descends onto it.
        let apex_3d = WorldPoint3D {
            x: pt_destination_jump.x,
            y: pt_destination_jump.y + z_destination,
            z: z_destination + TELEPORT_JUMPING_UP,
        };
        let land_3d = WorldPoint3D {
            x: pt_destination.x,
            y: pt_destination.y + z_destination,
            z: z_destination,
        };

        steps.push(JumpStep {
            anim: OrderType::JumpingUp,
            target_3d: Some(apex_3d),
            airborne: true,
            max_frames: None,
        });
        steps.push(JumpStep {
            anim: OrderType::TransitionJumpingUpWaitingCrouched,
            target_3d: Some(land_3d),
            airborne: false,
            max_frames: None,
        });

        if posture_before != Posture::Crouched && !dest_forces_crouched {
            steps.push(JumpStep {
                anim: OrderType::TransitionCrouchingUp,
                target_3d: None,
                airborne: false,
                max_frames: None,
            });
        }

        return steps;
    }

    // ── Jump down ──────────────────────────────────────────────────
    let normal = MapVec {
        x: -v_line_n.y,
        y: v_line_n.x,
    };
    let to_dest_x = destination.point_b.x - source.point_a.x;
    let to_dest_y = destination.point_b.y - source.point_a.y;
    let sign = if normal.x * to_dest_x + normal.y * to_dest_y > 0.0 {
        1.0
    } else {
        -1.0
    };
    let v_normal_src = MapVec {
        x: normal.x * sign,
        y: normal.y * sign,
    };
    let pt_source_jump = MapPoint {
        x: pt_source.x + 15.0 * v_normal_src.x,
        y: pt_source.y + 15.0 * v_normal_src.y,
    };

    if posture_before != Posture::Crouched {
        steps.push(JumpStep {
            anim: OrderType::TransitionCrouchingDown,
            target_3d: None,
            airborne: false,
            max_frames: None,
        });
    }

    steps.push(JumpStep {
        anim: OrderType::TransitionWaitingCrouchedJumpingDown,
        target_3d: Some(WorldPoint3D {
            x: pt_source_jump.x,
            y: pt_source_jump.y,
            z: 0.0,
        }),
        airborne: false,
        max_frames: None,
    });

    let land_3d = WorldPoint3D {
        x: pt_destination.x,
        y: pt_destination.y + z_destination,
        z: z_destination,
    };
    steps.push(JumpStep {
        anim: OrderType::JumpingDown,
        target_3d: Some(land_3d),
        airborne: true,
        max_frames: None,
    });

    steps.push(JumpStep {
        anim: OrderType::TransitionJumpingDownWaitingCrouched,
        target_3d: None,
        airborne: false,
        max_frames: None,
    });

    if posture_before != Posture::Crouched && !dest_forces_crouched {
        steps.push(JumpStep {
            anim: OrderType::TransitionCrouchingUp,
            target_3d: None,
            airborne: false,
            max_frames: None,
        });
    }

    steps
}

// ═══════════════════════════════════════════════════════════════════
//  Per-line reachability
// ═══════════════════════════════════════════════════════════════════

/// Returns `true` when the given jump line sits in the PC's current
/// sector and the owning jump gate authorizes this PC to take it.
///
/// `return_true_on_no_test_posture` is hardcoded to `true` at this
/// call site.  The owning gate is resolved here by scanning the door
/// table for a jump gate that references this line (`JumpLine` has no
/// back-pointer to its gate).
pub fn is_jumpable(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    doors: &[crate::gate::Door],
    jump_line_idx: u32,
    pc_sector_grid_idx: u32,
    pc_auth: &crate::gate::ActorAuthInfo,
    test_posture: bool,
) -> bool {
    let Some(line) = fast_grid.level.jump_lines.get(jump_line_idx as usize) else {
        return false;
    };
    // Jump line's home sector must match the PC's current sector.
    let Some(home_sector_idx) = line.sector_index else {
        return false;
    };
    if u32::from(home_sector_idx) != pc_sector_grid_idx {
        return false;
    }

    // Find the owning jump gate — the door whose `jump_line_out` or
    // `jump_line_in` references this line.
    let Some(gate) = doors.iter().find(|d| {
        d.gate_type == crate::gate::GateType::Jump
            && (d.jump_line_out == Some(jump_line_idx) || d.jump_line_in == Some(jump_line_idx))
    }) else {
        return false;
    };

    // Inline jump-gate authorization with
    // `return_true_on_no_test_posture = true`.  The generic
    // `Door::is_actor_authorized` path can't see the destination
    // line's `helper_needed` flag, so we do the posture check here.
    if !(pc_auth.kind.is_pc() && pc_auth.has_jump) {
        return false;
    }
    // `direct ⇔ jump_line == gate.jump_line_out` — PC is on the
    // out-side line, so the *destination* (helper check) is the
    // in-side line, and vice versa.
    let direct = gate.jump_line_out == Some(jump_line_idx);
    let dest_line_idx = if direct {
        gate.jump_line_in
    } else {
        gate.jump_line_out
    };
    let helper_needed = dest_line_idx
        .and_then(|idx| fast_grid.level.jump_lines.get(idx as usize))
        .map(|l| l.helper_needed)
        .unwrap_or(false);
    if helper_needed {
        if test_posture {
            pc_auth.posture == crate::element::Posture::OnShoulders
        } else {
            // `return_true_on_no_test_posture` — authorize the jump
            // even though the helper test was skipped.
            true
        }
    } else {
        true
    }
}

/// Walks the clicked jump sector's jump lines, filters through
/// [`is_jumpable`] against the PC's home sector, and returns the index
/// of the line whose paired (destination) line's midpoint is nearest
/// `pt_goal` plus own midpoint nearest `pt_start`.
///
/// This mirrors `RHSectorJump::GetNearestJumpableJumpLine`: candidate
/// lines come from the hovered/clicked jump zone, while authorization
/// rejects lines whose source sector is not the actor's current sector.
#[allow(clippy::too_many_arguments)]
pub fn get_nearest_jumpable_jump_line(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    doors: &[crate::gate::Door],
    pc_sector_grid_idx: u32,
    candidate_sector_grid_idx: u32,
    pc_auth: &crate::gate::ActorAuthInfo,
    pt_start: MapPoint,
    pt_goal: MapPoint,
    test_posture: bool,
    preferred_destination_sector: Option<u16>,
) -> Option<u32> {
    let sector = fast_grid
        .level
        .sectors
        .get(candidate_sector_grid_idx as usize)?;
    let mut best_preferred: Option<(u32, f32)> = None;
    let mut best_any: Option<(u32, f32)> = None;
    for &line_idx in &sector.jump_line_indices {
        let line_idx_u32 = u32::from(line_idx);
        if !is_jumpable(
            fast_grid,
            doors,
            line_idx_u32,
            pc_sector_grid_idx,
            pc_auth,
            test_posture,
        ) {
            continue;
        }
        let Some(line) = fast_grid.level.jump_lines.get(usize::from(line_idx)) else {
            continue;
        };
        let Some(assoc_idx) = line.associated_line_index else {
            continue;
        };
        let Some(assoc) = fast_grid.level.jump_lines.get(assoc_idx as usize) else {
            continue;
        };

        let line_mid = line.get_middle_point();
        let assoc_mid = assoc.get_middle_point();
        let dx_g = assoc_mid.x - pt_goal.x;
        let dy_g = assoc_mid.y - pt_goal.y;
        let dx_s = line_mid.x - pt_start.x;
        let dy_s = line_mid.y - pt_start.y;
        let d = dx_g * dx_g + dy_g * dy_g + dx_s * dx_s + dy_s * dy_s;
        if best_any.map(|(_, bd)| d < bd).unwrap_or(true) {
            best_any = Some((line_idx_u32, d));
        }
        let destination_sector_matches = preferred_destination_sector
            .map(|sector| jump_line_sector_number(fast_grid, assoc) == Some(sector))
            .unwrap_or(false);
        if destination_sector_matches && best_preferred.map(|(_, bd)| d < bd).unwrap_or(true) {
            best_preferred = Some((line_idx_u32, d));
        }
    }
    best_preferred.or(best_any).map(|(idx, _)| idx)
}

fn jump_line_sector_number(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    line: &JumpLine,
) -> Option<u16> {
    let sector_index = line.sector_index?;
    let sector = fast_grid.level.sectors.get(usize::from(sector_index))?;
    Some(u16::from(sector.sector_number))
}

// ═══════════════════════════════════════════════════════════════════
//  EngineInner-side driver: start / tick / advance the jump.
// ═══════════════════════════════════════════════════════════════════

impl EngineInner {
    /// Convenience wrapper around [`is_jumpable`] that resolves the
    /// PC entity's sector + auth info through the engine.  Returns
    /// `false` when any of the required data is missing (no mission
    /// script, entity, sector mapping, etc.).
    pub fn is_jumpable(&self, jump_line_idx: u32, pc_entity: EntityId, test_posture: bool) -> bool {
        let Some(entity) = self.world.entities.get(pc_entity) else {
            return false;
        };
        let Some(sector_num) = entity.element_data().sector() else {
            return false;
        };
        let Some(&pc_sector_grid_idx) =
            self.world
                .fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(
                    u16::from(sector_num) as i16
                ))
        else {
            return false;
        };
        let Some(doors) = self
            .scripts
            .mission
            .as_ref()
            .map(|_| self.script_domains.interactables.doors.as_slice())
        else {
            return false;
        };
        let pc_auth = entity.actor_auth_info();
        is_jumpable(
            &self.world.fast_grid,
            doors,
            jump_line_idx,
            pc_sector_grid_idx as u32,
            &pc_auth,
            test_posture,
        )
    }

    /// Convenience wrapper around [`get_nearest_jumpable_jump_line`].
    pub fn get_nearest_jumpable_jump_line(
        &self,
        pc_entity: EntityId,
        candidate_sector_grid_idx: u32,
        pt_start: MapPoint,
        pt_goal: MapPoint,
        test_posture: bool,
        preferred_destination_sector: Option<u16>,
    ) -> Option<u32> {
        let entity = self.world.entities.get(pc_entity)?;
        let sector_num = entity.element_data().sector()?;
        let &pc_sector_grid_idx =
            self.world
                .fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(
                    u16::from(sector_num) as i16
                ))?;
        let doors = self
            .scripts
            .mission
            .as_ref()
            .map(|_| self.script_domains.interactables.doors.as_slice())?;
        let pc_auth = entity.actor_auth_info();
        get_nearest_jumpable_jump_line(
            &self.world.fast_grid,
            doors,
            pc_sector_grid_idx as u32,
            candidate_sector_grid_idx,
            &pc_auth,
            pt_start,
            pt_goal,
            test_posture,
            preferred_destination_sector,
        )
    }

    /// Dispatcher entry point for `Command::JumpCmd`.  Reads jump-line
    /// source/destination from the sequence element's properties,
    /// builds the step list via [`build_jump_steps`], installs
    /// [`ActiveJump`] on the actor, and marks the element in-progress.
    ///
    /// Returns `true` if the jump was installed, `false` if required
    /// data (jump lines, actor) was missing — in which case the
    /// caller should terminate the element so the sequence does not
    /// stall.
    pub(super) fn start_jump(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> bool {
        // Read jump-line IDs from the element.
        let (src_id, dst_id) = {
            let elem = match self.orders.sequence_manager.get_element(seq_id, elem_idx) {
                Some(e) => e,
                None => return false,
            };
            let src = elem
                .get_property(crate::sequence::Field::JumplineSource)
                .and_then(|v| match v {
                    crate::sequence::FieldValue::LineId(id) => Some(*id),
                    crate::sequence::FieldValue::Integer(id) => {
                        crate::jump_line::JumpLineIndex::new(*id)
                    }
                    _ => None,
                });
            let dst = elem
                .get_property(crate::sequence::Field::JumplineDestination)
                .and_then(|v| match v {
                    crate::sequence::FieldValue::LineId(id) => Some(*id),
                    crate::sequence::FieldValue::Integer(id) => {
                        crate::jump_line::JumpLineIndex::new(*id)
                    }
                    _ => None,
                });
            match (src, dst) {
                (Some(s), Some(d)) => (s, d),
                _ => return false,
            }
        };

        // Clone the jump lines so we can call build_jump_steps without
        // borrowing self.world.fast_grid while we need &mut self.world.entities.
        let (src_line, dst_line) = {
            let src = self
                .world
                .fast_grid
                .level
                .jump_lines
                .get(usize::from(src_id))
                .cloned();
            let dst = self
                .world
                .fast_grid
                .level
                .jump_lines
                .get(usize::from(dst_id))
                .cloned();
            match (src, dst) {
                (Some(s), Some(d)) => (s, d),
                _ => return false,
            }
        };

        // Destination sector's force-crouched flag.  Looked up via
        // the destination line's `sector_index`.
        let dest_forces_crouched = dst_line
            .sector_index
            .and_then(|idx| self.world.fast_grid.level.sectors.get(usize::from(idx)))
            .map(|s| s.force_crouched)
            .unwrap_or(false);

        let dest_sector = jump_line_sector_number(&self.world.fast_grid, &dst_line);
        let dest_layer = dst_line.layer;
        let dest_projection_point = dst_line.get_middle_point();
        let source_vector = src_line.vector();
        let source_length =
            (source_vector.x * source_vector.x + source_vector.y * source_vector.y).sqrt();
        if source_length <= f32::EPSILON {
            tracing::warn!(src_id = %src_id, "Jump: source line has zero length");
            return false;
        }
        let source_direction_goal = crate::position_interface::vector_to_sector_0_to_15(
            -source_vector.y / source_length,
            source_vector.x / source_length,
        );

        // `jump_height = associated.z_a - line.z_a`.  For our source
        // line, `associated` is the paired dst line.
        let jump_height = dst_line.z_a - src_line.z_a;

        let (pt_source, posture_before, is_swordfighting) = {
            let Some(entity) = self.world.entities.get(owner) else {
                return false;
            };
            let elem_data = entity.element_data();
            let pos = elem_data.position_map();
            let posture = elem_data.posture;
            let is_sf = entity
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false);
            (pos, posture, is_sf)
        };

        // A vertical (up / down) jump forces `quit_swordfight` on the
        // jumper because the jump-up / jump-down animations have no
        // sword-variant pair — continuing to fight would leave both
        // parties dangling in combat state with no valid animations.
        // Long jumps have a dedicated sword branch and keep the fight
        // going.  The `long_jump_forced || |h| < pc_height` test
        // decides the branch here.
        let pc_height_est = TELEPORT_JUMPING_UP
            + if posture_before == Posture::OnShoulders {
                40.0
            } else {
                0.0
            };
        let is_long_branch = src_line.long_jump_forced || jump_height.abs() < pc_height_est;
        if is_swordfighting && !is_long_branch {
            self.quit_swordfight(sim, assets, owner);
        }

        let steps = build_jump_steps(
            &src_line,
            &dst_line,
            pt_source,
            posture_before,
            // After `quit_swordfight` the actor's opponent list is
            // empty, so the long-jump branch itself never runs the
            // sword path when we've already quit.  Pass the updated
            // flag to keep `build_jump_steps` consistent with state.
            is_swordfighting && is_long_branch,
            dest_forces_crouched,
            jump_height,
        );

        if steps.is_empty() {
            return false;
        }

        // The jump translator authors the whole order list before Instruct
        // reads the element's current order, so the actor's order pointer is
        // already the first jump order on the frame the command is accepted.
        // Rust drives the steps from `ActiveJump` and republishes one order
        // per step as it starts; author the head order here so the pointer is
        // live immediately instead of one frame later. `start_step` reuses an
        // order id whose animation already matches, so the head order keeps
        // its identity when the first step actually begins.
        let first_step_order = {
            let step = &steps[0];
            let target_map = step
                .target_3d
                .filter(|_| !step.airborne)
                .map(crate::coordinates::WorldPoint3D::to_map)
                .unwrap_or_default();
            let order_id = self.orders.allocate_order_id();
            let mut order =
                crate::order::Order::new(step.anim, target_map.x, target_map.y, order_id);
            order.compute_direction = false;
            order.completion = crate::order::OrderCompletion::NextJumpStep;
            order
        };
        if let Some(elem) = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
        {
            elem.orders.clear();
            elem.orders.push_back(first_step_order);
        }

        let active = ActiveJump {
            steps: steps.into(),
            current: None,
            sequence_id: seq_id,
            element_index: elem_idx,
            dest_sector,
            dest_layer,
            dest_projection_point,
            source_direction_goal,
        };

        // Install on the actor and reset any stale flight state.
        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.clear_path();
            actor.active_jump = Some(active);
            actor.jump_z_offset = 0.0;
            // Translation only installs the jump orders. The outgoing
            // movement state remains observable until the first jump order
            // executes in the actor's next creation slot.
        }

        tracing::debug!(
            entity = ?owner,
            src_id = %src_id,
            dst_id = %dst_id,
            ?posture_before,
            jump_height,
            "Jump: starting RHCOMMAND_JUMP sequence"
        );
        true
    }

    /// Per-frame tick of all active jumps.  Advances position
    /// interpolation for the currently-executing step (if any), pops
    /// steps when the animation finishes, and terminates the sequence
    /// element once the step list is drained.
    ///
    /// Animation advance is handled by the normal animation tick; this
    /// function reads the completion signal via `active_ai_anim` being
    /// cleared (with `AiAnimCompletion::NextJumpStep`) and forwards it
    /// to [`EngineInner::advance_jump_step`].  Position interpolation is
    /// done here so it runs every frame, not just on animation end.
    pub(super) fn tick_active_jumps(&mut self, assets: &LevelAssets) {
        // Entities whose current order reached its exact Execute termination
        // boundary. We advance them after the entity borrow closes.
        let mut force_advance: Vec<EntityId> = Vec::new();
        // Collected here during the main loop, applied after — each
        // entry is `(seq_id, elem_idx, order)` for the step that just
        // started.  `next_order_id` is stamped into the order AFTER
        // the loop closes so the sequence-manager borrow doesn't
        // overlap with the entity borrow.
        let mut jump_orders: Vec<(crate::sequence::SequenceId, usize, crate::order::Order)> =
            Vec::new();
        // PCs whose just-popped step is a jump-init transition — they
        // need an `MSG_DISABLE_ALL_ACTIONS_TEMP` message dispatched
        // after the entity loop closes.
        let mut pending_init_messages: Vec<EntityId> = Vec::new();
        // Disjoint-borrow trick: we need `&mut self.world.entities` for the
        // loop AND `&mut self.orders.next_order_id` for the new step's order
        // tag. Splitting them through a local re-borrow.
        let next_order_id = &mut self.orders.next_order_id;
        let sequence_manager = &self.orders.sequence_manager;
        for (entity_id, entity) in self.world.entities.actors_mut() {
            let Some(actor) = entity.actor_data_mut() else {
                continue;
            };
            let Some(jump) = actor.active_jump.as_mut() else {
                continue;
            };

            // Start the next step if we don't have a current one.
            if jump.current.is_none() {
                let step = match jump.steps.pop_front() {
                    Some(s) => s,
                    None => {
                        // No more steps — jump is done.  Signal the
                        // sequence element and swap layer/sector.
                        let seq_id = jump.sequence_id;
                        let elem_idx = jump.element_index;
                        actor.active_jump = None;
                        actor.jump_z_offset = 0.0;
                        actor.action_state = ActionState::Waiting;
                        // Defer sequence termination to after the loop.
                        actor.pending_jump_done = Some((seq_id, elem_idx));
                        continue;
                    }
                };
                // For the four jump initiation transitions: forward
                // `MSG_DISABLE_ALL_ACTIONS_TEMP` so the action strip
                // greys out abilities for the duration of the jump.
                // Collected and dispatched after the entity-loop borrow
                // closes.
                if matches!(
                    step.anim,
                    OrderType::TransitionWaitingUprightJumpingUp
                        | OrderType::TransitionWaitingCrouchedJumpingDown
                        | OrderType::TransitionWaitingUprightJumpingLong
                        | OrderType::TransitionWaitingSwordJumpingLongSword
                ) && entity.is_pc()
                {
                    pending_init_messages.push(entity_id.into());
                }
                if let Some(order) = start_step(
                    entity,
                    entity_id.into(),
                    step,
                    next_order_id,
                    sequence_manager,
                ) {
                    jump_orders.push(order);
                }
                if entity.actor_data().is_some_and(|actor| {
                    actor.active_jump.as_ref().is_some_and(|jump| {
                        jump.current
                            .as_ref()
                            .is_some_and(|state| state.step.airborne && actor.wait_time == 0)
                    })
                }) {
                    force_advance.push(entity_id.into());
                }
                continue;
            }

            // A step is in progress — advance interpolation.
            let current = jump.current.as_ref().expect("current step exists");
            let current_anim = current.step.anim;
            let transition_reached_terminal_tick =
                !current.step.airborne && current.frames_elapsed >= current.total_frames;
            if transition_reached_terminal_tick
                && matches!(
                    current_anim,
                    OrderType::TransitionWaitingUprightJumpingUp
                        | OrderType::TransitionWaitingCrouchedJumpingDown
                )
            {
                // These Execute arms apply flight state on
                // RHMOTION_TERMINATED. Jump stepping precedes the owner
                // animation pass, so expose it once the authored duration
                // elapsed on the preceding frames. The long take-off arms
                // instead terminate from their own sprite motion inside the
                // owner slot, which applies the same state there.
                entity.set_posture(Posture::Flying);
                if let Some(actor) = entity.actor_data_mut() {
                    actor.action_state = ActionState::Moving;
                }
                force_advance.push(entity_id.into());
            }
            if jump_step_turns(current_anim) {
                entity.position_iface_mut().turn();
            }
            advance_step_interpolation(entity);
            if entity.actor_data().is_some_and(|actor| {
                actor.active_jump.as_ref().is_some_and(|jump| {
                    jump.current
                        .as_ref()
                        .is_some_and(|state| state.step.airborne && actor.wait_time == 0)
                })
            }) {
                force_advance.push(entity_id.into());
            }

            // If the step has a max-frames cap (TIME_FLYSEGMENT for
            // airborne trajectory segments), mark it for early
            // advance once the cap is reached.
            if let Some(actor) = entity.actor_data()
                && let Some(jump) = actor.active_jump.as_ref()
                && let Some(state) = jump.current.as_ref()
                && let Some(cap) = state.step.max_frames
                && state.frames_elapsed >= cap
            {
                force_advance.push(entity_id.into());
            }
        }

        // Push each new-step order onto the jump's sequence element
        // after the entity-loop borrow closes.
        for (seq_id, elem_idx, order) in jump_orders {
            if let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(seq_id, elem_idx)
            {
                elem.orders.clear();
                elem.orders.push_back(order);
            }
        }

        // Dispatch `MSG_DISABLE_ALL_ACTIONS_TEMP` for jump-init
        // transitions whose steps just started this tick — addressed
        // to the PC actor.  `value` carries the PC entity id so the
        // dispatch in `tick.rs` targets the specific PC rather than
        // fanning over the selection.
        for pc_id in pending_init_messages {
            self.orders.messenger.send(crate::messenger::Message::pc(
                crate::messenger::PcMessage::DisableAllActionsTemp,
                Some(pc_id),
            ));
        }

        // Force-advance entities whose current step reached its Execute
        // termination boundary.
        for entity_id in force_advance {
            if let Some((new_layer, new_sector, projection_point)) =
                self.advance_jump_step(entity_id)
            {
                self.finalize_airborne_jump_landing(
                    assets,
                    entity_id,
                    new_layer,
                    new_sector,
                    projection_point,
                );
            }
        }

        // Drain pending_jump_done — terminate sequence elements for
        // jumps that finished this tick.
        let mut to_terminate: Vec<(SequenceId, usize)> = Vec::new();
        for (_, entity) in self.world.entities.actors_mut() {
            let Some(actor) = entity.actor_data_mut() else {
                continue;
            };
            if let Some((seq_id, elem_idx)) = actor.pending_jump_done.take() {
                to_terminate.push((seq_id, elem_idx));
            }
        }
        for (seq_id, elem_idx) in to_terminate {
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
        }
    }

    /// Called from the animation tick when a jump step's
    /// `active_ai_anim` completes.  Snaps position to the step's 3D
    /// target (eliminating any lingering drift from the linear-
    /// interpolation frame counter), applies the end-of-animation
    /// posture transition, and clears `current` so the next tick pops
    /// the next step.
    pub(super) fn advance_jump_step(
        &mut self,
        entity_id: EntityId,
    ) -> Option<(u16, Option<u16>, MapPoint)> {
        let Some(entity) = self.world.entities.get_mut(entity_id) else {
            return None;
        };

        // Take the completed step out of the jump state.
        let (finished, next_anim, landing_finalize, jump_completion, next_step_publish) = {
            let Some(actor) = entity.actor_data_mut() else {
                return None;
            };
            let Some(jump) = actor.active_jump.as_mut() else {
                return None;
            };
            match jump.current.take() {
                Some(s) => {
                    let next_anim = jump.steps.front().map(|step| step.anim);
                    let landing_finalize = (s.step.airborne && next_anim != Some(s.step.anim))
                        .then_some((
                            jump.dest_layer,
                            jump.dest_sector,
                            jump.dest_projection_point,
                        ));
                    let jump_completion = next_anim
                        .is_none()
                        .then_some((jump.sequence_id, jump.element_index));
                    // Retiring an order makes the following one current in the
                    // same frame: the actor's order pointer never reads the
                    // exhausted animation once its motion terminated. The step
                    // itself does not begin until the next frame, so only the
                    // order is authored here.
                    let next_step_publish = jump.steps.front().map(|step| {
                        let target_map = step
                            .target_3d
                            .filter(|_| !step.airborne)
                            .map(crate::coordinates::WorldPoint3D::to_map)
                            .unwrap_or_default();
                        (jump.sequence_id, jump.element_index, step.anim, target_map)
                    });
                    (
                        s,
                        next_anim,
                        landing_finalize,
                        jump_completion,
                        next_step_publish,
                    )
                }
                None => return None,
            }
        };

        // ── Snap position to the step's end ──────────────────────
        // If the step had a 3D target, ensure the position lands
        // exactly on it, independent of frame-count drift. Airborne
        // steps mirror C++ `SetPosition(pointDestination3D)`.
        if let Some(target) = finished.step.target_3d {
            if finished.step.airborne && next_anim != Some(finished.step.anim) {
                entity.element_data_mut().set_position(target);
            }
            if let Some(actor) = entity.actor_data_mut() {
                actor.jump_z_offset = 0.0;
            }
        } else if let Some(actor) = entity.actor_data_mut() {
            // No target: in-place transition.  A non-airborne
            // in-place step should have `jump_z_offset == 0` when it
            // finishes (the previous airborne step's snap already set
            // it, but guard anyway).
            if !finished.step.airborne {
                actor.jump_z_offset = 0.0;
            }
        }

        // ── Apply posture transition ─────────────────────────────
        // Per-order posture assignment for each transition / climbing
        // animation family.  Without this the actor's posture state
        // desyncs from what the sprite is visually showing and
        // downstream movement / animation picks the wrong idle.
        let posture_after = match finished.step.anim {
            OrderType::TransitionCrouchingDown => Some(crate::element::Posture::Crouched),
            OrderType::TransitionCrouchingUp => Some(crate::element::Posture::Upright),
            OrderType::TransitionJumpingLongWaitingUpright => {
                Some(crate::element::Posture::Upright)
            }
            OrderType::TransitionJumpingUpWaitingCrouched
            | OrderType::TransitionJumpingDownWaitingCrouched => {
                Some(crate::element::Posture::Crouched)
            }
            // `TransitionJumpingLongSwordWaitingSword` lands back in
            // sword stance — posture (Upright) is unchanged, and the
            // `action_state` is restored to `WaitingSword` below so
            // the sword-specific idle animation picks up.
            OrderType::ClimbingDownFromShoulders => Some(crate::element::Posture::Upright),
            _ => None,
        };
        if let Some(p) = posture_after {
            entity.set_posture(p);
        }

        if matches!(
            finished.step.anim,
            OrderType::TransitionWaitingUprightJumpingUp
                | OrderType::TransitionWaitingCrouchedJumpingDown
                | OrderType::TransitionWaitingUprightJumpingLong
                | OrderType::TransitionWaitingSwordJumpingLongSword
        ) {
            entity.set_posture(Posture::Flying);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state =
                    if finished.step.anim == OrderType::TransitionWaitingSwordJumpingLongSword {
                        ActionState::MovingSword
                    } else {
                        ActionState::Moving
                    };
            }
        }

        // Sword long-jump returns the actor to `WaitingSword` idle so
        // the sword-specific idle animation picks up when the next
        // tick clears `active_ai_anim` — the post-
        // `JUMPING_LONG_SWORD_WAITING_SWORD` actor remains in sword
        // fighting state.
        if finished.step.anim == OrderType::TransitionJumpingLongSwordWaitingSword
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.action_state = ActionState::WaitingSword;
        }
        if jump_completion.is_some() {
            if let Some(actor) = entity.actor_data_mut() {
                actor.active_jump = None;
                actor.jump_z_offset = 0.0;
            }
            entity.position_iface_mut().set_anti_collision_on(true);
        }

        // For the three jump landing transitions: re-broadcast
        // `MSG_DISABLE_ALL_ACTIONS_TEMP` if the landing sector forces
        // crouching, otherwise `MSG_ENABLE_ALL_ACTIONS_TEMP`, and
        // unconditionally `MSG_STATURE` so the HUD picks up the
        // post-landing posture.
        let is_landing_pc = entity.is_pc()
            && matches!(
                finished.step.anim,
                OrderType::TransitionJumpingUpWaitingCrouched
                    | OrderType::TransitionJumpingDownWaitingCrouched
                    | OrderType::TransitionJumpingLongWaitingUpright
                    | OrderType::TransitionJumpingLongSwordWaitingSword
            );
        let landing_sector: Option<crate::sector::SectorNumber> = if is_landing_pc {
            entity
                .element_data()
                .sector()
                .map(|s| crate::sector::SectorNumber::from(i16::from(s)))
        } else {
            None
        };
        // `entity` borrow ends here so `self` can be re-borrowed below.
        if let Some((sequence_id, element_index, anim, target_map)) = next_step_publish {
            let order_id = self.orders.allocate_order_id();
            let mut order = crate::order::Order::new(anim, target_map.x, target_map.y, order_id);
            order.compute_direction = false;
            order.completion = crate::order::OrderCompletion::NextJumpStep;
            if let Some(elem) = self
                .orders
                .sequence_manager
                .get_element_mut(sequence_id, element_index)
            {
                elem.orders.clear();
                elem.orders.push_back(order);
            }
            // Retiring an order republishes the actor's order pointer within
            // the same slot, so it already names the following animation
            // before the frame ends. The step itself still begins next frame.
            self.world
                .entities
                .get_mut(entity_id)
                .and_then(crate::element::Entity::actor_data_mut)
                .expect("jump step owner disappeared before its next order was published")
                .installed_order = Some(crate::element::InstalledActorOrder {
                order_id,
                order_type: anim,
            });
        }
        if is_landing_pc {
            let force_crouched = landing_sector
                .map(|n| self.sector_forces_crouch(n))
                .unwrap_or(false);
            let pc_msg = if force_crouched {
                crate::messenger::PcMessage::DisableAllActionsTemp
            } else {
                crate::messenger::PcMessage::EnableAllActionsTemp
            };
            self.orders
                .messenger
                .send(crate::messenger::Message::pc(pc_msg, Some(entity_id)));
            self.orders.messenger.send(crate::messenger::Message::new(
                crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
            ));
        }
        if let Some((sequence_id, element_index)) = jump_completion {
            self.orders
                .sequence_manager
                .element_terminated(sequence_id, element_index);
        }
        landing_finalize
    }

    pub(super) fn finalize_airborne_jump_landing(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        new_layer: u16,
        new_sector: Option<u16>,
        projection_point: MapPoint,
    ) {
        let Some(position) = self
            .get_entity(entity_id)
            .map(|entity| entity.element_data().position())
        else {
            tracing::warn!(?entity_id, "jump landing lost actor");
            return;
        };
        self.finalize_special_move_position_with_ground(
            assets,
            entity_id,
            super::special_motion::SpecialMovePosition::World(position),
            Some(new_layer),
            new_sector,
            projection_point,
            "jump landing",
        );
    }
}

/// Initialize per-step state and install the animation on the actor.
///
/// When the new step's animation matches the one that just ended
/// (consecutive `JumpingLong` segments across a multi-waypoint
/// trajectory), we reuse the previous order_id so the sprite keeps
/// cycling instead of resetting to frame 0 — N sequential
/// `JUMPING_LONG` orders share the same sprite animation state
/// machine.
fn start_step(
    entity: &mut crate::element::Entity,
    entity_id: EntityId,
    step: JumpStep,
    next_order_id: &mut u32,
    sequence_manager: &crate::sequence::SequenceManager,
) -> Option<(crate::sequence::SequenceId, usize, crate::order::Order)> {
    if step.airborne {
        start_airborne_jump_motion(entity, &step);
        entity.position_iface_mut().turn();
    } else if matches!(
        step.anim,
        OrderType::TransitionWaitingOnShouldersJumpingUp
            | OrderType::TransitionWaitingOnShouldersJumpingLong
            | OrderType::TransitionWaitingUprightJumpingUp
            | OrderType::TransitionWaitingCrouchedJumpingDown
            | OrderType::TransitionWaitingUprightJumpingLong
            | OrderType::TransitionWaitingSwordJumpingLongSword
    ) {
        // Every jump take-off Execute arm faces the source line's normalized
        // normal, including jump-up orders that have no 2D destination.
        let source_direction_goal = entity
            .actor_data()
            .and_then(|actor| actor.active_jump.as_ref())
            .map(|jump| jump.source_direction_goal)?;
        let position_iface = entity.position_iface_mut();
        position_iface.set_direction(crate::position_interface::Direction::from_raw(
            source_direction_goal.into(),
        ));
        position_iface.set_anti_collision_on(false);
        // The take-off arms that move toward their authored point initialize
        // their motion order inside the shared sprite motion path, which is
        // what seeds both the goal and its increment.
        position_iface.turn();

        // Shoulder-assisted take-offs establish flight on START. The other
        // take-off transitions do so only when their animation terminates.
        if matches!(
            step.anim,
            OrderType::TransitionWaitingOnShouldersJumpingUp
                | OrderType::TransitionWaitingOnShouldersJumpingLong
        ) {
            entity.set_posture(Posture::Flying);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::Moving;
            }
        }
    } else if matches!(
        step.anim,
        OrderType::TransitionJumpingUpWaitingCrouched
            | OrderType::TransitionJumpingDownWaitingCrouched
            | OrderType::TransitionJumpingLongWaitingUpright
            | OrderType::TransitionJumpingLongSwordWaitingSword
    ) {
        // Landing Execute arms establish the waiting posture/action on
        // RHMOTION_START, before the landing animation completes.
        let (posture, action) = match step.anim {
            OrderType::TransitionJumpingUpWaitingCrouched
            | OrderType::TransitionJumpingDownWaitingCrouched => {
                (Posture::Crouched, ActionState::Waiting)
            }
            OrderType::TransitionJumpingLongSwordWaitingSword => {
                (Posture::Upright, ActionState::WaitingSword)
            }
            _ => (Posture::Upright, ActionState::Waiting),
        };
        entity.set_posture(posture);
        if let Some(actor) = entity.actor_data_mut() {
            actor.action_state = action;
        }
        entity.position_iface_mut().turn();
    }

    let (start_x, start_y, start_z) = {
        if step.airborne {
            let pos = entity.element_data().position();
            (pos.x, pos.y, pos.z)
        } else {
            let pos = entity.element_data().position_map();
            let z = entity.actor_data().map(|a| a.jump_z_offset).unwrap_or(0.0);
            (pos.x, pos.y, z)
        }
    };

    // Sprite's animation duration drives the per-frame increment.
    let total_frames = {
        let n = entity.element_data().sprite.total_ticks_for_anim(step.anim);
        if n > 0 { n } else { 1 }
    };

    // Reuse the previous order_id if we're restarting the same
    // animation — keeps the sprite's row/frame state machine in sync
    // instead of hard-resetting mid-jump.
    let (jump_seq, jump_elem) = entity
        .actor_data()
        .and_then(|a| a.active_jump.as_ref())
        .map(|j| (j.sequence_id, j.element_index))?;

    let prev_anim = sequence_manager
        .current_order_for_actor(entity_id)
        .map(|(_, _, o)| (o.order_type, o.order_id));
    let order_id = match prev_anim {
        Some((anim_type, order_id)) if anim_type == step.anim => order_id,
        _ => crate::order::alloc_order_id(next_order_id),
    };

    let state = CurrentStepState {
        start_x,
        start_y,
        start_z,
        total_frames,
        frames_elapsed: 0,
        order_id,
        step: step.clone(),
    };

    if let Some(actor) = entity.actor_data_mut()
        && let Some(jump) = actor.active_jump.as_mut()
    {
        jump.current = Some(state);
        actor.active_jump_target_3d = step.target_3d;
        actor.active_jump_airborne = step.airborne;
    }

    // Airborne Execute calls UpdatePosition on its first START tick. Ground
    // transition motion remains owned by the sprite motion driver, whose
    // first-frame distance may legitimately be zero.
    if step.airborne {
        advance_step_interpolation(entity);
    }

    // Build the order to push after the loop closes.  `NextJumpStep`
    // completion routes the motion-terminated signal through
    // `process_anim_completion_outcomes → advance_jump_step`.
    let target_map = step
        .target_3d
        .filter(|_| !step.airborne)
        .map(crate::coordinates::WorldPoint3D::to_map)
        .unwrap_or_default();
    let mut order = crate::order::Order::new(step.anim, target_map.x, target_map.y, order_id);
    tracing::trace!(
        ?entity_id,
        anim = ?step.anim,
        airborne = step.airborne,
        order_id = order_id.get(),
        reused_order_id = prev_anim.is_some_and(|(a, _)| a == step.anim),
        total_frames,
        target = ?target_map,
        "jump step started"
    );
    // Every order authored by Original's TranslateJump explicitly disables
    // generic movement-direction computation. Jump initiation sets its
    // source-line-normal facing above; airborne and landing orders retain it.
    order.compute_direction = false;
    order.completion = crate::order::OrderCompletion::NextJumpStep;
    Some((jump_seq, jump_elem, order))
}

fn start_airborne_jump_motion(entity: &mut crate::element::Entity, step: &JumpStep) {
    entity.set_posture(Posture::Flying);

    let Some(target) = step.target_3d else {
        tracing::warn!(?step.anim, "airborne jump step missing 3D target");
        return;
    };
    let position = entity.element_data().position();
    let dx = target.x - position.x;
    let dy = target.y - position.y;
    let dz = target.z - position.z;
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();
    if distance <= f32::EPSILON {
        tracing::warn!(?step.anim, "airborne jump step has zero-length motion");
        return;
    }
    let mut wait_time = (distance / jump_airborne_speed(step.anim) - 1.0) as u32;
    if wait_time == 0 {
        wait_time = 1;
    }

    if let Some(actor) = entity.actor_data_mut() {
        actor.action_state = if step.anim == OrderType::JumpingLongSword {
            ActionState::MovingSword
        } else {
            ActionState::Moving
        };
        // Original reuses mulWaitTime for seek refresh and jump segment
        // duration. Starting flight overwrites the old seek value rather than
        // preserving a second logical countdown.
        actor.seek_refresh_wait = 0;
        actor.wait_time = wait_time;
    }
}

/// Ground transition steps whose Execute arm drives the sprite through
/// `PerformMotion` with `MotionMethod::TillLastFrame` instead of the plain
/// action path: they initialize a real motion order (goal + increment) and
/// advance the animation once on their START tick.
///
/// Every other jump step plays its animation in place, even where the
/// authored order carries a 2D destination.
pub(crate) fn jump_step_uses_perform_motion(anim: OrderType) -> bool {
    matches!(
        anim,
        OrderType::TransitionWaitingUprightJumpingLong
            | OrderType::TransitionWaitingSwordJumpingLongSword
            | OrderType::TransitionJumpingUpWaitingCrouched
    )
}

/// Run one tick of a ground transition step through the shared sprite motion
/// path and commit the resulting displacement.
///
/// These arms disable anti-collision before their first motion tick, so the
/// per-frame distance goes straight onto the map position. On reaching the
/// goal the motion stops and snaps exactly onto it.
pub(crate) fn perform_jump_ground_motion(
    entity: &mut crate::element::Entity,
    sim: &crate::sim_rng::SimulationContext,
    motion_order: crate::sprite::MotionOrderContext,
    anim: OrderType,
    row: u16,
) -> crate::sprite::MotionState {
    let sprite = &mut entity.element_data_mut().sprite;
    let (state, frame_distance) = sprite.perform_motion(
        sim,
        Some(motion_order),
        anim,
        row,
        crate::sprite::FrameProgression::Default,
        false,
        crate::sprite::MotionMethod::TillLastFrame,
        false,
    );

    let pi = &mut sprite.position_iface;
    if pi.is_anti_collision_on() {
        tracing::warn!(
            ?anim,
            "jump ground transition executed with anti-collision still enabled"
        );
    }
    tracing::trace!(
        ?anim,
        order_id = motion_order.order_id.get(),
        ?state,
        frame_distance,
        current_frame = sprite.current_frame,
        frame_count = sprite.frame_count,
        pos = ?pi.map_position(),
        goal = ?pi.map_goal(),
        increment = ?pi.get_increment_map(),
        "jump ground transition motion tick"
    );
    let distance = super::movement::scaled_motion_distance(
        frame_distance,
        1.0,
        false,
        pi.get_direction() != pi.get_direction_goal(),
    );
    if distance != 0.0 {
        pi.update_position_map_scaled(distance);
        let wait = sprite.wait_time(sprite.current_row, sprite.current_frame);
        sprite
            .position_iface
            .update_forecasted_movement(distance, wait + 1);
        let pi = &mut sprite.position_iface;

        let increment = pi.get_increment_map();
        if (increment.x != 0.0 || increment.y != 0.0) && pi.is_goal_reached_undeviated() {
            pi.zero_all_increments();
            if pi.get_tolerance() == 0.0 {
                let goal = pi.map_goal();
                pi.set_map_position(goal);
            }
        }
        entity.element_data_mut().update_grid_cell();
    }

    state
}

fn jump_step_turns(anim: OrderType) -> bool {
    matches!(
        anim,
        OrderType::TransitionWaitingOnShouldersJumpingUp
            | OrderType::TransitionWaitingOnShouldersJumpingLong
            | OrderType::TransitionWaitingUprightJumpingUp
            | OrderType::TransitionWaitingCrouchedJumpingDown
            | OrderType::TransitionWaitingUprightJumpingLong
            | OrderType::TransitionWaitingSwordJumpingLongSword
            | OrderType::JumpingUp
            | OrderType::JumpingDown
            | OrderType::JumpingLong
            | OrderType::JumpingLongSword
            | OrderType::TransitionJumpingUpWaitingCrouched
            | OrderType::TransitionJumpingDownWaitingCrouched
            | OrderType::TransitionJumpingLongWaitingUpright
            | OrderType::TransitionJumpingLongSwordWaitingSword
    )
}

fn jump_airborne_speed(anim: OrderType) -> f32 {
    match anim {
        OrderType::JumpingLong | OrderType::JumpingLongSword => 8.0,
        OrderType::JumpingUp => 15.0,
        OrderType::JumpingDown => 20.0,
        _ => {
            tracing::warn!(
                ?anim,
                "airborne jump step used non-jump animation; falling back to long-jump speed"
            );
            8.0
        }
    }
}

/// Per-frame position interpolation for the in-progress airborne step.
///
/// Flight advances in absolute 3D at the animation's fixed speed, matching
/// the jump Execute arms that set a 3D increment and call UpdatePosition
/// every tick. Ground steps are driven by the sprite motion path instead and
/// only carry their frame counter here.
fn advance_step_interpolation(entity: &mut crate::element::Entity) {
    let (target_3d, airborne, mut state) = {
        let Some(actor) = entity.actor_data() else {
            return;
        };
        let Some(jump) = actor.active_jump.as_ref() else {
            return;
        };
        let Some(state) = jump.current.clone() else {
            return;
        };
        (
            actor.active_jump_target_3d,
            actor.active_jump_airborne,
            state,
        )
    };

    state.frames_elapsed = state.frames_elapsed.saturating_add(1);

    if airborne {
        if let Some(target) = target_3d {
            let full_dx = target.x - state.start_x;
            let full_dy = target.y - state.start_y;
            let full_dz = target.z - state.start_z;
            let full_dist = (full_dx * full_dx + full_dy * full_dy + full_dz * full_dz).sqrt();

            let frame_dist = jump_airborne_speed(state.step.anim);

            if full_dist > f32::EPSILON && frame_dist > 0.0 {
                let dir_x = full_dx / full_dist;
                let dir_y = full_dy / full_dist;
                let dir_z = full_dz / full_dist;
                let elem = entity.element_data_mut();
                let pos = elem.position();
                let new_x = pos.x + dir_x * frame_dist;
                let new_y = pos.y + dir_y * frame_dist;
                let new_z = pos.z + dir_z * frame_dist;
                let travelled_new = (new_x - state.start_x) * dir_x
                    + (new_y - state.start_y) * dir_y
                    + (new_z - state.start_z) * dir_z;
                if travelled_new >= full_dist {
                    elem.set_position(target);
                } else {
                    elem.set_position(crate::coordinates::WorldPoint3D {
                        x: new_x,
                        y: new_y,
                        z: new_z,
                    });
                }
            }
        } else {
            tracing::warn!(?state.step.anim, "airborne jump step missing 3D target");
        }

        if let Some(actor) = entity.actor_data_mut() {
            actor.jump_z_offset = 0.0;
            actor.wait_time = actor.wait_time.saturating_sub(1);
        }
    }

    // Save updated state.
    if let Some(actor) = entity.actor_data_mut()
        && let Some(jump) = actor.active_jump.as_mut()
    {
        jump.current = Some(state);
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Tests — kept at the bottom of the file.
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trajectory_ends_at_destination() {
        // Horizontal jump across 300 units with a 50-unit rise.
        let start = WorldPoint3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let dest = WorldPoint3D {
            x: 300.0,
            y: 0.0,
            z: 50.0,
        };
        let traj = compute_trajectory_jump(start, dest);
        assert!(
            !traj.is_empty(),
            "trajectory should produce at least one point"
        );
        let last = traj.last().unwrap();
        assert!(
            (last.x - dest.x).abs() < 1.0 && (last.z - dest.z).abs() < 5.0,
            "trajectory last point {:?} should be near destination {:?}",
            last,
            dest
        );
    }

    #[test]
    fn long_jump_step_list_has_trajectory() {
        // Two parallel jump lines 100 apart at the same elevation.
        let mut src = JumpLine::new(
            crate::coordinates::map_pt(0.0, 0.0),
            crate::coordinates::map_pt(100.0, 0.0),
            0.0,
            0.0,
        );
        let mut dst = JumpLine::new(
            crate::coordinates::map_pt(100.0, 100.0),
            crate::coordinates::map_pt(0.0, 100.0),
            0.0,
            0.0,
        );
        src.associated_line_index = Some(0);
        dst.associated_line_index = Some(0);

        let pt = crate::coordinates::MapPoint { x: 50.0, y: 0.0 };
        let steps = build_jump_steps(
            &src,
            &dst,
            pt,
            Posture::Upright,
            /* is_swordfighting */ false,
            /* dest_forces_crouched */ false,
            /* jump_height */ 0.0,
        );
        // Upright → long jump → transition + N×JumpingLong + closing transition
        assert!(
            steps.len() >= 3,
            "expected at least 3 steps, got {}",
            steps.len()
        );
        assert_eq!(
            steps.first().unwrap().anim,
            OrderType::TransitionWaitingUprightJumpingLong
        );
        assert!(steps.iter().any(|s| s.anim == OrderType::JumpingLong));
        assert_eq!(
            steps.last().unwrap().anim,
            OrderType::TransitionJumpingLongWaitingUpright
        );
    }

    #[test]
    fn airborne_jump_speeds_match_original_execute() {
        // RHelementactorpc.cpp Execute() scales jump increments by
        // 8/15/20 units per tick for long/up/down jump animations.
        assert_eq!(jump_airborne_speed(OrderType::JumpingLong), 8.0);
        assert_eq!(jump_airborne_speed(OrderType::JumpingLongSword), 8.0);
        assert_eq!(jump_airborne_speed(OrderType::JumpingUp), 15.0);
        assert_eq!(jump_airborne_speed(OrderType::JumpingDown), 20.0);
    }

    #[test]
    fn jump_up_emits_jumping_up_step() {
        let src = JumpLine::new(
            crate::coordinates::map_pt(0.0, 0.0),
            crate::coordinates::map_pt(100.0, 0.0),
            0.0,
            0.0,
        );
        let dst = JumpLine::new(
            crate::coordinates::map_pt(100.0, 100.0),
            crate::coordinates::map_pt(0.0, 100.0),
            100.0,
            100.0,
        );

        let pt = crate::coordinates::MapPoint { x: 50.0, y: 0.0 };
        let steps = build_jump_steps(
            &src,
            &dst,
            pt,
            Posture::Upright,
            false,
            false,
            /* jump_height */ 100.0,
        );
        assert!(steps.iter().any(|s| s.anim == OrderType::JumpingUp));
        assert!(
            steps
                .iter()
                .any(|s| s.anim == OrderType::TransitionJumpingUpWaitingCrouched)
        );
    }

    #[test]
    fn jump_down_emits_jumping_down_step() {
        let src = JumpLine::new(
            crate::coordinates::map_pt(0.0, 0.0),
            crate::coordinates::map_pt(100.0, 0.0),
            100.0,
            100.0,
        );
        let dst = JumpLine::new(
            crate::coordinates::map_pt(100.0, 100.0),
            crate::coordinates::map_pt(0.0, 100.0),
            0.0,
            0.0,
        );

        let pt = crate::coordinates::MapPoint { x: 50.0, y: 0.0 };
        let steps = build_jump_steps(
            &src,
            &dst,
            pt,
            Posture::Upright,
            false,
            false,
            /* jump_height */ -100.0,
        );
        assert!(steps.iter().any(|s| s.anim == OrderType::JumpingDown));
        assert!(
            steps
                .iter()
                .any(|s| s.anim == OrderType::TransitionJumpingDownWaitingCrouched)
        );
    }

    // ── is_jumpable ──

    /// Build a minimal FastFindGrid + doors fixture with two jump
    /// lines in distinct sectors joined by a single jump gate.  The
    /// line at index 0 lives in `sector_a` (grid idx 0), paired with
    /// the line at index 1 in `sector_b` (grid idx 1).  `dst_helper`
    /// controls the paired line's `helper_needed` flag.
    fn make_jumpable_fixture(
        dst_helper: bool,
    ) -> (crate::fast_find_grid::FastFindGrid, Vec<crate::gate::Door>) {
        use crate::fast_find_grid::{FastFindGrid, GridSector};
        use crate::sector::SectorType;

        let mut grid = FastFindGrid::new();
        grid.size_map(4, 4);
        grid.allocate_layers(1);

        // Two motion-area sectors.  Points / bboxes don't actually
        // matter for is_jumpable; what matters is `jump_line_indices`
        // and the grid-flat sector index.
        let make_sector = |sn: i16| GridSector {
            points: vec![
                MapPoint::new(0.0, 0.0),
                MapPoint::new(64.0, 0.0),
                MapPoint::new(64.0, 64.0),
                MapPoint::new(0.0, 64.0),
            ],
            bounding_box: {
                let mut b = crate::coordinates::MapBBox::new();
                b.expand_point(MapPoint::new(0.0, 0.0));
                b.expand_point(MapPoint::new(64.0, 64.0));
                b
            },
            sector_type: SectorType::MOUSE | SectorType::MOTION | SectorType::AREA,
            layer: 0,
            sector_number: crate::sector::SectorNumber::new(sn),
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
        };
        let sec_a = make_sector(10);
        let sec_b = make_sector(11);
        grid.add_sector(sec_a, 0);
        grid.add_sector(sec_b, 0);
        // Populate `sector_number_map` explicitly — not set by
        // `add_sector`.  Callers that resolve PC sectors via the map
        // rely on this.
        grid.level_mut()
            .sector_number_map
            .insert(crate::sector::SectorNumber::new(10), 0);
        grid.level_mut()
            .sector_number_map
            .insert(crate::sector::SectorNumber::new(11), 1);

        // Two paired jump lines.
        let mut jl_a = JumpLine::new(
            crate::coordinates::map_pt(0.0, 0.0),
            crate::coordinates::map_pt(64.0, 0.0),
            0.0,
            0.0,
        );
        jl_a.sector_index = crate::fast_find_grid::SectorIndex::new(0);
        jl_a.associated_line_index = Some(1);
        let mut jl_b = JumpLine::new(
            crate::coordinates::map_pt(0.0, 64.0),
            crate::coordinates::map_pt(64.0, 64.0),
            0.0,
            0.0,
        );
        jl_b.sector_index = crate::fast_find_grid::SectorIndex::new(1);
        jl_b.associated_line_index = Some(0);
        jl_b.helper_needed = dst_helper;
        grid.level_mut().jump_lines.push(jl_a);
        grid.level_mut().jump_lines.push(jl_b);
        // Register each line on its home sector so
        // `get_nearest_jumpable_jump_line` can find them.
        grid.level_mut().sectors[0]
            .jump_line_indices
            .push(crate::jump_line::JumpLineIndex::new(0).unwrap());
        grid.level_mut().sectors[1]
            .jump_line_indices
            .push(crate::jump_line::JumpLineIndex::new(1).unwrap());

        // Single jump gate covering the pair.
        let gate = crate::gate::Door {
            gate_type: crate::gate::GateType::Jump,
            jump_line_out: Some(1), // jl_b is the "out" side
            jump_line_in: Some(0),  // jl_a is the "in" side
            ..Default::default()
        };
        (grid, vec![gate])
    }

    fn pc_auth(has_jump: bool, posture: Posture) -> crate::gate::ActorAuthInfo {
        crate::gate::ActorAuthInfo {
            kind: crate::element_kinds::ElementKind::ActorPc,
            pc_auth_bit: 0x0001,
            has_lockpick: false,
            has_climb: false,
            has_jump,
            is_rider: false,
            posture,
        }
    }

    #[test]
    fn is_jumpable_same_sector_passes() {
        let (grid, doors) = make_jumpable_fixture(false);
        let pc = pc_auth(true, Posture::Upright);
        // PC is in sector 0 (grid idx 0).  jl_a (idx 0) is in that
        // sector and has a jump gate — jumpable.
        assert!(is_jumpable(&grid, &doors, 0, 0, &pc, false));
    }

    #[test]
    fn is_jumpable_different_sector_fails() {
        let (grid, doors) = make_jumpable_fixture(false);
        let pc = pc_auth(true, Posture::Upright);
        // PC is in sector 0 (grid idx 0) but we ask about jl_b (idx
        // 1), which lives in sector 1 — not jumpable.
        assert!(!is_jumpable(&grid, &doors, 1, 0, &pc, false));
    }

    #[test]
    fn is_jumpable_no_jump_action_fails() {
        let (grid, doors) = make_jumpable_fixture(false);
        let pc = pc_auth(/* has_jump */ false, Posture::Upright);
        assert!(!is_jumpable(&grid, &doors, 0, 0, &pc, false));
    }

    #[test]
    fn is_jumpable_helper_needed_respects_posture() {
        // PC wants to jump onto jl_b (helper_needed destination).
        // With test_posture=true and posture != OnShoulders → blocked.
        // With OnShoulders → allowed.  With test_posture=false →
        // allowed regardless (return_true_on_no_test_posture=true).
        let (grid, doors) = make_jumpable_fixture(true);
        let upright = pc_auth(true, Posture::Upright);
        let on_shoulders = pc_auth(true, Posture::OnShoulders);

        assert!(!is_jumpable(&grid, &doors, 0, 0, &upright, true));
        assert!(is_jumpable(&grid, &doors, 0, 0, &on_shoulders, true));
        // test_posture=false skips the posture gate.
        assert!(is_jumpable(&grid, &doors, 0, 0, &upright, false));
    }

    #[test]
    fn nearest_jumpable_picks_closest_destination() {
        let (grid, doors) = make_jumpable_fixture(false);
        let pc = pc_auth(true, Posture::Upright);
        // The clicked jump sector lists line 0, and line 0 is usable
        // from the PC's sector — it should be picked.
        let got = get_nearest_jumpable_jump_line(
            &grid,
            &doors,
            0,
            0,
            &pc,
            MapPoint::new(32.0, 0.0),
            MapPoint::new(32.0, 64.0),
            false,
            None,
        );
        assert_eq!(got, Some(0));
    }

    #[test]
    fn nearest_jumpable_rejects_unrelated_clicked_jump_sector() {
        let (grid, doors) = make_jumpable_fixture(false);
        let pc = pc_auth(true, Posture::Upright);

        // PC is in sector 0, but the clicked jump sector lists the
        // opposite-side line.  C++ iterates the clicked jump zone's
        // line list and then rejects this line because it does not
        // belong to the PC's current sector.
        let got = get_nearest_jumpable_jump_line(
            &grid,
            &doors,
            0,
            1,
            &pc,
            MapPoint::new(32.0, 0.0),
            MapPoint::new(32.0, 64.0),
            false,
            None,
        );
        assert_eq!(got, None);
    }

    #[test]
    fn nearest_jumpable_prefers_clicked_destination_sector() {
        let (mut grid, mut doors) = make_jumpable_fixture(false);
        let pc = pc_auth(true, Posture::Upright);

        let mut alternate = JumpLine::new(
            MapPoint::new(0.0, 10.0),
            MapPoint::new(64.0, 10.0),
            0.0,
            0.0,
        );
        alternate.sector_index = crate::fast_find_grid::SectorIndex::new(0);
        alternate.associated_line_index = Some(3);
        let mut alternate_dest = JumpLine::new(
            MapPoint::new(0.0, 80.0),
            MapPoint::new(64.0, 80.0),
            0.0,
            0.0,
        );
        alternate_dest.sector_index = crate::fast_find_grid::SectorIndex::new(2);
        alternate_dest.associated_line_index = Some(2);
        grid.level_mut().jump_lines.push(alternate);
        grid.level_mut().jump_lines.push(alternate_dest);
        grid.level_mut().sectors[0]
            .jump_line_indices
            .push(crate::jump_line::JumpLineIndex::new(2).unwrap());

        let mut preferred_sector = grid.level.sectors[1].clone();
        preferred_sector.sector_number = crate::sector::SectorNumber::new(22);
        preferred_sector.jump_line_indices = Vec::new();
        grid.level_mut().sectors.push(preferred_sector);
        grid.level_mut()
            .sector_number_map
            .insert(crate::sector::SectorNumber::new(22), 2);

        doors.push(crate::gate::Door {
            gate_type: crate::gate::GateType::Jump,
            jump_line_out: Some(2),
            jump_line_in: Some(3),
            ..doors[0].clone()
        });

        let got = get_nearest_jumpable_jump_line(
            &grid,
            &doors,
            0,
            0,
            &pc,
            MapPoint::new(32.0, 0.0),
            MapPoint::new(32.0, 64.0),
            false,
            Some(22),
        );
        assert_eq!(got, Some(2));
    }

    #[test]
    fn jump_destination_sector_uses_sector_number_not_grid_index() {
        let (grid, _doors) = make_jumpable_fixture(false);
        let destination_line = &grid.level.jump_lines[1];

        assert_eq!(
            destination_line.sector_index.map(usize::from),
            Some(1),
            "fixture should keep the grid index distinct from sector number"
        );
        assert_eq!(jump_line_sector_number(&grid, destination_line), Some(11));
    }

    #[test]
    fn sword_long_jump_uses_sword_variants() {
        let src = JumpLine::new(
            crate::coordinates::map_pt(0.0, 0.0),
            crate::coordinates::map_pt(100.0, 0.0),
            0.0,
            0.0,
        );
        let dst = JumpLine::new(
            crate::coordinates::map_pt(100.0, 100.0),
            crate::coordinates::map_pt(0.0, 100.0),
            0.0,
            0.0,
        );
        let pt = crate::coordinates::MapPoint { x: 50.0, y: 0.0 };
        let steps = build_jump_steps(
            &src,
            &dst,
            pt,
            Posture::Upright,
            /* is_swordfighting */ true,
            false,
            0.0,
        );
        assert_eq!(steps.len(), 3);
        assert_eq!(
            steps[0].anim,
            OrderType::TransitionWaitingSwordJumpingLongSword
        );
        assert_eq!(steps[1].anim, OrderType::JumpingLongSword);
        assert_eq!(
            steps[2].anim,
            OrderType::TransitionJumpingLongSwordWaitingSword
        );
    }
}
