//! Periodic frame work: `the_16th_frame`, `refresh_ambush_points`,
//! `check_ambush_point`.

use crate::ai::*;
use crate::parameters_ai;

use super::{AmbushPointStatus, EnemyAi, ProfileRank};

impl EnemyAi {
    // -----------------------------------------------------------------------
    // The16thFrame — periodic tasks
    // Port of RHArtificialMalignity::The16thFrame
    // -----------------------------------------------------------------------

    /// The16thFrame.
    /// Called every 16 frames (staggered per NPC) for periodic checks.
    /// `is_idle` corresponds to `command == RHCOMMAND_WAIT`.
    #[allow(clippy::too_many_arguments)]
    pub fn the_16th_frame(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        frame_phase: u8,
        ctx: &AiContext,
        global: &AiGlobalState,
        tick: &AiPerTickData,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        is_idle: bool,
        sequence_null_about_to_launch: bool,
    ) {
        // Scotch — wasp stuck recovery.
        // If the NPC is in WonderingWaspInArmour but the wasp sting
        // animation has finished (is_idle), dismiss the wasp.
        if self.base.current_substate == Substate::WonderingWaspInArmour && is_idle {
            self.base
                .outbox
                .reentrant
                .self_stimuli
                .push(StimulusType::EventWaspAway);
        }

        // Scotch — if stuck while fleeing to the map
        // exit, re-call MerryManForestCassos to try a different door.
        if self.base.current_substate == Substate::FleeingMerryManRunToLeaveMap && is_idle {
            self.merry_man_forest_cassos(ctx, global);
        }

        // Restart stalled timers for swordfight/observe.
        if !self.base.timer_is_running && !global.freeze {
            match self.base.current_substate {
                Substate::AttackingSwordfight | Substate::AttackingObserve => {
                    self.base.launch_timer(10, ctx.frame);
                }
                _ => {}
            }
        }

        // Bored remark roll for officers/VIPs in DEFAULT
        // state when the entity is playing the WaitingUprightBored
        // idle animation.  1-in-12 per call (every 16 frames).
        if ctx.self_animation == crate::order::OrderType::WaitingUprightBored
            && self.base.current_state == AiState::Default
            && crate::sim_rng::u32(sim, crate::sim_rng::RngSite::VipIdleRemark, 0..12) == 0
        {
            if self.get_rank() == ProfileRank::Officer {
                self.base.say(Remark::OfficerComplains);
            } else if self.is_vip {
                self.base.say(Remark::VipSpeaksToHimself);
            }
        }

        // RefreshArrowProtection(true) — every-16-frame sweep
        // that drives reactive shield-raising.
        let _ = self.refresh_arrow_protection(true, ctx, tick, grid);

        // Gate the rest on `frame_phase & 63`.
        if (frame_phase & 63) != 0 {
            return;
        }

        // ── Every-64-frame: stuck detector ───────────────────────────
        // Substates that are waiting on EVENT_REACHPOINT.
        // If the NPC sits idle for 3 consecutive 64-frame ticks without
        // a sequence transition (`SequenceElementIsAboutToBeLaunched(NULL)`),
        // re-issue the last GoTo or fire EventCouldntReachPoint.
        let in_reachpoint_arm = matches!(
            self.base.current_substate,
            Substate::DefaultGotoPost
                | Substate::DefaultGotoRoute
                | Substate::DefaultEnroute
                | Substate::DefaultPatrolEnroute
                | Substate::DefaultPatrolEnrouteRunning
                | Substate::DefaultGotoChief
                | Substate::DefaultPatrolChiefReturnToPatrol
                | Substate::WonderingApproachingAle
                | Substate::WonderingApproachingMoney
                | Substate::WonderingRunningForMoney
                | Substate::WonderingApproachingToLoot
                | Substate::WonderingBrawlApproaching
                | Substate::WonderingOfficerApproachingBrawl
                | Substate::WonderingApproachingBrawlVictim
                | Substate::SeekingHeardsteps
                | Substate::SeekingArrow
                | Substate::SeekingBody
                | Substate::SeekingNet
                | Substate::SeekingSeekpoint
                | Substate::SeekingSeekpointPassedAmbushPointLeft
                | Substate::SeekingSeekpointPassedAmbushPointRight
                | Substate::SeekingSeekpointApproachingBeggar
                | Substate::SeekingSoldierGoToOfficer
                | Substate::SeekingSoldierReturnToOfficer
                | Substate::SeekingOfficerLeavingHouseToInstructGroup
                | Substate::SeekingGroupGoToOfficer
                | Substate::SeekingRunningToOfficer
                | Substate::SeekingRunningToOfficerSeen
                | Substate::SeekingCharly
                | Substate::SeekingCharlyGoToOfficer
                | Substate::SeekingCharlyGoToOfficerSeen
                | Substate::SeekingCombatAlert
                | Substate::AttackingRunningToEnemy
                | Substate::AttackingWalkingToEnemy
                | Substate::AttackingChargingEnemy
                | Substate::AttackingSwordfightStepBack
                | Substate::AttackingTooProudToAttackRetire
                | Substate::AttackingTooProudToAttackApproach
                | Substate::AttackingObserveAndMove
                | Substate::AttackingApproachingNewEnemy
                | Substate::AttackingMovingAroundOldEnemy
                | Substate::AttackingApproachingSleepingEnemy
                | Substate::AttackingArcherRetireFromCombat
                | Substate::AttackingRunningToPhalanx
                | Substate::AttackingArcherRunOnShootingPath
                | Substate::AttackingArcherRunOnShootingPathFinalSprint
                | Substate::AttackingDoorFightLeaving
                | Substate::AttackingRiderChargingApproaching
                | Substate::AttackingRiderChargingPassing
                | Substate::AttackingRiderChargingGettingDistance
                | Substate::AttackingRiderChargingApproachingBlindly
                | Substate::AttackingRunningToLadder
                | Substate::AttackingRunToAvengerOnRoof
                | Substate::FleeingPanic
                | Substate::FleeingRunToHide
                | Substate::FleeingRunToDoor
                | Substate::FleeingHiding
                | Substate::FleeingRunToAlertSoldiers
                | Substate::FleeingRetireFromCombat
                | Substate::FleeingMerryManRunToLeaveMap
                | Substate::FleeingRunForArrowReserves,
        );

        if in_reachpoint_arm && is_idle {
            // A queued Null sequence element means a
            // transition is in-flight — don't bump the stuck counter.
            if sequence_null_about_to_launch {
                self.base.stuck_counter = 0;
            } else if self.base.stuck_counter < 3 {
                self.base.stuck_counter = self.base.stuck_counter.saturating_add(1);
            } else {
                // Relaunch the last GoTo, or escape to
                // EventCouldntReachPoint if there's no recorded
                // destination.
                let dest = self.base.last_goto_destination;
                let flags = self.base.last_goto_flags;
                if dest.x != 0.0 || dest.y != 0.0 {
                    let order = crate::ai::AiController::make_move_order(&dest, flags);
                    self.base.outbox.actor.orders.push(order);
                } else {
                    self.base
                        .outbox
                        .reentrant
                        .self_stimuli
                        .push(StimulusType::EventCouldntReachPoint);
                }
                self.base.stuck_counter = 0;
            }
        } else {
            self.base.stuck_counter = 0;
        }

        // Gate the blood-alcohol pass on `frame_phase & 255`.
        // (no-op mask in Rust since `frame_phase` is u8.)
        if frame_phase != 0 {
            return;
        }

        // ── Every-256-frame: blood-alcohol decay ─────────────────────
        if self.base.blood_alcohol > 0 {
            // Drunken remark on idle Royalist patrols.
            if self.base.current_music_alert_status == AlertLevel::Green
                && self.base.current_state != AiState::Sleeping
                && self.base.blood_alcohol > 20
            {
                self.base.say(Remark::Drunken);
            }
            self.base.blood_alcohol = self.base.blood_alcohol.saturating_sub(1);
        }
    }

    // -----------------------------------------------------------------------
    // RefreshAmbushPoints — ambush-point peek state machine
    // Port of RHArtificialMalignity::RefreshAmbushPoints
    //
    // Called every frame from the equivalent of Hourglass.  Iterates the
    // global ambush-point list, transitions the per-NPC slot status (Far/Near/Checked)
    // based on proximity + LOS, and dispatches `check_ambush_point`
    // when the NPC enters LOS for the first time.
    // -----------------------------------------------------------------------
    pub fn refresh_ambush_points(
        &mut self,
        ctx: &AiContext,
        eyes: crate::coordinates::WorldPoint3D,
        ambush_points: &[crate::ai::AmbushPoint],
        obstacles: crate::sight_obstacle::ObstacleList<'_>,
    ) {
        // Early-out for low-IQ NPCs.
        if self.get_iq(ctx) <= parameters_ai::AI_MIN_IQ_TO_CONTROL_AMBUSH_POINTS as u16 {
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
    // CheckAmbushPoint — left/right classification + state transition
    // Port of RHArtificialMalignity::CheckAmbushPoint
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
        ctx: &AiContext,
    ) {
        // direction_vector.Det(ambush_pos - my_pos)
        // SBVector2D::Det is `mX * vect.mY - mY * vect.mX` (2D cross).
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
    // InitOneAI — state-transition tail called by EngineInner::init_one_ai
    // Port of RHArtificialMalignity::InitOneAI
    //
    // The per-entity wiring (direction/view radius/detectables/
    // life-point snapshot/ambush point slots/patrol path) is handled
    // by `EngineInner::init_one_ai` before this runs; here we only handle
    // the initial-action / state-transition path.
    // -----------------------------------------------------------------------
}
