use super::*;

impl EnemyAi {
    /// 360°-detection check: the NPC can "feel" a target that is
    /// within its real view radius regardless of facing direction.
    /// Used by the `EVENT_OUTOFVIEW` handler for any swordfight substate
    /// to suppress the event when the target is actually still close —
    /// the LOS drop is just a transient cone flicker, not a real loss.
    ///
    /// Approximation: stretched-Y squared distance ≤ `sq_standard_view_radius`,
    /// plus an `is_reachable` (opaque sight obstacles) LOS check via
    /// `FastFindGrid`.
    // Forward the caller location into the recorded visibility query so
    // parity dumps attribute each check to the gate that asked for it,
    // not to this shared helper.
    #[track_caller]
    pub(super) fn is_detecting_360_degrees(&self, target: HumanHandle, ctx: &AiContext) -> bool {
        //   if (!viewer_active_and_outside_building || !target_active_and_outside_building)
        //       return false;
        // Viewer half: gate on the viewer's in-building flag.  Active is
        // implied by the AI tick running.
        if ctx.building_sector.is_some() {
            return false;
        }
        let Some(view) = ctx.entity_view(target) else {
            tracing::trace!(
                target,
                "is_detecting_360_degrees: entity_view lookup failed"
            );
            return false;
        };
        // Target half is the literal active-and-outside-building gate. Dead,
        // unconscious, tied, and otherwise non-fighting humans remain valid
        // while their raw element is active and its current sector is not a
        // building.
        if !view.active || view.in_building {
            return false;
        }
        // Viewer's eye point (forced upright in this overload) and
        // target's detection point.  The distance is the stretched-Y
        // 3D vector between them; the Z² term is what made the prior
        // 2D-only check over-detect when viewer and target sat at
        // very different elevations (e.g. tower guard above a
        // kneeling target on the ground).
        let viewer_eye = ctx.self_upright_eye_world;
        let viewer_eye_z = viewer_eye.z;
        let target_detection = crate::stealth::detection_point_world(
            view.detection_position_world,
            view.posture,
            view.direction as i16,
            view.is_rider,
        );
        let target_eye_z = target_detection.z;
        let viewer_eye_ground = crate::coordinates::GroundPoint::new(viewer_eye.x, viewer_eye.y);
        let target_detection_ground =
            crate::coordinates::GroundPoint::new(target_detection.x, target_detection.y);
        let dx = target_detection_ground.x - viewer_eye_ground.x;
        let dy = (target_detection_ground.y - viewer_eye_ground.y)
            * crate::position_interface::INVERSE_ASPECT_RATIO;
        let dz = target_eye_z - viewer_eye_z;
        let sq_distance = dx * dx + dy * dy + dz * dz;
        if sq_distance > ctx.sq_self_view_radius {
            tracing::trace!(
                target,
                sq_distance,
                sq_view_radius = ctx.sq_self_view_radius,
                detecting = false,
                "is_detecting_360_degrees: out of range"
            );
            return false;
        }
        // The original game's 360-degree detection checks the
        // upright eye point against the target detection point through the
        // 3D opaque sight-obstacle graph, not the 2D spatial LOS helper.
        let los_clear = crate::sight_obstacle::is_reachable_3d(
            ctx.obstacle_list(),
            [viewer_eye_ground.x, viewer_eye_ground.y, viewer_eye_z],
            [
                target_detection_ground.x,
                target_detection_ground.y,
                target_eye_z,
            ],
            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
        );
        tracing::trace!(
            target,
            sq_distance,
            sq_view_radius = ctx.sq_self_view_radius,
            los_clear,
            detecting = los_clear,
            "is_detecting_360_degrees"
        );
        los_clear
    }

    /// Admission gate for one member of a whole-patrol broadcast: a
    /// non-soldier member short-circuits before the detection query, so it
    /// costs no visibility traffic.
    ///
    /// The broadcast walk runs in the engine, which owns both the chief and
    /// the member, so the gate is evaluated through this accessor immediately
    /// before the member's `think`.
    pub(crate) fn detects_patrol_member_360(&self, member: NpcHandle, ctx: &AiContext) -> bool {
        ctx.entity_view(member)
            .map(|v| v.is_soldier())
            .unwrap_or(false)
            && self.is_detecting_360_degrees(member as HumanHandle, ctx)
    }

    /// Reverse of [`Self::is_detecting_360_degrees`]: does `viewer`
    /// feel *me*?  The radius belongs to the viewer and the detection
    /// point to me, so the resulting ray runs viewer→me — call sites
    /// that ask "can this soldier see me" must not substitute the
    /// forward check, which would swap the ray's endpoints.
    #[track_caller]
    pub(super) fn is_detected_360_degrees_by(
        &self,
        viewer: &CampSoldierInfo,
        ctx: &AiContext,
    ) -> bool {
        let Some(viewer_view) = ctx.entity_view(viewer.handle as HumanHandle) else {
            tracing::trace!(
                viewer = viewer.handle,
                "is_detected_360_degrees_by: entity_view lookup failed"
            );
            return false;
        };
        crate::ai_enemy::soldier_detects_target_360(
            viewer.position,
            viewer_view.elevation,
            viewer_view.is_rider,
            viewer.view_radius,
            viewer_view.in_building,
            ctx.position,
            ctx.elevation,
            ctx.posture,
            ctx.self_is_rider,
            ctx.direction as i16,
            ctx.in_building,
            ctx.obstacle_list(),
        )
    }

    /// Normal NPC detection check used by
    /// synchronous AI state-machine gates. Unlike the 360-degree helper,
    /// this uses the live post-refresh cone and opaque line of sight.
    pub(super) fn is_detecting(&self, target: impl IntoOptionalAiHandle, ctx: &AiContext) -> bool {
        let target = target
            .into_optional_ai_handle()
            .expect("is_detecting requires a non-null target")
            .get();
        let view = ctx.entity_view(target).unwrap_or_else(|| {
            panic!(
                "is_detecting: NPC {} requires missing target entity view {target}",
                self.base.me
            )
        });

        // Visibility calculation uses the sector's BUILDING flag, not the
        // broader engine-side "inside building or passing a door" helper.
        let viewer_in_building = ctx.building_sector.is_some();
        let target_in_same_building =
            viewer_in_building && ctx.building_sector == view.building_sector;

        // This gate exists only in the same-building branch. Outside,
        // bodies and unconscious humans are still valid visibility targets
        // as long as their raw element is active and outside a building.
        if viewer_in_building && (view.is_dead || view.is_unconscious || view.passing_door) {
            return false;
        }
        if !viewer_in_building && (!view.active || view.building_sector.is_some()) {
            return false;
        }

        let target_detection_xy = crate::stealth::detection_point_xy(
            view.detection_position,
            view.posture,
            view.direction as i16,
        );
        let target_detection = crate::stealth::detection_point_world(
            view.detection_position_world,
            view.posture,
            view.direction as i16,
            view.is_rider,
        );
        let sight_obstacles = ctx.obstacle_list();
        let target_obstacle = view.obstacle_idx.map(|handle| {
            sight_obstacles.get(usize::from(handle)).unwrap_or_else(|| {
                panic!("is_detecting: target {target} requires missing sight obstacle {handle}")
            })
        });
        let q = crate::ai_vision::VisibilityQuery {
            viewer_los: ctx.self_eye_position,
            viewer_world: crate::coordinates::WorldPoint3D::new(
                ctx.self_eye_position.x,
                ctx.self_eye_position.y + ctx.elevation,
                ctx.self_eye_z,
            ),
            viewer_direction: ctx.direction as i16,
            view_forward: (ctx.self_view_direction[0], ctx.self_view_direction[1]),
            view_radius: ctx.self_view_radius,
            viewer_eye_status: ctx.self_eye_status,
            real_half_aperture: ctx.self_real_half_aperture,
            viewer_in_building,
            target_in_same_building,
            forest_180_degree_view: ctx.is_forest_level && ctx.is_player_aligned(),
            golden_eye_mode: false,
            effective_view_radius: ctx.self_view_radius as f32,
            target_is_active_and_outside_building: view.active && view.building_sector.is_none(),
            target_los: target_detection_xy,
            target_world: target_detection,
            target_posture: view.posture,
            target_action_state: view.action_state,
            target_is_pc: view.is_pc,
            cloak_deception_applies: view.posture == crate::element::Posture::Cloaked
                && ctx.camp.is_hostile_to(view.camp),
            cloak_remembers_target: self.list_them.contains(&target)
                || self.base.primary_target == Some(AiEntityHandle::new(target)),
            // TODO(cloak-authoring): connect this seam only when an explicit
            // modded profile schema supplies detector data.
            cloak_authored_detector: crate::cloak::SHIPPED_AUTHORED_DETECTOR,
            sight_obstacles: ctx.obstacle_list(),
            fast_grid: &ctx.fast_grid,
            layer: ctx.position.level,
            target_unconscious: view.is_unconscious,
            target_passing_door: view.passing_door,
        };
        let viewer_entity = view_radius_memo_viewer(self.base.me, ctx);
        crate::ai_vision::compute_visibility_with_effective_radius(&q, || {
            ctx.compute_view_radius_cached(viewer_entity, view.obstacle_idx, || {
                crate::ai_vision::compute_view_radius(
                    q.viewer_world,
                    ctx.self_view_radius,
                    (ctx.self_view_direction[0], ctx.self_view_direction[1]),
                    ctx.self_real_half_aperture,
                    ctx.is_night_or_fog,
                    &ctx.fast_grid,
                    sight_obstacles,
                    target_obstacle,
                )
            })
        }) > 0.0
    }

    /// Complete the synchronous Charly-to-officer call after the engine
    /// has delivered `CALL_MR_OFFICER_I_AM_BACK` and obtained the
    /// officer's real `Think` return value.
    pub(crate) fn resolve_charly_officer_report(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        accepted: bool,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if accepted {
            self.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficerSeen);
            self.base.launch_timer(10, ctx.frame);
        } else {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
        }
    }

    pub(crate) fn resolve_alert_request(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        accepted: bool,
        continuation: crate::ai::AlertContinuation,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        assert!(matches!(
            continuation,
            crate::ai::AlertContinuation::SoldierSawOfficer
        ));
        if !accepted {
            self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            return;
        }

        self.set_state(AiState::Seeking, Substate::SeekingRunningToOfficerSeen);
        self.base
            .say_with_flags(Remark::CallsOfficer, SpeechFlags::MYTALK_0);
        let target = self.base.antagonist.unwrap_or_else(|| {
            panic!(
                "accepted soldier alert from {} requires a target officer",
                self.base.me
            )
        });
        let officer_target_pos = ctx
            .entity_view(target)
            .unwrap_or_else(|| {
                panic!(
                    "accepted soldier alert from {} requires target officer {} view",
                    self.base.me, target
                )
            })
            .forecasted_destination
            .resolve(sim)
            .position;
        self.base.go_near(
            officer_target_pos,
            parameters_ai::AI_TALK_DISTANCE,
            crate::ai::GotoFlags::RUN,
            ctx,
        );
        self.base.launch_timer(20, ctx.frame);
    }

    pub(crate) fn resolve_think_result(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        accepted: bool,
        target: NpcHandle,
        continuation: ThinkResultContinuation,
        global: &mut AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        match continuation {
            ThinkResultContinuation::SoldierFinishedAlertReportStart => {
                self.set_state(
                    AiState::Seeking,
                    Substate::SeekingSoldierGiveAlertingReportToOfficerPoint,
                );
                self.base.launch_timer(100, ctx.frame);
            }
            ThinkResultContinuation::OfficerCalledSoldier => {
                if accepted {
                    self.set_state(AiState::Seeking, Substate::SeekingOfficerWaitForSoldier);
                    self.base
                        .set_transient_emoticon(EmoticonType::XMark, 20, ctx.frame);
                    self.base.say(Remark::OfficerCallsSoldier);
                    self.base.launch_timer(20, ctx.frame);
                } else {
                    self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                }
            }
            ThinkResultContinuation::OfficerSentCharlyToOfficer => {
                if accepted {
                    self.base
                        .say_with_flags(Remark::SendsCharlyToOfficer, SpeechFlags::MYTALK_2);
                    self.base.point_to(self.officers_position, ctx);
                }
            }
            ThinkResultContinuation::OfficerInstructedGroupSoldier { last } => {
                if !accepted {
                    self.alerted_us.retain(|&handle| handle != target);
                } else if self.pending_group_instruction_clear_location_after_accept {
                    self.pending_group_instruction_seek_flags &= !SeekFlags::LOCATION_FIRST.bits();
                }
                let finished = if self.pending_group_instruction_candidates.is_empty() {
                    last
                } else {
                    self.queue_next_group_instruction();
                    false
                };
                if finished {
                    self.pending_group_instruction_seek_flags = 0;
                    self.pending_group_instruction_clear_location_after_accept = false;
                    if self.alerted_us.is_empty() {
                        self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
                    } else {
                        self.set_state(
                            AiState::Seeking,
                            Substate::SeekingOfficerWaitForInstructedGroup,
                        );
                        self.base.launch_timer(30, ctx.frame);
                    }
                }
            }
            ThinkResultContinuation::OfficerAlertedSoldier {
                last,
                use_formation,
                failure,
            } => {
                if accepted {
                    self.alerted_us.push(target);
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::ConsiderReport {
                            target,
                            report: self.base.my_reconnaissance_report.clone(),
                            flags: ReportUpdateFlags::UPDATE_CHARLY.bits()
                                | ReportUpdateFlags::UPDATE_TYPE.bits(),
                        },
                    );
                }
                let finished = if self.alerted_us.len() >= 20 {
                    self.pending_alert_soldier_candidates.clear();
                    true
                } else if !self.pending_alert_soldier_candidates.is_empty() {
                    let next = self.pending_alert_soldier_candidates.remove(0);
                    let next_is_last = self.pending_alert_soldier_candidates.is_empty();
                    self.base.outbox.reentrant.cross_npc_actions.push(
                        CrossNpcAction::RequestThinkResult {
                            target: next,
                            caller: self.base.me,
                            stimulus_type: StimulusType::CallAlert,
                            info: StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                            continuation: ThinkResultContinuation::OfficerAlertedSoldier {
                                last: next_is_last,
                                use_formation,
                                failure,
                            },
                        },
                    );
                    false
                } else {
                    last
                };
                if finished {
                    self.pending_alert_soldier_candidates.clear();
                    if accepted {
                        // Original resumes AlertSoldiers only after the
                        // accepted recipient's ConsiderReport call returns.
                        // Keep that callback and all owner-side effects ahead
                        // of formation/state/sequence work.
                        self.base.outbox.reentrant.cross_npc_actions.push(
                            CrossNpcAction::FinalizeAlertSoldiers {
                                caller: self.base.me,
                                use_formation,
                                failure,
                            },
                        );
                    } else {
                        // A refused final call has no ConsiderReport boundary.
                        self.finalize_alert_soldiers(
                            sim,
                            failure,
                            global,
                            grid.filter(|_| use_formation),
                            ctx,
                            tick,
                        );
                    }
                }
            }
            ThinkResultContinuation::OfficerCombatAlertedSoldier {
                last,
                use_formation,
            } => {
                if accepted {
                    self.alerted_us.push(target);
                }
                if last {
                    if self.finish_command_soldiers_to_attack(
                        global,
                        grid.filter(|_| use_formation),
                        ctx,
                        tick,
                    ) {
                        self.base.say(Remark::OfficerGivesAttackOrder);
                    } else {
                        self.enter_battle_reserve(ctx, tick);
                    }
                }
            }
        }
    }

    pub(super) fn queue_next_group_instruction(&mut self) {
        let (target, seek_point) = self
            .pending_group_instruction_candidates
            .first()
            .copied()
            .expect("group instruction continuation requires a pending recipient");
        self.pending_group_instruction_candidates.remove(0);
        let last = self.pending_group_instruction_candidates.is_empty();
        self.base
            .outbox
            .reentrant
            .cross_npc_actions
            .push(CrossNpcAction::RequestThinkResult {
                target,
                caller: self.base.me,
                stimulus_type: StimulusType::CallInstruction,
                info: StimulusInfo::Hint(Hint {
                    seek_point,
                    seek_flags: self.pending_group_instruction_seek_flags,
                    who_tells_me: AiEntityHandle::new(self.base.me),
                }),
                continuation: ThinkResultContinuation::OfficerInstructedGroupSoldier { last },
            });
    }

    pub(super) fn resume_failed_alert_soldiers(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        continuation: AlertSoldiersFailureContinuation,
        global: &mut AiGlobalState,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        match continuation {
            AlertSoldiersFailureContinuation::None => {}
            AlertSoldiersFailureContinuation::ReturnToDuty => {
                self.return_to_duty(sim, DutyFlags::empty(), ctx, tick);
            }
            AlertSoldiersFailureContinuation::SeekBody { center, radius } => {
                self.seek_area(
                    sim,
                    center,
                    radius,
                    SeekFlags::LOCATION_END | SeekFlags::BODY_SEEK,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
            AlertSoldiersFailureContinuation::SeekMissingInstructedSoldier => {
                self.seek_area(
                    sim,
                    ctx.position,
                    parameters_ai::AI_DEAD_BODY_SEEK_RADIUS as u16,
                    SeekFlags::LOCATION_FIRST | self.seek_flags,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
            AlertSoldiersFailureContinuation::SeekMissedCharly { center } => {
                let charly_has_path = ctx
                    .entity_view(self.base.checkpoint_charly)
                    .is_some_and(|view| view.has_patrol_path);
                let radius = if charly_has_path {
                    parameters_ai::AI_PATROL_CHARLY_SEEK_RADIUS as u16
                } else {
                    parameters_ai::AI_FIX_CHARLY_SEEK_RADIUS as u16
                };
                self.seek_area(
                    sim,
                    center,
                    radius,
                    SeekFlags::LOCATION_FIRST | SeekFlags::CHARLY_SEEK,
                    UNDEFINED_DIRECTION,
                    global,
                    ctx,
                    tick,
                );
            }
            AlertSoldiersFailureContinuation::FleeingRunToDoor => {
                self.set_state(AiState::Fleeing, Substate::FleeingRunToDoor);
                self.base.fire_self_stimulus(StimulusType::EventReachPoint);
            }
        }
    }

    pub(crate) fn finalize_alert_soldiers(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        failure: AlertSoldiersFailureContinuation,
        global: &mut AiGlobalState,
        grid: Option<&crate::fast_find_grid::FastFindGrid>,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // Missing-PC search is synchronous. The
        // DEFAULT_LOOKING_FOR_CHARLY timer handler calls it and then arms its
        // regular check timer, so that trailing 10-frame timer overwrites the
        // 20-frame wait timer installed by a successful AlertSoldiers call.
        // Rust suspends AlertSoldiers at the cross-NPC Think boundary; by the
        // time this continuation runs, the caller tail has already armed the
        // check timer.  Remember that exact suspended call site and replay
        // its timer write after the alert finalization.
        let resume_looking_for_charly_timer = matches!(
            failure,
            AlertSoldiersFailureContinuation::SeekMissedCharly { .. }
        ) && self.base.current_state == AiState::Default
            && matches!(
                self.base.current_substate,
                Substate::DefaultLookingForCharly | Substate::DefaultLookingSidewardsForCharly
            )
            && self.base.timer_is_running
            && self.base.when_does_timer_ring
                == ctx
                    .frame
                    .wrapping_add(parameters_ai::AI_CHECKFOR_TIME_INTERVAL as u32);
        let first_new_order = self.base.outbox.actor.orders.len();
        if !self.finish_alert_soldiers(global, grid, ctx, tick) {
            self.resume_failed_alert_soldiers(sim, failure, global, ctx, tick);
        }
        if resume_looking_for_charly_timer {
            self.base
                .launch_timer(parameters_ai::AI_CHECKFOR_TIME_INTERVAL as u32, ctx.frame);
        }
        // AlertSoldiers synchronously calls each recipient's Think before
        // returning to its caller. Rust resumes that caller tail from the
        // cross-NPC action queue, after the physical recursion counter has
        // unwound. A route authored by the resumed formation/failure tail is
        // nevertheless still inside the original enclosing Think and must
        // deliver a same-frame failure back through decision-tick completion.
        if self.base.outbox.actor.orders.len() > first_new_order {
            self.base.completion_latch_inside_think = true;
        }
    }

    /// 180°-detection (the simple-geometry half that can be answered
    /// from AI context alone).
    ///
    /// Short-circuits:
    ///   1. viewer sector is a building → false
    ///   2. either side inactive → false
    ///   3. beyond real view radius → false
    ///   4. within 50 units and "beside me" (perpendicular > forward
    ///      component length) → true (no LOS required)
    ///   5. dot(view, forward) < 0 (target is behind me) → false
    ///   6. beyond the spherical, light-modulated view radius computed
    ///      on the target's surface → false
    ///   7. full-ray opaque LOS check → final answer
    ///
    /// Step 6 is not just a filter: at night and in fog computing the
    /// radius samples the surrounding shadow-light sectors, and the
    /// results land in the shared per-surface radius cache, so it has
    /// to run for exactly the targets that reach it.
    pub(crate) fn is_detecting_180_degrees(
        &self,
        target: impl IntoOptionalAiHandle,
        ctx: &AiContext,
    ) -> bool {
        let target = target
            .into_optional_ai_handle()
            .expect("is_detecting_180_degrees requires a non-null target")
            .get();
        tracing::trace!(
            target,
            viewer_x = ctx.position.x,
            viewer_y = ctx.position.y,
            in_building = ctx.in_building,
            "is_detecting_180_degrees: entry"
        );
        context_detects_180_degrees(self.base.me, target, ctx)
    }

    /// Forward-half-plane detection evaluated on another soldier's behalf.
    ///
    /// The too-proud-to-attack check asks whether a lower-pride ally is observing
    /// our primary target, so the viewer of that test is the ally, not the
    /// deciding soldier. The geometry comes from the ally's entity view;
    /// the post-refresh radius, cone direction and aperture come from
    /// its camp-soldier snapshot.
    pub(super) fn is_detecting_180_degrees_from(
        &self,
        viewer_handle: HumanHandle,
        target: HumanHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        let Some(viewer_view) = ctx.entity_view(viewer_handle) else {
            tracing::warn!(
                viewer = viewer_handle,
                target,
                "is_detecting_180_degrees_from: viewer has no entity view"
            );
            return false;
        };
        let Some(viewer_snapshot) = tick
            .camp_soldiers
            .iter()
            .find(|soldier| soldier.handle == viewer_handle)
        else {
            tracing::warn!(
                viewer = viewer_handle,
                target,
                "is_detecting_180_degrees_from: viewer is absent from the camp-soldier snapshot"
            );
            return false;
        };
        let viewer_eye = crate::stealth::eye_point_xy(
            viewer_view.detection_position,
            viewer_view.posture,
            viewer_view.direction as i16,
            false,
        );
        let viewer = Viewer180 {
            entity: view_radius_memo_viewer(viewer_handle, ctx),
            eye_ground: crate::coordinates::GroundPoint::from_map_and_z(
                viewer_eye,
                viewer_view.elevation,
            ),
            eye_z: viewer_view.elevation
                + crate::stealth::eye_z_for_posture(viewer_view.posture, viewer_view.is_rider),
            direction: viewer_view.direction,
            in_building: viewer_view.in_building,
            view_radius: viewer_snapshot.view_radius,
            sq_view_radius: (viewer_snapshot.view_radius as f32)
                * (viewer_snapshot.view_radius as f32),
            view_direction: viewer_snapshot.view_direction,
            real_half_aperture: viewer_snapshot.real_half_aperture,
        };
        detects_180_degrees(&viewer, target, ctx)
    }
}

/// Standalone actor-side forward-half-plane detection, shared with engine-owned
/// sweeps whose viewer can be a civilian and therefore has no `EnemyAi`.
pub(crate) fn context_detects_180_degrees(
    viewer_handle: HumanHandle,
    target: HumanHandle,
    ctx: &AiContext,
) -> bool {
    let viewer = Viewer180 {
        entity: view_radius_memo_viewer(viewer_handle, ctx),
        // `self_eye_position` is built directly from the element's raw
        // position for eye-point calculation. `ctx.position` may instead be an
        // AI-facing substituted position (a door endpoint/carrier).
        eye_ground: crate::coordinates::GroundPoint::from_map_and_z(
            ctx.self_eye_position,
            ctx.elevation,
        ),
        eye_z: ctx.self_eye_z,
        direction: ctx.direction,
        in_building: ctx.in_building,
        view_radius: ctx.self_view_radius,
        sq_view_radius: ctx.sq_self_view_radius,
        view_direction: ctx.self_view_direction,
        real_half_aperture: ctx.self_real_half_aperture,
    };
    detects_180_degrees(&viewer, target, ctx)
}

/// Resolve the identity a view-radius result is stored under.
/// The memo lives on the surface and records which viewer produced it, so
/// the acting NPC and any ally it reasons through must be distinguishable.
pub(super) fn view_radius_memo_viewer(
    handle: HumanHandle,
    ctx: &AiContext,
) -> crate::element::EntityId {
    ctx.entity_id(handle).unwrap_or_else(|| {
        panic!("view-radius memo viewer {handle} is absent from the AI entity view")
    })
}

/// Viewer half of a 180° detection test, so the test can be evaluated
/// either from the acting NPC or from an ally it is reasoning about.
pub(super) struct Viewer180 {
    /// Identity the surface radius memo is keyed by — the ally when the
    /// test runs through an ally's eyes, not the deciding soldier.
    entity: crate::element::EntityId,
    eye_ground: crate::coordinates::GroundPoint,
    eye_z: f32,
    direction: u16,
    in_building: bool,
    view_radius: u16,
    sq_view_radius: f32,
    view_direction: [f32; 2],
    real_half_aperture: f32,
}

pub(super) fn detects_180_degrees(
    viewer: &Viewer180,
    target: HumanHandle,
    ctx: &AiContext,
) -> bool {
    // Step 1: viewer in a building — always returns false.
    if viewer.in_building {
        return false;
    }
    let Some(view) = ctx.entity_view(target) else {
        tracing::trace!(
            target,
            "is_detecting_180_degrees: entity_view lookup failed"
        );
        return false;
    };
    // Step 2: the original game checks the raw active flag, not whether
    // the target can fight. An unconscious actor remains active and can
    // therefore still pass this standalone 180-degree visibility test.
    if !view.active {
        return false;
    }

    let viewer_eye_z = viewer.eye_z;
    // Detection-point calculation starts from the raw element position. The
    // AI-facing `view.position` may be a substituted door endpoint/carrier.
    let target_detection_world = crate::stealth::detection_point_world(
        view.detection_position_world,
        view.posture,
        view.direction as i16,
        view.is_rider,
    );
    let target_detection_z = target_detection_world.z;
    let viewer_eye_ground = viewer.eye_ground;
    let target_detection_ground =
        crate::coordinates::GroundPoint::new(target_detection_world.x, target_detection_world.y);

    // Aspect-ratio-stretched view vector (`INVERSE_ASPECT_RATIO`
    // on the Y component), from viewer eye to target detection point.
    let dx = target_detection_ground.x - viewer_eye_ground.x;
    let dy = (target_detection_ground.y - viewer_eye_ground.y)
        * crate::position_interface::INVERSE_ASPECT_RATIO;
    let sq_distance = dx * dx + dy * dy;
    tracing::trace!(
        target,
        viewer_x = viewer_eye_ground.x,
        viewer_y = viewer_eye_ground.y,
        viewer_z = viewer_eye_z,
        target_x = target_detection_ground.x,
        target_y = target_detection_ground.y,
        sq_distance,
        sq_view_radius = viewer.sq_view_radius,
        "is_detecting_180_degrees: geometry"
    );
    if sq_distance > viewer.sq_view_radius {
        return false;
    }

    // Direction-vector calculation first compresses the table Y by
    // ASPECT_RATIO; Original then stretches it back here.  The shared
    // Rust table is already the resulting uncompressed unit vector, so
    // applying INVERSE_ASPECT_RATIO a second time would narrow the
    // forward half-plane incorrectly.
    let dir = crate::shadow_polygon::sector_to_direction(viewer.direction as i16);
    let fx = dir[0];
    let fy = dir[1];

    // Step 4: very-near "beside me" short-circuit.
    if sq_distance < 50.0 * 50.0 {
        let fwd_len = dx * fx + dy * fy;
        let fc_x = fx * fwd_len;
        let fc_y = fy * fwd_len;
        let perp_sq = (dx - fc_x) * (dx - fc_x) + (dy - fc_y) * (dy - fc_y);
        if perp_sq >= fwd_len {
            return true;
        }
    }

    // Step 5: forward half-plane.
    if dx * fx + dy * fy < 0.0 {
        return false;
    }

    // Step 6: second, tighter radius gate against the spherical and
    // light-modulated radius. At night and in fog this is where the
    // viewer samples the surrounding shadow-light sectors, so it must
    // run for every target that survives the gates above — and only for
    // those, since the sampling is observable through the shared
    // per-surface radius cache.
    let sight_obstacles = ctx.obstacle_list();
    let target_obstacle = view.obstacle_idx.map(|handle| {
        sight_obstacles.get(usize::from(handle)).unwrap_or_else(|| {
            panic!(
                "is_detecting_180_degrees: target {target} requires missing sight obstacle {handle}"
            )
        })
    });
    let compute_radius = || {
        crate::ai_vision::compute_view_radius(
            crate::coordinates::WorldPoint3D::new(
                viewer_eye_ground.x,
                viewer_eye_ground.y,
                viewer_eye_z,
            ),
            viewer.view_radius,
            (viewer.view_direction[0], viewer.view_direction[1]),
            viewer.real_half_aperture,
            ctx.is_night_or_fog,
            &ctx.fast_grid,
            sight_obstacles,
            target_obstacle,
        )
    };
    let effective_view_radius =
        ctx.compute_view_radius_cached(viewer.entity, view.obstacle_idx, compute_radius);
    if sq_distance > effective_view_radius * effective_view_radius {
        return false;
    }

    crate::sight_obstacle::is_reachable_3d(
        ctx.obstacle_list(),
        [viewer_eye_ground.x, viewer_eye_ground.y, viewer_eye_z],
        [
            target_detection_ground.x,
            target_detection_ground.y,
            target_detection_z,
        ],
        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
    )
}
