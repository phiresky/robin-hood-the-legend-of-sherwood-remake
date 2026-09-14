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

    pub(super) fn awake_next_money_fight_victim_if_any(
        &mut self,
        _env: ThinkEnv<'_>,
    ) -> AiFlow<()> {
        Err(DutyCall {
            tail: crate::ai::DutyTail::MoneyFight {
                operation: crate::ai::MoneyFightOperation::AwakeNextVictim,
            },
            ..DutyCall::new(DutyFlags::empty(), false)
        })
    }
}
