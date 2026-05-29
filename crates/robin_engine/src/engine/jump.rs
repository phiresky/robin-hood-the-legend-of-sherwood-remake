//! Line-jump sequence.
//!
//! A line-jump is a `Command::Jump` sequence element that moves the PC
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
//! position over the step's animation duration and lifting the actor's
//! [`ActorData::jump_z_offset`] during airborne segments so the renderer
//! can place the sprite above the ground.  When the last step terminates,
//! the owning sequence element is notified via
//! [`SequenceManager::element_terminated`].

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::element::{ActionState, EntityId, Point2D, Point3D, Posture};
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
/// actor's 2D position plus `jump_z_offset` interpolate linearly from
/// the start of the step to the target across the animation's duration.
/// If `None`, the animation plays in place (transition crouch up/down,
/// waiting↔jumping transitions, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct JumpStep {
    /// The animation to play during this step.
    pub anim: OrderType,
    /// Optional 3D destination for the step.  `None` means the animation
    /// plays in place with no position change.
    pub target_3d: Option<Point3D>,
    /// Whether this step's animation places the actor airborne.  During
    /// airborne steps the renderer lifts the sprite by `jump_z_offset`;
    /// on a ground step `jump_z_offset` is snapped to zero on arrival.
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
/// Created by [`EngineInner::start_jump`] from a `Command::Jump` sequence
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
}

/// Produces a polyline of 3D waypoints from `start` to `dest` under
/// gravity with the character-mass apex.
///
/// The loop iterates at most 50 steps with `TIME_FLYSEGMENT = 4`
/// frames per segment; each step advances position by `2 * velocity`
/// and decreases `vz` by `2 * g * mass`.  When
/// `(destination - newPosition) · direction > -0.1`, the final point is
/// snapped to the destination and the loop ends.
pub fn compute_trajectory_jump(start: Point3D, dest: Point3D) -> Vec<Point3D> {
    let mut trajectory = Vec::new();

    let direction = Point3D {
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

        let new_position = Point3D {
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
    let v_line_n = Point2D {
        x: v_line.x / v_line_norm,
        y: v_line.y / v_line_norm,
    };

    // Project current position onto the source line.
    let dot = v_line_n.x * (pt_source.x - source.point_a.x)
        + v_line_n.y * (pt_source.y - source.point_a.y);
    let f_dot = dot.clamp(0.0, v_line_norm);

    // Destination on the paired line at the same parametric offset:
    // `destination.point_b + dot * line_reference`.
    let pt_destination = Point2D {
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
        let normal = Point2D {
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
        let v_normal_src = Point2D {
            x: normal.x * sign,
            y: normal.y * sign,
        };

        // Launch point sits 15u inside the destination side of the line.
        let pt_source_jump = Point2D {
            x: pt_source.x + 15.0 * v_normal_src.x,
            y: pt_source.y + 15.0 * v_normal_src.y,
        };

        // 3D positions are stored as (x, y + z, z) — the world Y that
        // the sprite renders at bakes in the elevation.  Keeping this
        // convention means linear interpolation of the trajectory
        // produces the correct visual.
        let src_3d = Point3D {
            x: pt_source_jump.x,
            y: pt_source_jump.y + z_source,
            z: z_source,
        };
        let dst_3d = Point3D {
            x: pt_destination.x,
            y: pt_destination.y + z_destination,
            z: z_destination,
        };

        let trajectory = compute_trajectory_jump(src_3d, dst_3d);

        if is_swordfighting {
            // Sword variant (3 orders).
            steps.push(JumpStep {
                anim: OrderType::TransitionWaitingSwordJumpingLongSword,
                target_3d: Some(Point3D {
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
            target_3d: Some(Point3D {
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
                // Each trajectory segment runs for 4 frames
                // (`TIME_FLYSEGMENT`), rolling over to the next before
                // the full JumpingLong cycle.
                max_frames: Some(TIME_FLYSEGMENT),
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
        let normal = Point2D {
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
        let v_normal_src = Point2D {
            x: normal.x * sign,
            y: normal.y * sign,
        };

        let pt_destination_jump = Point2D {
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
        let apex_3d = Point3D {
            x: pt_destination_jump.x,
            y: pt_destination_jump.y + z_destination,
            z: z_destination + TELEPORT_JUMPING_UP,
        };
        let land_3d = Point3D {
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
    let normal = Point2D {
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
    let v_normal_src = Point2D {
        x: normal.x * sign,
        y: normal.y * sign,
    };
    let pt_source_jump = Point2D {
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
        target_3d: Some(Point3D {
            x: pt_source_jump.x,
            y: pt_source_jump.y,
            z: 0.0,
        }),
        airborne: false,
        max_frames: None,
    });

    let land_3d = Point3D {
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

/// Walks the PC's home sector's jump lines, filters through
/// [`is_jumpable`], and returns the index of the line whose paired
/// (destination) line's midpoint is nearest `pt_goal` plus own midpoint
/// nearest `pt_start`.
pub fn get_nearest_jumpable_jump_line(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    doors: &[crate::gate::Door],
    pc_sector_grid_idx: u32,
    pc_auth: &crate::gate::ActorAuthInfo,
    pt_start: crate::geo2d::Point2D,
    pt_goal: crate::geo2d::Point2D,
    test_posture: bool,
) -> Option<u32> {
    let sector = fast_grid.level.sectors.get(pc_sector_grid_idx as usize)?;
    let mut best: Option<(u32, f32)> = None;
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
        if best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((line_idx_u32, d));
        }
    }
    best.map(|(idx, _)| idx)
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
        let Some(Some(entity)) = self.entities.get(pc_entity.0 as usize) else {
            return false;
        };
        let Some(sector_num) = entity.element_data().sector() else {
            return false;
        };
        let Some(&pc_sector_grid_idx) =
            self.fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(
                    u16::from(sector_num) as i16
                ))
        else {
            return false;
        };
        let Some(doors) = self
            .mission_script
            .as_ref()
            .and_then(|s| s.game_host())
            .map(|gh| gh.doors.as_slice())
        else {
            return false;
        };
        let pc_auth = entity.actor_auth_info();
        is_jumpable(
            &self.fast_grid,
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
        pt_start: crate::geo2d::Point2D,
        pt_goal: crate::geo2d::Point2D,
        test_posture: bool,
    ) -> Option<u32> {
        let entity = self.entities.get(pc_entity.0 as usize)?.as_ref()?;
        let sector_num = entity.element_data().sector()?;
        let &pc_sector_grid_idx =
            self.fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(
                    u16::from(sector_num) as i16
                ))?;
        let doors = self
            .mission_script
            .as_ref()
            .and_then(|s| s.game_host())
            .map(|gh| gh.doors.as_slice())?;
        let pc_auth = entity.actor_auth_info();
        get_nearest_jumpable_jump_line(
            &self.fast_grid,
            doors,
            pc_sector_grid_idx as u32,
            &pc_auth,
            pt_start,
            pt_goal,
            test_posture,
        )
    }

    /// Dispatcher entry point for `Command::Jump`.  Reads jump-line
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
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> bool {
        // Read jump-line IDs from the element.
        let (src_id, dst_id) = {
            let elem = match self.sequence_manager.get_element(seq_id, elem_idx) {
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
        // borrowing self.fast_grid while we need &mut self.entities.
        let (src_line, dst_line) = {
            let src = self
                .fast_grid
                .level
                .jump_lines
                .get(usize::from(src_id))
                .cloned();
            let dst = self
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
            .and_then(|idx| self.fast_grid.level.sectors.get(usize::from(idx)))
            .map(|s| s.force_crouched)
            .unwrap_or(false);

        let dest_sector = dst_line.sector_index.map(|i| u32::from(i) as u16);
        let dest_layer = dst_line.layer;

        // `jump_height = associated.z_a - line.z_a`.  For our source
        // line, `associated` is the paired dst line.
        let jump_height = dst_line.z_a - src_line.z_a;

        let (pt_source, posture_before, is_swordfighting) = {
            let Some(Some(entity)) = self.entities.get(owner.0 as usize) else {
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
            self.quit_swordfight(assets, owner);
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

        let active = ActiveJump {
            steps: steps.into(),
            current: None,
            sequence_id: seq_id,
            element_index: elem_idx,
            dest_sector,
            dest_layer,
        };

        // Install on the actor and reset any stale flight state.
        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.clear_path();
            actor.active_jump = Some(active);
            actor.jump_z_offset = 0.0;
            // Flip to Waiting so the movement tick skips this actor
            // while the jump drives position directly.
            actor.action_state = ActionState::Waiting;
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
        let mut layer_updates: Vec<(EntityId, u16, Option<u16>)> = Vec::new();
        // Entities whose current step has reached its `max_frames`
        // cap — we force-advance them after the loop (can't call
        // advance_jump_step inline due to the mutable borrow on
        // `self.entities`).  Each trajectory segment pops after
        // `TIME_FLYSEGMENT` frames regardless of the sprite
        // animation's natural length.
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
        // Disjoint-borrow trick: we need `&mut self.entities` for the
        // loop AND `&mut self.next_order_id` for the new step's order
        // tag. Splitting them through a local re-borrow.
        let next_order_id = &mut self.next_order_id;
        let sequence_manager = &self.sequence_manager;
        for (idx, slot) in self.entities.iter_mut().enumerate() {
            let entity = match slot {
                Some(e) => e,
                None => continue,
            };
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
                        let dest_sector = jump.dest_sector;
                        let dest_layer = jump.dest_layer;
                        actor.active_jump = None;
                        actor.jump_z_offset = 0.0;
                        actor.action_state = ActionState::Waiting;
                        layer_updates.push((EntityId(idx as u32), dest_layer, dest_sector));
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
                    pending_init_messages.push(EntityId(idx as u32));
                }
                if let Some(order) = start_step(
                    entity,
                    EntityId(idx as u32),
                    step,
                    next_order_id,
                    sequence_manager,
                ) {
                    jump_orders.push(order);
                }
                continue;
            }

            // A step is in progress — advance interpolation.
            advance_step_interpolation(entity);

            // If the step has a max-frames cap (TIME_FLYSEGMENT for
            // airborne trajectory segments), mark it for early
            // advance once the cap is reached.
            if let Some(actor) = entity.actor_data()
                && let Some(jump) = actor.active_jump.as_ref()
                && let Some(state) = jump.current.as_ref()
                && let Some(cap) = state.step.max_frames
                && state.frames_elapsed >= cap
            {
                force_advance.push(EntityId(idx as u32));
            }
        }

        // Push each new-step order onto the jump's sequence element
        // after the entity-loop borrow closes.
        for (seq_id, elem_idx, order) in jump_orders {
            if let Some(elem) = self.sequence_manager.get_element_mut(seq_id, elem_idx) {
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
            self.messenger.send(crate::messenger::Message::pc(
                crate::messenger::PcMessage::DisableAllActionsTemp,
                Some(pc_id),
            ));
        }

        // Force-advance entities whose current step hit its frame cap.
        for entity_id in force_advance {
            self.advance_jump_step(entity_id);
        }

        // Apply destination layer/sector swaps and dispatch sequence
        // termination for jumps that finished this tick.
        for (entity_id, new_layer, new_sector) in layer_updates {
            if let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) {
                let elem = entity.element_data_mut();
                elem.set_layer(new_layer);
                if let Some(s) = new_sector {
                    elem.set_sector(crate::position_interface::SectorHandle::new(s));
                }
            }
            // Refresh opponent jump-line state after every
            // layer/sector swap.
            self.update_opponents_jump_lines(assets, entity_id);
        }

        // Drain pending_jump_done — terminate sequence elements for
        // jumps that finished this tick.
        let mut to_terminate: Vec<(SequenceId, usize)> = Vec::new();
        for slot in self.entities.iter_mut() {
            let entity = match slot {
                Some(e) => e,
                None => continue,
            };
            let Some(actor) = entity.actor_data_mut() else {
                continue;
            };
            if let Some((seq_id, elem_idx)) = actor.pending_jump_done.take() {
                to_terminate.push((seq_id, elem_idx));
            }
        }
        for (seq_id, elem_idx) in to_terminate {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        }
    }

    /// Called from the animation tick when a jump step's
    /// `active_ai_anim` completes.  Snaps position to the step's 3D
    /// target (eliminating any lingering drift from the linear-
    /// interpolation frame counter), applies the end-of-animation
    /// posture transition, and clears `current` so the next tick pops
    /// the next step.
    pub(super) fn advance_jump_step(&mut self, entity_id: EntityId) {
        let Some(Some(entity)) = self.entities.get_mut(entity_id.0 as usize) else {
            return;
        };

        // Take the completed step out of the jump state.
        let finished = {
            let Some(actor) = entity.actor_data_mut() else {
                return;
            };
            let Some(jump) = actor.active_jump.as_mut() else {
                return;
            };
            match jump.current.take() {
                Some(s) => s,
                None => return,
            }
        };

        // ── Snap position to the step's end ──────────────────────
        // If the step had a 3D target, ensure `position_map` + the
        // visual lift land exactly on it, independent of how well the
        // per-frame counter tracked the sprite's true duration.
        if let Some(target) = finished.step.target_3d {
            let elem = entity.element_data_mut();
            elem.set_position_map(crate::coordinates::MapPoint {
                x: target.x,
                y: target.y - target.z,
            });
            if let Some(actor) = entity.actor_data_mut() {
                actor.jump_z_offset = if finished.step.airborne {
                    target.z
                } else {
                    0.0
                };
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
        if is_landing_pc {
            let force_crouched = landing_sector
                .map(|n| self.sector_forces_crouch(n))
                .unwrap_or(false);
            let pc_msg = if force_crouched {
                crate::messenger::PcMessage::DisableAllActionsTemp
            } else {
                crate::messenger::PcMessage::EnableAllActionsTemp
            };
            self.messenger
                .send(crate::messenger::Message::pc(pc_msg, Some(entity_id)));
            self.messenger.send(crate::messenger::Message::new(
                crate::messenger::MessageType::Simple(crate::messenger::SimpleMessage::Stature),
            ));
        }
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
    let pos = entity.element_data().position_map();
    let (start_x, start_y, start_z) = {
        let z = entity.actor_data().map(|a| a.jump_z_offset).unwrap_or(0.0);
        (pos.x, pos.y, z)
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

    // Build the order to push after the loop closes.  `NextJumpStep`
    // completion routes the motion-terminated signal through
    // `process_anim_completion_outcomes → advance_jump_step`.
    let mut order = crate::order::Order::new(step.anim, 0.0, 0.0, order_id);
    order.completion = crate::order::OrderCompletion::NextJumpStep;
    Some((jump_seq, jump_elem, order))
}

/// Per-frame position interpolation for the in-progress step.
///
/// Advances the actor by the sprite's per-frame distance along a
/// fixed direction vector.  Distance is only non-zero on the first
/// tick of a new animation frame (`frame_count == 0`) — between frames
/// the actor is stationary — so motion is discrete and synced to the
/// animation's per-frame distance table.  `jump_z_offset` uses the
/// same distance ratio so the vertical lift curve tracks the sprite
/// speed exactly.
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

    if let Some(target) = target_3d {
        // Target Y in map coords = target.y - target.z (isometric).
        let target_map_y = target.y - target.z;
        let full_dx = target.x - state.start_x;
        let full_dy = target_map_y - state.start_y;
        let full_dist = (full_dx * full_dx + full_dy * full_dy).sqrt();

        // Read this frame's distance from the sprite's distance table.
        // 0 on non-first ticks of a frame.
        let frame_dist = entity.element_data().sprite.current_frame_distance();

        if full_dist > f32::EPSILON && frame_dist > 0.0 {
            let dir_x = full_dx / full_dist;
            let dir_y = full_dy / full_dist;
            let elem = entity.element_data_mut();
            let new_x = elem.position_map().x + dir_x * frame_dist;
            let new_y = elem.position_map().y + dir_y * frame_dist;
            // Don't overshoot the target along the direction axis.
            // Projecting `(new - start) · dir` measures travelled
            // distance; clamp to `full_dist`.
            let travelled_new = (new_x - state.start_x) * dir_x + (new_y - state.start_y) * dir_y;
            if travelled_new >= full_dist {
                elem.set_position_map(crate::coordinates::MapPoint {
                    x: target.x,
                    y: target_map_y,
                });
            } else {
                elem.set_position_map(crate::coordinates::MapPoint { x: new_x, y: new_y });
            }
        }

        // `jump_z_offset` uses the ratio of *travelled* distance to
        // full distance, so the vertical lift tracks the sprite's
        // encoded motion profile instead of wall-clock frames.
        let pos = entity.element_data().position_map();
        let travelled = if full_dist > f32::EPSILON {
            let px = pos.x - state.start_x;
            let py = pos.y - state.start_y;
            ((px * full_dx + py * full_dy) / full_dist).clamp(0.0, full_dist)
        } else {
            full_dist
        };
        let ratio = if full_dist > f32::EPSILON {
            (travelled / full_dist).clamp(0.0, 1.0)
        } else {
            // No horizontal distance (pure z-motion, e.g. vertical
            // jump-up apex) — fall back to frame-count ratio.
            (state.frames_elapsed as f32 / state.total_frames.max(1) as f32).clamp(0.0, 1.0)
        };

        if airborne {
            let nz = state.start_z + (target.z - state.start_z) * ratio;
            if let Some(actor) = entity.actor_data_mut() {
                actor.jump_z_offset = nz;
            }
        } else if let Some(actor) = entity.actor_data_mut() {
            // Ground step — ease jump_z_offset back to zero.
            actor.jump_z_offset = state.start_z * (1.0 - ratio);
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
    use crate::geo2d;

    #[test]
    fn trajectory_ends_at_destination() {
        // Horizontal jump across 300 units with a 50-unit rise.
        let start = Point3D {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        let dest = Point3D {
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
        let mut src = JumpLine::new(geo2d::pt(0.0, 0.0), geo2d::pt(100.0, 0.0), 0.0, 0.0);
        let mut dst = JumpLine::new(geo2d::pt(100.0, 100.0), geo2d::pt(0.0, 100.0), 0.0, 0.0);
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
    fn jump_up_emits_jumping_up_step() {
        let src = JumpLine::new(geo2d::pt(0.0, 0.0), geo2d::pt(100.0, 0.0), 0.0, 0.0);
        let dst = JumpLine::new(geo2d::pt(100.0, 100.0), geo2d::pt(0.0, 100.0), 100.0, 100.0);

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
        let src = JumpLine::new(geo2d::pt(0.0, 0.0), geo2d::pt(100.0, 0.0), 100.0, 100.0);
        let dst = JumpLine::new(geo2d::pt(100.0, 100.0), geo2d::pt(0.0, 100.0), 0.0, 0.0);

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
        use crate::geo2d::{BBox2D, pt};
        use crate::sector::SectorType;

        let mut grid = FastFindGrid::new();
        grid.size_map(4, 4);
        grid.allocate_layers(1);

        // Two motion-area sectors.  Points / bboxes don't actually
        // matter for is_jumpable; what matters is `jump_line_indices`
        // and the grid-flat sector index.
        let make_sector = |sn: i16| GridSector {
            points: vec![pt(0.0, 0.0), pt(64.0, 0.0), pt(64.0, 64.0), pt(0.0, 64.0)],
            bounding_box: {
                let mut b = BBox2D::new();
                b.expand_point(pt(0.0, 0.0));
                b.expand_point(pt(64.0, 64.0));
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
            crate::geo2d::pt(0.0, 0.0),
            crate::geo2d::pt(64.0, 0.0),
            0.0,
            0.0,
        );
        jl_a.sector_index = crate::fast_find_grid::SectorIndex::new(0);
        jl_a.associated_line_index = Some(1);
        let mut jl_b = JumpLine::new(
            crate::geo2d::pt(0.0, 64.0),
            crate::geo2d::pt(64.0, 64.0),
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
        // Only one jumpable line in sector 0 — it should be picked.
        let got = get_nearest_jumpable_jump_line(
            &grid,
            &doors,
            0,
            &pc,
            crate::geo2d::pt(32.0, 0.0),
            crate::geo2d::pt(32.0, 64.0),
            false,
        );
        assert_eq!(got, Some(0));
    }

    #[test]
    fn sword_long_jump_uses_sword_variants() {
        let src = JumpLine::new(geo2d::pt(0.0, 0.0), geo2d::pt(100.0, 0.0), 0.0, 0.0);
        let dst = JumpLine::new(geo2d::pt(100.0, 100.0), geo2d::pt(0.0, 100.0), 0.0, 0.0);
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
