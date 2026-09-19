//! AI level and per-NPC initialization.
//!
//! This follows the initialization boundary in the Original engine: the
//! engine-wide AI pass prepares houses and shared views, then invokes the
//! enemy/friendly AI initialization in authored NPC order.

use super::*;
use crate::engine::TickCtx;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{AiState, EmoticonType, Substate};
    use crate::element::{ActionState, EyeStatus, Posture};
    use crate::order::OrderType;

    #[test]
    fn soldier_alert_counts_follow_brain_publication_and_survive_removal_and_restore() {
        use crate::ai::AlertLevel;
        let mut engine = EngineInner::new();
        let mut owners = Vec::new();
        for (camp, level) in [
            (crate::element::Camp::Lacklandists, AlertLevel::Green),
            (crate::element::Camp::Royalists, AlertLevel::Yellow),
            (crate::element::Camp::Custom(2), AlertLevel::Red),
        ] {
            let mut entity = crate::engine::test_support::actors::make_test_ai_soldier(camp);
            entity
                .ai_controller_mut()
                .unwrap()
                .current_music_alert_status = level;
            owners.push(engine.add_test_entity(entity));
        }
        assert_eq!(engine.ai.global.green_alert_soldiers, 1);
        assert_eq!(engine.ai.global.yellow_alert_soldiers, 1);
        assert_eq!(engine.ai.global.red_alert_soldiers, 1);
        engine.remove_entity(owners[0]);
        assert_eq!(engine.ai.global.green_alert_soldiers, 1);
        let json = serde_json::to_string(&engine).unwrap();
        let restored: EngineInner = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.ai.global.green_alert_soldiers, 1);
        assert_eq!(restored.ai.global.yellow_alert_soldiers, 1);
        assert_eq!(restored.ai.global.red_alert_soldiers, 1);
        assert_eq!(
            robin_util::state_hash::compute(&engine.ai.global),
            robin_util::state_hash::compute(&restored.ai.global)
        );
    }

    fn fixture(action: OrderType, indoors: bool) -> (EngineInner, LevelAssets, EntityId) {
        let mut engine = EngineInner::new();
        engine.world.fast_grid_mut().size_map(128, 128);
        engine.world.fast_grid_mut().allocate_layers(1);
        let mut sector = crate::engine::test_support::square_sector(
            1,
            0,
            MapPoint::new(0.0, 0.0),
            MapPoint::new(2000.0, 2000.0),
        );
        if indoors {
            sector.sector_type |= crate::sector::SectorType::BUILDING;
        }
        let index = engine.world.fast_grid_mut().add_sector(sector, 0);
        let sector = crate::position_interface::SectorHandle::new(1)
            .unwrap()
            .with_arena_index(crate::fast_find_grid::SectorIndex::new(index).unwrap());
        let mut entity =
            crate::engine::test_support::actors::make_test_ai_soldier(Camp::Lacklandists);
        entity
            .element_data_mut()
            .set_position_map(MapPoint::new(100.0, 100.0));
        entity
            .element_data_mut()
            .set_position(crate::coordinates::WorldPoint3D::new(100.0, 100.0, 0.0));
        entity.element_data_mut().set_sector(Some(sector));
        entity.npc_data_mut().unwrap().life_points = 100;
        let owner = engine.add_test_entity(entity);
        let ai = engine
            .world
            .entities
            .get_mut(owner)
            .unwrap()
            .ai_controller_mut()
            .unwrap();
        ai.owner_entity_id = Some(owner);
        ai.me = owner.index();
        ai.initial_action = action as u32;
        let mut assets = LevelAssets::new();
        crate::engine::complete_test_runtime_fixture(&mut engine, &mut assets);
        (engine, assets, owner)
    }

    #[test]
    fn authored_waiting_enters_post_and_clears_previous_pose_preferences() {
        for action in [
            OrderType::WaitingUpright,
            OrderType::WaitingUprightBored,
            OrderType::WaitingUprightBoredRandom,
        ] {
            let (mut engine, assets, owner) = fixture(action, false);
            let ai = engine
                .world
                .entities
                .get_mut(owner)
                .unwrap()
                .ai_controller_mut()
                .unwrap();
            ai.likes_to_sit_around = true;
            ai.special_action = true;
            ai.is_stay_at_home = true;
            assert!(engine.initialize_ai_state(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                owner
            ));
            let ai = engine
                .world
                .entities
                .get(owner)
                .unwrap()
                .ai_controller()
                .unwrap();
            assert_eq!(ai.current_state, AiState::Default);
            assert_eq!(ai.current_substate, Substate::DefaultOnPost);
            assert!(ai.timer_is_running);
            assert!(!ai.likes_to_sit_around && !ai.special_action && !ai.is_stay_at_home);
        }
    }

    #[test]
    fn authored_sleep_and_leisure_apply_live_actor_pose() {
        for (action, posture, action_state) in [
            (
                OrderType::SleepingUpright,
                Posture::Upright,
                ActionState::Sleeping,
            ),
            (OrderType::Sitting, Posture::Sitting, ActionState::Waiting),
            (OrderType::Special, Posture::Leisure, ActionState::Waiting),
        ] {
            let (mut engine, assets, owner) = fixture(action, false);
            assert!(!engine.initialize_ai_state(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                owner
            ));
            let entity = engine.world.entities.get(owner).unwrap();
            let ai = entity.ai_controller().unwrap();
            assert_eq!(entity.posture(), posture);
            assert_eq!(entity.actor_data().unwrap().action_state, action_state);
            assert_eq!(ai.likes_to_sit_around, action == OrderType::Sitting);
            assert_eq!(ai.special_action, action == OrderType::Special);
            if action == OrderType::SleepingUpright {
                assert_eq!(ai.current_substate, Substate::SleepingNapping);
                assert_eq!(ai.current_emoticon_type, EmoticonType::Zzz);
                assert_eq!(
                    entity.ai_actor_data().unwrap().eye_status,
                    EyeStatus::Closed
                );
            } else {
                assert_eq!(ai.current_substate, Substate::DefaultOnPost);
            }
        }
    }

    #[test]
    fn authored_dead_and_unconscious_poses_commit_human_state() {
        for (action, posture, substate) in [
            (
                OrderType::BeingDead,
                Posture::Dead,
                Substate::SleepingForever,
            ),
            (
                OrderType::BeingDeadFallenBack,
                Posture::DeadBack,
                Substate::SleepingForever,
            ),
            (
                OrderType::BeingUnconscious,
                Posture::Lying,
                Substate::SleepingUnconscious,
            ),
        ] {
            let (mut engine, assets, owner) = fixture(action, false);
            assert!(!engine.initialize_ai_state(
                TickCtx::new(&crate::sim_rng::test_context(), &assets),
                owner
            ));
            let entity = engine.world.entities.get(owner).unwrap();
            assert_eq!(entity.posture(), posture);
            assert_eq!(entity.ai_controller().unwrap().current_substate, substate);
            let human = entity.human_data().unwrap();
            if action == OrderType::BeingUnconscious {
                assert!(human.unconscious);
                assert_eq!(human.concussion_of_the_brain, crate::combat::CONCUSSION_MAX);
                assert_eq!(entity.human_life_points(), 100);
            } else {
                assert_eq!(entity.human_life_points(), 0);
                assert!(human.killed_by_accident);
            }
        }
    }

    #[test]
    fn building_membership_overrides_authored_initial_action() {
        let (mut engine, assets, owner) = fixture(OrderType::BeingDead, true);
        assert!(!engine.initialize_ai_state(
            TickCtx::new(&crate::sim_rng::test_context(), &assets),
            owner
        ));
        let entity = engine.world.entities.get(owner).unwrap();
        let ai = entity.ai_controller().unwrap();
        assert!(ai.is_stay_at_home);
        assert_eq!(ai.current_substate, Substate::DefaultHomeSweetHome);
        assert_eq!(entity.human_life_points(), 100);
        assert_eq!(entity.posture(), Posture::Upright);
    }
}

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
        self.ai.global.soldier_camps.clear();
        // Alert counters already include each published soldier's constructed brain.
        // Preserve them across AI initialization and restoration.

        // golden_eye_mode is set from CliArgs after initialize() returns

        // Build the houses list and door rally points.  Collects every
        // building sector, attaches its doors, records occupants, and
        // anchors a rally point outside each door at
        // `AI_DOOR_RALLY_POINT_DISTANCE`.  Must run before the NPC init
        // loop below, because per-NPC initialization reads `leave_house_number`
        // off the AI controller which is assigned here.
        self.initialize_buildings();

        // Beam hiking-path waypoints that sit just outside a building
        // door into the building's interior.  Mutates the shared
        // `hiking_paths` arc in place through `Arc::make_mut` so
        // subsequent NPC clones see the beamed paths.
        {
            let paths = std::sync::Arc::make_mut(&mut assets.navigation.hiking_paths);
            let waypoint_sectors = assets
                .navigation
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
        let npc_ids: Vec<EntityId> = self.entities().ai_owner_ids().collect();
        let hiking_paths = assets.navigation.hiking_paths.clone();
        let ambush_points_count = self.ai.global.ambush_points.len();

        let all_soldier_entity_ids = assets.entities.soldier_entity_ids.clone();
        let soldier_subordinate_ids = assets.entities.soldier_subordinate_ids.clone();
        for &npc_id in &npc_ids {
            self.init_one_ai(
                TickCtx::new(sim, assets),
                npc_id,
                &hiking_paths,
                ambush_points_count,
                &all_soldier_entity_ids,
                &soldier_subordinate_ids,
            );
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
    /// 1. Seed `direction_old`
    ///    from the current body direction so the vision pipeline has a
    ///    stable starting value.
    /// 2. Clamp `view_radius` / `view_radius_base`
    ///    / `view_radius_goal` to the engine's standard view radius
    ///    for this level (day/night dependent).
    /// 3. Give Merry Man archers in forest levels their starting bow
    ///    ammo (`MERRY_MAN_ARROWS`).
    /// 4. Build the per-NPC "detectable enemies" list from a snapshot
    ///    of live humans.
    /// 5. Stuck-in-obstacle correction (Malignity only): if the NPC
    ///    starts inside a motion obstacle, push its move box out to
    ///    an authorized position and rewrite its map position.
    /// 6. Freeze the NPC's current
    ///    position / sector / level / facing as the "initial" values
    ///    that the AI returns to after idle wanders.
    /// 7. Initialize this NPC's patrol path from `path_id`, then check
    ///    its segments against the fast-find grid; clear the
    ///    patrol if any segment intersects an obstacle.
    /// 8. Seed `old_life_points` / `initial_life_points` on enemy AIs
    ///    for the "still has his initial HP" check.  Difficulty-based
    ///    life-point scaling is already applied at entity-spawn time
    ///    in `engine::level_loading::spawn_soldier`, so we just
    ///    snapshot the current value here.
    /// 9. Fill this enemy's `ambush_point_status` vector with
    ///    `Far` × `ambush_points_count` so ambush-point updates
    ///    has a slot per global ambush point.
    /// 10. Execute authored state transitions and duty with live callbacks.
    fn init_one_ai(
        &mut self,
        tcx: TickCtx<'_>,
        npc_id: EntityId,
        hiking_paths: &std::sync::Arc<Vec<crate::level_data::RawHikingPath>>,
        ambush_points_count: usize,
        all_soldier_entity_ids: &[EntityId],
        soldier_subordinate_ids: &[Vec<u16>],
    ) {
        // -- Phase 1: Peek at the entity to classify (enemy / friendly,
        //    camp) and read the fields we need for the obstacle fix. --
        let (is_enemy, is_friendly, self_camp, move_box_opt) = {
            let Some(entity) = self.entities().get(npc_id) else {
                return;
            };
            let (is_enemy, is_friendly, self_camp) = match entity {
                Entity::Pc(pc) => (
                    pc.pc
                        .ai
                        .as_deref()
                        .is_some_and(|ai| ai.ai_brain.enemy().is_some()),
                    false,
                    pc.pc.cached_camp,
                ),
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
            let move_box = entity
                .actor_data()
                .map(|_| *entity.position_iface().get_move_box());
            (is_enemy, is_friendly, self_camp, move_box)
        };
        if !(is_enemy || is_friendly) {
            return;
        }

        let standard_view_radius = if self.ai.standard_view_polygon_radius > 0 {
            self.ai.standard_view_polygon_radius
        } else {
            ai_vision::DEFAULT_VIEW_RADIUS
        };
        // Patrol admission observes live members before the chief's position
        // correction and state initialization. Later NPCs must already know
        // their chief when their own initialization begins.
        {
            let entity = self
                .entities_mut()
                .get_mut(npc_id)
                .expect("AI initialization owner");
            let direction = entity.element_data().direction();
            if let Some(enemy) = entity.enemy_ai_mut() {
                enemy.old_odds = 50;
            }
            let npc = entity.ai_actor_data_mut().expect("AI initialization actor");
            npc.direction_old = direction;
            npc.view_radius = standard_view_radius;
            npc.view_radius_base = standard_view_radius;
            npc.view_radius_goal = standard_view_radius;
        }
        if is_enemy {
            let ai = self.ai_mut(npc_id, "patrol initialization owner");
            // Resolve patrol member IDs
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
            self.initialize_patrol_for_npc(tcx.assets, npc_id);
        }

        // -- Phase 3: Build the detectable-enemy list for this NPC. --
        let detectables = self
            .entities()
            .humans()
            .filter_map(|(id, entity)| {
                let id: EntityId = id.into();
                if id == npc_id {
                    return None;
                }
                let (is_pc, is_soldier, camp) = match entity {
                    Entity::Pc(pc) => (true, false, pc.pc.cached_camp),
                    Entity::Soldier(soldier) => (false, true, soldier.soldier.cached_camp),
                    _ => return None,
                };
                crate::ai_detectable_filter::should_add_enemy_detectable_with(
                    &self.mission_domain.diplomacy,
                    self_camp,
                    !is_friendly,
                    is_pc,
                    is_soldier,
                    camp,
                )
                .then_some(Detectable {
                    element: Some(id),
                    detectable_type: DetectableType::Enemy,
                    seen_last_frame: false,
                    heard_last_frame: false,
                    seen_now: false,
                    shadow_seen_now: false,
                    shadow_seen_last_frame: false,
                    last_visibility: 0.0,
                })
            })
            .collect();

        {
            let entity = self
                .entities_mut()
                .expect_entity_mut(npc_id, format_args!("AI initial state owner"));
            let life = entity.human_life_points().clamp(0, 255) as u8;
            entity
                .ai_actor_data_mut()
                .expect("AI initialization actor")
                .detectable_lists[DetectableType::Enemy as usize] = detectables;
            if let Some(enemy) = entity.enemy_ai_mut() {
                enemy.old_life_points = life;
                enemy.initial_life_points = life;
            }
        }
        if is_enemy && self_camp != Camp::Error {
            self.ai.global.soldier_camps.insert(self_camp);
        }
        let state_allows_duty = self.initialize_ai_state(tcx, npc_id);
        let go_to_duty = {
            let ai = self.ai(npc_id, "AI initialization duty gate");
            state_allows_duty && !ai.ai_is_script_locked() && !ai.ai_is_locked()
        };

        // -- Phase 2: Stuck-in-obstacle correction (enemy only). --
        // If the NPC's move-box overlaps the playable area, attempt to
        // push it to an authorized position via `find_authorized_position`.
        if is_enemy && let Some(move_box) = move_box_opt {
            let entity = self
                .entities()
                .expect_entity(npc_id, format_args!("AI bootstrap obstacle owner"));
            let pos_map = entity.element_data().position_map();
            let layer = entity.element_data().layer();
            let mut abs_box = move_box.translated(pos_map);
            if !self.world.fast_grid.is_position_authorized(&abs_box, layer)
                && self
                    .world
                    .fast_grid
                    .find_authorized_position(&mut abs_box, layer)
            {
                let new_center = abs_box.center();
                if let Some(entity) = self.entities_mut().get_mut(npc_id)
                    && entity.actor_data().is_some()
                {
                    let new_center_map = new_center;
                    let pi = entity.position_iface_mut();
                    pi.set_map_position(new_center_map);
                    entity.element_data_mut().set_position_map(new_center_map);
                }
            }
        }

        // -- Phase 4: Re-read entity (post-fix) and mutate all the
        //    per-NPC state fields in one shot. --
        let is_forest_level = self.world.weather.is_forest_level;

        // Determine whether this NPC is a Merry-Man archer (Royalist
        // soldier, forest level, archer flag set by the level loader).
        let is_merry_man_archer = if is_enemy {
            let entity = self.expect_entity(npc_id, "AI initialization Merry-Man archer owner");
            // `is_enemy` was classified from a present enemy brain above.
            let is_archer = entity
                .enemy_ai()
                .expect("AI initialization enemy owner lost its enemy brain")
                .is_archer();
            // AI-driven PCs carry an enemy brain but no soldier data; they are never riders.
            let is_rider = entity.soldier_data().is_some_and(|s| s.rider);
            self.is_player_aligned_camp(self_camp) && is_forest_level && is_archer && !is_rider
        } else {
            false
        };

        // Grab the (possibly corrected) map position / direction /
        // sector / layer before the write-back borrow.
        let (pos_map_final, direction_final, sector_final, layer_final) = {
            let entity = self.expect_entity(npc_id, "AI initialization owner before write-back");
            let elem = entity.element_data();
            (
                elem.position_map(),
                elem.direction(),
                elem.sector(),
                elem.layer(),
            )
        };

        // Write-back block: mutate every field this init pass owns.
        {
            let Some(entity) = self.entities_mut().get_mut(npc_id) else {
                return;
            };
            if let Some(npc) = entity.ai_actor_data_mut() {
                if is_merry_man_archer {
                    // Seed the bow ammo for forest-level Merry Man archers.
                    npc.number_of_arrows = MERRY_MAN_ARROWS;
                }

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
        }

        // -- Phase 5: Patrol path init + TestIfPathIsFine. --
        // Initialize the path from path_id, then test it; on failure,
        // assert in debug and silently clear in release.
        let patrol_path_opt = {
            let Some(entity) = self.entities().get(npc_id) else {
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

        {
            let Some(entity) = self.entities_mut().get_mut(npc_id) else {
                return;
            };
            if let Some(ai) = entity.ai_controller_mut() {
                ai.initial_position = crate::ai::Position {
                    x: pos_map_final.x,
                    y: pos_map_final.y,
                    sector: sector_final,
                    level: layer_final,
                };
                // Initial-position setup stores a direction vector
                // from the direction sector (default aspect 1),
                // while return-to-post later faces that vector, selecting
                // a sector with the isometric aspect ratio. Those two
                // operations deliberately do not round-trip diagonal body
                // sectors (for example body sector 2 becomes view sector 1).
                // Keep the controller's discrete cache equal to that later
                // facing result; storing the body sector directly made an NPC
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
            }
        }

        if is_friendly {
            let entity = self
                .entities_mut()
                .expect_entity_mut(npc_id, format_args!("civilian bootstrap owner"));
            let is_beggar = matches!(&*entity, Entity::Civilian(civilian)
                if civilian.civilian.cached_civilian_type == crate::profiles::CivilianType::Beggar);
            let friendly = entity.friendly_ai_mut().expect("civilian bootstrap brain");
            friendly.wants_to_talk = false;
            if is_beggar {
                friendly
                    .base
                    .non_script_lock(crate::ai::AiLockFlags::BEGGAR);
            }
        }
        let has_path = {
            let ai = self.ai_mut(npc_id, "AI bootstrap path owner");
            let has_path = ai.has_patrol_path && (!is_friendly || !ai.ai_is_locked());
            ai.has_patrol_path = has_path;
            if has_path {
                ai.substate_at_last_timer_launch = ai.current_substate;
            }
            has_path
        };
        if has_path && go_to_duty {
            if is_enemy {
                self.duty_set_state(
                    tcx,
                    npc_id,
                    crate::ai::AiState::Default,
                    crate::ai::Substate::DefaultEnroute,
                );
            }
            self.execute_ai_return_to_duty(tcx, npc_id, crate::ai::DutyFlags::empty());
        } else if is_friendly && go_to_duty {
            let duration = crate::parameters_ai::AB_MIN_DEFAULT_LOOK_TIME
                + crate::sim_rng::i32(
                    tcx.sim,
                    crate::sim_rng::RngSite::CivilianFirstLookTimer,
                    0..crate::parameters_ai::AB_DELTA_DEFAULT_LOOK_TIME,
                );
            let frame = self.control.frame_counter;
            self.ai_mut(npc_id, "civilian bootstrap timer")
                .launch_timer(duration as u32, frame);
            self.duty_set_state(
                tcx,
                npc_id,
                crate::ai::AiState::Default,
                crate::ai::Substate::DefaultOnPost,
            );
            let ai = self.ai_mut(npc_id, "civilian bootstrap timer state");
            ai.substate_at_last_timer_launch = ai.current_substate;
        }
        let frame = self.control.frame_counter;
        let entity = self
            .entities_mut()
            .expect_entity_mut(npc_id, format_args!("AI bootstrap completion"));
        if let Some(enemy) = entity.enemy_ai_mut() {
            enemy.ambush_point_array_reset = true;
            enemy.ambush_point_status.clear();
            enemy
                .ambush_point_status
                .resize(ambush_points_count, crate::ai_enemy::AmbushPointStatus::Far);
        }
        entity
            .ai_controller_mut()
            .expect("AI bootstrap controller")
            .last_hint_actuality = frame;
    }

    fn initialize_ai_state(&mut self, tcx: TickCtx<'_>, owner: EntityId) -> bool {
        use crate::ai::{AiState, EmoticonType, Substate};
        use crate::element::{ActionState, EyeStatus, Posture};
        use crate::order::OrderType;

        let in_building = self
            .entity_building_sector(
                self.entities()
                    .expect_entity(owner, format_args!("initial AI building owner"))
                    .element_data()
                    .sector(),
            )
            .is_some();
        let initial_action = {
            let ai = self.ai_mut(owner, "initial AI state");
            ai.likes_to_sit_around = false;
            ai.special_action = false;
            ai.is_stay_at_home = in_building;
            ai.initial_action
        };
        if in_building {
            self.duty_set_state(tcx, owner, AiState::Default, Substate::DefaultHomeSweetHome);
            return false;
        }
        let action = OrderType::try_from(initial_action).ok();
        let (state, substate) = match action {
            Some(OrderType::SleepingUpright) => (AiState::Sleeping, Substate::SleepingNapping),
            Some(OrderType::BeingDead | OrderType::BeingDeadFallenBack) => {
                (AiState::Sleeping, Substate::SleepingForever)
            }
            Some(OrderType::BeingUnconscious) => (AiState::Sleeping, Substate::SleepingUnconscious),
            _ => (AiState::Default, Substate::DefaultOnPost),
        };
        self.duty_set_state(tcx, owner, state, substate);
        let posture = match action {
            Some(OrderType::SleepingUpright) => Some(Posture::Upright),
            Some(OrderType::Sitting) => Some(Posture::Sitting),
            Some(OrderType::BeingDeadFallenBack) => Some(Posture::DeadBack),
            Some(OrderType::BeingDead) => Some(Posture::Dead),
            Some(OrderType::BeingUnconscious) => Some(Posture::Lying),
            Some(OrderType::Special) => Some(Posture::Leisure),
            _ => None,
        };
        if posture.is_none() || action == Some(OrderType::Sitting) {
            let bored = self.ai_bored_time(tcx, owner);
            let frame = self.control.frame_counter;
            let ai = self.ai_mut(owner, "initial AI bored timer");
            ai.launch_timer(bored as u32, frame);
        }
        let Some(posture) = posture else {
            if !matches!(
                action,
                Some(
                    OrderType::WaitingUpright
                        | OrderType::WaitingUprightBored
                        | OrderType::WaitingUprightBoredRandom
                )
            ) {
                tracing::warn!(npc = ?owner, initial_action, "Unsupported initial AI action; using the default post state");
            }
            return true;
        };
        {
            let entity = self
                .entities_mut()
                .expect_entity_mut(owner, format_args!("initial AI posture owner"));
            if action == Some(OrderType::SleepingUpright) {
                crate::ai_vision::set_view_status(
                    entity.ai_actor_data_mut().expect("initial AI vision"),
                    EyeStatus::Closed,
                );
            }
            if matches!(
                action,
                Some(OrderType::BeingDead | OrderType::BeingDeadFallenBack)
            ) {
                match entity {
                    Entity::Pc(pc) => pc.pc.life_points = 0,
                    Entity::Soldier(soldier) => soldier.npc.life_points = 0,
                    Entity::Civilian(civilian) => civilian.npc.life_points = 0,
                    _ => panic!("initial AI state requires a human"),
                }
            }
            if action == Some(OrderType::BeingUnconscious) {
                let human = entity
                    .human_data_mut()
                    .expect("initial AI unconscious human");
                human.concussion_of_the_brain = crate::combat::CONCUSSION_MAX;
                human.unconscious = true;
            }
            entity.set_posture(posture);
            entity
                .actor_data_mut()
                .expect("initial AI actor")
                .action_state = if action == Some(OrderType::SleepingUpright) {
                ActionState::Sleeping
            } else {
                ActionState::Waiting
            };
        }
        self.actor_wait(tcx, owner);

        let entity = self
            .entities_mut()
            .expect_entity_mut(owner, format_args!("initial AI state completion"));
        match action {
            Some(OrderType::SleepingUpright) => entity
                .ai_controller_mut()
                .expect("initial AI sleeping controller")
                .set_emoticon(EmoticonType::Zzz),
            Some(OrderType::Sitting) => {
                entity
                    .ai_controller_mut()
                    .expect("initial AI sitting controller")
                    .likes_to_sit_around = true
            }
            Some(OrderType::Special) => {
                entity
                    .ai_controller_mut()
                    .expect("initial AI leisure controller")
                    .special_action = true
            }
            Some(OrderType::BeingDead | OrderType::BeingDeadFallenBack) => {
                entity
                    .human_data_mut()
                    .expect("initial AI dead human")
                    .killed_by_accident = true
            }
            _ => {}
        }

        false
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

        // Include every building's doors, including initially empty houses.
        // AI initialization walks every gate owned by
        // each building when it builds rally points, and
        // Door-battle initialization walks that same gate list. This includes a
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
                    door_index: crate::gate::DoorIndex::new(idx as u32).expect("valid door index"),
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

        for (entity_id, entity) in self.entities().actors() {
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
                if let Some(entity) = self.entities_mut().get_mut(eid)
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
            // default building state.
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
