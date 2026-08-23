use super::*;

/// Resolve the entry point used when `ReconsiderEnemyApproach`'s final target
/// position belongs to a lift.
///
/// Original `RHSectorLift::InitializeFromProtoStream` identifies the high and
/// low doors by minimum/maximum `PointOut.Y`, irrespective of their authored
/// door-type tags (`RHsector.cpp:1492-1525`). `ReconsiderEnemyApproach` then
/// uses the high entry only when its outside layer equals the attacker's
/// current layer; every other layer uses the low entry
/// (`RHartificialmalignity.cpp:6649-6678`).
///
/// The outer option distinguishes an ordinary sector from a lift. Stairs are
/// lifts, but deliberately return `Some(None)` because they suppress charging
/// without taking the ladder-entry detour.
impl AiContext {
    pub(crate) fn enemy_lift_approach_for_position(
        fast_grid: &crate::fast_find_grid::FastFindGrid,
        target: Position,
        attacker_layer: Option<u16>,
    ) -> Option<Option<Position>> {
        let target_sector = target.sector?;
        let sector_number = crate::sector::SectorNumber::new(target_sector.get() as i16);
        let grid_index = *fast_grid
            .level
            .sector_number_map
            .get(&sector_number)
            .unwrap_or_else(|| {
                panic!("primary target sector {sector_number} is absent from the grid")
            });
        let sector = fast_grid.level.sectors.get(grid_index).unwrap_or_else(|| {
            panic!("primary target sector {sector_number} maps to missing grid index {grid_index}")
        });
        if !sector.sector_type.is_lift() && sector.lift_type.is_none() {
            return None;
        }
        let lift_type = sector.lift_type.unwrap_or_else(|| {
            panic!("lift sector {sector_number} has no lift type during enemy approach")
        });
        if lift_type == crate::sector::LiftType::Stairs {
            return Some(None);
        }

        // A lift sector's own doors have the lift as `sector_in`, hence their
        // outside endpoint differs from the lift sector. Gate indices can also
        // include a door whose outside is this lift; exclude that reverse edge.
        let mut endpoints = sector.gate_indices.iter().filter_map(|index| {
            let door = fast_grid
                .level
                .door_projection_infos
                .get(index.0 as usize)
                .unwrap_or_else(|| {
                    panic!(
                        "lift sector {sector_number} references missing door projection {}",
                        index.0
                    )
                });
            (door.sector_out != sector_number).then_some((*index, door))
        });
        let first = endpoints.next().unwrap_or_else(|| {
            panic!("non-stairs lift sector {sector_number} has no authored entry doors")
        });
        let (mut high, mut low) = (first, first);
        for endpoint in endpoints {
            if endpoint.1.point_out.y < high.1.point_out.y {
                high = endpoint;
            }
            if endpoint.1.point_out.y > low.1.point_out.y {
                low = endpoint;
            }
        }
        assert!(
            high.0 != low.0,
            "non-stairs lift sector {sector_number} has fewer than two distinct authored entry doors"
        );

        let attacker_layer = attacker_layer.unwrap_or_else(|| {
            panic!("enemy lift approach for sector {sector_number} has no live attacker layer")
        });
        let selected = if high.1.layer_out == attacker_layer {
            high.1
        } else {
            low.1
        };
        Some(Some(Position {
            x: selected.point_out.x,
            y: selected.point_out.y,
            sector: crate::position_interface::SectorHandle::new(u16::from(selected.sector_out)),
            level: selected.layer_out,
        }))
    }
}

// AiContext — per-frame entity state passed into think()
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn seek_point(x: f32) -> SeekPoint {
        SeekPoint {
            position: Position {
                x,
                ..Position::default()
            },
            frame_when_full_interest: 0,
            directions: Vec::new(),
            last_calculated_interest: 100,
            locked: false,
            id: 0,
        }
    }

    #[test]
    fn near_seek_candidates_use_truncated_uword_distances() {
        let sim = crate::sim_rng::test_context();
        let mut global = AiGlobalState::default();
        global.seek_points.push(seek_point(9.1));
        let mut target = Position::default();
        let me = Position {
            x: 31.9,
            ..Position::default()
        };

        // FLOAT comparison would accept 9.1 < 31.9 * 0.3 (9.57).
        // Original narrows both sides first, so 9 < 9 is false.
        assert!(!global.set_pos_on_near_seek_point(&sim, me, &mut target, 0.3, 0));

        global.seek_points[0].position.x = 8.9;
        assert!(global.set_pos_on_near_seek_point(&sim, me, &mut target, 0.3, 0));
        assert_eq!(target.x, 8.9);
    }

    #[test]
    fn near_seek_layer_penalty_wraps_as_uword() {
        let sim = crate::sim_rng::test_context();
        let mut global = AiGlobalState::default();
        let mut point = seek_point(65_500.0);
        point.position.level = 1;
        global.seek_points.push(point);
        let mut target = Position::default();

        // (UWORD)65500 + 100 wraps to 64 and is below the limit.
        assert!(
            global.set_pos_on_near_seek_point(&sim, Position::default(), &mut target, 0.0, 65,)
        );
        assert_eq!(target.x, 65_500.0);
    }
}

/// Per-frame entity state passed into `think()` by the engine.
/// Replaces the stale-prone `cached_*` fields on `AiBase` for data that
/// changes every frame (position, direction, posture, etc.).
#[derive(Debug, Clone, Default)]
pub struct AiContext {
    pub difficulty: crate::player_profile::DifficultyLevel,
    /// Original `RHElement::GetCreationOrder()` for the evaluating actor.
    /// This is distinct from the Rust entity-table slot and is required by
    /// `ForbidRemark(..., THIS_GUY)` identity checks.
    pub original_creation_order: Option<u32>,
    pub position: Position,
    /// Live `RHElement::GetLayer()` for the evaluating actor.  This can
    /// differ from [`Self::position`]'s level while `Position(actor)` snaps a
    /// door-passing actor to the committed gate side.  Original GoNear uses
    /// the snapped point for its distance but the live actor layer for its
    /// same-layer gate.
    pub self_layer: u16,
    /// Live `RHElement::GetPosition()` body point for the evaluating actor.
    /// Unlike [`Self::position`], this never snaps a door-passing actor to a
    /// gate endpoint. Direct element-distance tests must use this point.
    pub self_body_position_world: crate::coordinates::WorldPoint3D,
    pub frame: u32,
    pub direction: u16,
    pub posture: crate::element::Posture,
    /// Live eye point and view-cone parameters after `RefreshView`.
    /// Used by synchronous `IsDetecting(human)` checks inside AI state
    /// handlers, as distinct from the periodic detectable-list pass.
    pub self_eye_position: MapPoint,
    pub self_eye_z: f32,
    /// Direct `ComputeEyesPoint(..., UPRIGHT)` result. Unlike `position`, this
    /// is never snapped through AI `Position()` while passing a door.
    pub self_upright_eye_world: crate::coordinates::WorldPoint3D,
    /// Live `mViewParameters.starePoint` in ground-plane coordinates.
    /// Unexpected OUTOFVIEW handling compares this against the actor's
    /// ground position before deciding whether the lost target is behind.
    pub self_stare_point: crate::coordinates::GroundPoint,
    pub self_view_direction: [f32; 2],
    pub self_view_radius: u16,
    pub self_real_half_aperture: f32,
    pub self_eye_status: crate::element::EyeStatus,
    /// Current ambiance is Night or Fog. Used by synchronous normal
    /// `IsDetecting(human)` calls to run the authoritative light-sector
    /// modulation in `ComputeViewRadius`.
    pub is_night_or_fog: bool,
    /// `IsVeryVeryBusy`'s sequence-element arm: `true` when the actor's
    /// current in-flight sequence element is `Command::PassDoor` or
    /// `Command::Fall`. The posture arm is covered separately via
    /// `posture` above. Used by `FriendlyAi::return_to_duty` to lock
    /// `AILOCK_BUSY` and defer `EventReturnToDuty` mid-door-pass.
    /// Defaults to `false` for AiContexts not built through the
    /// per-tick engine path (unit tests, fallback fields).
    pub in_uninterruptible_command: bool,
    pub in_building: bool,
    /// The evaluating element's own active flag. Paired with `in_building`
    /// it forms the "active and outside a building" predicate that gates the
    /// outdoor arm of `answer_question`: an inactive actor answers from the
    /// indoor arm even when it is standing outdoors.
    pub self_is_active: bool,
    pub building_sector: Option<SectorHandle>,
    pub camp: crate::element::Camp,
    pub is_swordfighting: bool,
    /// `true` when the sequence manager has a pending `ENTER_SWORDFIGHT`
    /// element for this NPC. `ReconsiderSwordfight` bails out early when
    /// an enter-swordfight sequence is already queued.
    pub enter_swordfight_pending: bool,
    /// True when the current level is Sherwood Forest. Used by
    /// `is_merry_man_forest()` and the 180° detection cone for Royalist
    /// NPCs.
    pub is_forest_level: bool,
    /// The evaluating entity's zero-centred collision bounding box.
    pub move_box: crate::coordinates::MoveBox,
    /// NPC's remaining arrow count (`GetAmmoAmount(RHACTION_BOW)`). Used
    /// by archer decision logic.
    pub remaining_arrows: u16,
    /// Square of the engine's standard view-polygon radius. Used to gate
    /// cover-position acceptance for archers behind shield bearers (the
    /// cover point must be within view radius of the primary target).
    pub sq_standard_view_radius: f32,
    /// Square of this NPC's live `mViewParameters.uwRealRadius`.
    /// Detection helpers use this instead of the level standard radius
    /// because alertness, drunk view, lean-out, and scripts can mutate
    /// the real radius independently.
    pub sq_self_view_radius: f32,
    /// Entity elevation (Z coordinate). Used by archer bow-down/bow-up
    /// decisions.
    pub elevation: f32,

    /// Self is a civilian beggar (`CIVILIAN_BEGGAR`). `false` for
    /// non-civilians.
    pub self_is_beggar: bool,
    /// Self is a civilian child (`CIVILIAN_CHILD`). `false` for
    /// non-civilians.
    pub self_is_child: bool,
    /// `true` when the evaluating NPC is a soldier (enemy AI variant),
    /// `false` for civilians. Used by the waypoint-macro executor to gate
    /// soldier-only opcodes (CHECK_4, LOOK_LEFT, LOOK_RIGHT, BEND,
    /// PATROL_*).
    pub self_is_soldier: bool,
    /// `true` when the evaluating NPC is a mounted soldier (rider).
    /// Sourced from [`SoldierData::rider`] each tick. `false` for
    /// non-soldiers.
    pub self_is_rider: bool,
    /// Self's `ActionState` (`Waiting` / `Moving` / `MovingFast` / sword
    /// states / etc.). Used by `EventViewStandardProcedure` to branch on
    /// `RHACTIONSTATE_MOVING_FAST` (sprint-into-engage path). Defaults to
    /// `Waiting` for unit tests built off `AiContext::default()`.
    pub self_action_state: crate::element::ActionState,
    /// Self's soldier rank if soldier; `ProfileRank::None` otherwise.
    /// Used by `GetBoredTime` to pick officer-length intervals.
    pub self_rank: crate::profiles::ProfileRank,
    /// Self's soldier pride. `0` for non-soldiers or soldiers with no
    /// pride. Used by `GetBoredTime` to pick the long "pride" bored
    /// interval.
    pub self_pride: u16,

    /// Self's current life points, read live from the element rather than
    /// from an AI-side cache. `MakeBattlePredecisions` scales the battle
    /// odds by `life_points / max_life_points`, so a stale copy makes a
    /// badly wounded soldier fight on instead of calling for help.
    pub self_life_points: i16,

    /// Self's maximum life points — the soldier profile value after the
    /// difficulty modifier, `100` for civilians and PCs.
    pub self_max_life_points: i16,

    /// `true` when this NPC is dead (`life_points <= 0`). Read by the
    /// `start_think` dead-gate to short-circuit stimulus processing —
    /// defence-in-depth against cross-NPC actions or scripts that fire
    /// stimuli at a corpse after the tick loop would normally skip it.
    pub self_is_dead: bool,

    /// `true` when the evaluating human's physical unconscious flag is set.
    /// Original `StartThink` checks this flag independently of the AI state:
    /// a postponed injury can leave an unconscious actor in a non-sleeping
    /// substate, but ordinary stimuli are still refused.
    pub self_is_unconscious: bool,

    /// Number of entries in this NPC's
    /// `detectable_lists[DetectableType::Friend]`. Used by
    /// `return_to_duty_common_stuff` to decide whether to clear
    /// `detected_body`.
    pub self_detectable_friend_count: u16,

    /// `true` for soldier NPCs whose `forced_attentive` flag is set,
    /// `false` for civilians and non-forced soldiers. Read by
    /// `set_alert_status_with_flags` to pin the view alert to YELLOW when
    /// the music alert drops to GREEN.
    pub self_forced_attentive: bool,

    /// Number of entries in this NPC's
    /// `detectable_lists[DetectableType::MissedFriend]`. Used by
    /// `EnemyAi::return_to_duty` to detect that the NPC was searching for
    /// a missed-in-action friend (`checkpoint_charly`) when bailing out.
    pub self_detectable_missed_friend_count: u16,

    /// Enemy detectable handles whose authoritative `seen_now` latch is set,
    /// in the detectable-list's pointer order. Original
    /// `ReinitializeThemList` walks this live list at the exact Think
    /// boundary; geometric visibility products in [`AiPerTickData`] are not
    /// interchangeable with it.
    pub self_seen_enemy_handles: Vec<HumanHandle>,

    /// Live animation (`OrderType`) currently playing on this NPC. Read
    /// by AI gates that inspect the actor's current animation directly,
    /// e.g. `DefaultBoredStandardProcedure` skips its head-turn
    /// transition while the `WAITING_UPRIGHT_BORED_RANDOM` idle is
    /// already playing.
    pub self_animation: crate::order::OrderType,

    /// Live actor motion lifecycle for `self_animation`. A newly installed
    /// move-to-wait transition (`Start`) is still a real Original GoTo input;
    /// only a transition that has already advanced can represent Rust's
    /// one-owner-boundary lag behind Original's idle successor.
    pub self_animation_motion_state: crate::sprite::MotionState,

    /// The sequence-manager element currently selected by the actor is its
    /// default Wait element (`mpWaitSequenceElement` in the Original).
    /// `RHElementActor::Stop` deliberately skips that exact element, so a
    /// deferred `Halt` must not be projected as clearing its live animation.
    /// Refreshed from the sequence manager at every filtered Think boundary.
    pub self_selected_element_is_default_wait: Option<bool>,

    /// Priority of the sequence-manager element currently selected by the
    /// actor. The outer `Option` distinguishes a context that has not been
    /// refreshed at the live owner boundary; the inner `Option` represents
    /// an actor with no selected `mpSequenceElement`. `Halt()` uses
    /// `Stop(PREFERENCE)`, so deferred Halt projection must consult this
    /// priority before pretending that the selected animation was stopped.
    pub self_selected_element_priority: Option<Option<crate::sequence::SequencePriority>>,

    /// `true` when the sprite backing `self_animation` has reached or passed
    /// its authored action-done frame/counter. The Original actor hourglass
    /// retires a completed move-to-wait transition before later NPC timer
    /// callbacks inspect `GetAnimation`; Rust's split phase keeps the order
    /// installed until the sequence drain, so GoTo uses this bit to project
    /// that narrow completion boundary.
    pub self_animation_reached_action_done: bool,

    /// Resolved info about the stimulus's antagonist entity — the
    /// "other" human the stimulus is about (the observed PC for an
    /// `EventView`, the body for `EventSeesBody`, etc.).  The engine
    /// populates this before dispatching any stimulus whose
    /// `StimulusInfo::Human(_)` payload identifies a live entity, so
    /// that `event_*_standard_procedure` handlers don't need to reach
    /// back into the entity table.  `None` for stimuli without a
    /// human payload, or if the referenced entity has been removed.
    pub antagonist: Option<AntagonistInfo>,

    /// Handle → snapshot map for **every** entity visible to the AI
    /// this tick. Populated once at the top of the AI tick by
    /// `EngineInner::build_sim_scratch` and shared into each
    /// `AiContext` via an [`Arc`] so cloning / re-building contexts is
    /// cheap. Used to answer per-entity field reads (position, camp,
    /// ai_state, …) for any handle the AI has stashed (antagonist,
    /// primary target, interesting object, detected body, friend, …).
    pub entity_views: crate::ai_entity_view::SharedAiEntityViews,

    /// Per-tick `Arc`-shared snapshot of the engine's sight obstacles.
    /// Built once by `EngineInner::build_sim_scratch` and
    /// embedded into every `AiContext` so AI-side helpers can answer
    /// `ai_vision::los_clear` (opaque-LOS) without a mutable engine
    /// borrow. Use `obstacle_list()` for the borrowed `ObstacleList<'_>`
    /// shape that `ai_vision::los_clear` accepts.
    pub sight_obstacles: crate::sight_obstacle::SharedSightObstacles,
    /// Owner-local view of the engine's surface-owned radius cache. Engine
    /// dispatch seeds this immediately before Think and commits it immediately
    /// after Think, allowing immutable AI handlers to preserve Original's
    /// synchronous cache semantics without sharing rollback state by `Arc`.
    /// Each surface holds exactly one entry, tagged with the viewer that
    /// wrote it: a lookup by any other viewer misses, and its recomputation
    /// replaces the entry. That is the surface-slot behaviour the AI has to
    /// reproduce when it evaluates a detection test through an ally's eyes.
    pub(crate) view_radius_cache: std::cell::RefCell<
        std::collections::HashMap<
            Option<crate::position_interface::ObstacleHandle>,
            (crate::element::EntityId, f32),
        >,
    >,
    /// FastFindGrid snapshot used for `IsReachable` line-of-sight queries
    /// from AI code that only has an `AiContext`. `Arc`-shared with the
    /// engine's copy-on-write grid, so building a context is a refcount
    /// bump while the snapshot stays frozen at its build instant.
    pub fast_grid: std::sync::Arc<crate::fast_find_grid::FastFindGrid>,
    /// Shared mission hiking paths from [`LevelAssets`]. Static level data
    /// threaded through context so individual AI controllers do not each cache
    /// their own Arc attachment.
    pub hiking_paths: Arc<Vec<crate::level_data::RawHikingPath>>,
    /// Exact sector handles keyed by stable `(hiking path, waypoint)`.
    /// Real loaded missions always provide this; `None` explicitly denotes
    /// synthetic number-only test data.
    pub hiking_waypoint_sectors: Option<Arc<Vec<Vec<crate::position_interface::SectorHandle>>>>,

    /// Soldier load-order index → entity slot mapping (cloned from
    /// [`AiGlobalState::all_soldier_handles`]). Used by waypoint-macro
    /// opcodes (`CMD_CHECK_4` / `CMD_CHECK_4_SYNC`) that resolve a
    /// friend ID baked into the script bytecode.
    pub all_soldier_handles: std::sync::Arc<Vec<u32>>,
}

impl AiContext {
    pub(crate) fn hiking_waypoint_sector(
        &self,
        path_index: usize,
        waypoint_index: usize,
        public_sector: u16,
    ) -> Option<crate::position_interface::SectorHandle> {
        let Some(paths) = &self.hiking_waypoint_sectors else {
            return crate::position_interface::SectorHandle::new(public_sector);
        };
        let exact = paths
            .get(path_index)
            .and_then(|path| path.get(waypoint_index))
            .copied()
            .unwrap_or_else(|| {
                panic!(
                    "required exact hiking waypoint identity is missing for path {path_index} waypoint {waypoint_index}"
                )
            });
        assert_eq!(
            exact.get(),
            public_sector,
            "hiking waypoint path {path_index} waypoint {waypoint_index} public/exact identity conflict"
        );
        Some(exact)
    }

    /// Import every surface entry stored for the current frame, keeping the
    /// viewer that wrote it. Entries belonging to other viewers have to be
    /// carried too: they are what makes this Think miss a surface an ally
    /// already claimed earlier in the same frame.
    pub(crate) fn seed_view_radius_cache(&self, cache: &crate::ai_vision::ViewRadiusCache) {
        let mut values = self.view_radius_cache.borrow_mut();
        values.clear();
        let mut seed = |surface, entry: Option<crate::ai_vision::ViewRadiusCacheEntry>| {
            if let Some(entry) = entry
                && entry.frame == self.frame
                && entry.radius != 0.0
            {
                values.insert(surface, (entry.viewer, entry.radius));
            }
        };
        seed(None, cache.ground);
        for (index, entry) in cache.obstacles.iter().enumerate() {
            let Some(handle) = u16::try_from(index)
                .ok()
                .and_then(crate::position_interface::ObstacleHandle::new)
            else {
                continue;
            };
            seed(Some(handle), *entry);
        }
    }

    #[track_caller]
    pub(crate) fn compute_view_radius_cached(
        &self,
        viewer: crate::element::EntityId,
        surface: Option<crate::position_interface::ObstacleHandle>,
        compute: impl FnOnce() -> f32,
    ) -> f32 {
        if let Some(&(stored_viewer, radius)) = self.view_radius_cache.borrow().get(&surface)
            && stored_viewer == viewer
        {
            crate::ai_vision::debug_view_radius_cache_event(
                "owner_hit",
                "ai_context",
                surface,
                viewer,
                self.frame,
                Some(crate::ai_vision::ViewRadiusCacheEntry {
                    viewer: stored_viewer,
                    frame: self.frame,
                    radius,
                }),
                Some(radius),
                std::panic::Location::caller(),
            );
            return radius;
        }
        let stored = self.view_radius_cache.borrow().get(&surface).copied().map(
            |(stored_viewer, radius)| crate::ai_vision::ViewRadiusCacheEntry {
                viewer: stored_viewer,
                frame: self.frame,
                radius,
            },
        );
        crate::ai_vision::debug_view_radius_cache_event(
            "owner_miss",
            "ai_context",
            surface,
            viewer,
            self.frame,
            stored,
            None,
            std::panic::Location::caller(),
        );
        let radius = compute();
        // Original's getter uses zero as the miss sentinel even after its
        // setter stored a computed zero.
        if radius != 0.0 {
            self.view_radius_cache
                .borrow_mut()
                .insert(surface, (viewer, radius));
        }
        crate::ai_vision::debug_view_radius_cache_event(
            if radius == 0.0 {
                "owner_compute_zero"
            } else {
                "owner_store"
            },
            "ai_context",
            surface,
            viewer,
            self.frame,
            Some(crate::ai_vision::ViewRadiusCacheEntry {
                viewer,
                frame: self.frame,
                radius,
            }),
            Some(radius),
            std::panic::Location::caller(),
        );
        radius
    }

    /// Fold another context's surface-radius entries into this one.
    ///
    /// The memo lives on the surface, not on the context, so a radius any
    /// clone of this context computed during a Think has to survive back to
    /// the context the caller later commits from. Without this the writes are
    /// dropped with the clone and the next Think in the same frame recomputes
    /// a radius the surface already knows — visible as extra night/fog
    /// barycentre rays.
    pub(crate) fn absorb_view_radius_cache(&self, other: &Self) {
        let mut values = self.view_radius_cache.borrow_mut();
        for (&surface, &entry) in other.view_radius_cache.borrow().iter() {
            values.insert(surface, entry);
        }
    }

    pub(crate) fn commit_view_radius_cache(&self, cache: &mut crate::ai_vision::ViewRadiusCache) {
        for (&surface, &(viewer, radius)) in self.view_radius_cache.borrow().iter() {
            cache.set(surface, viewer, self.frame, radius);
        }
    }

    /// Look up a handle in the per-tick entity view map.
    ///
    /// Returns `None` for handle `0`, for handles that were never
    /// populated (non-human entities not included in the snapshot),
    /// and for entities that have since been removed. Callers that
    /// need a specific field (position, ai_state, …) should pattern-
    /// match on the result and fall back to a safe default only
    /// when it makes sense for the call site.
    pub fn entity_view(&self, handle: u32) -> Option<&crate::ai_entity_view::AiEntityView> {
        if handle == 0 {
            return None;
        }
        self.entity_views.get(&handle)
    }

    /// Look up a handle that the calling logic has already established as a
    /// live participant (an active brawl partner, a primary target mid-
    /// engagement, a loot-list entry, …).  Such a handle failing to resolve
    /// means the snapshot lost a required entity — corrupted sim state or a
    /// port bug — so this panics instead of letting the caller silently take
    /// a default gameplay branch.  Callers must still guard the handle-`0`
    /// "no entity" sentinel themselves where "none" is a legal state.
    #[track_caller]
    pub fn expect_entity_view(
        &self,
        handle: u32,
        ctx: &str,
    ) -> &crate::ai_entity_view::AiEntityView {
        self.entity_view(handle)
            .unwrap_or_else(|| panic!("required entity view for handle {handle} missing ({ctx})"))
    }

    /// Resolve a raw legacy human/object handle through the live entity-view
    /// snapshot without guessing its typed [`crate::element::EntityId`] kind.
    pub fn entity_id(&self, handle: u32) -> Option<crate::element::EntityId> {
        self.entity_view(handle)?.entity_id(handle)
    }

    /// Convenience wrapper around [`Self::entity_view`] that returns
    /// just the position.
    pub fn entity_position(&self, handle: u32) -> Option<Position> {
        self.entity_view(handle).map(|v| v.position)
    }

    /// Resolve a C++ `PositionToPoint3D` style point from a sector/layer
    /// position. The returned y coordinate is screen-space (`y + z`).
    pub fn position_to_point_3d(&self, position: Position) -> crate::coordinates::WorldPoint3D {
        let z = match position.sector {
            None => 0.0,
            Some(handle) => {
                let sector_number = crate::sector::SectorNumber::new(handle.get() as i16);
                let grid_idx = self
                    .fast_grid
                    .level
                    .sector_number_map
                    .get(&sector_number)
                    .copied();
                let grid_sector = grid_idx.and_then(|idx| self.fast_grid.level.sectors.get(idx));
                let is_motion = grid_sector.is_some_and(|sector| sector.sector_type.is_motion());
                if grid_idx.is_some() && !is_motion {
                    panic!(
                        "position_to_point_3d: sector {} is not a motion sector",
                        handle.get()
                    );
                }

                // Building motion sectors have no projection area of their
                // own. Original `PositionToPoint3D` walks the sector's gates
                // in order, finds the first door whose inside point is within
                // MaxNorm < 20, and samples the outside sector at PointOut.
                let (projection_sector, projection_layer, point) = if grid_sector
                    .is_some_and(|sector| sector.sector_type.is_building())
                {
                    let sector = grid_sector.expect("building sector disappeared");
                    let door = sector
                            .gate_indices
                            .iter()
                            .filter_map(|index| {
                                self.fast_grid
                                    .level
                                    .door_projection_infos
                                    .get(index.0 as usize)
                            })
                            .find(|door| {
                                (door.point_in.x - position.x)
                                    .abs()
                                    .max((door.point_in.y - position.y).abs())
                                    < 20.0
                            })
                            .unwrap_or_else(|| {
                                panic!(
                                    "position_to_point_3d: building sector {} has no door near ({}, {})",
                                    handle.get(), position.x, position.y
                                )
                            });
                    (u16::from(door.sector_out), door.layer_out, door.point_out)
                } else {
                    (
                        handle.get(),
                        position.level,
                        MapPoint::new(position.x, position.y),
                    )
                };
                let mut best: Option<(f32, f32)> = None;
                for (_, obstacle) in self.sight_obstacles.list().iter_indexed() {
                    if !obstacle.is_projection_area()
                        || obstacle.sector != projection_sector
                        || obstacle.layer != projection_layer
                        || !obstacle.box_projection.contains_point(point)
                        || !obstacle.contains_point_projection(point)
                    {
                        continue;
                    }
                    let z_max = obstacle.box_3d_max[2];
                    let z = obstacle.compute_top_z_from_projection(point.x, point.y);
                    match best {
                        None => best = Some((z_max, z)),
                        Some((prev_z_max, _)) if z_max > prev_z_max => best = Some((z_max, z)),
                        _ => {}
                    }
                }

                best.map(|(_, z)| z).unwrap_or(0.0)
            }
        };

        crate::coordinates::WorldPoint3D {
            x: position.x,
            y: position.y + z,
            z,
        }
    }

    /// Borrowed [`crate::sight_obstacle::ObstacleList`] view over this
    /// tick's sight-obstacle snapshot — the shape that
    /// `ai_vision::los_clear` and the visibility query helpers accept.
    pub fn obstacle_list(&self) -> crate::sight_obstacle::ObstacleList<'_> {
        self.sight_obstacles.list()
    }

    /// Resolve a soldier register number (load-order index) to an
    /// entity slot handle. Returns `None` when the ID is out of range
    /// — the caller should treat that as a null actor and warn/abort
    /// the operation.
    pub fn all_soldier_handle(&self, register: u16) -> Option<u32> {
        self.all_soldier_handles.get(register as usize).copied()
    }

    /// Number of soldiers in the level.
    pub fn number_of_all_soldiers(&self) -> u16 {
        self.all_soldier_handles.len() as u16
    }

    /// Raw-point variant of the 360° detection check. Used by
    /// `InitializeFriendCheck` to ask "can I still see my friend's post
    /// / waypoint from here?".
    /// Steps:
    /// 1. viewer in a building → false
    /// 2. stretched-Y 3D distance vs. `sq_self_view_radius`
    /// 3. opaque-LOS via the context-only LOS helper.
    pub fn is_detecting_point_360(&self, pt: crate::coordinates::WorldPoint3D) -> bool {
        if self.in_building {
            return false;
        }
        let viewer_eye = crate::stealth::eye_point_xy(
            crate::coordinates::MapPoint::new(self.position.x, self.position.y),
            self.posture,
            self.direction as i16,
            false,
        );
        let viewer_eye_z =
            self.elevation + crate::stealth::eye_z_for_posture(self.posture, self.self_is_rider);
        let viewer_eye_ground =
            crate::coordinates::GroundPoint::from_map_and_z(viewer_eye, self.elevation);
        let dx = pt.x - viewer_eye_ground.x;
        let dy = (pt.y - viewer_eye_ground.y) * crate::position_interface::INVERSE_ASPECT_RATIO;
        let dz = pt.z - viewer_eye_z;
        let sq_distance = dx * dx + dy * dy + dz * dz;
        if sq_distance > self.sq_self_view_radius {
            return false;
        }
        crate::sight_obstacle::is_reachable_3d(
            self.obstacle_list(),
            [viewer_eye_ground.x, viewer_eye_ground.y, viewer_eye_z],
            [pt.x, pt.y, pt.z],
            crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
        )
    }
}

#[cfg(test)]
mod hiking_waypoint_identity_tests {
    use super::*;

    #[test]
    fn waypoint_route_position_carries_exact_arena_identity() {
        let exact = crate::position_interface::SectorHandle::new(82)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(17).unwrap());
        let ctx = AiContext {
            hiking_waypoint_sectors: Some(Arc::new(vec![vec![exact]])),
            ..AiContext::default()
        };

        let route_position = Position {
            x: 1432.0,
            y: 930.0,
            sector: ctx.hiking_waypoint_sector(0, 0, 82),
            level: 6,
        };

        assert_eq!(route_position.sector.unwrap().get(), 82);
        assert_eq!(
            route_position.sector.unwrap().arena_index(),
            crate::fast_find_grid::SectorIndex::new(17)
        );
    }
}

/// Lightweight view of an entity other than the evaluating NPC, used
/// by AI stimulus handlers. All fields come from the live entity at
/// the moment the stimulus is dispatched.
#[derive(Debug, Clone, Default)]
pub struct AntagonistInfo {
    /// The antagonist's map position.
    pub position: Position,
    /// The antagonist's camp.
    pub camp: crate::element::Camp,
    /// True when the antagonist is in a sword-fighting action state.
    pub is_swordfighting: bool,
    /// True when the antagonist is a player character.
    pub is_pc: bool,
    /// True when the antagonist is the Robin Hood PC. Civilian reactions
    /// (`CivAdmiresRobin`) special-case this.
    pub is_robin: bool,
    /// True when the antagonist is a VIP civilian / VIP soldier.
    pub is_vip: bool,
    /// True when the antagonist is inside a building sector.
    pub in_building: bool,
}

/// Summary of an unconscious or otherwise-disabled enemy that an NPC
/// could approach and finish off.
///
/// Used by the two "sleeping enemy" paths in `BattleDecisions`:
///
///  * `unconscious_enemies` — enemies that were in `list_them` when
///    the cleanup pass filtered them out because they weren't
///    `IsAbleToFight()`.
///  * `nearby_sleeping_enemies` — ordered unconscious, non-carried fighter
///    candidates. The final `KillNearbySleepingEnemies` fallback performs its
///    360°-range and LOS query lazily at the Original call site.
#[derive(Debug, Clone)]
pub struct SleepingEnemyInfo {
    pub handle: HumanHandle,
    pub position: Position,
    /// True if the target is a player character (as opposed to an
    /// enemy NPC in an opposing camp).
    pub is_pc: bool,
    /// True if this PC is Robin Hood (used for VIP rules).
    pub is_robin: bool,
    /// True if the target is a VIP (hero PC or VIP NPC).
    pub is_vip: bool,
}

/// Same-camp swordfighter considered by `ReconsiderSwordfight`.
///
/// This deliberately is not derived from [`crate::ai_enemy::FighterSnapshot`].
/// The Original rebuilds this particular list from the complete camp fighter
/// registry and gates it with `MaxNormDistance`, which uses the actors' 3D
/// world positions.  The general fighter snapshot is instead a map-space,
/// able-to-fight scan used by several other combat systems.
#[derive(Debug, Clone, Copy)]
pub struct ReconsiderSwordfightFriend {
    pub handle: HumanHandle,
    /// `(UWORD)MaxNormDistance(friend)` after the Original's isometric-Y
    /// stretch. The cast precedes the `< 500` radius comparison.
    pub max_norm_distance: u16,
    pub number_of_opponents: u16,
}

/// Complete fighter-registry entry for `ReconsiderSwordfightObservation`.
///
/// The Original admits fighters with `RHElement::GetPosition()`, while the
/// shared [`crate::ai_enemy::FighterSnapshot`] intentionally applies
/// `RHArtificialIntelligence::Position`'s door-side forecast. Keep this
/// call-site-specific raw position separate so other AI consumers retain the
/// shared door-resolved semantics.
#[derive(Debug, Clone, Copy)]
pub struct ReconsiderSwordfightObservationFighter {
    pub handle: HumanHandle,
    pub raw_world_position: crate::coordinates::WorldPoint3D,
    pub is_friendly: bool,
    pub is_able_to_fight: bool,
    pub is_soldier: bool,
    pub primary_target: HumanHandle,
    pub current_substate: u32,
}

/// Per-tick analysis data computed by the engine's detection loop.
/// Populated once per detection tick, consumed by battle_decisions
/// and swordfight tactics. Passed alongside AiContext.
#[derive(Debug, Clone)]
pub struct AiPerTickData {
    /// Shared immutable profile table used by combat evaluation.
    ///
    /// `None` is valid only for narrow non-combat dispatches and test
    /// fixtures. Combat code must call [`Self::required_profile_manager`]
    /// rather than manufacturing an empty profile table. Original:
    /// `RHProfileManager.h::GetHandToHandProfile` asserts that the requested
    /// profile exists; `RHSword` therefore always owns a real profile.
    pub profile_manager: Option<std::sync::Arc<crate::profiles::ProfileManager>>,
    /// Owner's literal `RHElement::GetPosition()` value for call sites that
    /// bypass `RHArtificialIntelligence::Position`. During a door pass this
    /// remains the interpolated body position rather than the committed
    /// destination-side forecast carried by [`AiContext::position`].
    pub owner_live_position: Option<Position>,
    pub patrol_chief_position: Position,
    pub patrol_chief_state: AiState,
    pub enemy_sq_distances: Vec<(HumanHandle, i32)>,
    pub min_sq_enemy_distance: i32,
    pub friends_lower_company: u16,
    pub soldiers_lower_pride: bool,
    pub friends_nearer_to_enemy: u16,
    /// Sum of battle points for our side — 100 + pride per soldier, 100
    /// per PC. Used by `MakeBattlePredecisions`.
    pub us_battle_points: u32,
    /// True if any friend (not self) in `list_us` has officer rank.
    pub has_officer_nearby: bool,
    /// True if any friend in `list_us` has RANK_SOLDIER.
    pub simple_soldiers_near: bool,
    pub primary_target_multiplicity: Vec<(HumanHandle, u32)>,
    /// Complete fighter-registry snapshot for direct pointer dereferences.
    ///
    /// Original AI lists (`mlistUs`, `mlistThem`, primary targets, etc.) hold
    /// pointers into the level-wide camp fighter arrays. Those lists can
    /// legitimately contain a fighter outside the 500-unit radius used by
    /// `FillListWithAllNearFighters`, especially after `BattleDecisions` uses
    /// the owner's larger 360-degree detection radius. Keep this separate
    /// from `nearby_fighters` so radius-based scans retain their exact domain.
    pub fighter_registry: Vec<crate::ai_enemy::FighterSnapshot>,
    pub nearby_fighters: Vec<crate::ai_enemy::FighterSnapshot>,
    /// Complete registry in Original order for the observation-only raw
    /// `GetPosition()` radius test. This must not replace `nearby_fighters`:
    /// generic combat scans use the door-resolved AI position instead.
    pub reconsider_swordfight_observation_fighters: Vec<ReconsiderSwordfightObservationFighter>,
    /// Complete opposing-camp fighter registry for `ReconsiderSwordfight`.
    /// Original applies no 500-unit prefilter here; each entry is admitted
    /// by `IsDetecting360Degrees`, whose radius depends on the observer.
    pub reconsider_swordfight_enemies: Vec<crate::ai_enemy::FighterSnapshot>,
    /// Complete same-camp, actively swordfighting registry scan for
    /// `ReconsiderSwordfight`. Unlike `nearby_fighters`, its radius uses 3D
    /// world positions and does not apply `IsAbleToFight`.
    pub reconsider_swordfight_friends: Vec<ReconsiderSwordfightFriend>,
    /// Same-camp soldiers snapshot for alert functions (`alert_officer`,
    /// `alert_soldiers`).  Populated every tick from the engine's soldier
    /// snapshot list, filtered to the evaluating NPC's camp.
    pub camp_soldiers: Vec<crate::ai_enemy::CampSoldierInfo>,
    /// Rank-soldier NPCs of every camp in registry order, the domain
    /// `CommandSoldiersToAttack` scans.
    pub alert_soldier_candidates: Vec<crate::ai_enemy::AlertSoldierCandidate>,
    /// Same-camp soldiers who are currently unconscious + alive.
    /// Populated alongside `camp_soldiers`, which skips unconscious
    /// entries; the money-fight scans walk the whole camp registry, so
    /// they merge this list back into `camp_soldiers` by handle to
    /// recover registry order.
    pub camp_unconscious_soldiers: Vec<crate::ai_enemy::CampUnconsciousSoldierInfo>,
    pub visible_seeking_friends: u16,
    pub friend_seek_clears_help_flag: bool,
    /// Pre-computed destination forecast for the primary target.
    /// Populated by the engine from the target entity's live state
    /// (door-pass, lift, building traversal). See [`forecast_destination_for_ia`].
    pub primary_target_forecast: Option<PreparedForecastDestination>,
    /// Pre-computed forecasts for the NPC's complete Enemy-detectable
    /// pointer order. `EVENT_OUTOFVIEW` is delivered for the detectable
    /// whose visibility edge fell, which need not be `primary_target`.
    pub enemy_detectable_forecasts: Vec<(HumanHandle, PreparedForecastDestination)>,
    /// Owner-boundary `Position(enemy)` values for detection stimuli. Rust
    /// batches movement globally, so the live entity map can still trail the
    /// position the Original has committed when this NPC handles EVENT_VIEW.
    pub enemy_detectable_positions: Vec<(HumanHandle, Position)>,
    /// Literal live `GetPosition()` values for Enemy detectables. Unlike
    /// `enemy_detectable_positions`, these bypass AI-position door forecasts
    /// and creation-slot boundary rewinding. Direct geometry helpers such as
    /// `EnemyIsBelowMe` read the element itself.
    pub enemy_detectable_live_world_positions: Vec<(HumanHandle, crate::coordinates::WorldPoint3D)>,
    /// True when the primary target is a player character.
    /// Used by lost-sight logic in `reconsider_swordfight` to decide
    /// whether to chase (PC) or pull a battle overview (NPC).
    pub primary_target_is_pc: bool,
    /// Pre-computed destination forecast for the missed PC (if any).
    /// Used by `get_battle_overview` to re-predict position before seeking.
    pub missed_pc_forecast: Option<PreparedForecastDestination>,
    /// Target identity paired with `missed_pc_forecast`. A queued Think can
    /// change the AI's `missed_pc` after this snapshot was prepared.
    pub missed_pc_forecast_handle: HumanHandle,
    /// True when `missed_pc` refers to a player character.
    pub missed_pc_is_pc: bool,
    /// Number of enemies this soldier personally detected (not shared by
    /// friends). Used for observe decisions where the count should
    /// reflect only what this NPC can see, not the merged `list_them`.
    pub personally_visible_enemies: u16,
    /// Enemies that showed up in detection this tick but were filtered
    /// out of `enemy_sq_distances` / `list_them` because they are
    /// unconscious (or otherwise unable to fight) and not being carried.
    /// Consumed by the "approach unconscious enemy" branch in
    /// `battle_decisions`.
    pub unconscious_enemies: Vec<SleepingEnemyInfo>,
    /// All unconscious, non-carried enemies within the NPC's 360°
    /// real-view radius (with LOS), regardless of whether they were
    /// in the detection list. Consumed by the final
    /// `KillNearbySleepingEnemies` fallback.
    pub nearby_sleeping_enemies: Vec<SleepingEnemyInfo>,
    /// Precomputed jump-line index for table swordfight with the primary
    /// target. `Some(line_idx)` when the NPC and primary target are in
    /// different sectors reachable via a jump-line pair. Used during
    /// `ReconsiderEnemyApproach`.
    pub primary_target_jump_line: Option<u32>,
    /// Position returned by `RHArtificialIntelligence::Position(actor)`.
    /// During a door pass this is the committed destination-side position,
    /// not the actor's interpolated body position.
    pub primary_target_position: Option<Position>,
    /// Handle for which the primary-target metadata in this snapshot was
    /// built. A synchronous AI callback can replace `base.primary_target`
    /// before a later handler consumes the same tick data; consumers must
    /// not pair that new handle with this old target's geometry.
    pub primary_target_snapshot_handle: HumanHandle,
    /// The target element's literal current position and sector. This differs
    /// from [`Self::primary_target_position`] while passing a door and is for
    /// source sites that call `GetPosition` / `GetSector` directly.
    pub primary_target_live_position: Option<Position>,
    /// Live posture of `primary_target` this tick.
    pub primary_target_posture: Option<crate::element::Posture>,
    /// Live animation (order type) of `primary_target` this tick.
    pub primary_target_animation: Option<crate::order::OrderType>,
    /// If `primary_target` is on another entity's shoulders
    /// (`RHPOSTURE_ON_SHOULDERS`), the live position of the carrier.
    /// The AI retargets to the carrier in that case.
    pub primary_target_carrier_position: Option<Position>,
    /// If `primary_target` is on another entity's shoulders
    /// (`RHPOSTURE_ON_SHOULDERS`), the carrier's handle. The AI re-points
    /// `primary_target` to this handle so all downstream
    /// position / friend-swap / focus / `BeginSwordfight` reads target
    /// the carrier rather than the carried entity.
    pub primary_target_carrier_handle: Option<HumanHandle>,
    /// Friend target-swap candidates: same-camp soldiers currently
    /// approaching their own primary target.
    pub friend_swap_candidates: Vec<FriendSwapCandidate>,

    /// Pre-computed fallback positions for the "avenger on the roof"
    /// branch, keyed by target handle. Populated by the engine when
    /// `couldnt_reachpoint` is set, for the current primary target and
    /// every personal enemy-list candidate a decision arm could re-pick,
    /// wherever [`crate::gate::compute_avenger_wait_position`] finds a
    /// blocking gate on the path from that target back to the
    /// evaluating NPC. Empty when the branch doesn't apply.
    pub avenger_on_roof_wait_positions: Vec<(HumanHandle, Position)>,

    /// Handles in `me`'s `DETECTABLE_ENEMY` list whose `seen_last_frame`
    /// flag is set. Used by `RefreshArrowProtection` so a shield bearer
    /// doesn't raise his shield against a bow-armed enemy who is occluded
    /// or has slipped out of his cone of vision this frame.
    pub seen_last_frame_enemies: Vec<HumanHandle>,

    /// Geometry of the door this NPC would walk *out* of when commanding
    /// soldiers from inside a building. Used by the `AlertSoldiers`
    /// indoor branch. `None` when the NPC is not inside a building or no
    /// exit door is reachable.
    pub my_exit_door: Option<MyExitDoorInfo>,

    /// Current detection snapshots for every member of this NPC's
    /// phalanx right-chain, including self. Consumed by
    /// `PhalanxReinitializeThemList` so every recursive step uses that
    /// member's own radius, viewer geometry, and live enemy inputs.
    /// The snapshots are pulled up-front to avoid mutating sibling AI
    /// brains mid-tick.
    pub phalanx_member_them_lists: Vec<PhalanxMemberThemList>,
}

/// One human target needed by a phalanx member's step-1 or step-2
/// detection pass. These are explicit live values rather than a bare
/// persistent handle so stale `list_them` entries still have to pass
/// the member's current LOS/radius test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhalanxEnemySnapshot {
    pub handle: HumanHandle,
    pub position: Position,
    pub direction: u16,
    pub posture: crate::element::Posture,
    pub elevation: f32,
    pub is_rider: bool,
    pub active: bool,
    pub able_to_fight: bool,
    pub dead: bool,
    pub unconscious: bool,
    pub friend: bool,
    pub in_building: bool,
    /// Projection obstacle the target stands on. `ComputeViewRadius` caches
    /// and slices its view sphere per target surface before the final LOS.
    pub obstacle: Option<crate::position_interface::ObstacleHandle>,
}

/// One phalanx member's live viewer state and enemy inputs. Equivalent
/// to recursing into
/// `right_combat_neighbour->PhalanxReinitializeThemList`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhalanxMemberThemList {
    /// Member's element handle (matches `FighterSnapshot::handle`).
    pub handle: HumanHandle,
    /// Concrete viewer identity used by the per-surface view-radius memo.
    pub entity: crate::element::EntityId,
    /// Persistent `mlistThem` entries evaluated by step 1.
    pub current_them_list: Vec<PhalanxEnemySnapshot>,
    /// Live `GetEnemy(i)` entries evaluated by step 2.
    pub detectable_enemies: Vec<PhalanxEnemySnapshot>,
    /// Member viewer state used by both detection variants.
    pub position: Position,
    pub direction: u16,
    pub posture: crate::element::Posture,
    pub elevation: f32,
    pub is_rider: bool,
    /// Live actor activity. Original still follows an inactive member's
    /// phalanx link, but both detection variants reject that member before
    /// doing geometry or LOS work.
    pub active: bool,
    pub in_building: bool,
    /// Live view-cone values consumed by `ComputeViewRadius`.
    pub view_radius: u16,
    pub view_direction: [f32; 2],
    pub real_half_aperture: f32,
    /// Square of this member's live `mViewParameters.uwRealRadius`.
    pub sq_view_radius: f32,
}

/// Snapshot of the door an NPC inside a building would use to step
/// outside. Populated by the engine each tick from the NPC's stored
/// door reference (or, lazily, the nearest building door when none is
/// set). Geometry-only — the door's runtime state (open/closed, lock
/// counter) doesn't affect formation placement.
#[derive(Debug, Clone, Copy)]
pub struct MyExitDoorInfo {
    /// Outside-edge anchor point.
    pub point_out: MapPoint,
    /// Door midpoint.
    pub point_mid: MapPoint,
    /// Outside-layer index.
    pub layer_out: u16,
    /// Outside-sector handle. Wrapped in `Option` because
    /// `SectorHandle::new(0)` returns `None` for the no-sector sentinel.
    pub sector_out: Option<crate::position_interface::SectorHandle>,
    /// Outside-edge as a full Position (for slot construction).
    pub position_out: Position,
}

/// Same-camp soldier that is currently approaching its primary target,
/// exposed to `ReconsiderEnemyApproach` for the target-swap heuristic.
#[derive(Debug, Clone, Copy)]
pub struct FriendSwapCandidate {
    pub friend_id: EntityId,
    pub friend_position: Position,
    pub friend_primary_target: HumanHandle,
    pub friend_primary_target_position: Position,
}

impl AiPerTickData {
    /// Look up the precomputed avenger-on-roof wait position for a
    /// specific target handle. Decision arms re-pick their target
    /// mid-tick, so each caller resolves its own live handle here
    /// instead of reusing a single snapshot-target position.
    pub fn avenger_wait_position_for(&self, target: HumanHandle) -> Option<Position> {
        self.avenger_on_roof_wait_positions
            .iter()
            .find(|(handle, _)| *handle == target)
            .map(|&(_, pos)| pos)
    }

    pub fn enemy_detectable_position(&self, target: HumanHandle) -> Option<Position> {
        self.enemy_detectable_positions
            .iter()
            .find(|(handle, _)| *handle == target)
            .map(|&(_, position)| position)
    }

    pub fn enemy_detectable_live_world_position(
        &self,
        target: HumanHandle,
    ) -> Option<crate::coordinates::WorldPoint3D> {
        self.enemy_detectable_live_world_positions
            .iter()
            .find(|(handle, _)| *handle == target)
            .map(|&(_, position)| position)
    }

    /// Return the profile table required by swordfight evaluation.
    pub fn required_profile_manager(&self) -> &crate::profiles::ProfileManager {
        self.profile_manager.as_deref().expect(
            "combat AI requires the level profile manager; construct tick data from the AI world view",
        )
    }

    /// Construct an empty/stub `AiPerTickData` with all fields zeroed
    /// or empty. **Use sparingly** — every call site is shipping a
    /// stripped-down snapshot to whatever AI dispatch follows, and any
    /// AI logic that needs the missing fields will silently see empty
    /// data instead of the real engine state. The user-visible bug
    /// class this caused: `battle_decisions` reads `enemy_sq_distances`
    /// and falls back to `return_to_duty` when the list is empty even
    /// if the soldier has a valid `primary_target` — soldier wedges in
    /// a Reactiontime/Default ping-pong because the timer-dispatch
    /// path passes `stub()` instead of the rich per-NPC tick data
    /// that the detection-dispatch path builds.
    ///
    /// This used to be the `Default` trait impl, but `Default` was
    /// removed so call sites can no longer accidentally pull in
    /// stripped data via the `..Default::default()` shorthand without
    /// noticing. Renaming to `stub` and requiring an explicit call
    /// makes the loss-of-fidelity visible at every dispatch site.
    ///
    /// Most engine-side dispatch paths now use the centralized
    /// `EngineInner::build_npc_tick_data(sim, npc_id)` builder.  Remaining
    /// direct stubs should stay limited to call sites that provably
    /// dispatch non-combat AI paths (init before target selection,
    /// friendly panic, or non-soldier entities); otherwise add a
    /// builder call instead of silently feeding empty combat context.
    pub fn stub() -> Self {
        Self {
            profile_manager: None,
            owner_live_position: None,
            patrol_chief_position: Position::default(),
            patrol_chief_state: AiState::Default,
            enemy_sq_distances: Vec::new(),
            min_sq_enemy_distance: i32::MAX,
            friends_lower_company: 0,
            soldiers_lower_pride: false,
            friends_nearer_to_enemy: 0,
            us_battle_points: 0,
            has_officer_nearby: false,
            simple_soldiers_near: false,
            primary_target_multiplicity: Vec::new(),
            fighter_registry: Vec::new(),
            nearby_fighters: Vec::new(),
            reconsider_swordfight_observation_fighters: Vec::new(),
            reconsider_swordfight_enemies: Vec::new(),
            reconsider_swordfight_friends: Vec::new(),
            camp_soldiers: Vec::new(),
            alert_soldier_candidates: Vec::new(),
            camp_unconscious_soldiers: Vec::new(),
            visible_seeking_friends: 0,
            friend_seek_clears_help_flag: false,
            primary_target_forecast: None,
            enemy_detectable_forecasts: Vec::new(),
            enemy_detectable_positions: Vec::new(),
            enemy_detectable_live_world_positions: Vec::new(),
            primary_target_is_pc: false,
            missed_pc_forecast: None,
            missed_pc_forecast_handle: 0,
            missed_pc_is_pc: false,
            personally_visible_enemies: 0,
            unconscious_enemies: Vec::new(),
            nearby_sleeping_enemies: Vec::new(),
            primary_target_jump_line: None,
            primary_target_position: None,
            primary_target_snapshot_handle: 0,
            primary_target_live_position: None,
            primary_target_posture: None,
            primary_target_animation: None,
            primary_target_carrier_position: None,
            primary_target_carrier_handle: None,
            friend_swap_candidates: Vec::new(),
            avenger_on_roof_wait_positions: Vec::new(),
            seen_last_frame_enemies: Vec::new(),
            my_exit_door: None,
            phalanx_member_them_lists: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ReinforcementDoorInfo — cached door data for MerryManForestCassos
// ---------------------------------------------------------------------------

/// Cached info for a reinforcement door, used by `MerryManForestCassos`
/// to find the nearest map exit and animate running to its PointOut.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct ReinforcementDoorInfo {
    /// Inner position of the door (where the NPC walks *to*).
    pub position_in: Position,
    /// Index into the canonical interactable door array.
    pub door_index: crate::gate::DoorIndex,
    /// Outer point of the door (where the NPC exits the map).
    pub point_out: MapPoint,
    /// Mid-point of the door interior. Used by `AlertSoldiers` to
    /// compute the door-out vector for the indoor officer formation
    /// sweep.
    pub point_mid: MapPoint,
    /// Layer index of the outer (outside) end of the door. Used by
    /// indoor formation paths to place gather slots on the outside
    /// layer.
    pub layer_out: u16,
    /// Sector handle of the outer (outside) end of the door.
    pub sector_out: Option<crate::position_interface::SectorHandle>,
    /// Inner door point as raw coordinates. `position_in` already
    /// carries this with layer/sector tagging, but the raw f32 pair is
    /// convenient for the door-vector math.
    pub point_in: MapPoint,
}

// ---------------------------------------------------------------------------
// Global AI state
// ---------------------------------------------------------------------------

/// A building interior known to the AI.
///
/// Populated during `InitAI()` by collecting every sector whose
/// `IsBuilding()` is true. Houses carry their occupant list so AI
/// code can ask "who's inside?" without scanning all entities, and
/// their door indices so pursuers / investigators can pick the right
/// gate to enter / exit through.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct House {
    /// Sector index (into `FastFindGrid::sectors`) of the building's
    /// interior motion area.
    pub sector_index: u32,
    /// Building index (into canonical `BuildingState`) if this sector is
    /// linked to one. The same index addresses the tenant list. `None` when the
    /// sector isn't proto-linked to a building (e.g. script-synthesised
    /// portals).
    pub building_index: Option<crate::sector::BuildingIdx>,
    /// Doors that connect this building to the outside.  Indices into
    /// the canonical interactable door table.
    pub door_indices: Vec<u32>,
    /// Entities currently inside the building.  Kept live by the
    /// `PassDoor` Enter / Leave hooks in `engine::door_pass`.
    pub occupant_ids: Vec<crate::element::EntityId>,
    /// Whether this building carries an arrow reserve. Populated from
    /// the GUYS/CAVE tenant chunk.
    pub arrow_reserve: bool,
}

impl House {
    /// Number of actors currently inside the building.
    #[inline]
    pub fn occupant_count(&self) -> usize {
        self.occupant_ids.len()
    }

    /// Match `RHSectorBuilding::IsAuthorized()`.
    ///
    /// The original proto constructor initializes
    /// `muwMaxNumberOfOccupants` to `0xFFFF`, and the proto loader does not
    /// overwrite it. The occupant count is nevertheless tested live on each
    /// authorization call.
    #[inline]
    pub fn is_authorized(&self) -> bool {
        self.occupant_count() < usize::from(u16::MAX)
    }

    /// Whether the given entity is currently an occupant.
    #[inline]
    pub fn contains_occupant(&self, eid: crate::element::EntityId) -> bool {
        self.occupant_ids.contains(&eid)
    }
}

// ─── On the actor-handle vs EntityId dual ─────────────────────────
//
// Building occupancy is tracked in two parallel data structures:
//
//   * `ai::House::occupant_ids: Vec<EntityId>` — the AI-facing view.
//     Populated at `EngineInner::initialize_buildings` and maintained
//     live by the `execute_pass_door` Enter / Leave hooks.  New AI
//     code should query this.
//
//   * `ScriptDomains::buildings` — the script-facing view, indexed by
//     `building_index` with actor
//     script handles. Kept in sync by the same hooks so script
//     natives (`GetNumberOfOccupants`, `GetOccupant`, etc.) see the
//     same occupancy that AI code does.
//
// Both are kept consistent; the dual exists because script identity
// (`i32` handle) and AI identity (`EntityId`) co-exist across the
// codebase and neither can be dropped independently.  Long-term
// consolidation would either migrate script natives to `EntityId`
// or delete `building_occupants` once all natives query via a
// a canonical `occupants_of(building_index) -> &[i32]` helper that
// derives from the House list on demand.
//

/// A rally point positioned just outside a building door.
///
/// Where NPCs regroup after exiting a building before resuming patrol.
/// Built in `InitAI()` at a fixed `AI_DOOR_RALLY_POINT_DISTANCE` from
/// each building door's `PointOut`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct DoorRallyPoint {
    /// World position (outside the door).
    pub position: Position,
    /// Door index in the canonical interactable door table.
    pub door_index: crate::gate::DoorIndex,
    /// Radius around `position` within which NPCs are "at" the rally
    /// point.
    pub radius: f32,
}

/// Distance offset (from `PointOut`) at which door rally points are
/// anchored.
pub const AI_DOOR_RALLY_POINT_DISTANCE: f32 = 100.0;

/// Global / shared AI state, conceptually module-static.
#[derive(Debug, Clone, Serialize, Deserialize, robin_state_hash_derive::StateHash)]
pub struct AiGlobalState {
    pub green_alert_soldiers: u16,
    pub yellow_alert_soldiers: u16,
    pub red_alert_soldiers: u16,

    pub there_are_royalist_soldiers: bool,
    pub there_are_lacklandist_soldiers: bool,

    pub stupid_soldiers_cheat: bool,
    pub freeze: bool,

    pub overall_alert_status: AlertLevel,
    pub overall_villain_alert_status: AlertLevel,

    /// Ambush points in the current mission.
    pub ambush_points: Vec<AmbushPoint>,
    /// Seek points shared between all NPCs.
    pub seek_points: Vec<SeekPoint>,
    /// Archery sectors in the current mission.
    pub archery_sectors: Vec<SectorArchery>,

    /// Saved random seed value for deterministic replay.
    pub saved_random_seed: i64,

    /// Per-remark forbidden-until-frame table.
    pub remarks_forbidden_till_frame: Vec<u32>,
    /// Active forbidden remarks.
    pub forbidden_remarks: Vec<ForbiddenRemark>,
    /// Screen remarks to display.
    pub screen_remarks: Vec<ScreenRemark>,

    // Display toggles (not serialized, debug only)
    pub attribute_display: bool,
    pub speech_display: bool,
    pub golden_eye_mode: bool,
    /// `DIES IRAE` cheat — "thunder of God" toggle. While active the
    /// vengeance path (Ezekiel 25:17 reference) is applied, killing
    /// targets chosen by the selected-view-element overlay. Consumers
    /// that still need porting should read this flag on
    /// `AiGlobalState`.
    pub ezekiel_2517: bool,

    pub current_speech_variant: u16,

    /// Repulsive points: NPCs avoid these areas during pathfinding.
    /// Scripts add/remove them by integer ID.
    pub repulsive_points: Vec<RepulsivePoint>,

    /// Next auto-incrementing ID for repulsive points.
    pub next_repulsive_point_id: i32,

    /// Cached door geometry for `FindDoorEnemyCouldBeBehind`.
    /// Populated at level load and kept in the save snapshot so any
    /// door-state-dependent authorization cache survives restore.
    pub door_seek_infos: Vec<DoorSeekInfo>,

    /// Reinforcement doors: (inner position, door index, PointOut).
    /// Used by `MerryManForestCassos` to find the nearest map exit
    /// and animate the NPC running to the door's PointOut.
    /// Populated at level load.
    pub reinforcement_doors: Vec<ReinforcementDoorInfo>,

    /// Buildings the AI knows about — populated at `InitAI()` from
    /// every sector whose `sector_type.is_building()` is true, with
    /// each house's occupant list and doors filled in.
    pub houses: Vec<House>,

    /// Rally points anchored just outside each building door, created in
    /// `InitAI()` per house gate.
    pub door_rally_points: Vec<DoorRallyPoint>,

    /// Soldier load-order index → entity-handle (slot) mapping. Scripts
    /// and waypoint commands address NPCs by their soldier register
    /// index (the position in the all-soldiers list at level load), not
    /// by their entity slot. Cloned out of
    /// `LevelAssets::all_soldier_entity_ids` once at level load so the
    /// AI tick can resolve a friend ID without re-borrowing the engine.
    pub all_soldier_handles: std::sync::Arc<Vec<u32>>,

    /// Same-frame combat claims made by soldiers during the current AI
    /// dispatch. Some engine side effects are batched, so this transient
    /// list carries the live claim until the normal entity state catches
    /// up — letting later soldiers in the same frame see earlier
    /// `AttackEnemy` decisions.
    pub same_frame_target_claims: Vec<(HumanHandle, HumanHandle)>,

    /// Owner-ordered mirror of `RHElementActorHuman::muwPrimaryTargetMultiplicity`.
    /// Original AI routines reset and increment these UWORD counters directly
    /// on target humans, so later owners in the same actor pass observe the
    /// exact serial mutation history. Original explicitly does not serialize
    /// this scratch field; a loaded session bootstraps from its live actors,
    /// then preserves owner-ordered mutations.
    #[serde(skip)]
    pub primary_target_multiplicity_scratch: std::collections::BTreeMap<HumanHandle, u32>,
    #[serde(skip)]
    pub primary_target_multiplicity_initialized: bool,
}

impl Default for AiGlobalState {
    fn default() -> Self {
        Self {
            green_alert_soldiers: 0,
            yellow_alert_soldiers: 0,
            red_alert_soldiers: 0,
            there_are_royalist_soldiers: false,
            there_are_lacklandist_soldiers: false,
            stupid_soldiers_cheat: false,
            freeze: false,
            overall_alert_status: AlertLevel::Green,
            overall_villain_alert_status: AlertLevel::Green,
            ambush_points: Vec::new(),
            seek_points: Vec::new(),
            archery_sectors: Vec::new(),
            saved_random_seed: 0,
            remarks_forbidden_till_frame: Vec::new(),
            forbidden_remarks: Vec::new(),
            screen_remarks: Vec::new(),
            attribute_display: false,
            speech_display: false,
            golden_eye_mode: false,
            ezekiel_2517: false,
            current_speech_variant: 0,
            repulsive_points: Vec::new(),
            next_repulsive_point_id: 1,
            door_seek_infos: Vec::new(),
            reinforcement_doors: Vec::new(),
            houses: Vec::new(),
            door_rally_points: Vec::new(),
            all_soldier_handles: std::sync::Arc::new(Vec::new()),
            same_frame_target_claims: Vec::new(),
            primary_target_multiplicity_scratch: std::collections::BTreeMap::new(),
            primary_target_multiplicity_initialized: false,
        }
    }
}

impl AiGlobalState {
    pub fn npcs_can_be_enemies(&self) -> bool {
        self.there_are_royalist_soldiers && self.there_are_lacklandist_soldiers
    }

    pub fn overall_villain_alert(&self) -> AlertLevel {
        if self.red_alert_soldiers > 0 {
            AlertLevel::Red
        } else if self.yellow_alert_soldiers > 0 {
            AlertLevel::Yellow
        } else {
            AlertLevel::Green
        }
    }

    pub fn reset_seek_points(&mut self) {
        self.seek_points.clear();
    }

    pub fn reset_ambush_points(&mut self) {
        self.ambush_points.clear();
    }

    pub fn reset_archery_sectors(&mut self) {
        self.archery_sectors.clear();
    }

    pub fn init_green_yellow_red_alert_soldiers(&mut self) {
        self.green_alert_soldiers = 0;
        self.yellow_alert_soldiers = 0;
        self.red_alert_soldiers = 0;
    }

    /// Add a seek-point direction, either merging it into an existing
    /// nearby seek point or creating a new one.
    pub fn add_seek_point_direction(&mut self, dir: &SeekPointDirection) {
        // Check all existing seek points in reverse order
        for sp in self.seek_points.iter_mut().rev() {
            if sp.add_if_near(dir) {
                return;
            }
        }
        // No nearby point found — create a new one
        let mut new_sp = SeekPoint::from_direction(dir);
        new_sp.id = self.seek_points.len() as u16;
        self.seek_points.push(new_sp);
    }

    /// Snap `pos` onto a nearby seek point, chosen at random from those
    /// within `MaxNorm(me_pos - pos) * distance_factor + abs_distance`
    /// (with a +100 penalty for level changes). Returns `true` if a
    /// candidate was found and `pos` was overwritten.
    pub fn set_pos_on_near_seek_point(
        &self,
        sim: &crate::sim_rng::SimulationContext,
        me_pos: Position,
        pos: &mut Position,
        distance_factor: f32,
        abs_distance: u16,
    ) -> bool {
        let base_dx = (me_pos.x - pos.x).abs();
        let base_dy = (me_pos.y - pos.y).abs();
        // Original narrows the computed FLOAT limit to UWORD before it
        // examines candidates. This truncation is observable for ordinary
        // fractional actor positions, not merely at overflow boundaries.
        let distance_limit = (base_dx.max(base_dy) * distance_factor + abs_distance as f32) as u16;

        let mut candidates: Vec<usize> = Vec::new();
        for (idx, sp) in self.seek_points.iter().enumerate() {
            let dx = (sp.position.x - pos.x).abs();
            let dy = (sp.position.y - pos.y).abs();
            let mut distance = dx.max(dy) as u16;
            if sp.position.level != pos.level {
                distance = distance.wrapping_add(100);
            }
            if distance < distance_limit {
                candidates.push(idx);
            }
        }

        if candidates.is_empty() {
            return false;
        }
        let pick = crate::sim_rng::usize(
            sim,
            crate::sim_rng::RngSite::NearSeekPoint,
            0..candidates.len(),
        );
        *pos = self.seek_points[candidates[pick]].position;
        true
    }

    /// Post-process seek points near building doors: teleport them inside.
    pub fn teleport_seek_points_inside_doors(&mut self) {
        for sp in &mut self.seek_points {
            for door_info in &self.door_seek_infos {
                if door_info.door_type == crate::gate::DoorType::Building {
                    let dx = sp.position.x - door_info.point_out.x;
                    let dy = sp.position.y - door_info.point_out.y;
                    let max_norm = dx.abs().max(dy.abs());
                    if max_norm <= 5.0 {
                        sp.position = door_info.position_in;
                        // First matching door wins: the point has moved
                        // inside, so later doors must not re-teleport it.
                        break;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
