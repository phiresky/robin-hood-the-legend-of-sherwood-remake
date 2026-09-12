use super::*;

impl EnemyAi {
    /// Returns true when this NPC is a royalist foot-soldier on a forest
    /// (Sherwood) level — gates several special behaviours: no tying,
    /// archer forest retreats, 180° vision cone, fast
    /// reaction time.
    pub(super) fn is_merry_man_forest(&self, ctx: &AiContext) -> bool {
        ctx.is_player_aligned() && ctx.is_forest_level && !ctx.self_is_rider
    }

    /// Returns true if any same-camp soldier (other than us) is currently
    /// in a take-money or fight-for-money substate (minus the reaction-time
    /// intro arm) and is detected by my 180° cone.
    pub(super) fn there_is_another_guy_in_sight_approaching_to_money(
        &self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        tick.camp_soldiers.iter().any(|s| {
            s.handle != self.base.me
                && (s.ai_substate.is_take_money() || s.ai_substate.is_fight_for_money())
                && s.ai_substate != Substate::WonderingMoneyReactiontime
                && self.is_detecting_180_degrees(s.handle as HumanHandle, ctx)
        })
    }

    /// A merry man in the forest flees to the nearest reinforcement door
    /// (map exit). Returns `true` if an exit was found and the flee state
    /// was set, `false` if no exit is available.
    pub(super) fn merry_man_forest_cassos(
        &mut self,
        ctx: &AiContext,
        global: &AiGlobalState,
    ) -> bool {
        // Find nearest reinforcement door
        let my_pos = &ctx.position;
        let mut min_dist = f32::MAX;
        let mut best_door_idx: Option<usize> = None;

        for (i, door) in global.reinforcement_doors.iter().enumerate() {
            let dx = my_pos.x - door.position_in.x;
            let dy = my_pos.y - door.position_in.y;
            let dist = dx.abs().max(dy.abs()); // Maximum norm
            if dist < min_dist {
                min_dist = dist;
                best_door_idx = Some(i);
            }
        }

        let Some(idx) = best_door_idx else {
            // No way out!
            return false;
        };

        let door = &global.reinforcement_doors[idx];
        let door_pos = door.position_in;

        // Store the chosen door's canonical global index for exit-point
        // movement later.  We use the global door index (not the
        // position into `reinforcement_doors`) so a single
        // `my_door_index` semantics — a global door-table index — is
        // shared between merry-man flee, RunAndAlertSoldiers, and the
        // AlertSoldiers indoor formation flow.
        self.base.my_door_index = Some(door.door_index);

        // State change, movement, and timer launch first. The `couldnt_reachpoint`
        // check is deliberately *after* the movement request so that any prior-tick
        // reachpoint failure is cleared by `go_to` on entry, and only
        // a synchronously-raised failure on the fresh movement request bails the
        // routine.
        self.go_to(
            AiState::Fleeing,
            Substate::FleeingMerryManRunToLeaveMap,
            door_pos,
            crate::ai::GotoFlags::RUN,
            ctx,
        );
        self.base.launch_timer(30, ctx.frame);

        // Only the just-issued movement request's out-of-bounds / null-sector
        // failure should bail.
        if self.base.couldnt_reachpoint {
            self.base.couldnt_reachpoint = false;
            return false;
        }

        true
    }

    pub(super) fn forget_attentive_mode(&mut self) {
        self.attentive = false;
        self.will_be_attentive = false;
        if let Some(request) = self.base.outbox.actor.last_pending_attentive_mode_mut() {
            // The state change set attentive mode synchronously before the
            // Attentive mode has ended. Preserve the transition
            // launch, then restore this helper's later flag writes after the
            // deferred engine-side request settles.
            request.forget_after = true;
        }
    }

    /// Two-step purge:
    ///
    /// 1. Walk our `DETECTABLE_OBJECT` list and drop every coin entry
    ///    within maximum-norm distance 500 of `pos` so the soldier doesn't
    ///    immediately re-spot the same drops on the next perception
    ///    pass.  This is queued as a pending engine request because
    ///    the AI side keeps no copy of `detectable_lists`.
    /// 2. Clear the parallel `other_seen_money` list.
    pub(super) fn forget_all_nearby_coins(&mut self, ctx: &AiContext) {
        self.base.outbox.actor.forget_nearby_coins = Some(ctx.position);
        self.other_seen_money.clear();
    }

    /// Drops entries from `other_seen_money` whose referenced object
    /// is no longer active, then clears `interesting_object` if it
    /// now points at an inactive coin.
    ///
    /// An inactive entity is absent from `AiContext::entity_views`,
    /// so the filter is `entity_position(handle).is_some()`.
    pub(super) fn clean_up_list_of_seen_money(&mut self, ctx: &AiContext) {
        self.other_seen_money
            .retain(|handle| ctx.entity_position(*handle).is_some());

        if self.base.interesting_object.is_some()
            && ctx.entity_position(self.base.interesting_object).is_none()
        {
            self.base.interesting_object = None;
        }
    }

    /// Tests whether the beer currently held in `interesting_object`
    /// is still reachable and not being claimed by a closer friend.
    ///
    /// Returns `None` when everything is fine (the soldier should keep
    /// approaching / re-arm its poll timer).  Returns `Some(lost_pos)`
    /// when the beer is gone — either because the object became
    /// inactive, or because another friend in an ale-related substate
    /// is approaching the same bottle and is closer than us, or is
    /// already drinking it.  `lost_pos` is the position the caller
    /// should `Face()` before transitioning to `WonderingAleAway`.
    pub(super) fn is_beer_still_available(&self, ctx: &AiContext) -> Option<Position> {
        let Some(interesting) = self.base.interesting_object else {
            // No beer assigned: nothing to check against.  Fall back
            // to the soldier's own position so downstream `Face()` is
            // a no-op rather than pointing at the origin.
            return Some(ctx.position);
        };

        // Object inactive → gone.  An inactive entity is absent from
        // the view map, so we fall back to the soldier's last known
        // seek target (set when it committed to this bottle) for the
        // `look_there_if_not` out-param.
        let Some(obj_pos) = ctx.entity_position(interesting) else {
            return Some(self.base.seek_position);
        };

        // My squared distance to the object.
        let dx = ctx.position.x - obj_pos.x;
        let dy = ctx.position.y - obj_pos.y;
        let my_sq_distance = dx * dx + dy * dy;

        // The original game walks NPC actors by registration order and
        // returns on the first qualifying friend. The shared AI view is a
        // HashMap, so its iteration order cannot decide which friend's
        // position becomes the AleAway facing point (or which LOS query is
        // issued). Restore the NPC registry order through the retained
        // creation ordinal; autonomous PCs and objects are not members of
        // NPC actor collection.
        let mut friends = ctx
            .entity_views
            .iter()
            .filter(|(_, view)| view.is_soldier() || view.is_civilian())
            .collect::<Vec<_>>();
        friends.sort_by_key(|entry| (entry.1.original_creation_order, *entry.0));
        for (&handle, view) in friends {
            if handle == self.base.me {
                continue;
            }
            let beer_away = match view.ai_substate {
                Substate::WonderingApproachingAle | Substate::WonderingAleReactiontime => {
                    if view.interesting_object != Some(interesting) {
                        continue;
                    }
                    if !self.is_detecting_180_degrees(handle, ctx) {
                        continue;
                    }
                    let fx = view.position.x - obj_pos.x;
                    let fy = view.position.y - obj_pos.y;
                    fx * fx + fy * fy < my_sq_distance
                }
                Substate::WonderingDrinkingAle => {
                    view.interesting_object == Some(interesting)
                        && self.is_detecting_180_degrees(handle, ctx)
                }
                _ => continue,
            };
            if beer_away {
                return Some(view.position);
            }
        }

        None
    }

    /// Sweeps inactive entries, then picks the coin with the smallest
    /// Maximum-norm distance to the soldier (with a +300 malus for coins
    /// on a different layer), removes it from `other_seen_money`, and
    /// returns it.  Returns `None` when the list is empty after the
    /// sweep.
    pub(super) fn get_nearest_seen_money_and_remove_it_from_list(
        &mut self,
        ctx: &AiContext,
    ) -> Option<ObjectHandle> {
        self.clean_up_list_of_seen_money(ctx);

        let my_pos = ctx.position;
        let my_layer = my_pos.level;
        let mut best: Option<(usize, u32)> = None;
        for (idx, &handle) in self.other_seen_money.iter().enumerate() {
            let Some(coin_pos) = ctx.entity_position(handle) else {
                continue;
            };
            let dx = (coin_pos.x - my_pos.x).abs();
            let dy = (coin_pos.y - my_pos.y).abs();
            let mut distance = dx.max(dy) as u32;
            if coin_pos.level != my_layer {
                distance = distance.saturating_add(300);
            }
            match best {
                Some((_, best_d)) if distance >= best_d => {}
                _ => best = Some((idx, distance)),
            }
        }

        best.map(|(idx, _)| self.other_seen_money.remove(idx))
    }

    /// Walks same-camp soldiers (via per-tick `camp_soldiers` snapshot),
    /// sends CALL_FINISH_BRAWL to every soldier rank currently in a
    /// take-money / fight-for-money substate within detection range,
    /// stores them in `list_us`, and sets `antagonist` to the first.
    pub(super) fn finish_brawl(&mut self, ctx: &AiContext, tick: &AiPerTickData) {
        debug_assert_eq!(self.get_rank(), ProfileRank::Officer);
        self.base.list_us.clear();
        self.base.antagonist = None;

        // Each `CALL_FINISH_BRAWL` send is gated on 360-degree
        // detection (radius + opaque LOS), computed lazily here only
        // for soldiers passing the cheap rank/substate filter (eager
        // pre-compute was O(N²) per tick).  The brawler is the viewer
        // and the officer the target, so the ray runs soldier→officer.
        let me = &*self;
        let targets: Vec<NpcHandle> = tick
            .camp_soldiers
            .iter()
            .filter(|s| {
                s.rank == ProfileRank::Soldier
                    && (s.ai_substate.is_take_money() || s.ai_substate.is_fight_for_money())
                    && me.is_detected_360_degrees_by(s, ctx)
            })
            .map(|s| s.handle)
            .collect();

        for h in targets {
            self.base.list_us.push(h);
            if self.base.antagonist.is_none() {
                self.base.antagonist = Some(AiEntityHandle::new(h));
            }
            self.base
                .outbox
                .reentrant
                .cross_npc_actions
                .push(CrossNpcAction::SendStimulus {
                    target: h,
                    stimulus_type: StimulusType::CallFinishBrawl,
                    // Send the officer (`me`); receiver reads it as
                    // `stimulus_info.human` for Face/antagonist.
                    info: crate::ai::StimulusInfo::Human(AiEntityHandle::new(self.base.me)),
                    fallback_to_sender: None,
                    to_whole_patrol: false,
                });
        }

        // No `friend_in_trouble` fallback: when the camp-soldier scan
        // finds nothing we leave `antagonist = 0` and skip the
        // Face/Say. This avoids over-broadcasting `CALL_FINISH_BRAWL`
        // and spurious `OfficerEndsBrawl` remarks against a cached
        // friend.

        if self.base.antagonist.is_some() {
            // Face(antagonist); Say(OfficerEndsBrawl, MyTalk1)
            self.base.face_entity(self.base.antagonist, ctx);
            self.base.say(Remark::OfficerEndsBrawl);
        }
    }

    /// Shared helper for `WonderingOfficerApproachingBrawl`: transition
    /// to `FinishingBrawl`, run the brawl walk, set mood, re-arm timer.
    pub(super) fn begin_finishing_brawl(&mut self, ctx: &AiContext, tick: &AiPerTickData) {
        self.set_state(AiState::Wondering, Substate::WonderingOfficerFinishingBrawl);
        self.finish_brawl(ctx, tick);
        self.base.set_emoticon(EmoticonType::Thunderstorm);
        self.base.launch_timer(200, ctx.frame);
    }

    /// Money-fight anti-loop guard: returns true when any same-camp
    /// soldier is currently in one of the
    /// `WonderingOfficer{Seeing,Approaching,Finishing}Brawl` substates
    /// within maximum-norm distance < 150 of the coin. Called before a soldier
    /// commits to picking up a coin so that once an officer has
    /// intervened in a brawl, nearby grabbers back off instead of
    /// re-engaging.
    pub fn is_any_angry_officer_near(&self, pos_money: Position, tick: &AiPerTickData) -> bool {
        for cs in &tick.camp_soldiers {
            match cs.ai_substate {
                Substate::WonderingOfficerSeeingBrawl
                | Substate::WonderingOfficerApproachingBrawl
                | Substate::WonderingOfficerFinishingBrawl => {
                    let dx = (cs.position.x - pos_money.x).abs();
                    let dy = (cs.position.y - pos_money.y).abs();
                    if dx.max(dy) < 150.0 {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Officer-only eligibility predicate for alerting a specific
    /// soldier: rejects the candidate if it belongs to another
    /// officer's patrol (its `PatrolChief` is not me and is within
    /// maximum-norm distance < 700 of the soldier) or if it is already mid-dialogue
    /// with another antagonist.  Called from `alert_soldiers` and
    /// from the EVENT_SEES_SOLDIER officer→soldier arm.
    pub fn can_call_this_soldier(
        &self,
        cs: &CampSoldierInfo,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) -> bool {
        let my_handle = self.base.me;
        let my_id = crate::element::EntityId::Soldier(crate::entity_id::SoldierId(my_handle));

        // Belongs-to-another-patrol gate.
        if let Some(chief_id) = cs.patrol_chief
            && chief_id != my_id
        {
            let chief_pos_opt = tick
                .camp_soldiers
                .iter()
                .find(|o| o.handle == chief_id.index())
                .map(|o| o.position)
                .or_else(|| ctx.entity_view(chief_id.index()).map(|v| v.position));
            if let Some(chief_pos) = chief_pos_opt {
                let ddx = (cs.position.x - chief_pos.x).abs();
                let ddy = (cs.position.y - chief_pos.y).abs();
                if ddx.max(ddy) < 700.0 {
                    return false;
                }
            }
        }

        // In-dialogue-with-someone-else gate.
        !cs.antagonist
            .is_some_and(|antagonist| antagonist.get() != my_handle)
    }

    /// Pops the next queued money-fight victim and approaches it;
    /// returns to duty when the queue drains.  Sets `detected_body`
    /// before going near.
    pub(super) fn awake_next_money_fight_victim_if_any(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        if self.money_fight_victims.is_empty() {
            self.return_to_duty_default(sim, ctx, tick);
            return;
        }
        let next = self.money_fight_victims.remove(0);
        self.base.detected_body = Some(AiEntityHandle::new(next));
        // Enter the wondering / approaching-brawl-victim state.
        self.set_state(
            AiState::Wondering,
            Substate::WonderingApproachingBrawlVictim,
        );
        if let Some(view) = ctx.entity_view(next as HumanHandle) {
            self.base.go_near(
                view.position,
                parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                crate::ai::GotoFlags::empty(),
                ctx,
            );
        }
    }

    /// After a brawl ends the soldier scans its seen-money list for
    /// the nearest still-active coin, runs for it, or falls back to a
    /// left/right scan when nothing remains.
    pub(super) fn stop_brawling_and_collect_money(
        &mut self,
        ctx: &AiContext,
        _tick: &AiPerTickData,
    ) {
        // Clean up seen money, then select and remove the nearest entry.
        if let Some(coin) = self.get_nearest_seen_money_and_remove_it_from_list(ctx) {
            // interesting_object = nearest coin.
            self.base.interesting_object = Some(AiEntityHandle::new(coin));
            if let Some(coin_pos) = ctx.entity_position(coin) {
                // Enter the wondering / running-for-money state and run to an
                // accessible point within AI_STOP_BEFORE_MONEY_DISTANCE of the coin.
                self.go_near(
                    AiState::Wondering,
                    Substate::WonderingRunningForMoney,
                    coin_pos,
                    parameters_ai::AI_STOP_BEFORE_MONEY_DISTANCE,
                    crate::ai::GotoFlags::RUN | crate::ai::GotoFlags::FIND_ACCESSIBLE,
                    ctx,
                );
            }
        } else {
            // No coins left — look around for more.
            self.set_state(AiState::Wondering, Substate::WonderingWatchingForMoreMoney);
            self.base.outbox.actor.look_sidewards = Some(LookDirection::LeftRight);
        }
    }

    /// Rebuilds `money_fight_victims` from the same-camp soldiers
    /// currently unconscious + alive + `was_knocked_out_in_money_fight`,
    /// gated on 360° detection, sorted ascending by squared stretch-Y
    /// distance.
    ///
    /// The engine already materialises the unconscious + alive filter
    /// into `tick.camp_unconscious_soldiers`, so we walk that instead of
    /// iterating all soldiers per call.
    ///
    /// The comparator reads the locally-computed `sq` so no
    /// per-soldier scratchpad is needed.
    pub(super) fn create_list_of_near_money_fight_victims(
        &mut self,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // Clear the list.
        self.money_fight_victims.clear();

        // Collect (handle, stretched-Y sq_distance) for candidates that
        // pass the 360° detection gate.
        let mut candidates: Vec<(NpcHandle, f32)> = Vec::new();
        for us in tick.camp_unconscious_soldiers.iter() {
            if !us.knocked_out_in_money_fight {
                continue;
            }
            let handle = us.handle;
            if handle == self.base.me {
                continue;
            }
            let Some(victim_view) = ctx.entity_view(handle as HumanHandle) else {
                continue;
            };
            if victim_view.in_building {
                continue;
            }
            let victim_pos = crate::stealth::detection_point_xy(
                crate::coordinates::MapPoint::new(victim_view.position.x, victim_view.position.y),
                victim_view.posture,
                victim_view.direction as i16,
            );
            let viewer_eye_z = ctx.elevation
                + crate::stealth::eye_z_for_posture(
                    crate::element::Posture::Upright,
                    ctx.self_is_rider,
                );
            let target_eye_z = victim_view.elevation
                + crate::stealth::detection_z_for_posture(
                    victim_view.posture,
                    victim_view.is_rider,
                );
            let viewer_eye_ground = crate::coordinates::GroundPoint::from_map_and_z(
                crate::coordinates::MapPoint::new(ctx.position.x, ctx.position.y),
                ctx.elevation,
            );
            let target_detection_ground =
                crate::coordinates::GroundPoint::from_map_and_z(victim_pos, victim_view.elevation);
            // Squared distance — dx² + (dy * INVERSE_ASPECT_RATIO)².
            let dx = target_detection_ground.x - viewer_eye_ground.x;
            let dy = (target_detection_ground.y - viewer_eye_ground.y)
                * crate::position_interface::INVERSE_ASPECT_RATIO;
            let dz = target_eye_z - viewer_eye_z;
            let sq = dx * dx + dy * dy + dz * dz;
            if ctx.in_building || sq > ctx.sq_standard_view_radius {
                continue;
            }
            if !crate::sight_obstacle::is_reachable_3d(
                ctx.obstacle_list(),
                [viewer_eye_ground.x, viewer_eye_ground.y, viewer_eye_z],
                [
                    target_detection_ground.x,
                    target_detection_ground.y,
                    target_eye_z,
                ],
                crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
            ) {
                continue;
            }
            candidates.push((handle, sq));
        }
        // Sort by ascending sq distance.
        candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        self.money_fight_victims = candidates.into_iter().map(|(h, _)| h).collect();
    }

    /// Handle the thief-stole-my-coin case.  Dispatched from the
    /// `EventObjectAway` arm in `think_unexpected_event` after the
    /// 180° detection / interesting-object gate has passed on the
    /// caller side for the type check.
    pub(super) fn stolen_money_standard_procedure(
        &mut self,
        thief: NpcHandle,
        ctx: &AiContext,
        tick: &AiPerTickData,
    ) {
        // 180° gate on the thief.
        if !self.is_detecting_180_degrees(thief as HumanHandle, ctx) {
            return;
        }
        // Assert: thief cannot be me.
        if thief == self.base.me {
            return;
        }
        // Question the soldier profile.
        if !self.answer_question(Question::ShallIFightForMoney, ctx) {
            return;
        }
        // Morale check; bail and collect on no.
        if !self.wants_to_continue_money_fight(tick, ctx) {
            self.money_fight_enemies.clear();
            self.stop_brawling_and_collect_money(ctx, tick);
            return;
        }
        // Substate dispatch.
        if self.base.current_substate.is_take_money() {
            // "Hey, this coin is MINE!"
            self.base.break_macro();
            self.base.face_entity(thief, ctx);
            self.base.set_emoticon(EmoticonType::QuestionMark);
            self.set_state(AiState::Wondering, Substate::WonderingBrawlReactiontime);
            self.money_fight_enemies.push(thief);
            self.react(parameters_ai::AI_MAX_ENEMY_REACTIONTIME as u16, ctx, tick);
            self.base.friend_in_trouble = Some(AiEntityHandle::new(thief));
        } else if self.base.current_substate.is_fight_for_money() {
            // Already brawling; queue this guy.
            self.money_fight_enemies.push(thief);
        }
    }

    /// Walks `money_fight_enemies` and returns the handle of the one
    /// at minimum maximum-norm distance, adding a +300 malus when the
    /// enemy is on a different layer.  Returns `None` when the list
    /// is empty.
    pub(super) fn get_nearest_money_fight_enemy(&self, ctx: &AiContext) -> Option<NpcHandle> {
        let my_layer = ctx.position.level;
        let mut best: Option<(NpcHandle, u32)> = None;
        for &handle in self.money_fight_enemies.iter() {
            let Some(view) = ctx.entity_view(handle as HumanHandle) else {
                continue;
            };
            let dx = (view.position.x - ctx.position.x).abs();
            let dy = (view.position.y - ctx.position.y).abs();
            let mut distance = dx.max(dy) as u32;
            if view.position.level != my_layer {
                distance = distance.saturating_add(300);
            }
            match best {
                Some((_, best_d)) if distance >= best_d => {}
                _ => best = Some((handle, distance)),
            }
        }
        best.map(|(h, _)| h)
    }

    /// Rebuilds `money_fight_enemies` from the current same-camp
    /// soldier snapshot — conscious, alive, 360°-detected soldiers
    /// whose substate is take/fight-for-money.
    ///
    /// `tick.camp_soldiers` is built before the creation-order AI pass, so an
    /// earlier soldier can become unconscious or die before this actor scans
    /// it.  Original reads those lifecycle flags live at this point.
    ///
    /// The 360° check comes before the substate test: it runs for every
    /// conscious camp soldier, not just the ones already brawling, and
    /// each query perturbs the shared visibility cache.
    pub(super) fn create_new_list_of_money_fight_enemies(
        &mut self,
        tick: &AiPerTickData,
        ctx: &AiContext,
    ) {
        self.money_fight_enemies.clear();
        for cs in tick.camp_soldiers.iter() {
            if cs.handle == self.base.me {
                continue;
            }
            let view = ctx
                .expect_entity_view(cs.handle as HumanHandle, "money-fight enemy-list candidate");
            if view.is_unconscious || view.is_dead {
                continue;
            }
            if !self.is_detecting_360_degrees(cs.handle as HumanHandle, ctx) {
                continue;
            }
            if !(cs.ai_substate.is_take_money() || cs.ai_substate.is_fight_for_money()) {
                continue;
            }
            self.money_fight_enemies.push(cs.handle);
        }
    }

    /// Morale check for whether to keep brawling based on
    /// upright-vs-sleeping money-fighter ratio.
    ///
    /// This is one scan over every alive same-camp soldier in the Original
    /// camp-registry order, with a single 360° query per candidate. Query
    /// order matters because each one perturbs the shared visibility cache.
    /// Preexisting sleepers can occur only in the parallel unconscious list,
    /// while a same-frame transition can leave the same handle in both. Merge
    /// the ordered snapshots and coalesce equal handles so neither shape is
    /// lost or queried twice.
    pub(super) fn wants_to_continue_money_fight(
        &self,
        tick: &AiPerTickData,
        ctx: &AiContext,
    ) -> bool {
        // Berserker fast path + drunken override.
        if self.soldier_profile_money == 100 || self.base.blood_alcohol > 0 {
            return true;
        }

        let mut upright: u32 = 1; // counts self
        let mut sleeping: u32 = 0;

        let mut soldiers = tick.camp_soldiers.iter().peekable();
        let mut sleepers = tick.camp_unconscious_soldiers.iter().peekable();
        loop {
            let (handle, knocked_out_in_money_fight) = match (soldiers.peek(), sleepers.peek()) {
                (Some(soldier), Some(sleeper)) if soldier.handle == sleeper.handle => {
                    let soldier = soldiers.next().expect("peeked camp soldier");
                    let sleeper = sleepers.next().expect("peeked unconscious soldier");
                    (soldier.handle, sleeper.knocked_out_in_money_fight)
                }
                (Some(soldier), Some(sleeper)) => {
                    // Both lists are ordered subsequences of Original's
                    // camp array. Creation order is the stable identity
                    // for that authored order; runtime slot numbers are
                    // not interchangeable after reuse.
                    let soldier_order = ctx
                        .expect_entity_view(
                            soldier.handle as HumanHandle,
                            "money-fight morale camp-order candidate",
                        )
                        .original_creation_order;
                    let sleeper_order = ctx
                        .expect_entity_view(
                            sleeper.handle as HumanHandle,
                            "money-fight morale sleeper-order candidate",
                        )
                        .original_creation_order;
                    if soldier_order < sleeper_order {
                        let soldier = soldiers.next().expect("peeked camp soldier");
                        (soldier.handle, soldier.knocked_out_in_money_fight)
                    } else {
                        let sleeper = sleepers.next().expect("peeked unconscious soldier");
                        (sleeper.handle, sleeper.knocked_out_in_money_fight)
                    }
                }
                (None, Some(_)) => {
                    let sleeper = sleepers.next().expect("peeked unconscious soldier");
                    (sleeper.handle, sleeper.knocked_out_in_money_fight)
                }
                (Some(_), None) => {
                    let soldier = soldiers.next().expect("peeked camp soldier");
                    (soldier.handle, soldier.knocked_out_in_money_fight)
                }
                (None, None) => break,
            };
            if handle == self.base.me {
                continue;
            }
            let live_view =
                ctx.expect_entity_view(handle as HumanHandle, "money-fight morale candidate");
            if live_view.is_dead {
                continue;
            }
            if !self.is_detecting_360_degrees(handle as HumanHandle, ctx) {
                continue;
            }
            let live_substate = live_view.ai_substate;
            if live_substate.is_take_money() || live_substate.is_fight_for_money() {
                upright += 1;
            } else if live_substate == Substate::SleepingUnconscious && knocked_out_in_money_fight {
                // TODO(parity): `AiEntityView` does not yet expose the live
                // money-fight knockout flag. The ordered camp snapshots
                // carry the boundary value; add it to the view if a proven
                // cross-actor transition can flip this flag before our scan.
                sleeping += 1;
            }
        }

        let total = upright + sleeping;
        // `total >= 1` because `upright` starts at 1.
        let knocked_out_percentage = (100 * sleeping) / total;
        knocked_out_percentage < self.soldier_profile_money as u32
    }
}
