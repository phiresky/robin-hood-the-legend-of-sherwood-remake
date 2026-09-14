//! Ambush-point proximity and peek decisions.

use crate::ai::*;
use crate::parameters_ai;

use super::{AmbushPointStatus, EnemyAi};

/// Ambush refresh reads only its owner, never another actor's AI observation.
/// Keep this input separate so the per-soldier update cannot rebuild the world.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct AmbushPointContext {
    pub frame: u32,
    pub position: Position,
    pub direction: u16,
    pub intelligence: u16,
}

impl EnemyAi {
    // -----------------------------------------------------------------------
    // Ambush-point peek state machine
    // Original-game hostile AI ambush-point refresh.
    //
    // Called every frame during the actor update. Iterates the
    // global ambush-point list, transitions the per-NPC slot status (Far/Near/Checked)
    // based on proximity + LOS, and dispatches `check_ambush_point`
    // when the NPC enters LOS for the first time.
    // -----------------------------------------------------------------------
    pub(crate) fn refresh_ambush_points(
        &mut self,
        ctx: &AmbushPointContext,
        eyes: crate::coordinates::WorldPoint3D,
        ambush_points: &[crate::ai::AmbushPoint],
        obstacles: crate::sight_obstacle::ObstacleList<'_>,
    ) {
        // Early-out for low-IQ NPCs.
        if ctx.intelligence <= parameters_ai::AI_MIN_IQ_TO_CONTROL_AMBUSH_POINTS as u16 {
            return;
        }

        let substate = self.base.current_substate;
        let in_seekpoint_or_passed = matches!(
            substate,
            Substate::SeekingSeekpoint
                | Substate::SeekingSeekpointPassedAmbushPointLeft
                | Substate::SeekingSeekpointPassedAmbushPointRight
        );

        if !in_seekpoint_or_passed {
            // Default arm — reset every slot to Far
            // exactly once when leaving the seekpoint substates.
            if !self.ambush_point_array_reset {
                self.ambush_point_status.fill(AmbushPointStatus::Far);
                self.ambush_point_array_reset = true;
            }
            return;
        }

        // Only the SUBSTATE_SEEKING_SEEKPOINT arm
        // counts near points up front; the two PASSED_AMBUSH_POINT
        // substates fall through with the count left at zero, so
        // `more_than_one_near` stays false (matches the reference:
        // a deferred re-check should not defer again).
        let more_than_one_near = if substate == Substate::SeekingSeekpoint {
            let near_count = self
                .ambush_point_status
                .iter()
                .filter(|s| **s == AmbushPointStatus::Near)
                .count();
            near_count > 1
        } else {
            false
        };

        let my_point = crate::coordinates::MapPoint::new(ctx.position.x, ctx.position.y);
        let level = ctx.position.level;
        let sector = ctx.position.sector;

        // Slots and points are paired by index.  If the slot vec is
        // shorter than the global ambush-point list (shouldn't happen
        // outside tests — `init_one_ai` resizes it to match), skip the
        // overflow rather than panic.
        let n = ambush_points.len().min(self.ambush_point_status.len());
        // Parallel-array indexing into `ambush_points` + `ambush_point_status`.
        #[allow(clippy::needless_range_loop)]
        for idx in 0..n {
            let ap = &ambush_points[idx];
            let point_is_near = ap.is_near(my_point, level, sector);

            match self.ambush_point_status[idx] {
                AmbushPointStatus::Far => {
                    if point_is_near {
                        // LOS check from eye position to
                        // the ambush-point 3D anchor.
                        let reachable = crate::sight_obstacle::is_reachable_3d(
                            obstacles,
                            [eyes.x, eyes.y, eyes.z],
                            [ap.position_3d.x, ap.position_3d.y, ap.position_3d.z],
                            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                        );
                        self.ambush_point_status[idx] = if reachable {
                            // Came from the harmless side — no peek.
                            AmbushPointStatus::Checked
                        } else {
                            // Came from a blind side — peek when we
                            // get LOS.
                            AmbushPointStatus::Near
                        };
                        self.ambush_point_array_reset = false;
                    }
                }
                AmbushPointStatus::Near => {
                    if point_is_near {
                        let reachable = crate::sight_obstacle::is_reachable_3d(
                            obstacles,
                            [eyes.x, eyes.y, eyes.z],
                            [ap.position_3d.x, ap.position_3d.y, ap.position_3d.z],
                            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                        );
                        if reachable {
                            // Snapshot the point we need; the
                            // `check_ambush_point` call below mutates
                            // `self`, so we can't keep the borrow.
                            let ap_pos_x = ap.position.x;
                            let ap_pos_y = ap.position.y;
                            self.check_ambush_point(ap_pos_x, ap_pos_y, more_than_one_near, ctx);
                            self.ambush_point_status[idx] = AmbushPointStatus::Checked;
                        }
                    } else {
                        self.ambush_point_status[idx] = AmbushPointStatus::Far;
                    }
                }
                AmbushPointStatus::Checked => {
                    if !point_is_near {
                        self.ambush_point_status[idx] = AmbushPointStatus::Far;
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Ambush-point check — left/right classification + state transition
    // Original-game hostile AI ambush-point check.
    //
    // Computes the 2D cross product between the NPC's facing direction
    // and the vector from the NPC to the ambush point.  Positive →
    // point is on the right; non-positive → point is on the left.
    // Six branches total: same shape mirrored across left/right.
    // -----------------------------------------------------------------------
    fn check_ambush_point(
        &mut self,
        ambush_x: f32,
        ambush_y: f32,
        more_than_one_near: bool,
        ctx: &AmbushPointContext,
    ) {
        // direction_vector.Det(ambush_pos - my_pos)
        // The original game's determinant is the standard 2D cross product.
        let (dir_x, dir_y) = crate::element::direction_vector_16(ctx.direction as i16);
        let delta_x = ambush_x - ctx.position.x;
        let delta_y = ambush_y - ctx.position.y;
        let det = dir_x * delta_y - dir_y * delta_x;

        if det > 0.0 {
            // ---- Point on the right ----
            if self.base.current_substate == Substate::SeekingSeekpointPassedAmbushPointLeft {
                // We deferred to the left earlier; now it's on the
                // right, so look both ways.
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSeekpointCheckingAmbushPoint,
                );
                self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
            } else if !more_than_one_near {
                // Single point — peek right immediately.
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSeekpointCheckingAmbushPoint,
                );
                self.base.outbox.actor.look_sidewards = Some(LookDirection::Right);
            } else {
                // Multiple near — defer the look so a second nearby
                // point can join the decision.
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSeekpointPassedAmbushPointRight,
                );
                self.base.launch_timer(3, ctx.frame);
            }
        } else {
            // ---- Point on the left ----
            if self.base.current_substate == Substate::SeekingSeekpointPassedAmbushPointRight {
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSeekpointCheckingAmbushPoint,
                );
                self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
            } else if !more_than_one_near {
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSeekpointCheckingAmbushPoint,
                );
                self.base.outbox.actor.look_sidewards = Some(LookDirection::Left);
            } else {
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSeekpointPassedAmbushPointLeft,
                );
                self.base.launch_timer(3, ctx.frame);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Initialization state-transition tail called by EngineInner::init_one_ai
    // Original-game hostile AI initialization.
    //
    // The per-entity wiring (direction/view radius/detectables/
    // life-point snapshot/ambush point slots/patrol path) is handled
    // by `EngineInner::init_one_ai` before this runs; here we only handle
    // the initial-action / state-transition path.
    // -----------------------------------------------------------------------
}
