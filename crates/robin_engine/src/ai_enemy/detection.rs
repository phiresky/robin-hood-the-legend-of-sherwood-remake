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
        let target_detection = crate::stealth::detection_point_world(
            view.detection_position_world,
            view.posture,
            view.direction as i16,
            view.is_rider,
        );
        let detection = detects_360(
            Viewer360 {
                eye: viewer_eye,
                sq_radius: ctx.sq_self_view_radius,
                in_building: ctx.building_sector.is_some(),
            },
            Target360 {
                detection: target_detection,
                in_building: view.in_building,
            },
            ctx.obstacle_list(),
        );
        let sq_distance = detection
            .sq_distance
            .expect("both building gates were checked above");
        let los_clear = detection.visible;
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
        frame: u32,
        accepted: bool,
    ) -> AiFlow<()> {
        if accepted {
            self.set_state(AiState::Seeking, Substate::SeekingCharlyGoToOfficerSeen);
            self.base.launch_timer(10, frame);
        } else {
            return Err(DutyCall::new(DutyFlags::empty(), false));
        }
        Ok(())
    }

    /// Apply local result bookkeeping. A true result asks the caller to
    /// finish the speech and point toward the live officer position.
    pub(crate) fn resolve_think_result(
        &mut self,
        frame: u32,
        accepted: bool,
        target: NpcHandle,
        continuation: ThinkResultContinuation,
    ) -> AiFlow<bool> {
        match continuation {
            ThinkResultContinuation::OfficerCalledSoldier => {
                if accepted {
                    self.set_state(AiState::Seeking, Substate::SeekingOfficerWaitForSoldier);
                    self.base
                        .set_transient_emoticon(EmoticonType::XMark, 20, frame);
                    self.base.say(Remark::OfficerCallsSoldier);
                    self.base.launch_timer(20, frame);
                } else {
                    return Err(DutyCall::new(DutyFlags::empty(), false));
                }
            }
            ThinkResultContinuation::OfficerSentCharlyToOfficer => {
                if accepted {
                    self.base
                        .say_with_flags(Remark::SendsCharlyToOfficer, SpeechFlags::MYTALK_2);
                    // The caller reads the live pointing destination after
                    // the speech callback has completed.
                    return Ok(true);
                }
            }
        }
        Ok(false)
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
/// from the acting NPC, from an ally it is reasoning about, or from a
/// phalanx member's snapshot.
pub(crate) struct Viewer180 {
    /// Identity the surface radius memo is keyed by — the ally when the
    /// test runs through an ally's eyes, not the deciding soldier.
    pub(crate) entity: crate::element::EntityId,
    pub(crate) eye_ground: crate::coordinates::GroundPoint,
    pub(crate) eye_z: f32,
    pub(crate) direction: u16,
    pub(crate) in_building: bool,
    pub(crate) view_radius: u16,
    pub(crate) sq_view_radius: f32,
    pub(crate) view_direction: [f32; 2],
    pub(crate) real_half_aperture: f32,
}

/// Target half of a 180° detection test, built from an entity view or from
/// a phalanx enemy snapshot.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct Target180 {
    pub(crate) handle: HumanHandle,
    /// Raw active flag, not able-to-fight: an unconscious actor remains
    /// active and can still pass the 180-degree visibility test.
    pub(crate) active: bool,
    /// World-space detection point. Detection-point calculation starts from
    /// the raw element position; an AI-facing position may be a substituted
    /// door endpoint/carrier.
    pub(crate) detection_world: crate::coordinates::WorldPoint3D,
    /// Projection obstacle the target stands on (view-radius memo key).
    pub(crate) obstacle: Option<crate::position_interface::ObstacleHandle>,
}

/// Entity-view adapter over [`detects_180_degrees_core`].
///
/// Deliberately not `#[track_caller]`: every entity-view 180° check
/// attributes its recorded view-radius and visibility queries to this one
/// site, as it did before the core was split out.
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
    let target = Target180 {
        handle: target,
        active: view.active,
        detection_world: crate::stealth::detection_point_world(
            view.detection_position_world,
            view.posture,
            view.direction as i16,
            view.is_rider,
        ),
        obstacle: view.obstacle_idx,
    };
    detects_180_degrees_core(viewer, &target, ctx)
}

#[track_caller]
pub(super) fn detects_180_degrees_core(
    viewer: &Viewer180,
    target: &Target180,
    ctx: &AiContext,
) -> bool {
    detects_180_degrees_live(viewer, target, ctx.obstacle_list(), || {
        let viewer_eye_ground = viewer.eye_ground;
        let viewer_eye_z = viewer.eye_z;
        let target_handle = target.handle;
        let sight_obstacles = ctx.obstacle_list();
        let target_obstacle = target.obstacle.map(|handle| {
        sight_obstacles.get(usize::from(handle)).unwrap_or_else(|| {
            panic!(
                "is_detecting_180_degrees: target {target_handle} requires missing sight obstacle {handle}"
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
            ctx.compute_view_radius_cached(viewer.entity, target.obstacle, compute_radius);
        effective_view_radius
    })
}

/// The single 180° detection implementation (steps listed on
/// [`EnemyAi::is_detecting_180_degrees`]). `#[track_caller]` so adapters
/// choose where the recorded queries are attributed.
#[track_caller]
pub(crate) fn detects_180_degrees_live(
    viewer: &Viewer180,
    target: &Target180,
    sight_obstacles: crate::sight_obstacle::ObstacleList<'_>,
    radius: impl FnOnce() -> f32,
) -> bool {
    // Step 1: viewer in a building — always returns false.
    if viewer.in_building {
        return false;
    }
    // Step 2: raw active flag of the target.
    if !target.active {
        return false;
    }

    let viewer_eye_z = viewer.eye_z;
    let target_detection_z = target.detection_world.z;
    let viewer_eye_ground = viewer.eye_ground;
    let target_detection_ground =
        crate::coordinates::GroundPoint::new(target.detection_world.x, target.detection_world.y);
    let target_handle = target.handle;

    // Aspect-ratio-stretched view vector (`INVERSE_ASPECT_RATIO`
    // on the Y component), from viewer eye to target detection point.
    let dx = target_detection_ground.x - viewer_eye_ground.x;
    let dy = (target_detection_ground.y - viewer_eye_ground.y)
        * crate::position_interface::INVERSE_ASPECT_RATIO;
    let sq_distance = dx * dx + dy * dy;
    tracing::trace!(
        target = target_handle,
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

    // Step 4: very-near "beside me" short-circuit; step 5: forward
    // half-plane (shared with the planar `detects_position_180_raw`).
    match half_plane_180(dx, dy, sq_distance, viewer.direction) {
        HalfPlane180::Beside => return true,
        HalfPlane180::NotBeside { forward_dot } => {
            if forward_dot < 0.0 {
                return false;
            }
        }
    }

    // Step 6: second, tighter radius gate against the spherical and
    // light-modulated radius. At night and in fog this is where the
    // viewer samples the surrounding shadow-light sectors, so it must
    // run for every target that survives the gates above — and only for
    // those, since the sampling is observable through the shared
    // per-surface radius cache.
    let effective_view_radius = radius();
    if sq_distance > effective_view_radius * effective_view_radius {
        return false;
    }

    crate::sight_obstacle::is_reachable_3d(
        sight_obstacles,
        [viewer_eye_ground.x, viewer_eye_ground.y, viewer_eye_z],
        [
            target_detection_ground.x,
            target_detection_ground.y,
            target_detection_z,
        ],
        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
    )
}
