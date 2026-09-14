use super::*;

pub(crate) fn ai_position_to_point_3d(
    fast_grid: &crate::fast_find_grid::FastFindGrid,
    obstacles: crate::sight_obstacle::ObstacleList<'_>,
    position: Position,
) -> crate::coordinates::WorldPoint3D {
    let z = match position.sector {
        None => 0.0,
        Some(handle) => {
            let sector_number = crate::sector::SectorNumber::new(handle.get() as i16);
            // Navigation requires an exact sector reference here. Schema-16 NPC
            // session-boundary transients predate exact arena identities,
            // though, and can restore number-only seek positions. Recover
            // that omitted pointer from the position, layer, and authored
            // polygons rather than consulting the lossy public-number map:
            // shipped levels may contain duplicate public sector numbers.
            let grid_idx = handle.arena_index().map(usize::from).or_else(|| {
                    let point = MapPoint::new(position.x, position.y);
                    let candidates = fast_grid
                        .level
                        .sectors
                        .iter()
                        .enumerate()
                        .filter(|(_, sector)| sector.sector_number == sector_number)
                        .collect::<Vec<_>>();
                    let matches = candidates
                        .iter()
                        .copied()
                        .filter(|(_, sector)| {
                            sector.layer == position.level && sector.contains_point(point)
                        })
                        .collect::<Vec<_>>();
                    match matches.as_slice() {
                        [(index, _)] => Some(*index),
                        [] => {
                            let same_layer = candidates
                                .iter()
                                .copied()
                                .filter(|(_, sector)| sector.layer == position.level)
                                .collect::<Vec<_>>();
                            match (same_layer.as_slice(), candidates.as_slice()) {
                                ([(index, _)], _) | (_, [(index, _)]) => Some(*index),
                                ([], []) => None,
                                _ => panic!(
                                    "AI position sector {sector_number} layer {} at ({}, {}) is ambiguous in the exact arena",
                                    position.level, position.x, position.y
                                ),
                            }
                        }
                        _ => panic!(
                            "AI position sector {sector_number} layer {} at ({}, {}) has multiple containing exact sectors",
                            position.level, position.x, position.y
                        ),
                    }
                });
            let grid_sector = grid_idx.and_then(|idx| fast_grid.level.sectors.get(idx));
            let is_motion = grid_sector.is_some_and(|sector| sector.sector_type.is_motion());
            if grid_idx.is_some() && !is_motion {
                panic!(
                    "position_to_point_3d: sector {} is not a motion sector",
                    handle.get()
                );
            }

            // Building motion sectors have no projection area of their
            // own. Position conversion walks the sector's gates
            // in order, finds the first door whose inside point is within
            // maximum norm < 20, and samples the outside sector at the exit point.
            let (projection_sector, projection_layer, point) =
                if grid_sector.is_some_and(|sector| sector.sector_type.is_building()) {
                    let sector = grid_sector.expect("building sector disappeared");
                    let door = sector
                    .gate_indices
                    .iter()
                    .filter_map(|index| {
                        fast_grid
                            .level
                            .door_projection_infos
                            .get(usize::from(*index))
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
                            handle.get(),
                            position.x,
                            position.y
                        )
                    });
                    (
                        crate::position_interface::SectorHandle::from_number(door.sector_out)
                            .with_arena_index(door.sector_out_index.unwrap_or_else(|| {
                                panic!("building exit door has no outside sector arena identity")
                            })),
                        door.layer_out,
                        door.point_out,
                    )
                } else {
                    (
                        grid_idx.map_or(handle, |index| {
                            handle.with_arena_index(
                                crate::fast_find_grid::SectorIndex::new(index as u32)
                                    .expect("AI projection sector index exceeds the arena range"),
                            )
                        }),
                        position.level,
                        MapPoint::new(position.x, position.y),
                    )
                };
            let projection_layer = crate::position_interface::Layer::new(projection_layer)
                .expect("AI projection lookup cannot use absent layer");
            let projection = crate::sight_obstacle::ProjectionAreaRef {
                layer: projection_layer,
                sector: projection_sector
                    .arena_index()
                    .unwrap_or_else(|| panic!("AI projection lookup lacks exact sector identity")),
            };
            let mut best: Option<(f32, f32)> = None;
            for (_, obstacle) in obstacles.iter_indexed() {
                if obstacle.projection_area_ref() != Some(projection)
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

/// Resolve the entry point used when enemy approach reconsideration's final target
/// position belongs to a lift.
///
/// Original-game lift-sector initialization identifies the high and
/// low doors by minimum/maximum exit-point Y, irrespective of their authored
/// door-type tags. Enemy approach reconsideration then
/// uses the high entry only when its outside layer equals the attacker's
/// current layer; every other layer uses the low entry
/// during enemy-approach reconsideration.
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
        let target_has_exact_sector = matches!(
            target_sector.reference(),
            crate::position_interface::SectorReference::Exact { .. }
        );
        // The original game follows the target's exact sector reference. Shipped levels can
        // contain duplicate public sector numbers, so consult the retained
        // arena identity first and use the legacy number map only for
        // explicitly number-only synthetic/compatibility positions.
        let grid_index = match target_sector.reference() {
            crate::position_interface::SectorReference::NumberOnly(_) => *fast_grid
                .level
                .sector_number_map
                .get(&sector_number)
                .unwrap_or_else(|| {
                    panic!("primary target sector {sector_number} is absent from the grid")
                }),
            crate::position_interface::SectorReference::Exact { index, .. } => usize::from(index),
        };
        let sector = fast_grid.level.sectors.get(grid_index).unwrap_or_else(|| {
            panic!("primary target sector {sector_number} maps to missing grid index {grid_index}")
        });
        assert_eq!(
            sector.sector_number, sector_number,
            "primary target exact arena index {grid_index} conflicts with public sector {sector_number}"
        );
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
                .get(usize::from(*index))
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
        let mut selected_sector =
            crate::position_interface::SectorHandle::new(u16::from(selected.sector_out));
        match selected.sector_out_index {
            Some(index) => {
                selected_sector = selected_sector.map(|sector| sector.with_arena_index(index));
            }
            None if target_has_exact_sector => {
                panic!(
                    "exact lift sector {sector_number} selected an endpoint without an exact outside sector"
                );
            }
            None => {}
        }
        Some(Some(Position {
            x: selected.point_out.x,
            y: selected.point_out.y,
            sector: selected_sector,
            level: selected.layer_out,
        }))
    }
}

// AiContext — per-frame entity state passed into think()
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

/// Per-frame entity state passed into `think()` by the engine.
/// Replaces the stale-prone `cached_*` fields on `AiController` for data that
/// changes every frame (position, direction, posture, etc.).
#[derive(Debug, Clone, Default)]
pub struct AiContext {
    pub difficulty: crate::player_profile::DifficultyLevel,
    /// Original-game creation order for the evaluating actor.
    /// This is distinct from the Rust entity-table slot and is required by
    /// actor-specific remark suppression identity checks.
    pub original_creation_order: Option<u32>,
    pub position: Position,
    /// Live element layer for the evaluating actor. This can
    /// differ from [`Self::position`]'s level while `Position(actor)` snaps a
    /// door-passing actor to the committed gate side. The original game's nearby movement uses
    /// the snapped point for its distance but the live actor layer for its
    /// same-layer gate.
    pub self_layer: u16,
    /// Live raw body position for the evaluating actor.
    /// Unlike [`Self::position`], this never snaps a door-passing actor to a
    /// gate endpoint. Direct element-distance tests must use this point.
    pub self_body_position_world: crate::coordinates::WorldPoint3D,
    pub frame: u32,
    /// Depth of the current engine decision call, supplied at the call boundary.
    pub think_depth: u8,
    pub direction: u16,
    pub posture: crate::element::Posture,
    /// Live eye point and view-cone parameters after view refresh.
    /// Used by synchronous human-detection checks inside AI state
    /// handlers, as distinct from the periodic detectable-list pass.
    pub self_eye_position: MapPoint,
    pub self_eye_z: f32,
    /// Direct upright eye point. Unlike `position`, this
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
    /// human-detection calls to run the authoritative light-sector
    /// modulation in view-radius calculation.
    pub is_night_or_fog: bool,
    /// The busy check's sequence-element branch: `true` when the actor's
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
    /// element for this NPC. Swordfight reconsideration bails out early when
    /// an enter-swordfight sequence is already queued.
    pub enter_swordfight_pending: bool,
    /// True when the current level is Sherwood Forest. Used by
    /// `is_merry_man_forest()` and the 180° detection cone for Royalist
    /// NPCs.
    pub is_forest_level: bool,
    /// The evaluating entity's zero-centred collision bounding box.
    pub move_box: crate::coordinates::MoveBox,
    /// NPC's remaining arrow count. Used
    /// by archer decision logic.
    pub remaining_arrows: u16,
    /// Square of the engine's standard view-polygon radius. Used to gate
    /// cover-position acceptance for archers behind shield bearers (the
    /// cover point must be within view radius of the primary target).
    pub sq_standard_view_radius: f32,
    /// Square of this NPC's live view radius.
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
    /// states / etc.). Used by enemy-sighting processing to branch on
    /// the fast-movement action state (sprint-into-engage path). Defaults to
    /// `Waiting` for unit tests built off `AiContext::test_fixture()`.
    pub self_action_state: crate::element::ActionState,
    /// Self's soldier rank if soldier; `ProfileRank::None` otherwise.
    /// Used by boredom timing to pick officer-length intervals.
    pub self_rank: crate::profiles::ProfileRank,
    /// Self's soldier pride. `0` for non-soldiers or soldiers with no
    /// pride. Used by boredom timing to pick the long "pride" bored
    /// interval.
    pub self_pride: u16,

    /// Self's current life points, read live from the element rather than
    /// from an AI-side cache. Battle predecisions scale the battle
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
    /// The original game checks this flag independently of the AI state:
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
    /// in the detectable list's identity order. The original game
    /// Enemy-list rebuilding walks this live list at the exact AI decision
    /// boundary; geometric visibility products in [`AiPerTickData`] are not
    /// interchangeable with it.
    pub self_seen_enemy_handles: Vec<HumanHandle>,

    /// Live animation (`OrderType`) currently playing on this NPC. Read
    /// by AI gates that inspect the actor's current animation directly,
    /// e.g. default bored-state processing skips its head-turn
    /// transition while the `WAITING_UPRIGHT_BORED_RANDOM` idle is
    /// already playing.
    pub self_animation: crate::order::OrderType,

    /// Live actor motion lifecycle for `self_animation`. A newly installed
    /// move-to-wait transition is still a real original-game movement input;
    /// only a transition that has already advanced can represent Rust's
    /// one-owner-boundary lag behind Original's idle successor.
    pub self_animation_motion_state: crate::sprite::MotionState,

    /// The sequence-manager element currently selected by the actor is its
    /// original game's default Wait element.
    /// Actor stopping deliberately skips that exact element, so a
    /// deferred `Halt` must not be projected as clearing its live animation.
    /// Refreshed from the sequence manager at every filtered Think boundary.
    pub self_selected_element_is_default_wait: Option<bool>,

    /// Priority of the sequence-manager element currently selected by the
    /// actor. The outer `Option` distinguishes a context that has not been
    /// refreshed at the live owner boundary; the inner `Option` represents
    /// an actor with no selected sequence element. Halting uses
    /// `Stop(PREFERENCE)`, so deferred Halt projection must consult this
    /// priority before pretending that the selected animation was stopped.
    pub self_selected_element_priority: Option<Option<crate::sequence::SequencePriority>>,

    /// `true` when the sprite backing `self_animation` has reached or passed
    /// its authored action-done frame/counter. The Original actor hourglass
    /// retires a completed move-to-wait transition before later NPC timer
    /// callbacks inspect the animation; Rust's split phase keeps the order
    /// installed until the sequence drain, so movement requests use this bit to project
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
    /// FastFindGrid snapshot used for reachability-based line-of-sight queries
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
    pub fn player_relationship(&self) -> crate::diplomacy::Relationship {
        self.entity_views
            .diplomacy
            .relationship_to_player(self.camp)
    }

    pub fn is_player_aligned(&self) -> bool {
        self.entity_views.diplomacy.is_player_aligned(self.camp)
    }

    pub fn is_hostile_to_player(&self) -> bool {
        self.player_relationship() == crate::diplomacy::Relationship::Hostile
    }

    pub fn is_allied_with(&self, camp: crate::element::Camp) -> bool {
        self.entity_views.diplomacy.is_allied(self.camp, camp)
    }

    pub fn is_hostile_with(&self, camp: crate::element::Camp) -> bool {
        self.entity_views.diplomacy.is_hostile(self.camp, camp)
    }

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
            {
                values.insert(surface, (entry.viewer, entry.radius));
            }
        };
        seed(None, cache.ground);
        for (index, entry) in cache.obstacles.iter().enumerate() {
            let Some(handle) = u32::try_from(index)
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
            && radius != 0.0
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
        // setter stored a computed zero. Keep that last writer to invalidate
        // a previous viewer's radius and publish it through clones/commits.
        self.view_radius_cache
            .borrow_mut()
            .insert(surface, (viewer, radius));
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

    /// Resolve spatial state without confusing entity existence with admission
    /// to the AI snapshot. No observation does not imply entity removal.
    pub fn entity_observation(
        &self,
        handle: impl IntoOptionalAiHandle,
    ) -> Result<&crate::ai_entity_view::AiEntityView, crate::ai_entity_view::AiObservationUnavailable>
    {
        use crate::ai_entity_view::AiObservationUnavailable;
        let handle = handle
            .into_optional_ai_handle()
            .ok_or(AiObservationUnavailable::NoHandle)?
            .get();
        self.entity_views.get(&handle).ok_or_else(|| {
            self.entity_views
                .unavailable_entities
                .get(&handle)
                .copied()
                .unwrap_or(AiObservationUnavailable::EntityAbsent)
        })
    }

    /// Convenience lookup for callers whose policy intentionally treats all
    /// unavailable observations alike. Use `entity_observation` when the reason
    /// matters, including diagnostics about removed entities.
    pub fn entity_view(
        &self,
        handle: impl IntoOptionalAiHandle,
    ) -> Option<&crate::ai_entity_view::AiEntityView> {
        let handle = handle.into_optional_ai_handle()?.get();
        self.entity_views.get(&handle)
    }

    /// [`Self::entity_view`] for callers where a *null* handle is a legal,
    /// silent "nobody", but a non-null handle whose view is unavailable is an
    /// anomaly the caller tolerates by taking its documented fallback branch.
    /// That fallback stays unchanged; this only makes it visible in the log
    /// (with the reason and the calling site).
    #[track_caller]
    pub fn entity_view_logged(
        &self,
        handle: impl IntoOptionalAiHandle + Copy,
        what: &'static str,
    ) -> Option<&crate::ai_entity_view::AiEntityView> {
        let raw = handle.into_optional_ai_handle()?.get();
        match self.entity_observation(handle) {
            Ok(view) => Some(view),
            Err(reason) => {
                tracing::warn!(
                    handle = raw,
                    ?reason,
                    what,
                    caller = %std::panic::Location::caller(),
                    "AI entity view unavailable; taking the caller's absent-entity fallback"
                );
                None
            }
        }
    }

    /// Look up a handle that the calling logic has already established as a
    /// live participant (an active brawl partner, a primary target mid-
    /// engagement, a loot-list entry, …).  Such a handle failing to resolve
    /// means the snapshot lost a required entity — corrupted sim state or a
    /// port bug — so this panics instead of letting the caller silently take
    /// a default gameplay branch. Callers must still guard `None` themselves
    /// where an absent handle is legal; raw slot zero is a valid entity handle.
    #[track_caller]
    pub fn expect_entity_view(
        &self,
        handle: impl IntoOptionalAiHandle + Copy,
        ctx: &str,
    ) -> &crate::ai_entity_view::AiEntityView {
        let raw = handle.into_optional_ai_handle().map(AiEntityHandle::get);
        self.entity_observation(handle)
            .unwrap_or_else(|reason| match raw {
                Some(raw) => {
                    panic!("required entity view for handle {raw} missing ({ctx}): {reason:?}")
                }
                None => {
                    panic!("required entity view for absent handle missing ({ctx}): {reason:?}")
                }
            })
    }

    /// Resolve a raw legacy human/object handle through the live entity-view
    /// snapshot without guessing its typed [`crate::element::EntityId`] kind.
    pub fn entity_id(
        &self,
        handle: impl IntoOptionalAiHandle + Copy,
    ) -> Option<crate::element::EntityId> {
        let raw = handle.into_optional_ai_handle()?.get();
        self.entity_view(handle)?.entity_id(raw)
    }

    /// Convenience wrapper around [`Self::entity_view`] that returns
    /// just the position.
    pub fn entity_position(&self, handle: impl IntoOptionalAiHandle) -> Option<Position> {
        self.entity_view(handle).map(|v| v.position)
    }

    /// Resolve the original game's 3D point from a sector/layer
    /// position. The returned y coordinate is screen-space (`y + z`).
    pub fn position_to_point_3d(&self, position: Position) -> crate::coordinates::WorldPoint3D {
        ai_position_to_point_3d(&self.fast_grid, self.obstacle_list(), position)
    }

    /// Borrowed [`crate::sight_obstacle::ObstacleList`] view over this
    /// tick's sight-obstacle snapshot — the shape that
    /// `ai_vision::los_clear` and the visibility query helpers accept.
    pub fn obstacle_list(&self) -> crate::sight_obstacle::ObstacleList<'_> {
        self.sight_obstacles.list()
    }
}

#[cfg(test)]
mod hiking_waypoint_identity_tests;

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

/// Per-tick analysis data computed by the engine's detection loop.
/// Populated once per detection tick, consumed by battle_decisions
/// and swordfight tactics. Passed alongside AiContext.
#[derive(Debug, Clone)]
pub struct AiPerTickData {
    /// Whether to use the intended Hard reaction-time multiplier instead of
    /// the Easy multiplier selected by the original copy-paste bug.
    pub fix_hard_reaction_times: bool,
    /// Shared immutable profile table used by combat evaluation.
    ///
    /// `None` is valid only for narrow non-combat dispatches and test
    /// fixtures. Combat code must call [`Self::required_profile_manager`]
    /// rather than manufacturing an empty profile table. The requested
    /// hand-to-hand profile must exist; a sword requires a real profile.
    pub profile_manager: Option<std::sync::Arc<crate::profiles::ProfileManager>>,
    /// Owner's raw element position for call sites that bypass AI position.
    /// During a door pass this
    /// remains the interpolated body position rather than the committed
    /// destination-side forecast carried by [`AiContext::position`].
    pub owner_live_position: Option<Position>,
    pub patrol_chief_state: AiState,
    pub primary_target_multiplicity: Vec<(HumanHandle, u32)>,
    /// Complete fighter-registry snapshot for direct pointer dereferences.
    ///
    /// Original-game AI lists (allies, enemies, primary targets, etc.) hold
    /// pointers into the level-wide camp fighter arrays. Those lists can
    /// legitimately contain a fighter outside the 500-unit radius used by
    /// nearby-fighter collection, especially after battle planning uses
    /// the owner's larger 360-degree detection radius. Keep this separate
    /// from `nearby_fighters` so radius-based scans retain their exact domain.
    pub fighter_registry: Vec<crate::ai_enemy::FighterSnapshot>,
    pub nearby_fighters: Vec<crate::ai_enemy::FighterSnapshot>,
    /// Same-camp soldiers snapshot for alert functions (`alert_officer`,
    /// `alert_soldiers`).  Populated every tick from the engine's soldier
    /// snapshot list, filtered to the evaluating NPC's camp.
    pub camp_soldiers: Vec<crate::ai_enemy::CampSoldierInfo>,
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
    /// Literal live world positions for enemy detectables. Unlike
    /// `enemy_detectable_positions`, these bypass AI-position door forecasts
    /// and creation-slot boundary rewinding. Direct geometry helpers such as
    /// enemy-elevation checks read the element itself.
    pub enemy_detectable_live_world_positions: Vec<(HumanHandle, crate::coordinates::WorldPoint3D)>,
    /// Pre-computed destination forecast for the missed PC (if any).
    /// Used by `get_battle_overview` to re-predict position before seeking.
    pub missed_pc_forecast: Option<PreparedForecastDestination>,
    /// Target identity paired with `missed_pc_forecast`. A queued Think can
    /// change the AI's `missed_pc` after this snapshot was prepared.
    pub missed_pc_forecast_handle: Option<AiEntityHandle>,
    /// True when `missed_pc` refers to a player character.
    pub missed_pc_is_pc: bool,
    /// AI position returned for the actor.
    /// During a door pass this is the committed destination-side position,
    /// not the actor's interpolated body position.
    pub primary_target_position: Option<Position>,
    /// Handle for which the primary-target metadata in this snapshot was
    /// built. A synchronous AI callback can replace `base.primary_target`
    /// before a later handler consumes the same tick data; consumers must
    /// not pair that new handle with this old target's geometry.
    pub primary_target_snapshot_handle: Option<AiEntityHandle>,
    /// The target element's literal current position and sector. This differs
    /// from [`Self::primary_target_position`] while passing a door and is for
    /// operations that read position / sector directly.
    pub primary_target_live_position: Option<Position>,

    /// Pre-computed fallback positions for the "avenger on the roof"
    /// branch, keyed by target handle. Populated by the engine when
    /// `couldnt_reachpoint` is set, for the current primary target and
    /// every personal enemy-list candidate a decision arm could re-pick,
    /// wherever [`crate::gate::compute_avenger_wait_position`] finds a
    /// blocking gate on the path from that target back to the
    /// evaluating NPC. Empty when the branch doesn't apply.
    pub avenger_on_roof_wait_positions: Vec<(HumanHandle, Position)>,
}

impl AiPerTickData {
    /// Look up the precomputed avenger-on-roof wait position for a
    /// specific target handle. Decision arms re-pick their target
    /// mid-tick, so each caller resolves its own live handle here
    /// instead of reusing a single snapshot-target position.
    pub fn avenger_wait_position_for(&self, target: impl IntoOptionalAiHandle) -> Option<Position> {
        let target = target.into_optional_ai_handle()?.get();
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

    /// Empty inputs for dispatches that do not enter tactical decisions.
    /// Combat handlers require the relevant target and registry inputs.
    pub fn stub() -> Self {
        Self {
            fix_hard_reaction_times: false,
            profile_manager: None,
            owner_live_position: None,
            patrol_chief_state: AiState::Default,
            primary_target_multiplicity: Vec::new(),
            fighter_registry: Vec::new(),
            nearby_fighters: Vec::new(),
            camp_soldiers: Vec::new(),
            primary_target_forecast: None,
            enemy_detectable_forecasts: Vec::new(),
            enemy_detectable_positions: Vec::new(),
            enemy_detectable_live_world_positions: Vec::new(),
            missed_pc_forecast: None,
            missed_pc_forecast_handle: None,
            missed_pc_is_pc: false,
            primary_target_position: None,
            primary_target_snapshot_handle: None,
            primary_target_live_position: None,
            avenger_on_roof_wait_positions: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ReinforcementDoorInfo — cached door data for forest retreats
// ---------------------------------------------------------------------------

/// Cached info for a reinforcement door, used by forest retreats
/// to find the nearest map exit and animate running to its exit point.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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
/// Populated during AI initialization by collecting every sector whose
/// building classification is true. Houses carry their occupant list so AI
/// code can ask "who's inside?" without scanning all entities, and
/// their door indices so pursuers / investigators can pick the right
/// gate to enter / exit through.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
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

    /// Match original-game building-sector authorization.
    ///
    /// Original-game prototype initialization sets
    /// maximal occupant count to `0xFFFF`, and the proto loader does not
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
/// Built during AI initialization at a fixed `AI_DOOR_RALLY_POINT_DISTANCE` from
/// each building door's exit point.
#[derive(
    Debug,
    Clone,
    Default,
    Serialize,
    Deserialize,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct DoorRallyPoint {
    /// World position (outside the door).
    pub position: Position,
    /// Door index in the canonical interactable door table.
    pub door_index: crate::gate::DoorIndex,
    /// Radius around `position` within which NPCs are "at" the rally
    /// point.
    pub radius: f32,
}

/// Distance offset (from the exit point) at which door rally points are
/// anchored.
pub const AI_DOOR_RALLY_POINT_DISTANCE: f32 = 100.0;

/// Global / shared AI state, conceptually module-static.
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    smart_default::SmartDefault,
    robin_state_hash_derive::StateHash,
    bitcode::Encode,
    bitcode::Decode,
)]
pub struct AiGlobalState {
    pub green_alert_soldiers: u16,
    pub yellow_alert_soldiers: u16,
    pub red_alert_soldiers: u16,

    /// Valid soldier allegiances sampled during mission AI initialization.
    /// Like Original's camp-presence flags, this is not recomputed after
    /// deaths or camp changes. The set also supports multi-team hostility.
    pub soldier_camps: std::collections::BTreeSet<crate::element_kinds::Camp>,

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

    /// Next auto-incrementing ID for repulsive points. IDs start at 1.
    #[default(1)]
    pub next_repulsive_point_id: i32,

    /// Cached door geometry for finding a door an enemy could be behind.
    /// Populated at level load and kept in the save snapshot so any
    /// door-state-dependent authorization cache survives restore.
    pub door_seek_infos: Vec<DoorSeekInfo>,

    /// Reinforcement doors: (inner position, door index, exit point).
    /// Used by forest retreats to find the nearest map exit
    /// and animate the NPC running to the door's exit point.
    /// Populated at level load.
    pub reinforcement_doors: Vec<ReinforcementDoorInfo>,

    /// Buildings the AI knows about — populated during AI initialization from
    /// every sector whose `sector_type.is_building()` is true, with
    /// each house's occupant list and doors filled in.
    pub houses: Vec<House>,

    /// Rally points anchored just outside each building door, created in
    /// AI initialization per house gate.
    pub door_rally_points: Vec<DoorRallyPoint>,

    /// Soldier load-order index → entity-handle (slot) mapping. Scripts
    /// and waypoint commands address NPCs by their soldier register
    /// index (the position in the all-soldiers list at level load), not
    /// by their entity slot. Cloned out of
    /// `LevelAssets::all_soldier_entity_ids` once at level load so the
    /// AI tick can resolve a friend ID without re-borrowing the engine.
    pub all_soldier_handles: std::sync::Arc<Vec<u32>>,

    /// Owner-ordered mirror of human primary-target multiplicity.
    /// Original-game AI resets and increments these 16-bit counters directly
    /// on target humans, so later owners in the same actor pass observe the
    /// exact serial mutation history. Original explicitly does not serialize
    /// this scratch field, so a loaded session starts it empty and then
    /// preserves owner-ordered mutations.
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    pub primary_target_multiplicity_scratch: std::collections::BTreeMap<HumanHandle, u32>,
    #[serde(skip)]
    #[state_hash(skip)]
    #[bitcode(skip)]
    pub primary_target_multiplicity_initialized: bool,
}

impl AiGlobalState {
    pub fn npcs_can_be_enemies(&self, diplomacy: &crate::diplomacy::DiplomacyState) -> bool {
        if !diplomacy.npc_faction_wars() {
            return false;
        }
        self.soldier_camps.iter().enumerate().any(|(index, camp)| {
            self.soldier_camps
                .iter()
                .skip(index + 1)
                .any(|other| diplomacy.is_hostile(*camp, *other))
        })
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
    /// within the maximum-axis distance from self to `pos`, scaled by
    /// `distance_factor` and increased by `abs_distance`
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
        // The original game narrows the computed floating-point limit to 16 bits before it
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
