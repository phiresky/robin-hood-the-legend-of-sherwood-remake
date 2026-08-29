use super::*;

impl EngineInner {
    // ─── Patrol coordination ───────────────────────────────────

    /// Close `CMD_PATROL_DIRECTION` at the macro owner's synchronous engine
    /// boundary. Original iterates the live patrol immediately and each
    /// waiting member may `FaceTo` before the macro advances.
    pub(in crate::engine) fn drain_patrol_direction_broadcast_for(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        owner: EntityId,
        assets: &LevelAssets,
    ) {
        let (direction, members) = {
            let Some(entity) = self.world.entities.get_mut(owner) else {
                return;
            };
            let Some(ai) = entity.ai_controller_mut() else {
                return;
            };
            let Some(direction) = ai.outbox.patrol.direction_broadcast.take() else {
                return;
            };
            (direction, ai.patrol.clone())
        };
        let scratch = self.build_owner_context_scratch_without_forecast(assets);
        for member in members {
            let in_uninterruptible_command = self.is_very_very_busy(member);
            let building_sector = self
                .world
                .entities
                .get(member)
                .map(|entity| self.entity_building_sector(entity.element_data().sector()))
                .unwrap_or_else(|| {
                    panic!(
                        "patrol direction owner {} references missing member {}",
                        owner.index(),
                        member.index()
                    )
                });
            let entity = self.world.entities.get_mut(member).unwrap_or_else(|| {
                panic!(
                    "patrol direction owner {} lost member {}",
                    owner.index(),
                    member.index()
                )
            });
            let mut ctx = build_ai_context_from_entity(
                entity,
                self.control.frame_counter,
                building_sector,
                self.world.weather.is_forest_level,
                self.world.weather.ambiance,
                self.ai.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &assets.hiking_waypoint_sectors,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            ctx.in_uninterruptible_command = in_uninterruptible_command;
            entity
                .ai_controller_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "patrol direction member {} lost AI for owner {}",
                        member.index(),
                        owner.index()
                    )
                })
                .set_instructed_patrol_direction(direction, &ctx);
            // GetInstructedPatrolDirection invokes FaceTo synchronously, but
            // FaceTo only registers its Turn element with RHSequenceManager.
            // A patrol direction broadcast can run from the chief's actor
            // slot, after RHSequenceManager::Hourglass has already run for
            // this frame. Close the member's AI side effects now while
            // leaving the registered Turn uninstructed until the next
            // sequence-manager pass.
            self.drain_direct_ai_owner_boundary_without_forecast_deferred_instruct(
                sim, member, assets,
            );
        }
    }

    /// Per-frame patrol coordination tick.
    ///
    /// The chief-side patrol management of the base AI class:
    /// 1. **`initialize_patrol`** — build active patrol from
    ///    theoretical members (check state, sort by distance,
    ///    pair-swap) on the `needs_patrol_reinit` one-shot flag.
    /// 2. **`refresh_patrol`** — every frame record chief history,
    ///    every 8th frame compute formation positions and dispatch
    ///    `CALL_PATROL_COORDINATE` to each minion.
    ///
    /// `transform_patrol_ids_to_real_patrol` is no longer part of
    /// this tick — it lives in `EngineInner::init_one_ai`, invoked
    /// once at AI bootstrap.
    pub(in crate::engine) fn tick_patrol_coordination_for_npc(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        positions_before_movement: &EntitySlots<Option<crate::entities::BoundaryPosition>>,
    ) {
        use crate::ai::{AiState, Position, Stimulus, StimulusType, Substate};

        if self.actors_frozen() {
            return;
        }
        let has_patrol_work = self
            .world
            .entities
            .get(owner)
            .and_then(Entity::ai_controller)
            .is_some_and(|ai| {
                ai.needs_patrol_reinit
                    || !ai.patrol.is_empty()
                    || !ai.missed_patrol_members.is_empty()
            });
        if !has_patrol_work {
            return;
        }
        if self.is_very_very_busy(owner) {
            return;
        }
        let scratch = self.build_owner_context_scratch_at_slot_without_forecast(
            assets,
            owner,
            positions_before_movement,
            false,
        );

        let frame = self.control.frame_counter;
        let all_npc_ids: Vec<_> = self.world.entities.ai_owner_ids().collect();
        let npc_ids = [owner];

        // ── Phase 2: Snapshot NPC states ──
        // Needed for patrol initialization and missed-member checks.
        #[derive(Clone, Copy)]
        struct NpcSnap {
            position: Position,
            detection_position_world: crate::coordinates::WorldPoint3D,
            direction: u16,
            ground_z: f32,
            posture: crate::element::Posture,
            is_rider: bool,
            in_building: bool,
            ai_state: AiState,
            is_alive: bool,
            is_active: bool,
            real_view_radius: u16,
            move_box: crate::coordinates::MoveBox,
            // Missed-member reacquisition calls virtual `IsAbleToHelp`.
            // Civilians inherit the Human default (`false`); soldiers use
            // their state/substate-aware override.
            is_able_to_help: bool,
            // Patrol admit gate (`initialize_patrol`):
            // `is_civilian() || is_able_to_fight()`.
            is_civilian: bool,
            is_able_to_fight: bool,
        }
        let mut snaps: std::collections::HashMap<EntityId, NpcSnap> =
            std::collections::HashMap::new();
        for &npc_id in &all_npc_ids {
            let Some(entity) = self.world.entities.get(npc_id) else {
                continue;
            };
            // Every position read in `RefreshPatrol` goes through the AI
            // `Position(actor)` helper. In particular, a member whose current
            // sequence command is PassDoor reports the committed gate side,
            // not its interpolating sprite position. The owner-slot view also
            // preserves creation-order map positions for ordinary actors.
            let view = scratch
                .ai_entity_views
                .get(&npc_id.index())
                .unwrap_or_else(|| {
                    panic!(
                        "patrol owner {} is missing AI position view for NPC {}",
                        owner.index(),
                        npc_id.index()
                    )
                });
            let position = view.position;
            let detection_position_world = view.detection_position_world;
            let dir = entity.element_data().direction();
            let npc = entity.ai_actor_data().unwrap_or_else(|| {
                panic!(
                    "patrol owner {} found AI-owner slot {} without AI actor data",
                    owner.index(),
                    npc_id.index()
                )
            });
            let ai_state = npc.ai_state();
            // IsDetecting360Degrees uses the post-RefreshView real radius,
            // which is the growing/goal radius already multiplied by the
            // long-range, stare/follow, rider and drunkenness factors. Using
            // the pre-factor base radius loses every member a staring chief
            // can still feel.
            let real_view_radius = npc.view_radius;
            let move_box = *entity.position_iface().get_move_box();
            let is_civilian = entity.is_civilian();
            let is_able_to_help = match entity {
                crate::element::Entity::Soldier(soldier) => {
                    crate::ai_enemy::soldier_is_able_to_help_state(
                        !entity.is_dead() && !soldier.human.unconscious,
                        ai_state,
                        npc.ai_substate(),
                    )
                }
                _ => false,
            };
            let is_able_to_fight = match entity {
                crate::element::Entity::Soldier(s) => {
                    use crate::element::Human as _;
                    s.is_able_to_fight()
                }
                crate::element::Entity::Pc(pc) => {
                    use crate::element::Human as _;
                    pc.is_able_to_fight()
                }
                // Civilians, props, etc.: the default
                // `is_able_to_fight()` is `false` — but civilians flow
                // through the `is_civilian()` arm of the patrol gate
                // instead.
                _ => false,
            };

            snaps.insert(
                npc_id,
                NpcSnap {
                    position: Position {
                        x: position.x,
                        y: position.y,
                        sector: position.sector,
                        level: position.level,
                    },
                    detection_position_world,
                    direction: dir as u16,
                    ground_z: entity.element_data().position().z,
                    posture: entity.element_data().posture,
                    is_rider: entity.soldier_data().is_some_and(|soldier| soldier.rider),
                    in_building: self.entity_data_in_building_sector(entity.element_data()),
                    ai_state,
                    is_alive: !entity.is_dead(),
                    is_active: entity.is_active(),
                    real_view_radius,
                    move_box,
                    is_able_to_help,
                    is_civilian,
                    is_able_to_fight,
                },
            );
        }

        // ── Phase 3: Initialize patrols + compute formation positions ──
        struct PatrolCmd {
            minion: EntityId,
            target: Position,
            direction: u16,
        }
        let mut patrol_cmds: Vec<PatrolCmd> = Vec::new();
        let mut chief_assigns: Vec<(EntityId, EntityId)> = Vec::new(); // (minion, chief)

        for &npc_id in &npc_ids {
            // `refresh_patrol`: chiefs in {Flying, OnLadder, OnWall}
            // or mid-{PassDoor, Fall} sequence command skip the
            // entire tick — formation targets would trail an unusable
            // position and the 16-pixel side offset would still get
            // dispatched.  Check before acquiring the entity/ai borrow
            // so the engine-level helper can read `self`.
            let Some(entity) = self.world.entities.get_mut(npc_id) else {
                continue;
            };
            let ai = entity.ai_controller_mut().unwrap_or_else(|| {
                panic!(
                    "patrol owner {} has no required AI controller",
                    npc_id.index()
                )
            });

            // ── Initialize patrol on the one-shot reinit trigger ──
            // `initialize_patrol()` is called explicitly from
            // `init_one_ai`, `return_to_duty`, the `CMD_PATROL_START`
            // macro opcode, and the `Substate::DefaultGotoRoute`
            // EVENT_REACHPOINT handler — all of which set
            // `needs_patrol_reinit` on the chief.  Switching on the
            // flag (instead of "both lists empty") prevents a chief
            // whose minions all died/were promoted out from silently
            // re-initialising every tick — such chiefs stay in the
            // `patrol_size == 0 && missed == 0` early-return.  When
            // the flag fires we clear `patrol` and
            // `missed_patrol_members` before re-populating from
            // `theoretical_patrol`.
            if ai.needs_patrol_reinit {
                ai.needs_patrol_reinit = false;
                ai.patrol.clear();
                ai.missed_patrol_members.clear();
                let theoretical = ai.theoretical_patrol.clone();
                let chief_snap = snaps.get(&npc_id).copied().unwrap_or_else(|| {
                    panic!(
                        "patrol owner {} is missing its owner-boundary position snapshot",
                        npc_id.index()
                    )
                });
                let chief_pos = chief_snap.position;
                let obstacles_owned = scratch.ai_sight_obstacles.clone();
                let obstacles = obstacles_owned.list();

                for &member in &theoretical {
                    if member == npc_id {
                        continue;
                    }
                    if let Some(snap) = snaps.get(&member) {
                        // `initialize_patrol`: admit only if
                        // `is_detecting_360_degrees(member) &&
                        // ai_state == Default && (is_civilian() ||
                        // is_able_to_fight())`.  Members failing the
                        // gate but still alive flow into the missed
                        // list for later re-acquisition.
                        // `IsDetecting360Degrees(RHElementActorHuman*)`
                        // is the first operand in Original's admission chain.
                        // It uses the chief's upright eye point and the
                        // member's posture-dependent detection point for both
                        // its 3-D distance and opaque-obstacle ray. Preserve
                        // that call before the state/fighting predicates so
                        // rejected active members still produce the same LOS.
                        let admit = patrol_member_admitted(
                            chief_snap.is_active && snap.is_active,
                            || {
                                patrol_member_visible_from_raw_world(
                                    chief_snap.detection_position_world,
                                    chief_snap.is_rider,
                                    chief_snap.real_view_radius,
                                    chief_snap.in_building,
                                    snap.detection_position_world,
                                    snap.posture,
                                    snap.is_rider,
                                    snap.direction as i16,
                                    snap.in_building,
                                    obstacles,
                                )
                            },
                            snap.ai_state,
                            snap.is_civilian,
                            snap.is_able_to_fight,
                        );
                        if admit {
                            ai.patrol.push(member);
                            chief_assigns.push((member, npc_id));
                        } else if snap.is_alive {
                            ai.missed_patrol_members.push(member);
                        }
                    }
                }

                // `InitializePatrol` orders members with
                // `SquareDistance`: subtract the full 3-D ground
                // positions, stretch world Y by the inverse isometric
                // aspect ratio, then take the squared norm.  Map Y is
                // `world_y - z`, so reconstruct world Y before taking
                // the delta.
                //
                // Preserve the Original's insertion semantics too:
                // it advances only while `new_distance > old_distance`,
                // so a tie is inserted before existing entries.
                let snap_ref = &snaps;
                let patrol_distance = |member: EntityId| {
                    let snap = snap_ref.get(&member).unwrap_or_else(|| {
                        panic!(
                            "patrol member {} is missing its owner-boundary snapshot",
                            member.index()
                        )
                    });
                    let dx = snap.position.x - chief_pos.x;
                    let dy_world =
                        (snap.position.y + snap.ground_z) - (chief_pos.y + chief_snap.ground_z);
                    let dy = dy_world * crate::position_interface::INVERSE_ASPECT_RATIO;
                    let dz = snap.ground_z - chief_snap.ground_z;
                    dx * dx + dy * dy + dz * dz
                };
                let mut sorted_patrol = Vec::with_capacity(ai.patrol.len());
                for member in std::mem::take(&mut ai.patrol) {
                    let distance = patrol_distance(member);
                    let insert_at = sorted_patrol
                        .iter()
                        .position(|&existing| {
                            patrol_distance_inserts_before(distance, patrol_distance(existing))
                        })
                        .unwrap_or(sorted_patrol.len());
                    sorted_patrol.insert(insert_at, member);
                }
                ai.patrol = sorted_patrol;

                // Arrange left/right pairs: for each pair, ensure
                // even-index member is to the left of the odd-index
                // one (relative to chief).  Uses a 2D determinant.
                let patrol_size = ai.patrol.len();
                for i in (1..patrol_size).step_by(2) {
                    let even_h = ai.patrol[i - 1];
                    let odd_h = ai.patrol[i];
                    if let (Some(even_s), Some(odd_s)) =
                        (snap_ref.get(&even_h), snap_ref.get(&odd_h))
                    {
                        let ex = even_s.position.x - chief_pos.x;
                        let ey = even_s.position.y - chief_pos.y;
                        let ox = odd_s.position.x - chief_pos.x;
                        let oy = odd_s.position.y - chief_pos.y;
                        // 2D determinant: if even is on the wrong side, swap
                        if ex * oy - ey * ox < 0.0 {
                            ai.patrol.swap(i - 1, i);
                        }
                    }
                }
            }

            // ── Refresh patrol positions ──
            let patrol_size = ai.patrol.len();
            if patrol_size == 0 && ai.missed_patrol_members.is_empty() {
                continue;
            }
            if ai.patrol_stopped {
                continue;
            }
            if ai.current_state != AiState::Default {
                continue;
            }
            if ai.current_substate == Substate::DefaultPatrolChiefReturnToPatrol {
                continue;
            }

            // Must have a patrol path to track history
            let Some(ref mut path) = ai.patrol_path else {
                continue;
            };

            // Record history entry every frame
            if let Some(snap) = snaps.get(&npc_id) {
                path.add_history_entry(snap.position, snap.direction as u8);
            }

            // Every 8th frame: compute positions and coordinate minions
            if (frame & 7) != 0 {
                continue;
            }

            {
                // The Original calls ComputePatrolPositions even with zero
                // active members.  Its post-loop cleanup then discards every
                // history entry except the newest one before missed members
                // are considered for re-acquisition below.
                // Expand the chief's move box by 3 on each side
                // before feeding it to
                // `is_straight_movement_autorized` for the 3-step
                // side-offset fallback.
                let chief_box = match snaps.get(&npc_id).map(|s| s.move_box) {
                    Some(b) if b.is_somewhere() => crate::coordinates::MoveBox::from_coords(
                        b.x_min() - 3.0,
                        b.y_min() - 3.0,
                        b.x_max() + 3.0,
                        b.y_max() + 3.0,
                    ),
                    _ => crate::coordinates::MoveBox::new(),
                };
                let positions = path.compute_patrol_positions(
                    patrol_size,
                    Some(&self.world.fast_grid),
                    &chief_box,
                );
                let patrol_members = ai.patrol.clone();

                for (i, &member) in patrol_members.iter().enumerate() {
                    if let Some(&(ref pos, dir)) = positions.get(i) {
                        // Only coordinate if member is far enough from target (MaxNorm > 3)
                        if let Some(member_snap) = snaps.get(&member) {
                            let dx = (member_snap.position.x - pos.x).abs();
                            let dy = (member_snap.position.y - pos.y).abs();
                            if dx.max(dy) > 3.0 {
                                patrol_cmds.push(PatrolCmd {
                                    minion: member,
                                    target: *pos,
                                    direction: dir,
                                });
                            }
                        }
                    }
                }
            }

            // Check missed patrol members for re-acquisition.
            // `is_detecting_360_degrees`: isometric squared distance
            // check (Y stretched by INVERSE_ASPECT_RATIO) plus the
            // `FastFindGrid::is_reachable(OPAQUE)` LOS gate — a
            // separated minion behind a wall must NOT re-join even
            // within view radius.
            let chief_snap = snaps.get(&npc_id).copied();
            let missed = ai.missed_patrol_members.clone();
            let mut reacquired = Vec::new();
            let obstacles_owned = scratch.ai_sight_obstacles.clone();
            let obstacles = obstacles_owned.list();
            for (i, &member) in missed.iter().enumerate() {
                if let (Some(chief_s), Some(member_s)) = (chief_snap, snaps.get(&member))
                    && missed_patrol_member_reacquired(
                        chief_s.is_active && member_s.is_active,
                        || {
                            patrol_member_visible_from_raw_world(
                                chief_s.detection_position_world,
                                chief_s.is_rider,
                                chief_s.real_view_radius,
                                chief_s.in_building,
                                member_s.detection_position_world,
                                member_s.posture,
                                member_s.is_rider,
                                member_s.direction as i16,
                                member_s.in_building,
                                obstacles,
                            )
                        },
                        member_s.is_able_to_help,
                        member_s.ai_state,
                    )
                {
                    reacquired.push(i);
                    ai.patrol.push(member);
                    chief_assigns.push((member, npc_id));
                }
            }
            for &i in reacquired.iter().rev() {
                ai.missed_patrol_members.remove(i);
            }
        }

        // ── Phase 4: Set patrol_chief on minions ──
        for (minion, chief) in chief_assigns {
            if let Some(entity) = self.world.entities.get_mut(minion)
                && let Some(ai) = entity.ai_controller_mut()
            {
                ai.patrol_chief = Some(chief);
            }
        }

        // ── Phase 5: Build per-minion patrol tick data map ──
        // Build a map of minion → (chief_position, chief_state) for use
        // in the coordinate dispatch below.
        let mut patrol_tick_map: std::collections::HashMap<
            EntityId,
            (crate::ai::Position, crate::ai::AiState),
        > = std::collections::HashMap::new();
        for cmd in &patrol_cmds {
            let minion_id = cmd.minion;
            let Some(entity) = self.world.entities.get(minion_id) else {
                continue;
            };
            let Some(ai) = entity.ai_controller() else {
                continue;
            };
            if let Some(chief_id) = ai.patrol_chief
                && let Some(cs) = snaps.get(&chief_id)
            {
                patrol_tick_map.insert(minion_id, (cs.position, cs.ai_state));
            }
        }

        // ── Phase 6: Dispatch CALL_PATROL_COORDINATE to minions ──
        let patrol_frame = self.control.frame_counter;
        for cmd in patrol_cmds {
            let minion_id = cmd.minion;
            let ctx = {
                let Some(entity) = self.world.entities.get_mut(minion_id) else {
                    continue;
                };

                build_ai_context_from_entity(
                    entity,
                    patrol_frame,
                    None,
                    self.world.weather.is_forest_level,
                    self.world.weather.ambiance,
                    self.ai.standard_view_polygon_radius,
                    &scratch.ai_entity_views,
                    &scratch.ai_sight_obstacles,
                    &self.world.fast_grid,
                    &assets.hiking_paths,
                    &assets.hiking_waypoint_sectors,
                    &self.ai.global.all_soldier_handles,
                    self.control.sim_config.difficulty,
                )
            };

            // Build tick data with patrol chief info.  Use the
            // centralized builder so combat-path fields stay
            // populated — patrol minions can be alerted mid-patrol
            // and dispatched into battle decisions without losing
            // their primary target snapshot.
            let mut tick_data = self.build_npc_tick_data(sim, minion_id, &scratch, assets);
            if let Some(&(chief_pos, chief_state)) = patrol_tick_map.get(&cmd.minion) {
                tick_data.patrol_chief_position = chief_pos;
                tick_data.patrol_chief_state = chief_state;
            }

            // Dispatch CALL_PATROL_COORDINATE through the script filter.
            let stimulus = Stimulus::with_position(StimulusType::CallPatrolCoordinate, cmd.target);
            self.debug_patrol_turn_lifecycle("before_coordinate_think", minion_id);
            self.dispatch_think_with_drain_mode(
                sim, minion_id, &stimulus, &ctx, &tick_data, assets, true, true,
            );
            // `CoordinatePatrol` constructs its Move element inline in the
            // original, making `GetCommand()` report MOVE_OK immediately.
            // Owner instruction still belongs to the sequence-manager phase
            // later this hourglass, so promote the request to an element but
            // deliberately leave its deferred InstructOwner action queued.
            self.drain_pending_move_requests_for_owner(sim, minion_id);
            self.debug_patrol_turn_lifecycle("after_coordinate_think", minion_id);

            // Original applies the instructed direction only after the
            // member's synchronous CALL_PATROL_COORDINATE Think returns.
            let in_uninterruptible_command = self.is_very_very_busy(minion_id);
            let building_sector = self
                .world
                .entities
                .get(minion_id)
                .and_then(|entity| self.entity_building_sector(entity.element_data().sector()));
            let entity = self.world.entities.get_mut(minion_id).unwrap_or_else(|| {
                panic!(
                    "patrol chief {} lost member {} after coordinate Think",
                    owner.index(),
                    minion_id.index()
                )
            });
            let mut live_ctx = build_ai_context_from_entity(
                entity,
                patrol_frame,
                building_sector,
                self.world.weather.is_forest_level,
                self.world.weather.ambiance,
                self.ai.standard_view_polygon_radius,
                &scratch.ai_entity_views,
                &scratch.ai_sight_obstacles,
                &self.world.fast_grid,
                &assets.hiking_paths,
                &assets.hiking_waypoint_sectors,
                &self.ai.global.all_soldier_handles,
                self.control.sim_config.difficulty,
            );
            live_ctx.in_uninterruptible_command = in_uninterruptible_command;
            entity
                .ai_controller_mut()
                .unwrap_or_else(|| {
                    panic!(
                        "patrol member {} lost AI after coordinate Think from chief {}",
                        minion_id.index(),
                        owner.index()
                    )
                })
                .set_instructed_patrol_direction(cmd.direction, &live_ctx);
            self.debug_patrol_turn_lifecycle("after_instructed_direction_emit", minion_id);
            // GetInstructedPatrolDirection may synchronously FaceTo when the
            // member is still waiting. Close its AI/callback work before the
            // chief advances, but leave owner instruction to the later
            // SequenceManager hourglass just like the Original.
            self.drain_direct_ai_owner_boundary_without_forecast_deferred_instruct(
                sim, minion_id, assets,
            );
            self.debug_patrol_turn_lifecycle("after_instructed_direction_drain", minion_id);
        }
    }
}
