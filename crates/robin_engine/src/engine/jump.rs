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
//! Translation authors the complete order chain. Selected orders execute through
//! the ordinary actor pipeline, with flight held in the sprite increment and
//! the actor's shared wait timer.

use crate::coordinates::{MapPoint, MapVec, WorldPoint3D, WorldVec3D};
use crate::element::{ActionState, EntityId, Posture};
use crate::engine::TickCtx;
use crate::engine::{EngineInner, LevelAssets};
use crate::jump_line::JumpLine;
use crate::order::OrderType;
use crate::sequence::SequenceElementRef;

/// PC's vertical reach.  Jumps with `|Δh|` under this threshold run the
/// long-jump branch; above it they split into `jump-up` / `jump-down`.
pub const TELEPORT_JUMPING_UP: f32 = 60.0;

/// Vertical drop applied in one step when the crouched jump-down take-off
/// transition reaches its action point, so the airborne segment begins
/// below the ledge lip instead of on top of it.
pub const TELEPORT_JUMPING_DOWN: f32 = 50.0;

/// Gravity constant.
const GRAVITY: f32 = -8.01;

/// PC mass for the jump trajectory.
const MASS_CHARACTER: f32 = 0.7;

/// A single step in a jump sequence.
///
/// Translation-only animation and destination operands. Runtime progress belongs
/// to the ordinary order queue, sprite increment, and actor timer.
#[derive(Debug, Clone)]
pub struct JumpStep {
    /// The animation to play during this step.
    pub anim: OrderType,
    /// Optional 3D destination for the step.  `None` means the animation
    /// plays in place with no position change.
    pub target_3d: Option<WorldPoint3D>,
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

    tracing::trace!(
        target: "parity_jump",
        src_a = ?source.point_a,
        src_b = ?source.point_b,
        src_z_a = source.z_a,
        src_z_b = source.z_b,
        dst_a = ?destination.point_a,
        dst_b = ?destination.point_b,
        dst_z_a = destination.z_a,
        dst_z_b = destination.z_b,
        ?pt_source,
        f_dot,
        v_line_norm,
        ratio,
        z_source,
        z_destination,
        jump_height,
        ?pt_destination,
        ?posture_before,
        long_jump_forced = source.long_jump_forced,
        "jump geometry"
    );

    let mut steps: Vec<JumpStep> = Vec::new();

    // ── Straight long jump ────────────────────────────────────────
    // Forced long jump OR `|jump_height| < pc_height`.
    if source.long_jump_forced || jump_height.abs() < pc_height {
        // Jump translation
        // takes the direct normal of the normalized source line vector and
        // only *asserts* that it points at the destination side; the release
        // build never flips it, so neither may we.
        // The original game's direct normal is `(-y, x)`
        // by the vector helper.
        let v_normal_src = MapVec {
            x: -v_line_n.y,
            y: v_line_n.x,
        };

        // Launch point sits 15u off the source line along that normal.
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
            });
            steps.push(JumpStep {
                anim: OrderType::JumpingLongSword,
                target_3d: Some(dst_3d),
            });
            steps.push(JumpStep {
                anim: OrderType::TransitionJumpingLongSwordWaitingSword,
                target_3d: None,
            });
            return steps;
        }

        // Non-sword variant.
        if posture_before == Posture::Crouched {
            steps.push(JumpStep {
                anim: OrderType::TransitionCrouchingUp,
                target_3d: None,
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
        });

        // One JumpingLong order per trajectory point.
        for pt in &trajectory {
            steps.push(JumpStep {
                anim: OrderType::JumpingLong,
                target_3d: Some(*pt),
            });
        }

        steps.push(JumpStep {
            anim: OrderType::TransitionJumpingLongWaitingUpright,
            target_3d: None,
        });

        if posture_before == Posture::Crouched || dest_forces_crouched {
            steps.push(JumpStep {
                anim: OrderType::TransitionCrouchingDown,
                target_3d: None,
            });
        }

        return steps;
    }

    // ── Jump up ────────────────────────────────────────────────────
    if jump_height > 0.0 {
        // Use the direct normal again, with the
        // `normal · (source.A - destination.A) < 0` relationship only
        // asserted, never enforced.
        let v_normal_src = MapVec {
            x: -v_line_n.y,
            y: v_line_n.x,
        };

        let pt_destination_jump = MapPoint {
            x: pt_destination.x - 15.0 * v_normal_src.x,
            y: pt_destination.y - 15.0 * v_normal_src.y,
        };

        if posture_before == Posture::Crouched {
            steps.push(JumpStep {
                anim: OrderType::TransitionCrouchingUp,
                target_3d: None,
            });
        }

        if posture_before == Posture::OnShoulders {
            if jump_height < pc_height {
                // Descend from shoulders first, then take off as upright.
                steps.push(JumpStep {
                    anim: OrderType::ClimbingDownFromShoulders,
                    target_3d: None,
                });
                steps.push(JumpStep {
                    anim: OrderType::TransitionWaitingUprightJumpingUp,
                    target_3d: None,
                });
            } else {
                steps.push(JumpStep {
                    anim: OrderType::TransitionWaitingOnShouldersJumpingUp,
                    target_3d: None,
                });
            }
        } else {
            steps.push(JumpStep {
                anim: OrderType::TransitionWaitingUprightJumpingUp,
                target_3d: None,
            });
        }

        // The flight target sits TELEPORT_JUMPING_UP *below* the
        // landing elevation: the airborne segment only carries the
        // actor up to the lip of the platform, and the closing
        // transition adds the remaining lift back when its animation
        // reaches its last frame.  The subtraction also shortens the
        // flight distance, which is what sizes the segment's frame
        // countdown — a jump-up typically terminates within one or two
        // frames.
        let flight_3d = WorldPoint3D {
            x: pt_destination_jump.x,
            y: pt_destination_jump.y + z_destination,
            z: z_destination - TELEPORT_JUMPING_UP,
        };
        let land_3d = WorldPoint3D {
            x: pt_destination.x,
            y: pt_destination.y + z_destination,
            z: z_destination,
        };

        steps.push(JumpStep {
            anim: OrderType::JumpingUp,
            target_3d: Some(flight_3d),
        });
        steps.push(JumpStep {
            anim: OrderType::TransitionJumpingUpWaitingCrouched,
            target_3d: Some(land_3d),
        });

        if posture_before != Posture::Crouched && !dest_forces_crouched {
            steps.push(JumpStep {
                anim: OrderType::TransitionCrouchingUp,
                target_3d: None,
            });
        }

        return steps;
    }

    // ── Jump down ──────────────────────────────────────────────────
    // Direct normal, with only its sign asserted.
    let v_normal_src = MapVec {
        x: -v_line_n.y,
        y: v_line_n.x,
    };
    let pt_source_jump = MapPoint {
        x: pt_source.x + 15.0 * v_normal_src.x,
        y: pt_source.y + 15.0 * v_normal_src.y,
    };

    if posture_before != Posture::Crouched {
        steps.push(JumpStep {
            anim: OrderType::TransitionCrouchingDown,
            target_3d: None,
        });
    }

    steps.push(JumpStep {
        anim: OrderType::TransitionWaitingCrouchedJumpingDown,
        target_3d: Some(WorldPoint3D {
            x: pt_source_jump.x,
            y: pt_source_jump.y,
            z: 0.0,
        }),
    });

    let land_3d = WorldPoint3D {
        x: pt_destination.x,
        y: pt_destination.y + z_destination,
        z: z_destination,
    };
    steps.push(JumpStep {
        anim: OrderType::JumpingDown,
        target_3d: Some(land_3d),
    });

    steps.push(JumpStep {
        anim: OrderType::TransitionJumpingDownWaitingCrouched,
        target_3d: None,
    });

    if posture_before != Posture::Crouched && !dest_forces_crouched {
        steps.push(JumpStep {
            anim: OrderType::TransitionCrouchingUp,
            target_3d: None,
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
/// This mirrors original-game nearest jump-line selection: candidate
/// lines come from the hovered/clicked jump zone, while authorization
/// rejects lines whose source sector is not the actor's current sector.
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
    /// `false` with a diagnostic when a required actor/topology dependency
    /// is missing, instead of silently treating an invalid query as blocked.
    pub fn is_jumpable(&self, jump_line_idx: u32, pc_entity: EntityId, test_posture: bool) -> bool {
        let Some(entity) = self.world.entities.get(pc_entity) else {
            tracing::warn!(
                ?pc_entity,
                jump_line_idx,
                "jumpability query references missing actor"
            );
            return false;
        };
        let Some(sector_num) = entity.element_data().sector() else {
            tracing::warn!(
                ?pc_entity,
                jump_line_idx,
                "jumpability query actor has no sector"
            );
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
            tracing::warn!(
                ?pc_entity,
                ?sector_num,
                jump_line_idx,
                "jumpability query has no canonical source sector"
            );
            return false;
        };
        let Some(doors) = self
            .scripts
            .mission
            .as_ref()
            .map(|_| self.script_domains.interactables.doors.as_slice())
        else {
            tracing::warn!(
                ?pc_entity,
                jump_line_idx,
                "jumpability query has no mission script"
            );
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
    /// authors the full ordinary order queue before execution begins.
    ///
    /// Returns `true` if the jump was installed, `false` if required
    /// data (jump lines, actor) was missing — in which case the
    /// caller should terminate the element so the sequence does not
    /// stall.
    pub(super) fn start_jump(
        &mut self,
        tcx: TickCtx<'_>,
        owner: EntityId,
        elem_ref: SequenceElementRef,
    ) -> bool {
        // Read jump-line IDs from the element.
        let (src_id, dst_id) = {
            let elem = match self.orders.sequence_manager.get_element_at(elem_ref) {
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

        let (src_line, dst_line) = {
            let src = self
                .world
                .fast_grid
                .level
                .jump_lines
                .get(usize::from(src_id));
            let dst = self
                .world
                .fast_grid
                .level
                .jump_lines
                .get(usize::from(dst_id));
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

        // `jump_height = associated.z_a - line.z_a`.  For our source
        // line, `associated` is the paired dst line.
        let jump_height = dst_line.z_a - src_line.z_a;

        let (pt_source, posture_before, action_state_before, is_swordfighting) = {
            let Some(entity) = self.world.entities.get(owner) else {
                return false;
            };
            let elem_data = entity.element_data();
            let pos = elem_data.position_map();
            let posture = elem_data.posture();
            let action_state = entity
                .actor_data()
                .map(|actor| actor.action_state)
                .unwrap_or(ActionState::Waiting);
            let is_sf = entity
                .human_data()
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false);
            (pos, posture, action_state, is_sf)
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
            self.quit_swordfight(tcx, owner);
        }

        let src_line = &self.world.fast_grid.level.jump_lines[usize::from(src_id)];
        let dst_line = &self.world.fast_grid.level.jump_lines[usize::from(dst_id)];
        let mut steps = build_jump_steps(
            src_line,
            dst_line,
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

        prepend_lowering_shield_before_jump(
            &mut steps,
            posture_before,
            action_state_before,
            is_swordfighting,
        );

        if steps.is_empty() {
            return false;
        }

        let orders: std::collections::VecDeque<_> = steps
            .into_iter()
            .map(|step| {
                let target = step.target_3d.unwrap_or_default();
                let target_map = if jump_order_is_airborne(step.anim) {
                    MapPoint::default()
                } else {
                    target.to_map()
                };
                let mut order = crate::order::Order::new(
                    step.anim,
                    target_map.x,
                    target_map.y,
                    self.orders.allocate_order_id(),
                );
                order.compute_direction = false;
                if jump_order_is_airborne(step.anim) {
                    order.destination_3d = [target.x, target.y, target.z];
                }
                order
            })
            .collect();
        let element = self
            .orders
            .sequence_manager
            .get_element_at_mut(elem_ref)
            .expect("jump element disappeared during translation");
        element.orders.clear();
        element.orders.extend(orders);
        let installed = crate::element::InstalledActorOrder::new(
            crate::sequence::SequenceElementRef::new(elem_ref.sequence_id, elem_ref.element_index),
            element
                .current_order()
                .expect("jump translation produced no orders"),
        );
        self.install_actor_order(owner, Some(installed));

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

    /// Apply the selected jump order's initialization and turning in Execute.
    pub(super) fn prepare_jump_order(&mut self, tcx: TickCtx<'_>, owner: EntityId) {
        let Some((seq_id, elem_idx, order)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, owner)
        else {
            return;
        };
        let element = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("selected jump element disappeared");
        if element.command != crate::element::Command::JumpCmd {
            return;
        }
        let anim = order.order_type;
        let target = WorldPoint3D {
            x: order.destination_3d[0],
            y: order.destination_3d[1],
            z: order.destination_3d[2],
        };
        let source = jump_line_property(element, crate::sequence::Field::JumplineSource);
        let line = &self.world.fast_grid.level.jump_lines[usize::from(source)];
        let vector = line.vector();
        let direction = crate::position_interface::vector_to_sector_0_to_15(-vector.y, vector.x);
        let initialising = self
            .world
            .entities
            .get(owner)
            .expect("jump owner disappeared")
            .actor_data()
            .expect("jump owner is not an actor")
            .execute_order_initialising;
        if initialising
            && matches!(
                anim,
                OrderType::TransitionWaitingOnShouldersJumpingUp
                    | OrderType::TransitionWaitingOnShouldersJumpingLong
            )
        {
            let carrier = self
                .world
                .entities
                .get(owner)
                .expect("jump owner disappeared")
                .human_data()
                .expect("jump owner is not human")
                .carrier
                .expect("shoulder jump has no carrier");
            self.actor_wait(tcx, carrier);
            self.world
                .entities
                .get_mut(carrier)
                .expect("shoulder carrier disappeared")
                .pc_data_mut()
                .expect("shoulder carrier is not a PC")
                .carried = None;
            self.launch_element(
                tcx,
                crate::sequence::SequenceElement::new(
                    1,
                    crate::element::Command::LeaveHelpingClimb,
                    Some(carrier),
                ),
            );
            self.world
                .entities
                .get_mut(owner)
                .expect("jump owner disappeared")
                .human_data_mut()
                .expect("jump owner is not human")
                .carrier = None;
        }
        if initialising
            && matches!(
                anim,
                OrderType::TransitionWaitingUprightJumpingUp
                    | OrderType::TransitionWaitingCrouchedJumpingDown
                    | OrderType::TransitionWaitingUprightJumpingLong
                    | OrderType::TransitionWaitingSwordJumpingLongSword
            )
        {
            self.world
                .entities
                .get_mut(owner)
                .expect("jump owner disappeared")
                .position_iface_mut()
                .set_direction(crate::position_interface::Direction::from_raw(
                    direction.into(),
                ));
            self.forward_message(
                tcx,
                crate::messenger::Message::pc(
                    crate::messenger::PcMessage::DisableAllActionsTemp,
                    Some(owner),
                ),
            );
        }
        let entity = self
            .world
            .entities
            .get_mut(owner)
            .expect("jump owner disappeared");
        if initialising {
            initialize_jump_order(entity, anim, target, direction);
        } else if jump_step_turns(anim) {
            entity.position_iface_mut().turn();
        }
    }

    /// Execute-side termination effects run before ordinary order retirement.
    pub(super) fn apply_jump_order_state(
        &mut self,
        tcx: TickCtx<'_>,
        entity_id: EntityId,
        state: crate::sprite::MotionState,
    ) {
        let Some((seq_id, elem_idx, order)) = self
            .orders
            .sequence_manager
            .current_order_for_actor(&self.world.entities, entity_id)
        else {
            return;
        };
        let element = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .expect("selected jump element disappeared");
        if element.command != crate::element::Command::JumpCmd {
            return;
        }
        let anim = order.order_type;
        if state == crate::sprite::MotionState::Start {
            let states = match anim {
                OrderType::TransitionWaitingOnShouldersJumpingUp
                | OrderType::TransitionWaitingOnShouldersJumpingLong => {
                    Some((Posture::Flying, ActionState::Moving))
                }
                OrderType::TransitionJumpingUpWaitingCrouched
                | OrderType::TransitionJumpingDownWaitingCrouched => {
                    Some((Posture::Crouched, ActionState::Waiting))
                }
                OrderType::TransitionJumpingLongSwordWaitingSword => {
                    Some((Posture::Upright, ActionState::WaitingSword))
                }
                OrderType::TransitionJumpingLongWaitingUpright => {
                    Some((Posture::Upright, ActionState::Waiting))
                }
                _ => None,
            };
            if let Some((posture, action)) = states {
                self.set_entity_posture(entity_id, posture);
                let entity = self
                    .world
                    .entities
                    .get_mut(entity_id)
                    .expect("jump owner disappeared");
                entity
                    .actor_data_mut()
                    .expect("jump owner is not an actor")
                    .action_state = action;
            }
            return;
        }
        if state != crate::sprite::MotionState::Terminated {
            return;
        }
        let landing = jump_order_is_airborne(anim)
            && element
                .orders
                .get(1)
                .is_some_and(|next| next.order_type != anim);
        let target = WorldPoint3D {
            x: order.destination_3d[0],
            y: order.destination_3d[1],
            z: order.destination_3d[2],
        };
        if landing {
            let destination =
                jump_line_property(element, crate::sequence::Field::JumplineDestination);
            let line = &self.world.fast_grid.level.jump_lines[usize::from(destination)];
            let layer = line.layer;
            let sector = jump_line_sector_number(&self.world.fast_grid, line);
            let projection = line.get_middle_point();
            self.world
                .entities
                .get_mut(entity_id)
                .expect("jump owner disappeared")
                .element_data_mut()
                .set_position(target);
            self.finalize_airborne_jump_landing(tcx.assets, entity_id, layer, sector, projection);
            let pi = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("jump owner disappeared")
                .position_iface_mut();
            pi.set_old_map_position(pi.map_position());
        }
        if matches!(
            anim,
            OrderType::TransitionWaitingUprightJumpingUp
                | OrderType::TransitionWaitingCrouchedJumpingDown
                | OrderType::TransitionWaitingUprightJumpingLong
                | OrderType::TransitionWaitingSwordJumpingLongSword
        ) {
            self.set_entity_posture(entity_id, Posture::Flying);
            if let Some(actor) = self
                .world
                .entities
                .get_mut(entity_id)
                .expect("jump owner disappeared")
                .actor_data_mut()
            {
                actor.action_state = if anim == OrderType::TransitionWaitingSwordJumpingLongSword {
                    ActionState::MovingSword
                } else {
                    ActionState::Moving
                };
            }
        }

        let entity = self
            .world
            .entities
            .get_mut(entity_id)
            .expect("jump owner disappeared");
        if matches!(
            anim,
            OrderType::TransitionWaitingOnShouldersJumpingUp
                | OrderType::TransitionWaitingOnShouldersJumpingLong
        ) {
            let mut position = entity.element_data().position();
            position.z += 40.0;
            entity.element_data_mut().set_position_delayed(position);
        }
        // For the three jump landing transitions: re-broadcast
        // `MSG_DISABLE_ALL_ACTIONS_TEMP` if the landing sector forces
        // crouching, otherwise `MSG_ENABLE_ALL_ACTIONS_TEMP`, and
        // unconditionally `MSG_STATURE` so the HUD picks up the
        // post-landing posture.
        let is_landing_pc = entity.is_pc()
            && matches!(
                anim,
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
        if is_landing_pc {
            let force_crouched = landing_sector
                .map(|n| self.sector_forces_crouch(n))
                .unwrap_or(false);
            let pc_msg = if force_crouched {
                crate::messenger::PcMessage::DisableAllActionsTemp
            } else {
                crate::messenger::PcMessage::EnableAllActionsTemp
            };
            self.forward_message(tcx, crate::messenger::Message::pc(pc_msg, Some(entity_id)));
            self.forward_message(
                tcx,
                crate::messenger::Message::new(crate::messenger::MessageType::Simple(
                    crate::messenger::SimpleMessage::Stature,
                )),
            );

            if anim == OrderType::TransitionJumpingLongSwordWaitingSword {
                let owner = self
                    .world
                    .entities
                    .get(entity_id)
                    .expect("jump owner disappeared");
                if let Some(opponent) = owner
                    .human_data()
                    .expect("jump owner is not human")
                    .opponents
                    .first()
                    && owner.element_data().sector()
                        != self
                            .world
                            .entities
                            .get(*opponent)
                            .expect("jump opponent disappeared")
                            .element_data()
                            .sector()
                {
                    self.update_swordfight_distance(tcx, entity_id);
                }
            }
            if jump_landing_restores_anti_collision(anim) {
                self.world
                    .entities
                    .get_mut(entity_id)
                    .expect("jump owner disappeared")
                    .position_iface_mut()
                    .set_anti_collision_on(true);
            }
        }
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
        self.expect_entity_mut(entity_id, "jump landing depth")
            .sprite_mut()
            .compute_display_depth();
    }
}

/// Preserve player transition flags and the human action transition before
/// jump translation.
/// Jump permits a held sword but not a shield, so every upright shield state
/// first appends `LoweringShield`; jump translation then appends take-off.
fn prepend_lowering_shield_before_jump(
    steps: &mut Vec<JumpStep>,
    posture: Posture,
    action_state: ActionState,
    is_swordfighting: bool,
) {
    if posture == Posture::Upright && action_state.is_shield() && !is_swordfighting {
        steps.insert(
            0,
            JumpStep {
                anim: OrderType::LoweringShield,
                target_3d: None,
            },
        );
    }
}

/// Whether retiring a jump landing restores normal anti-collision.
///
/// The original down-jump landing arm deliberately leaves its
/// anti-collision reactivation disabled, and the following crouching-up
/// transition does not restore it either. The other landing arms restore
/// anti-collision immediately when they terminate, before any trailing posture
/// transition in the same sequence.
fn jump_landing_restores_anti_collision(landing_anim: OrderType) -> bool {
    matches!(
        landing_anim,
        OrderType::TransitionJumpingUpWaitingCrouched
            | OrderType::TransitionJumpingLongWaitingUpright
            | OrderType::TransitionJumpingLongSwordWaitingSword
    )
}

fn jump_line_property(
    element: &crate::sequence::SequenceElement,
    field: crate::sequence::Field,
) -> crate::jump_line::JumpLineIndex {
    match element.get_property(field) {
        Some(crate::sequence::FieldValue::LineId(id)) => *id,
        Some(crate::sequence::FieldValue::Integer(id)) => {
            crate::jump_line::JumpLineIndex::new(*id).expect("invalid jump line index")
        }
        _ => panic!("jump element has no line operand"),
    }
}

fn initialize_jump_order(
    entity: &mut crate::element::Entity,
    anim: OrderType,
    target: WorldPoint3D,
    source_direction_goal: i16,
) {
    if jump_order_is_airborne(anim) {
        start_airborne_jump_motion(entity, anim, target);
        entity.position_iface_mut().turn();
    } else if matches!(
        anim,
        OrderType::TransitionWaitingOnShouldersJumpingUp
            | OrderType::TransitionWaitingOnShouldersJumpingLong
            | OrderType::TransitionWaitingUprightJumpingUp
            | OrderType::TransitionWaitingCrouchedJumpingDown
            | OrderType::TransitionWaitingUprightJumpingLong
            | OrderType::TransitionWaitingSwordJumpingLongSword
    ) {
        // Every take-off faces the source line normal, including jump-up
        // orders that have no 2D destination.
        let position_iface = entity.position_iface_mut();
        position_iface.set_direction(crate::position_interface::Direction::from_raw(
            source_direction_goal.into(),
        ));
        position_iface.set_anti_collision_on(false);
        // The take-off arms that move toward their authored point initialize
        // their motion order inside the shared sprite motion path, which is
        // what seeds both the goal and its increment.
        position_iface.turn();
    } else if jump_step_turns(anim) {
        entity.position_iface_mut().turn();
    }
}

fn start_airborne_jump_motion(
    entity: &mut crate::element::Entity,
    anim: OrderType,
    target: WorldPoint3D,
) {
    let position = entity.element_data().position();
    let dx = target.x - position.x;
    let dy = target.y - position.y;
    let dz = target.z - position.z;
    let distance = (dx * dx + dy * dy + dz * dz).sqrt();
    let scale = jump_airborne_speed(anim) / distance;
    entity
        .position_iface_mut()
        .set_projectile_increment(WorldVec3D {
            x: dx * scale,
            y: dy * scale,
            z: dz * scale,
        });
    let wait_time = (distance * jump_flight_rate(anim) - 1.0).max(1.0) as u32;
    entity
        .actor_data_mut()
        .expect("airborne order requires an actor")
        .wait_time = wait_time;
}

/// Ground transition steps whose Execute arm drives the sprite through
/// motion processing with `MotionMethod::TillLastFrame` instead of the plain
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
        entity.sprite_mut().compute_display_depth();
    }

    // The jump-up flight stops TELEPORT_JUMPING_UP below the platform
    // top; the landing animation's last frame is where the body is
    // lifted the rest of the way. The lift is a straight write to the
    // 3D position, so the map position slides by the same amount and
    // no plane re-derivation happens here.
    if anim == OrderType::TransitionJumpingUpWaitingCrouched
        && state == crate::sprite::MotionState::Done
    {
        let pi = entity.position_iface_mut();
        let mut lifted = pi.get_position();
        lifted.z += TELEPORT_JUMPING_UP;
        pi.set_position(lifted);
        entity.element_data_mut().update_grid_cell();
        entity.sprite_mut().compute_display_depth();
    }

    state
}

/// Drop the jumper by [`TELEPORT_JUMPING_DOWN`] on the action point of the
/// crouched jump-down take-off transition.
///
/// The mirror image of the jump-up lift in [`perform_jump_ground_motion`]:
/// the take-off animation ends with the body already over the edge, and the
/// drop is a straight write to the 3D position, so the map position slides
/// by the same amount and no plane re-derivation happens. It runs inside the
/// jumper's own Execute, which makes the new elevation visible to every later
/// creation slot on this very frame rather than the next one.
pub(crate) fn apply_jump_down_takeoff_drop(
    entity: &mut crate::element::Entity,
    anim: OrderType,
    state: crate::sprite::MotionState,
) {
    if anim != OrderType::TransitionWaitingCrouchedJumpingDown
        || state != crate::sprite::MotionState::Done
    {
        return;
    }
    let pi = entity.position_iface_mut();
    let mut dropped = pi.get_position();
    dropped.z -= TELEPORT_JUMPING_DOWN;
    pi.set_position(dropped);
    entity.element_data_mut().update_grid_cell();
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

/// Per-frame fraction of the flight distance used to size a jump segment's
/// frame countdown.
///
/// These are the reciprocals of [`jump_airborne_speed`], but the countdown
/// must multiply by the literal rate rather than divide by the speed: the
/// two disagree in the last ulp, and the result is truncated to an integer
/// frame count, so a single ulp can flip a jump's length by a whole frame.
fn jump_flight_rate(anim: OrderType) -> f32 {
    match anim {
        OrderType::JumpingLong | OrderType::JumpingLongSword => 0.125,
        OrderType::JumpingUp => 0.066_666_67,
        OrderType::JumpingDown => 0.05,
        _ => panic!("airborne jump rate requested for non-jump order {anim:?}"),
    }
}

fn jump_airborne_speed(anim: OrderType) -> f32 {
    match anim {
        OrderType::JumpingLong | OrderType::JumpingLongSword => 8.0,
        OrderType::JumpingUp => 15.0,
        OrderType::JumpingDown => 20.0,
        _ => panic!("airborne jump speed requested for non-jump order {anim:?}"),
    }
}

/// Whether the actor's live jump step flies the body through the air under
/// its own Execute arm rather than through the sprite motion driver.
///
/// The airborne arms play their animation for the visual only and drive the
/// body along a fixed 3D increment, so the animation pass must route them to
/// [`perform_jump_airborne_motion`] instead of the plain action path.
pub(crate) fn jump_order_is_airborne(anim: OrderType) -> bool {
    matches!(
        anim,
        OrderType::JumpingUp
            | OrderType::JumpingDown
            | OrderType::JumpingLong
            | OrderType::JumpingLongSword
    )
}

/// Run one tick of an airborne jump segment.
///
/// The animation is played purely for the visual: its motion state is
/// discarded and the returned state comes from the flight countdown alone,
/// so a segment reports IN_PROGRESS on every tick including its first and
/// TERMINATED only on the tick that exhausts the countdown.
pub(crate) fn perform_jump_airborne_motion(
    entity: &mut crate::element::Entity,
    sim: &crate::sim_rng::SimulationContext,
    order_id: Option<std::num::NonZeroU32>,
    anim: OrderType,
    row: u16,
    globally_frozen: bool,
) -> crate::sprite::MotionState {
    if !globally_frozen {
        entity.element_data_mut().sprite.perform_action(
            sim,
            order_id,
            anim,
            row,
            crate::sprite::FrameProgression::FreezeWhenTerminated,
            false,
        );
    }
    advance_airborne_flight(entity);

    let wait_time = entity
        .actor_data()
        .map(|actor| actor.wait_time)
        .unwrap_or_else(|| {
            panic!("airborne jump step {anim:?} ticked on an entity without actor data")
        });
    if wait_time == 0 {
        crate::sprite::MotionState::Terminated
    } else {
        crate::sprite::MotionState::InProgress
    }
}

/// Advance the in-flight body one frame along its fixed 3D increment and
/// tick the segment's countdown.
fn advance_airborne_flight(entity: &mut crate::element::Entity) {
    let increment = entity.position_iface().get_increment();
    let pos = entity.element_data().position();
    entity.element_data_mut().set_position(WorldPoint3D {
        x: pos.x + increment.x,
        y: pos.y + increment.y,
        z: pos.z + increment.z,
    });
    let map = entity.element_data().position_map();
    let center = entity.element_data().sprite.center;
    entity
        .position_iface_mut()
        .finish_flight_position_update(MapPoint::new(
            (map.x - center.x).floor(),
            (map.y - center.y).floor(),
        ));
    entity.element_data_mut().update_grid_cell();
    entity
        .actor_data_mut()
        .expect("airborne order requires an actor")
        .wait_time -= 1;
}

// ═══════════════════════════════════════════════════════════════════
//  Tests — kept at the bottom of the file.
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn airborne_jump_retains_takeoff_depth_until_landing_publication() {
        use crate::engine::test_support::actors::TestActor;
        let mut entity = TestActor::pc(Posture::Flying)
            .map_position(MapPoint::new(10.0, 20.0))
            .build();
        entity.sprite_mut().display_depth = 17.001;
        entity.actor_data_mut().unwrap().wait_time = 2;
        entity
            .position_iface_mut()
            .set_projectile_increment(crate::coordinates::WorldVec3D::new(1.0, 4.0, 2.0));
        let before = entity.element_data().position();
        advance_airborne_flight(&mut entity);
        assert_eq!(entity.element_data().position().y, before.y + 4.0);
        assert_eq!(entity.sprite().display_depth, 17.001);
        assert_eq!(entity.actor_data().unwrap().wait_time, 1);
    }

    #[test]
    fn long_jump_reserves_full_order_chain_before_advancing() {
        use crate::engine::test_support::actors::TestActor;
        use crate::sequence::{Field, FieldValue, SequenceElement};

        let mut engine = EngineInner::new();
        let (grid, _) = make_jumpable_fixture(false);
        *engine.world.fast_grid_mut() = grid;
        let owner = engine.add_test_entity(
            TestActor::pc(Posture::Upright)
                .map_position(MapPoint::new(32.0, 0.0))
                .sector(10)
                .build(),
        );
        let mut element =
            SequenceElement::new_generic(1, crate::element::Command::JumpCmd, Some(owner));
        element.set_property(Field::JumplineSource, FieldValue::Integer(0));
        element.set_property(Field::JumplineDestination, FieldValue::Integer(1));
        let seq_id = engine.orders.sequence_manager.insert_element(element);
        let first_id = engine.orders.next_order_id;
        let sim = crate::sim_rng::test_context();
        let assets = LevelAssets::new();

        assert!(engine.start_jump(
            TickCtx::new(&sim, &assets),
            owner,
            SequenceElementRef::new(seq_id, 0)
        ));
        let element = engine
            .orders
            .sequence_manager
            .get_element(seq_id, 0)
            .unwrap();
        assert_eq!(
            element.orders.front().unwrap().order_type,
            OrderType::TransitionWaitingUprightJumpingLong
        );
        assert_eq!(
            element.orders.back().unwrap().order_type,
            OrderType::TransitionJumpingLongWaitingUpright
        );
        assert!(
            element.orders.len() > 3,
            "the curved flight has multiple authored segments"
        );
        for (index, order) in element.orders.iter().enumerate() {
            assert_eq!(order.order_id.get(), first_id + index as u32);
            assert!(!order.compute_direction);
            if index > 0 && index + 1 < element.orders.len() {
                assert_eq!(order.order_type, OrderType::JumpingLong);
                assert_ne!(order.destination_3d, [0.0; 3]);
            }
        }
        let successor = element.orders[1].order_id;
        let allocated_after_translation = engine.orders.next_order_id;
        assert_eq!(
            allocated_after_translation,
            first_id + element.orders.len() as u32
        );
        engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .actor_data_mut()
            .unwrap()
            .continuation
            .motion_state = crate::sprite::MotionState::Terminated;

        engine.do_next_order(
            TickCtx::new(&sim, &assets),
            SequenceElementRef::new(seq_id, 0),
        );

        let actor = engine
            .world
            .entities
            .get(owner)
            .unwrap()
            .actor_data()
            .unwrap();
        let installed = engine.actor_installed_order(owner).unwrap();
        assert_eq!(installed.order_id, successor);
        assert_eq!(installed.order_type, OrderType::JumpingLong);
        assert_eq!(
            actor.continuation.motion_state,
            crate::sprite::MotionState::InProgress
        );
        assert!(actor.execute_order_initialising);
        assert_eq!(engine.orders.next_order_id, allocated_after_translation);
    }

    #[test]
    fn frozen_jump_animation_still_advances_fixed_increment_and_shared_countdown() {
        use crate::engine::test_support::actors::TestActor;

        let sim = crate::sim_rng::test_context();
        for (animation, speed, ticks) in [
            (OrderType::JumpingLong, 8.0, 14),
            (OrderType::JumpingLongSword, 8.0, 14),
            (OrderType::JumpingUp, 15.0, 7),
            (OrderType::JumpingDown, 20.0, 5),
        ] {
            let mut actor = TestActor::pc(Posture::Flying)
                .at(WorldPoint3D::new(0.0, 0.0, 0.0))
                .build();
            start_airborne_jump_motion(&mut actor, animation, WorldPoint3D::new(120.0, 0.0, 0.0));
            assert_eq!(actor.actor_data().unwrap().wait_time, ticks);
            for tick in 1..=ticks {
                let motion =
                    perform_jump_airborne_motion(&mut actor, &sim, None, animation, 0, true);
                assert_eq!(actor.element_data().position().x, speed * tick as f32);
                assert_eq!(actor.position_iface().get_increment().x, speed);
                assert_eq!(actor.actor_data().unwrap().wait_time, ticks - tick);
                assert_eq!(
                    motion,
                    if tick == ticks {
                        crate::sprite::MotionState::Terminated
                    } else {
                        crate::sprite::MotionState::InProgress
                    }
                );
            }
        }
    }

    #[test]
    fn shield_holder_lowers_shield_before_jump_takeoff() {
        let mut steps = vec![JumpStep {
            anim: OrderType::TransitionWaitingUprightJumpingLong,
            target_3d: None,
        }];

        prepend_lowering_shield_before_jump(
            &mut steps,
            Posture::Upright,
            ActionState::HoldingShield,
            false,
        );

        assert_eq!(steps[0].anim, OrderType::LoweringShield);
        assert_eq!(
            steps[1].anim,
            OrderType::TransitionWaitingUprightJumpingLong
        );
    }

    #[test]
    fn trajectory_ends_at_destination() {
        // Horizontal jump across 300 units with a 50-unit rise.
        let start = WorldPoint3D::new(0.0, 0.0, 0.0);
        let dest = WorldPoint3D::new(300.0, 0.0, 50.0);
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
        // Jump execution scales increments by
        // 8/15/20 units per tick for long/up/down jump animations.
        assert_eq!(jump_airborne_speed(OrderType::JumpingLong), 8.0);
        assert_eq!(jump_airborne_speed(OrderType::JumpingLongSword), 8.0);
        assert_eq!(jump_airborne_speed(OrderType::JumpingUp), 15.0);
        assert_eq!(jump_airborne_speed(OrderType::JumpingDown), 20.0);
    }

    #[test]
    fn down_jump_landing_preserves_disabled_anti_collision() {
        assert!(!jump_landing_restores_anti_collision(
            OrderType::TransitionJumpingDownWaitingCrouched
        ));
        assert!(!jump_landing_restores_anti_collision(
            OrderType::TransitionCrouchingUp
        ));
        assert!(jump_landing_restores_anti_collision(
            OrderType::TransitionJumpingUpWaitingCrouched
        ));
        assert!(jump_landing_restores_anti_collision(
            OrderType::TransitionJumpingLongWaitingUpright
        ));
        assert!(jump_landing_restores_anti_collision(
            OrderType::TransitionJumpingLongSwordWaitingSword
        ));
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
    fn nearest_jumpable_preview_skips_posture_gate_but_execution_applies_it() {
        let (grid, doors) = make_jumpable_fixture(true);
        let upright = pc_auth(true, Posture::Upright);
        let args = (
            &grid,
            doors.as_slice(),
            0,
            0,
            &upright,
            MapPoint::new(32.0, 0.0),
            MapPoint::new(32.0, 64.0),
        );

        assert_eq!(
            get_nearest_jumpable_jump_line(
                args.0, args.1, args.2, args.3, args.4, args.5, args.6, false, None,
            ),
            Some(0),
            "cursor preview ignores the helper posture gate"
        );
        assert_eq!(
            get_nearest_jumpable_jump_line(
                args.0, args.1, args.2, args.3, args.4, args.5, args.6, true, None,
            ),
            None,
            "movement execution applies the helper posture gate"
        );
    }

    #[test]
    fn nearest_jumpable_rejects_unrelated_clicked_jump_sector() {
        let (grid, doors) = make_jumpable_fixture(false);
        let pc = pc_auth(true, Posture::Upright);

        // PC is in sector 0, but the clicked jump sector lists the
        // opposite-side line. The original game iterates the clicked jump zone's
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
