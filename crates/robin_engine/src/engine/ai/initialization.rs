//! AI level and per-NPC initialization.
//!
//! This follows the initialization boundary in the Original engine: the
//! engine-wide AI pass prepares houses and shared views, then invokes the
//! enemy/friendly `InitOneAI` implementations in authored NPC order.

use super::*;

impl EngineInner {
    // ─── AI initialization ──────────────────────────────────────

    /// Initialize AI for all NPCs and reset global AI state.
    ///
    /// Called from `initialize()` after level loading, and again after
    /// deserialization when re-initialization is requested.
    pub(crate) fn init_ai(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &mut LevelAssets,
    ) {
        // Script loading is intentionally recoverable so incomplete developer
        // data can still reach the renderer.  In that mode AI starts without
        // door-derived views, houses, or rally points; make the degraded state
        // explicit rather than silently manufacturing valid-looking caches.
        if self.scripts.mission.is_none() {
            tracing::warn!(
                "Initializing AI without a mission script; door-derived AI state will be unavailable"
            );
        }

        // Reset global AI state
        // think-method recursion depth = 0
        self.ai.global.there_are_royalist_soldiers = false;
        self.ai.global.there_are_lacklandist_soldiers = false;
        self.ai.global.soldier_camps.clear();
        self.ai.global.overall_alert_status = crate::ai::AlertLevel::Green;
        self.ai.global.overall_villain_alert_status = crate::ai::AlertLevel::Green;
        self.ai.global.init_green_yellow_red_alert_soldiers();

        // golden_eye_mode is set from CliArgs after initialize() returns

        // Build the houses list and door rally points.  Collects every
        // building sector, attaches its doors, records occupants, and
        // anchors a rally point outside each door at
        // `AI_DOOR_RALLY_POINT_DISTANCE`.  Must run before the NPC init
        // loop below, because `InitOneAI` reads `leave_house_number`
        // off the AI controller which is assigned here.
        self.initialize_buildings();

        // Beam hiking-path waypoints that sit just outside a building
        // door into the building's interior.  Mutates the shared
        // `hiking_paths` arc in place through `Arc::make_mut` so
        // subsequent NPC clones see the beamed paths.
        {
            let paths = std::sync::Arc::make_mut(&mut assets.hiking_paths);
            let waypoint_sectors = assets
                .hiking_waypoint_sectors
                .as_mut()
                .map(std::sync::Arc::make_mut);
            beam_door_waypoints_into_houses(
                paths,
                waypoint_sectors,
                &self.ai.global.door_seek_infos,
            );
        }

        // Teleport standalone seek points (those used by AI
        // investigators) that sit just outside a building door to
        // the door's inside position — same rule as the waypoint
        // beaming above.  Already implemented as
        // `AiGlobalState::teleport_seek_points_inside_doors`.
        self.ai.global.teleport_seek_points_inside_doors();

        // Initialize each NPC's AI.
        let npc_ids: Vec<EntityId> = self.world.entities.npc_ids().collect();
        let hiking_paths = assets.hiking_paths.clone();
        // Populate the handle → entity view map so the per-NPC
        // init_ctx hands each AI a usable map (even though init
        // mostly just reads self position).
        let scratch = self.build_sim_scratch(sim, assets);
        // For "get soldier from all by id" in the AI tick: copy the
        // level's soldier load-order array onto AiGlobalState so
        // AiContext can resolve script-baked friend IDs.
        self.ai.global.all_soldier_handles = std::sync::Arc::new(
            assets
                .entities
                .soldier_entity_ids
                .iter()
                .map(|eid| eid.index())
                .collect(),
        );
        let entity_views = scratch.ai_entity_views.clone();
        let sight_obstacles = scratch.ai_sight_obstacles.clone();
        let all_soldier_handles = self.ai.global.all_soldier_handles.clone();
        let ambiance = self.world.weather.ambiance;

        // Snapshot of every live human in the engine; every per-NPC
        // init pass reuses the same list to build its detectable enemy
        // array.  Equivalent to iterating the engine's element list
        // inside each per-NPC init.
        let potential_detectables = build_potential_detectables(self);
        let ambush_points_count = self.ai.global.ambush_points.len();

        let all_soldier_entity_ids = assets.entities.soldier_entity_ids.clone();
        let soldier_subordinate_ids = assets.entities.soldier_subordinate_ids.clone();
        let fast_grid = self.world.fast_grid.clone();
        for &npc_id in &npc_ids {
            self.init_one_ai(
                sim,
                npc_id,
                &hiking_paths,
                &assets.hiking_waypoint_sectors,
                &potential_detectables,
                ambush_points_count,
                &entity_views,
                &sight_obstacles,
                &fast_grid,
                ambiance,
                &all_soldier_handles,
                &all_soldier_entity_ids,
                &soldier_subordinate_ids,
            );

            // InitState may finish by calling the actor's Wait method for an
            // authored sleeping, sitting, dead, unconscious, or special
            // pose. Priority-WAIT launch is a synchronous call chain in the
            // Original: Wait -> LaunchSequenceElement ->
            // NextSequenceElementsGo -> Go -> Instruct. Finish that chain
            // before InitOneAI returns to the next NPC. Leaving the
            // instruction queued lets Actor::Hourglass execute a lazy
            // fallback first and restart the authored animation one frame
            // late.
            self.drain_script_synchronous_actions(sim, assets, &mut Vec::new())
                .unwrap_or_else(|error| {
                    panic!(
                        "NPC {npc_id:?} initialization failed synchronous sequence dispatch: {error:?}"
                    )
                });

            // Original InitOneAI invokes virtual SetState inline, after all
            // actor scripts have been initialized. Close the same owner's
            // callback/effect boundary before the next NPC initializes so a
            // state callback cannot leak to the first Hourglass tick or
            // observe later owners' initialized state.
            self.drain_direct_ai_owner_boundary_without_forecast(sim, npc_id, assets);
        }

        // Lift each ambush point's 2D position into 3D (eye height
        // = 32 units above the ground) and assign a sequential ID.
        // The 3D anchor feeds the sight-polygon query that decides
        // whether an NPC on the ambush point can be seen; the ID is
        // how AI scripts reference the point.
        let ambush_points_3d: Vec<_> = self
            .ai
            .global
            .ambush_points
            .iter()
            .map(|ap| {
                self.position_to_point_3d(
                    assets,
                    ap.position.sector,
                    ap.position.level,
                    ap.position.x,
                    ap.position.y,
                )
            })
            .collect();
        for (idx, (ap, mut point_3d)) in self
            .ai
            .global
            .ambush_points
            .iter_mut()
            .zip(ambush_points_3d)
            .enumerate()
        {
            point_3d.z += 32.0;
            ap.position_3d = point_3d;
            ap.id = idx as u16;
        }

        tracing::info!("AI initialized for {} NPCs", npc_ids.len(),);
    }

    /// Per-NPC initialization pass — runs the per-NPC init for both
    /// enemy and friendly AI.
    ///
    /// Runs every entity-level side effect that must happen once at
    /// level load:
    ///
    /// 1. `InitializeDirectionOffsetVeryOld` — seed `direction_old`
    ///    from the current body direction so the vision pipeline has a
    ///    stable starting value.
    /// 2. `InitViewRadius` — clamp `view_radius` / `view_radius_base`
    ///    / `view_radius_goal` to the engine's standard view radius
    ///    for this level (day/night dependent).
    /// 3. Give Merry Man archers in forest levels their starting bow
    ///    ammo (`MERRY_MAN_ARROWS`).
    /// 4. Build the per-NPC "detectable enemies" list from a snapshot
    ///    of live humans.
    /// 5. Stuck-in-obstacle correction (Malignity only): if the NPC
    ///    starts inside a motion obstacle, push its move box out to
    ///    an authorized position and rewrite its map position.
    /// 6. `StoreInitialPositionParameters` — freeze the NPC's current
    ///    position / sector / level / facing as the "initial" values
    ///    that the AI returns to after idle wanders.
    /// 7. Initialize this NPC's patrol path from `path_id`, then run
    ///    `TestIfPathIsFine` against the fast-find grid; clear the
    ///    patrol if any segment intersects an obstacle.
    /// 8. Seed `old_life_points` / `initial_life_points` on enemy AIs
    ///    for the "still has his initial HP" check.  Difficulty-based
    ///    life-point scaling is already applied at entity-spawn time
    ///    in `engine::level_loading::spawn_soldier`, so we just
    ///    snapshot the current value here.
    /// 9. Fill this enemy's `ambush_point_status` vector with
    ///    `Far` × `ambush_points_count` so `RefreshAmbushPoints`
    ///    has a slot per global ambush point.
    /// 10. Dispatch to the subclass's `init_one_ai` for the
    ///     initial-action / state-transition / return-to-duty logic.
    #[allow(clippy::too_many_arguments)]
    fn init_one_ai(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        npc_id: EntityId,
        hiking_paths: &std::sync::Arc<Vec<crate::level_data::RawHikingPath>>,
        hiking_waypoint_sectors: &Option<
            std::sync::Arc<Vec<Vec<crate::position_interface::SectorHandle>>>,
        >,
        potential_detectables: &[PotentialDetectable],
        ambush_points_count: usize,
        entity_views: &SharedAiEntityViews,
        sight_obstacles: &crate::sight_obstacle::SharedSightObstacles,
        fast_grid: &std::sync::Arc<crate::fast_find_grid::FastFindGrid>,
        ambiance: crate::engine::types::Ambiance,
        all_soldier_handles: &std::sync::Arc<Vec<u32>>,
        all_soldier_entity_ids: &[EntityId],
        soldier_subordinate_ids: &[Vec<u16>],
    ) {
        // -- Phase 1: Peek at the entity to classify (enemy / friendly,
        //    camp) and read the fields we need for the obstacle fix. --
        let (is_enemy, is_friendly, self_camp, pos_map, layer, move_box_opt) = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let (is_enemy, is_friendly, self_camp) = match entity {
                Entity::Soldier(s) => (
                    s.npc.ai_brain.enemy().is_some(),
                    false,
                    s.soldier.cached_camp,
                ),
                Entity::Civilian(c) => (
                    false,
                    c.npc.ai_brain.friendly().is_some(),
                    c.civilian.cached_camp,
                ),
                _ => return,
            };
            let elem = entity.element_data();
            let move_box = entity
                .actor_data()
                .map(|_| *entity.position_iface().get_move_box());
            (
                is_enemy,
                is_friendly,
                self_camp,
                elem.position_map(),
                elem.layer(),
                move_box,
            )
        };
        if !(is_enemy || is_friendly) {
            return;
        }

        // -- Phase 2: Stuck-in-obstacle correction (enemy only). --
        // If the NPC's move-box overlaps the playable area, attempt to
        // push it to an authorized position via `find_authorized_position`.
        if is_enemy && let Some(move_box) = move_box_opt {
            let mut abs_box = move_box.translated(pos_map);
            if !self.world.fast_grid.is_position_authorized(&abs_box, layer)
                && self
                    .world
                    .fast_grid
                    .find_authorized_position(&mut abs_box, layer)
            {
                let new_center = abs_box.center();
                if let Some(entity) = self.world.entities.get_mut(npc_id)
                    && entity.actor_data().is_some()
                {
                    let new_center_map = new_center;
                    let pi = entity.position_iface_mut();
                    pi.set_map_position(new_center_map);
                    entity.element_data_mut().set_position_map(new_center_map);
                }
            }
        }

        // -- Phase 3: Build the detectable-enemy list for this NPC. --
        let detectables =
            build_detectable_enemies_for(self_camp, is_friendly, npc_id, potential_detectables);

        // -- Phase 4: Re-read entity (post-fix) and mutate all the
        //    per-NPC state fields in one shot. --
        let standard_view_radius = if self.ai.standard_view_polygon_radius > 0 {
            self.ai.standard_view_polygon_radius
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS
        };
        let is_forest_level = self.world.weather.is_forest_level;

        // `entity_building_sector` needs a `&self` borrow; compute it
        // up-front while we don't hold a mutable entity borrow.
        let building_sector = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            self.entity_building_sector(entity.element_data().sector())
        };

        // Determine whether this NPC is a Merry-Man archer (Royalist
        // soldier, forest level, archer flag set by the level loader).
        let is_merry_man_archer = if is_enemy {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let is_archer = entity.enemy_ai().map(|e| e.is_archer()).unwrap_or(false);
            let is_rider = entity.soldier_data().map(|s| s.rider).unwrap_or(false);
            self_camp == Camp::Royalists && is_forest_level && is_archer && !is_rider
        } else {
            false
        };

        // Grab the (possibly corrected) map position / direction /
        // sector / layer before the write-back borrow.
        let (pos_map_final, direction_final, sector_final, layer_final, current_lp) = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            let elem = entity.element_data();
            let lp = entity.npc_data().map(|n| n.life_points).unwrap_or(0);
            (
                elem.position_map(),
                elem.direction(),
                elem.sector(),
                elem.layer(),
                lp,
            )
        };

        // Write-back block: mutate every field this init pass owns.
        {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            if let Some(npc) = entity.npc_data_mut() {
                // `initialize_direction_offset_very_old`: seed from current body dir.
                npc.direction_old = direction_final;

                // `init_view_radius`: real radius + goal = standard view
                // radius.  We also seed `view_radius_base` so subsequent
                // alert/drunk modifiers scale off the correct baseline.
                npc.view_radius = standard_view_radius;
                npc.view_radius_base = standard_view_radius;
                npc.view_radius_goal = standard_view_radius;

                if is_merry_man_archer {
                    // Seed the bow ammo for forest-level Merry Man archers.
                    npc.number_of_arrows = MERRY_MAN_ARROWS;
                }

                // Detectable enemies list (`DetectableType::Enemy`
                // slot).  Other slots (Body/Object/Friend/...) are
                // populated later by runtime events.
                let enemy_idx = DetectableType::Enemy as usize;
                npc.detectable_lists[enemy_idx] = detectables;

                // `store_initial_position_parameters`: snapshot current
                // position, sector, level, and facing into the
                // initial-position fields.
                npc.initial_position_x = pos_map_final.x;
                npc.initial_position_y = pos_map_final.y;
                npc.initial_position_sector = sector_final;
                npc.initial_position_level = layer_final;
                let dir_vec = crate::shadow_polygon::sector_to_direction(direction_final);
                npc.initial_view_direction.x = dir_vec[0];
                npc.initial_view_direction.y = dir_vec[1];
            }

            // Switch on the NPC's camp to set the static camp-present
            // flags (`there_are_royalist_soldiers` /
            // `there_are_lacklandist_soldiers`).  Reading
            // `npcs_can_be_enemies()` later gates mixed-camp soldier
            // hostility on both flags being true.  The life-point
            // Easy/Hard scaling on the Lacklandist arm is already
            // applied at spawn time in `level_loading::spawn_soldier`.
            if is_enemy {
                if self_camp != Camp::Error {
                    self.ai.global.soldier_camps.insert(self_camp);
                }
                match self_camp {
                    Camp::Royalists => self.ai.global.there_are_royalist_soldiers = true,
                    Camp::Lacklandists => self.ai.global.there_are_lacklandist_soldiers = true,
                    _ => {}
                }
            }

            // Enemy-specific state.
            if is_enemy && let Some(enemy) = entity.enemy_ai_mut() {
                // `old_life_points` = `initial_life_points` =
                // `get_life_points()`.  The level loader already applied
                // difficulty scaling to `cached_max_life_points` at
                // `engine::level_loading::spawn_soldier`, so the current
                // life points are already correct.
                let clamped = current_lp.clamp(0, 255) as u8;
                enemy.old_life_points = clamped;
                enemy.initial_life_points = clamped;

                // Reset the ambush-point-status array and insert
                // `AMBUSH_POINT_FAR` for every point in the global
                // ambush array.
                enemy.ambush_point_array_reset = true;
                enemy.ambush_point_status.clear();
                enemy
                    .ambush_point_status
                    .resize(ambush_points_count, crate::ai_enemy::AmbushPointStatus::Far);
            }
        }

        // -- Phase 5: Patrol path init + TestIfPathIsFine. --
        // Initialize the path from path_id, then test it; on failure,
        // assert in debug and silently clear in release.
        let patrol_path_opt = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            entity
                .ai_controller()
                .and_then(|ai| ai.path_id)
                .and_then(|pid| crate::ai::PatrolPath::new(pid, hiking_paths))
        };

        let patrol_path_ok = if let Some(ref patrol) = patrol_path_opt {
            // Grab the actual hiking-path waypoints + the NPC's move
            // box and run the obstacle check.
            let waypoints = hiking_paths
                .get(usize::from(patrol.hiking_path_index))
                .map(|p| p.waypoints.as_slice())
                .unwrap_or(&[]);
            let move_box = move_box_opt.unwrap_or_default();
            let ok = test_hiking_path_fine(&self.world.fast_grid, waypoints, &move_box);
            if !ok {
                tracing::warn!(
                    npc = npc_id.index(),
                    path_id = patrol.hiking_path_index.get(),
                    waypoints = waypoints.len(),
                    move_box = ?move_box,
                    "BUG: patrol path rejected by TestIfPathIsFine — debug asserts this \
                     never fails; in release the path is cleared and the NPC silently \
                     stops patrolling"
                );
            }
            ok
        } else {
            false
        };

        // -- Phase 6: Build the init ctx and commit patrol/path state. --
        let mut init_ctx = {
            let Some(entity) = self.world.entities.get(npc_id) else {
                return;
            };
            build_ai_context_from_entity(
                entity,
                0,
                building_sector,
                is_forest_level,
                ambiance,
                standard_view_radius,
                entity_views,
                sight_obstacles,
                fast_grid,
                hiking_paths,
                hiking_waypoint_sectors,
                all_soldier_handles,
                self.control.sim_config.difficulty,
            )
        };
        self.refresh_selected_default_wait_identity(npc_id, &mut init_ctx);

        {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            if let Some(ai) = entity.ai_controller_mut() {
                ai.initial_position = crate::ai::Position {
                    x: pos_map_final.x,
                    y: pos_map_final.y,
                    sector: sector_final,
                    level: layer_final,
                };
                // `StoreInitialPositionParameters` stores a direction vector
                // with `SetSector0to15(GetDirection())` (default aspect 1),
                // while return-to-post later calls `FaceTo(vector)`, which
                // bins it with `GetSector0to15(ASPECT_RATIO)`. Those two
                // operations deliberately do not round-trip diagonal body
                // sectors (for example body sector 2 becomes view sector 1).
                // Keep the controller's discrete cache equal to that later
                // FaceTo result; storing the body sector directly made an NPC
                // believe it was already facing its authored post direction.
                let initial_view_vector =
                    crate::shadow_polygon::sector_to_direction(direction_final);
                ai.initial_view_direction = crate::position_interface::vector_to_sector_0_to_15(
                    initial_view_vector[0] * crate::position_interface::ASPECT_RATIO,
                    initial_view_vector[1],
                ) as u16;
                if patrol_path_opt.is_some() && patrol_path_ok {
                    ai.patrol_path = patrol_path_opt;
                    ai.has_patrol_path = true;
                } else {
                    ai.detach_patrol_path(None, false);
                    ai.has_patrol_path = false;
                }

                // -- TransformPatrolIDsToRealPatrol --
                // Runs exactly once at AI init from the enemy AI's
                // `init_ai` before the first `initialize_patrol()`.
                // The raw mission subordinate IDs live on LevelAssets,
                // not on the serialized AI controller; runtime patrol
                // rebuilds use `theoretical_patrol`.
                if let Some(soldier_load_index) =
                    all_soldier_entity_ids.iter().position(|&eid| eid == npc_id)
                    && let Some(patrol_ids) = soldier_subordinate_ids.get(soldier_load_index)
                    && !patrol_ids.is_empty()
                {
                    ai.patrol.clear();
                    ai.missed_patrol_members.clear();
                    ai.theoretical_patrol.clear();
                    for &id in patrol_ids {
                        if let Some(&eid) = all_soldier_entity_ids.get(id as usize) {
                            ai.theoretical_patrol.push(eid);
                        } else {
                            tracing::warn!(
                                "NPC {} patrol ID {} out of range (max {})",
                                npc_id.index(),
                                id,
                                all_soldier_entity_ids.len()
                            );
                        }
                    }
                }
            }
        }

        // Original `RHArtificialMalignity::InitOneAI` transforms a patrol
        // chief's authored soldier IDs and immediately calls
        // `InitializePatrol` before evaluating the chief's initial state.
        // That synchronous pass also writes `patrol_chief` onto admitted
        // minions.  Some of those minions occur later in the NPC creation
        // order, so their own InitOneAI must already see the chief and return
        // to formation instead of independently starting their authored
        // hiking path.
        //
        // Runtime patrol refresh remains owner-ticked, but mission bootstrap
        // cannot defer this first pass: doing so changes ReturnToDuty,
        // consumes extra macro RNG draws, and starts the minion on the wrong
        // route for its first simulation frame.
        let theoretical_patrol = self
            .world
            .entities
            .get(npc_id)
            .and_then(|entity| entity.ai_controller())
            .map(|ai| ai.theoretical_patrol.clone())
            .unwrap_or_default();
        if !theoretical_patrol.is_empty() {
            let chief_view = entity_views.get(&npc_id.index()).unwrap_or_else(|| {
                panic!(
                    "patrol chief {} is absent from the AI initialization view map",
                    npc_id.index()
                )
            });
            let chief_position = chief_view.position;
            let chief_ground_z = chief_view.elevation;
            let obstacles = sight_obstacles.list();
            let mut patrol = Vec::new();
            let mut missed = Vec::new();

            for &member in &theoretical_patrol {
                let member_view = entity_views.get(&member.index()).unwrap_or_else(|| {
                    panic!(
                        "patrol chief {} references missing authored member {}",
                        npc_id.index(),
                        member.index()
                    )
                });
                let admitted = member_view.active
                    && !member_view.is_dead
                    && member_view.ai_state == crate::ai::AiState::Default
                    && (member_view.is_civilian() || member_view.is_able_to_fight)
                    && crate::ai_enemy::soldier_detects_target_360(
                        chief_position,
                        chief_ground_z,
                        chief_view.is_rider,
                        standard_view_radius,
                        chief_view.in_building,
                        member_view.position,
                        member_view.elevation,
                        member_view.posture,
                        member_view.is_rider,
                        member_view.direction as i16,
                        member_view.in_building,
                        obstacles,
                    );
                if admitted {
                    patrol.push(member);
                } else if !member_view.is_dead {
                    missed.push(member);
                }
            }

            // `InitializePatrol` inserts by increasing 3-D square distance;
            // ties insert before the existing member.
            let patrol_distance = |member: EntityId| {
                let view = entity_views.get(&member.index()).unwrap_or_else(|| {
                    panic!(
                        "patrol member {} disappeared from the AI initialization view map",
                        member.index()
                    )
                });
                let dx = view.position.x - chief_position.x;
                let dy_world =
                    (view.position.y + view.elevation) - (chief_position.y + chief_ground_z);
                let dy = dy_world * crate::position_interface::INVERSE_ASPECT_RATIO;
                let dz = view.elevation - chief_ground_z;
                dx * dx + dy * dy + dz * dz
            };
            let mut sorted_patrol = Vec::with_capacity(patrol.len());
            for member in patrol {
                let distance = patrol_distance(member);
                let insert_at = sorted_patrol
                    .iter()
                    .position(|&existing| {
                        patrol_distance_inserts_before(distance, patrol_distance(existing))
                    })
                    .unwrap_or(sorted_patrol.len());
                sorted_patrol.insert(insert_at, member);
            }

            // Arrange each pair left/right relative to the chief.
            for i in (1..sorted_patrol.len()).step_by(2) {
                let even = &entity_views[&sorted_patrol[i - 1].index()].position;
                let odd = &entity_views[&sorted_patrol[i].index()].position;
                let ex = even.x - chief_position.x;
                let ey = even.y - chief_position.y;
                let ox = odd.x - chief_position.x;
                let oy = odd.y - chief_position.y;
                if ex * oy - ey * ox < 0.0 {
                    sorted_patrol.swap(i - 1, i);
                }
            }

            {
                let chief = self.world.entities.get_mut(npc_id).unwrap_or_else(|| {
                    panic!(
                        "patrol chief {} disappeared during AI initialization",
                        npc_id.index()
                    )
                });
                let ai = chief.ai_controller_mut().unwrap_or_else(|| {
                    panic!(
                        "patrol chief {} has no AI controller during initialization",
                        npc_id.index()
                    )
                });
                ai.patrol = sorted_patrol.clone();
                ai.missed_patrol_members = missed;
                ai.needs_patrol_reinit = false;
            }
            for member in sorted_patrol {
                let entity = self.world.entities.get_mut(member).unwrap_or_else(|| {
                    panic!(
                        "patrol chief {} admitted missing member {}",
                        npc_id.index(),
                        member.index()
                    )
                });
                let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                    panic!(
                        "patrol member {} has no AI controller during initialization",
                        member.index()
                    )
                });
                ai.patrol_chief = Some(npc_id);
            }
        }

        // -- Phase 7: Dispatch to the subclass for state transitions. --
        // The InitState / ReturnToDuty / beggar-lock tail.  The subclass
        // commits the AI-side state transition via
        // `AiController::init_state` and returns the entity-side side
        // effects — posture / action state / eye status / life-point /
        // concussion writes that the AI layer can't reach on its own.
        let init_fx: crate::ai::InitStateSideEffects = {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };
            match &mut entity.npc_data_mut().map(|n| &mut n.ai_brain) {
                Some(crate::element::AiBrain::Enemy(e)) => {
                    // Init runs during `InitOneAI` before any detection
                    // or target selection — `primary_target` is 0 and
                    // no battle context exists yet, so the centralized
                    // `build_npc_tick_data` would return `stub()`
                    // anyway.  Skip the round-trip and pass stub
                    // directly.
                    let tick = AiPerTickData::stub();
                    e.init_one_ai(sim, &init_ctx, &tick)
                }
                Some(crate::element::AiBrain::Friendly(f)) => f.init_one_ai(sim, &init_ctx),
                _ => return,
            }
        };
        if let Some(enemy) = self
            .world
            .entities
            .get_mut(npc_id)
            .and_then(|entity| entity.enemy_ai_mut())
        {
            // Both InitializePatrol calls made by Original InitOneAI have
            // completed synchronously above.  Do not repeat the bootstrap
            // pass on the first owner tick.
            enemy.base.needs_patrol_reinit = false;
        }

        // -- Phase 8: Apply entity-side side effects from `init_state`. --
        // Posture, action state, eye status, life points, and
        // concussion all live on the entity, not the AI brain.  The
        // subclass dispatch already committed the AI-side state
        // transitions; here we flush the entity half.
        if init_fx.set_posture.is_some()
            || init_fx.set_action_state.is_some()
            || init_fx.set_eye_status.is_some()
            || init_fx.zero_life_points
            || init_fx.concussion_max_and_unconscious
        {
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                return;
            };

            // Posture: write to `ElementData::posture`.  Matches the
            // existing `pending_posture` drain path which deliberately
            // skips `PositionInterface::set_posture` — the move-box
            // recomputation is deferred and every other posture write
            // in the codebase (melee knock-out paths,
            // ability.CarryingCorpse, …) follows the same pattern.
            if let Some(posture) = init_fx.set_posture {
                entity.set_posture(posture);
            }

            // Action state: write on `ActorData`.  `set_states(...,
            // action_state)` + `wait()` collapse to a direct
            // `action_state = X` at init time, since the entity has no
            // active animation to interrupt.
            if let Some(action_state) = init_fx.set_action_state
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.action_state = action_state;
            }

            // Eye status: use the existing `ai_vision::set_view_status`
            // helper so `view_transition` is flipped alongside the raw
            // field.  Equivalent to `close_eyes` (which just calls
            // `set_view_status(EYES_CLOSED)`).
            if let Some(status) = init_fx.set_eye_status
                && let Some(npc) = entity.npc_data_mut()
            {
                crate::ai_vision::set_view_status(npc, status);
            }

            // Zero life points + killed-by-accident.  Bundled because
            // they are always written together at init
            // (`init_with_zero_life_points` followed by
            // `set_killed_by_accident(true)`).
            if init_fx.zero_life_points {
                if let Some(npc) = entity.npc_data_mut() {
                    npc.life_points = 0;
                }
                if let Some(human) = entity.human_data_mut() {
                    human.killed_by_accident = true;
                }
            }

            // Max concussion + unconscious.  Init-time bypasses
            // the full `combat::set_concussion` state machine
            // because none of its gates (script lock, tied,
            // carried) apply on a freshly-spawned NPC.
            if init_fx.concussion_max_and_unconscious
                && let Some(human) = entity.human_data_mut()
            {
                human.concussion_of_the_brain = crate::combat::CONCUSSION_MAX;
                human.unconscious = true;
            }
        }

        if init_fx.launch_wait {
            // RHArtificialIntelligence::InitState calls mpMe->Wait() only
            // after SetStates for authored sleeping/sitting/dead/etc. poses.
            // This must be a new launch, not ensure_wait_element: mission
            // script initialization may already have installed an Upright
            // wait whose translated orders are stale for the new posture.
            self.actor_wait(npc_id);
        }
    }

    /// Populate `AiGlobalState::houses` and `door_rally_points` from
    /// the currently-loaded level, and assign `leave_house_number` to
    /// each NPC occupant.
    ///
    /// The building-loop portion of AI init.  Each building sector
    /// becomes one [`House`]: its doors are looked up via the door
    /// table (doors whose `sector_in` matches the building), its
    /// occupants are found by scanning entities currently in that
    /// sector, and one [`DoorRallyPoint`] is anchored at every door's
    /// `point_out`.
    ///
    /// Runtime occupant tracking is wired at the `execute_pass_door`
    /// Enter / Leave branches in `engine::door_pass`: the same hook
    /// that updates canonical `BuildingState` occupants also updates
    /// `House::occupant_ids`.
    pub(super) fn initialize_buildings(&mut self) {
        use crate::ai::{AI_DOOR_RALLY_POINT_DISTANCE, DoorRallyPoint, House, Position};

        self.ai.global.houses.clear();
        self.ai.global.door_rally_points.clear();

        // Index canonical doors by their `sector_in` (building interior side).
        // BTreeMap (not HashMap) so the `for (sector_in, …) in
        // doors_by_building` iteration below assigns `leave_house_number`
        // in a stable, sector-ordered sequence — replay/lockstep multi-
        // player need deterministic AI state.
        let mut doors_by_building: std::collections::BTreeMap<
            crate::sector::SectorNumber,
            Vec<u32>,
        > = std::collections::BTreeMap::new();
        let mut rally_points: Vec<DoorRallyPoint> = Vec::new();

        // Include every building's doors — not just those occupied
        // at init time — so the runtime enter/leave hooks have a
        // pre-existing `House` to update when an NPC walks into a
        // previously-empty building.  The reference's restriction to
        // NPC-populated buildings (the houses list being built from
        // starting sectors) is an artifact of *how* it initializes,
        // not a semantic invariant; live occupant tracking supersedes
        // it. `RHArtificialIntelligence::InitAI` walks every gate owned by
        // each building when it builds rally points, and
        // `InitBattleBeforeDoor` walks that same gate list. This includes a
        // `DOOR_BUILDING_TRAP` whose inside sector is the building; excluding
        // it can select a farther ordinary door and changes the observable
        // door-fight RNG consumption.
        // A missing script is the explicitly warned degraded-load path from
        // `init_ai`; houses intentionally remain empty in that mode.
        if self.scripts.mission.is_some() {
            for (idx, door) in self.script_domains.interactables.doors.iter().enumerate() {
                if !door_belongs_to_ai_house(door.door_type) {
                    continue;
                }
                doors_by_building
                    .entry(door.sector_in)
                    .or_default()
                    .push(idx as u32);

                // Rally point: use the door's `point_out` directly
                // (the sectorised "outside" position).
                rally_points.push(DoorRallyPoint {
                    position: Position {
                        x: door.point_out.x,
                        y: door.point_out.y,
                        sector: crate::position_interface::SectorHandle::new(u16::from(
                            door.sector_out,
                        )),
                        level: door.layer_out,
                    },
                    door_index: crate::gate::DoorIndex(idx as u32),
                    radius: AI_DOOR_RALLY_POINT_DISTANCE,
                });
            }
        }

        // Collect occupants per building from the current entity set.
        // An entity is "in building X" if its sector is X *and* that
        // sector is flagged `is_building()`.  We skip entities without
        // actor data (objects, FX) since only actors can be NPCs.
        let mut occupants_by_building: std::collections::HashMap<
            crate::sector::SectorNumber,
            Vec<EntityId>,
        > = std::collections::HashMap::new();

        for (entity_id, entity) in self.world.entities.actors() {
            let elem = entity.element_data();
            let sector_raw = match elem.sector() {
                Some(s) => crate::sector::SectorNumber::new(u16::from(s) as i16),
                None => continue,
            };
            // Only record occupants of sectors we know are buildings
            // (have at least one building door pointing at them).
            if !doors_by_building.contains_key(&sector_raw) {
                continue;
            }
            occupants_by_building
                .entry(sector_raw)
                .or_default()
                .push(entity_id.into());
        }

        // Build the houses list from the collected door/occupant maps.
        for (sector_in, door_indices) in doors_by_building {
            let occupant_ids = occupants_by_building.remove(&sector_in).unwrap_or_default();

            // Distribute sequential `leave_house_number` to each
            // occupant — used by the departure scheduler to stagger
            // NPCs exiting during alerts.
            for (n, &eid) in occupant_ids.iter().enumerate() {
                if let Some(entity) = self.world.entities.get_mut(eid)
                    && let Some(ai) = entity.ai_controller_mut()
                {
                    ai.leave_house_number = n as u16;
                }
            }

            // Look up the building index on the grid sector.  `None`
            // for script-synthesised or otherwise proto-unlinked
            // building sectors — rare but non-fatal.
            let building_index = self
                .world
                .fast_grid
                .level
                .sector_number_map
                .get(&sector_in)
                .and_then(|&idx| self.world.fast_grid.level.sectors.get(idx))
                .and_then(|gs| gs.building_index);

            // Read `arrow_reserve` from the engine-owned building domain
            // (populated from the GUYS/CAVE tenant chunk at level
            // load).  `max_occupants` still has no proto source — we
            // leave it at the `0xFFFF` default (unlimited) matching
            // `BuildingData::default()`.
            let arrow_reserve = building_index
                .and_then(|bi| {
                    self.script_domains
                        .buildings
                        .arrow_reserves
                        .get(usize::from(bi))
                        .copied()
                })
                .unwrap_or(false);

            self.ai.global.houses.push(House {
                sector_index: u32::from(u16::from(sector_in)),
                building_index,
                door_indices,
                occupant_ids,
                arrow_reserve,
            });
        }

        self.ai.global.door_rally_points = rally_points;

        tracing::info!(
            houses = self.ai.global.houses.len(),
            rally_points = self.ai.global.door_rally_points.len(),
            "Initialized AI building data"
        );
    }
}
