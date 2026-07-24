//! Opponent list and swordfight engagement (enter/quit/principal).
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::combat::{self};
use crate::element::{ActionState, Command, Entity, EntityId, Posture};

impl EngineInner {
    // ─── Tie-up (public, called from natives/UI) ────────────────────

    // ─── Opponent list management ────────────────────────────────

    /// Add `opponent` to `entity`'s opponent list at index 0
    /// (principal slot).
    ///
    /// - If the opponent is already in the list at index > 0, swap
    ///   it to the front and overwrite the jump line at slot 0;
    ///   return `false` (already known).
    /// - If already at index 0, just leave it (return `false`).
    /// - Otherwise insert `(opponent, jump_line)` at the front;
    ///   return `true` (new).
    ///
    /// Returns `true` when this is a fresh addition.  The
    /// fighting-ability recompute and smalltalk-initiative
    /// side-effects are kept at the call sites
    /// (`enter_swordfight` / `take_smalltalk_initiative`) because
    /// they need access to `&mut self` and the asset profile manager
    /// that this helper, scoped over the entity slice, can't reach.
    pub(super) fn add_opponent(
        entities: &mut crate::entities::Entities,
        entity_id: EntityId,
        opponent_id: EntityId,
        jump_line: Option<crate::jump_line::JumpLineIndex>,
    ) -> bool {
        let Some(entity) = entities.get_mut(entity_id) else {
            return false;
        };
        let Some(human) = entity.human_data_mut() else {
            return false;
        };
        // Keep the parallel jump-line vector aligned (defensive against
        // older saves predating its addition).
        if human.opponent_jump_lines.len() < human.opponents.len() {
            human
                .opponent_jump_lines
                .resize(human.opponents.len(), None);
        }
        if let Some(pos) = human.opponents.iter().position(|&id| id == opponent_id) {
            if pos != 0 {
                human.opponents.swap(0, pos);
                human.opponent_jump_lines.swap(0, pos);
                human.opponent_jump_lines[0] = jump_line;
            }
            return false;
        }
        human.opponents.insert(0, opponent_id);
        human.opponent_jump_lines.insert(0, jump_line);
        true
    }

    /// Remove `opponent` from `entity`'s opponent list.
    pub(super) fn remove_opponent(
        entities: &mut crate::entities::Entities,
        entity_id: EntityId,
        opponent_id: EntityId,
    ) {
        if let Some(entity) = entities.get_mut(entity_id)
            && let Some(human) = entity.human_data_mut()
            && let Some(pos) = human.opponents.iter().position(|&id| id == opponent_id)
        {
            human.opponents.remove(pos);
            // Keep the parallel jump-line vector aligned with `opponents`.
            if pos < human.opponent_jump_lines.len() {
                human.opponent_jump_lines.remove(pos);
            }
        }
    }

    /// Re-evaluate every entry in `entity_id`'s opponent list and refresh
    /// the per-opponent jump line after `entity_id`'s sector changed.
    ///
    /// For each opponent:
    /// - same sector as `entity_id` → clear the jump line on both
    ///   sides,
    /// - different sectors with a stale or missing jump line → ask
    ///   `is_table_swordfight_needed` from each side; if the two
    ///   answers pair up via `associated_line_index`, store each
    ///   side's own line; otherwise clear both.
    ///
    /// Only meaningful for swordfighters — callers gate on
    /// `is_swordfighting` before invoking.
    pub(crate) fn update_opponents_jump_lines(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let this_sector_num = match self
            .get_entity(entity_id)
            .and_then(|e| e.element_data().sector())
        {
            Some(s) => i16::from(s),
            None => return,
        };
        let this_sector_idx = self
            .world
            .fast_grid
            .level
            .sector_number_map
            .get(&crate::sector::SectorNumber::new(this_sector_num))
            .copied();

        // Snapshot opponents + current jump-lines so we can mutate in a
        // second pass without holding a borrow on `self.world.entities`.
        let opponents: Vec<(EntityId, Option<crate::jump_line::JumpLineIndex>)> =
            match self.get_entity(entity_id).and_then(|e| e.human_data()) {
                Some(h) => {
                    let mut v = Vec::with_capacity(h.opponents.len());
                    for (i, &opp_id) in h.opponents.iter().enumerate() {
                        let jl = h.opponent_jump_lines.get(i).copied().flatten();
                        v.push((opp_id, jl));
                    }
                    v
                }
                None => return,
            };

        // (slot_index, new_this_jl, opponent_id, new_opp_jl)
        let mut updates: Vec<(
            usize,
            Option<crate::jump_line::JumpLineIndex>,
            EntityId,
            Option<crate::jump_line::JumpLineIndex>,
        )> = Vec::new();

        for (i, (opp_id, current_jl)) in opponents.iter().enumerate() {
            let opp_sector_num = match self
                .get_entity(*opp_id)
                .and_then(|e| e.element_data().sector())
            {
                Some(s) => i16::from(s),
                None => continue,
            };

            if opp_sector_num == this_sector_num {
                // Same sector → clear if currently set.
                if current_jl.is_some() {
                    updates.push((i, None, *opp_id, None));
                }
                continue;
            }

            // Different sectors — check if the stored jump line is still
            // valid (this side's line lives in our sector and its
            // associated line lives in the opponent's sector).
            let opp_sector_idx = self
                .world
                .fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(opp_sector_num))
                .copied();

            let stale = match current_jl {
                None => true,
                Some(idx) => {
                    let jl = self.world.fast_grid.level.jump_lines.get(usize::from(*idx));
                    match jl {
                        None => true,
                        Some(jl_data) => {
                            let this_idx_match =
                                jl_data.sector_index.map(usize::from) == this_sector_idx;
                            let assoc_jl = jl_data.associated_line_index.and_then(|i| {
                                self.world.fast_grid.level.jump_lines.get(i as usize)
                            });
                            let assoc_idx_match =
                                assoc_jl.and_then(|aj| aj.sector_index).map(usize::from)
                                    == opp_sector_idx;
                            !this_idx_match || !assoc_idx_match
                        }
                    }
                }
            };

            if !stale {
                continue;
            }

            // Ask `is_table_swordfight_needed` from both sides; only
            // commit when each side's returned line is the other's
            // associated line.
            let new_this_idx = is_table_swordfight_needed(
                &self.world.entities,
                &self.world.fast_grid,
                &assets.profile_manager,
                entity_id,
                *opp_id,
            );
            let mut paired: Option<(
                crate::jump_line::JumpLineIndex,
                crate::jump_line::JumpLineIndex,
            )> = None;
            if let Some(this_raw) = new_this_idx {
                let new_opp_idx = is_table_swordfight_needed(
                    &self.world.entities,
                    &self.world.fast_grid,
                    &assets.profile_manager,
                    *opp_id,
                    entity_id,
                );
                if let Some(opp_raw) = new_opp_idx {
                    let opp_associated = self
                        .world
                        .fast_grid
                        .level
                        .jump_lines
                        .get(opp_raw as usize)
                        .and_then(|j| j.associated_line_index);
                    if opp_associated == Some(this_raw)
                        && let (Some(this_jl), Some(opp_jl)) = (
                            crate::jump_line::JumpLineIndex::new(this_raw),
                            crate::jump_line::JumpLineIndex::new(opp_raw),
                        )
                    {
                        paired = Some((this_jl, opp_jl));
                    }
                }
            }

            match paired {
                Some((this_jl, opp_jl)) => {
                    updates.push((i, Some(this_jl), *opp_id, Some(opp_jl)));
                }
                None => {
                    updates.push((i, None, *opp_id, None));
                }
            }
        }

        // Phase 2: write back.
        for (i, this_jl, opp_id, opp_jl) in updates {
            if let Some(entity) = self.world.entities.get_mut(entity_id)
                && let Some(human) = entity.human_data_mut()
            {
                if human.opponent_jump_lines.len() < human.opponents.len() {
                    human
                        .opponent_jump_lines
                        .resize(human.opponents.len(), None);
                }
                if let Some(slot) = human.opponent_jump_lines.get_mut(i) {
                    *slot = this_jl;
                }
            }
            // Mirror onto the opponent's slot for `entity_id`.
            // Use a soft path here — opponent's list may legitimately
            // have removed the entry between snapshot and write-back.
            if let Some(entity) = self.world.entities.get_mut(opp_id)
                && let Some(human) = entity.human_data_mut()
                && let Some(pos) = human.opponents.iter().position(|&id| id == entity_id)
            {
                if human.opponent_jump_lines.len() < human.opponents.len() {
                    human
                        .opponent_jump_lines
                        .resize(human.opponents.len(), None);
                }
                if let Some(slot) = human.opponent_jump_lines.get_mut(pos) {
                    *slot = opp_jl;
                }
            }
        }
    }

    /// Re-evaluate the opponent list after a change.
    ///
    /// - Empty list → quit the swordfight entirely.
    /// - Two or more opponents → re-pick the principal.
    /// - Exactly one → leave the principal where it is.
    pub(crate) fn evaluate_opponents(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        _assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let count = self
            .get_entity(entity_id)
            .and_then(|e| e.human_data())
            .map(|h| h.opponents.len())
            .unwrap_or(0);

        if count == 0 {
            // C++ `EvaluateOpponents` launches the explicit command;
            // relationship teardown alone does not own the visible
            // lowering-sword transition.
            self.launch_element(crate::sequence::SequenceElement::new(
                1,
                Command::QuitSwordfight,
                Some(entity_id),
            ));
        } else if count >= 2 {
            self.choose_principal_opponent(sim, entity_id);
        }
    }

    /// Pick a new principal opponent from the entity's opponent list.
    ///
    /// 1. Build a candidate list of opponents within ±2 sectors of
    ///    the entity's facing direction.
    /// 2. If any face-cone candidates exist, pick one uniformly at
    ///    random.
    /// 3. Otherwise pick the nearest opponent by 2D distance.
    ///
    /// When the chosen opponent isn't already at index 0, swap them to
    /// the front and take the smalltalk initiative.
    pub(super) fn choose_principal_opponent(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        entity_id: EntityId,
    ) {
        let (self_pos, self_dir, opponents) = {
            let Some(entity) = self.get_entity(entity_id) else {
                return;
            };
            let Some(human) = entity.human_data() else {
                return;
            };
            if human.opponents.len() < 2 {
                return;
            }
            let elem = entity.element_data();
            (
                elem.position_map(),
                elem.direction(),
                human.opponents.clone(),
            )
        };

        // Face-cone candidates: relative sector within ±2 of 0.
        let mut candidates: Vec<usize> = Vec::new();
        for (idx, opp_id) in opponents.iter().enumerate() {
            let pos_opp = match self
                .get_entity(*opp_id)
                .map(|e| e.element_data().position_map())
            {
                Some(p) => p,
                None => continue,
            };
            // Use the world aspect ratio (not the sword-fight one)
            // for the opponent-to-self angle in the principal-opponent
            // face cone.
            let dir_to = crate::position_interface::vector_to_sector_0_to_15_iso(
                pos_opp.x - self_pos.x,
                pos_opp.y - self_pos.y,
            );
            let relative = ((self_dir - dir_to) & 15) as u16;
            if relative <= 2 || relative >= 14 {
                candidates.push(idx);
            }
        }

        let new_principal = if !candidates.is_empty() {
            let pick = crate::sim_rng::usize(
                sim,
                crate::sim_rng::RngSite::PrincipalOpponent,
                0..candidates.len(),
            );
            candidates[pick]
        } else {
            // Nearest-opponent fallback.
            let mut best = 0usize;
            let mut best_dist = f32::MAX;
            for (idx, opp_id) in opponents.iter().enumerate() {
                let dist = entity_distance(&self.world.entities, entity_id, *opp_id);
                if dist < best_dist {
                    best_dist = dist;
                    best = idx;
                }
            }
            best
        };

        if new_principal != 0 {
            if let Some(entity) = self.world.entities.get_mut(entity_id)
                && let Some(human) = entity.human_data_mut()
            {
                human.opponents.swap(0, new_principal);
                // Keep the parallel jump-line vector in lockstep.
                if new_principal < human.opponent_jump_lines.len() {
                    human.opponent_jump_lines.swap(0, new_principal);
                }
            }
            self.take_smalltalk_initiative(entity_id);
        }
    }

    /// Promote `new_opponent` to principal opponent (front of list).
    ///
    /// If the opponent is already in the list, swap it to index 0.
    /// If not found, request an enter-swordfight with the new target.
    ///
    pub(crate) fn set_as_new_principal_opponent(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        new_opponent_id: EntityId,
    ) {
        let found = {
            let Some(entity) = self.world.entities.get(entity_id) else {
                return;
            };
            let Some(human) = entity.human_data() else {
                return;
            };
            human.opponents.iter().position(|&id| id == new_opponent_id)
        };

        if let Some(idx) = found {
            // Swap to front — makes this opponent the principal.
            if let Some(entity) = self.world.entities.get_mut(entity_id)
                && let Some(human) = entity.human_data_mut()
            {
                human.opponents.swap(0, idx);
                if idx < human.opponent_jump_lines.len() {
                    human.opponent_jump_lines.swap(0, idx);
                }
            }
            self.take_smalltalk_initiative(entity_id);
        } else {
            // Gate on `can_enter_swordfight_with` and launch a
            // PostponeEverythingButInjuries `EnterSwordfight` element
            // rather than calling `enter_swordfight` directly.  This
            // lets the priority arbitration postpone/interrupt the
            // pending enter against any concurrent injury-priority
            // work, and defers the distance/LOS/sword-hurt guards to
            // the EnterSwordfight dispatcher.
            if can_enter_swordfight_with(
                &self.world.entities,
                entity_id,
                new_opponent_id,
                &assets.profile_manager,
                &self.world.fast_grid,
            ) {
                let mut elem = crate::sequence::SequenceElement::new_generic(
                    1,
                    Command::EnterSwordfight,
                    Some(entity_id),
                );
                elem.set_property(
                    crate::sequence::Field::Opponent,
                    crate::sequence::FieldValue::Element(new_opponent_id),
                );
                elem.set_property(
                    crate::sequence::Field::JumplineDestination,
                    crate::sequence::FieldValue::Integer(0),
                );
                self.launch_element(elem);
            }
        }
    }

    /// Enter swordfight between two entities.
    ///
    /// `initiator` is the entity requesting entry; `opponent` is the
    /// target.  `sword_hurted` is true when the initiator enters
    /// because they were hit — applies the single-opponent
    /// restriction.
    ///
    /// Returns true if swordfight was entered.
    pub(crate) fn enter_swordfight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        initiator: EntityId,
        opponent: EntityId,
        sword_hurted: bool,
    ) -> bool {
        self.enter_swordfight_with_jump_line(sim, assets, initiator, opponent, sword_hurted, None)
    }

    /// Variant of [`enter_swordfight`] that threads the
    /// table-swordfight jump line through to `add_opponent`: the
    /// aggressor gets `aggressor_jump_line` (their side of the
    /// table), and the opponent gets the associated paired line on
    /// the far side.
    pub(crate) fn enter_swordfight_with_jump_line(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        initiator: EntityId,
        opponent: EntityId,
        sword_hurted: bool,
        aggressor_jump_line: Option<crate::jump_line::JumpLineIndex>,
    ) -> bool {
        let scratch = self.build_sim_scratch(sim, assets);
        if opponent.index() == 0 {
            // Opponent==0 is a legitimate input upstream now — the
            // the `EnterSwordfightRequest::RaiseSword` drain branch in
            // `engine/ai/mod.rs::drain_pending_for_npc` routes around this
            // function and launches a bare `Command::EnterSwordfight`
            // element so the actor raises the sword without engaging.
            // Anything reaching this path with opponent==0 means a
            // direct caller skipped the drain — log at trace and
            // bail rather than fabricating an opponent.
            tracing::trace!(
                ?initiator,
                "enter_swordfight called with opponent index 0 — \
                 raise-sword-no-opponent should be routed via the \
                 EnterSwordfightRequest::RaiseSword drain instead"
            );
            return false;
        }

        // PC initiators clear shield-protection before entering the
        // fight to unlink any active shield-protection.  NPC
        // sword-fights don't carry the protection link.
        if self
            .get_entity(initiator)
            .map(|e| e.is_pc())
            .unwrap_or(false)
        {
            self.set_shield_protected(initiator, None);
        }

        // C++ `EnterSwordFight` calls `ClearShootList()` before the
        // validity gates. In Rust, repeated PC bow clicks are represented
        // as pending/postponed `ShootBow` sequence elements.
        {
            let resolver = Self::priority_resolver(&self.world.entities);
            self.orders.sequence_manager.stop_pending_elements_matching(
                initiator,
                crate::element::Command::ShootBow,
                crate::sequence::SequencePriority::Preference,
                &resolver,
            );
        }

        // Cancel any pending AI bow shot.
        if let Some(Entity::Soldier(s)) = self.world.entities.get_mut(initiator)
            && let Some(ai) = s.npc.ai_brain.base_mut()
        {
            ai.outbox.actor.shoot_target = None;
        }

        // The "prepare to enter swordfight" step runs `stop(PREFERENCE)`
        // on the opponent and synchronously pumps EventEnterSwordfight
        // through their think — both gated on
        // `is_swordfighting() == false`.  Done here at the top so a
        // rejection by the downstream gates doesn't strand the
        // opponent in their pre-swordfight state.
        let opponent_was_swordfighting = self
            .world
            .entities
            .get(opponent)
            .and_then(|e| e.human_data())
            .map(|h| !h.opponents.is_empty())
            .unwrap_or(false);
        if !opponent_was_swordfighting {
            self.stop_owner(opponent, crate::sequence::SequencePriority::Preference);
            // Synchronous Think on the opponent if they're a soldier.
            let is_soldier = matches!(self.world.entities.get(opponent), Some(Entity::Soldier(_)));
            if is_soldier {
                let ctx = {
                    let entity = self
                        .world
                        .entities
                        .get(opponent)
                        .expect("opponent existence checked above");
                    crate::engine::ai::build_ai_context_from_entity(
                        entity,
                        self.control.frame_counter,
                        None,
                        self.world.weather.is_forest_level,
                        self.world.weather.ambiance,
                        self.ai.standard_view_polygon_radius,
                        &scratch.ai_entity_views,
                        &scratch.ai_sight_obstacles,
                        &self.world.fast_grid,
                        &assets.hiking_paths,
                        &self.ai.global.all_soldier_handles,
                        self.control.sim_config.difficulty,
                    )
                };
                let stimulus = crate::ai::Stimulus::with_human(
                    crate::ai::StimulusType::EventEnterSwordfight,
                    initiator.index(),
                );
                let tick_data = self.build_npc_tick_data_for_target(
                    sim,
                    opponent,
                    &scratch,
                    assets,
                    Some(initiator),
                );
                self.dispatch_think_with_drain(sim, opponent, &stimulus, &ctx, &tick_data, assets);
            }
        }

        if !can_enter_swordfight_with(
            &self.world.entities,
            initiator,
            opponent,
            &assets.profile_manager,
            &self.world.fast_grid,
        ) {
            tracing::warn!(
                ?initiator,
                ?opponent,
                "enter_swordfight: rejected by can_enter_swordfight_with"
            );
            return false;
        }

        let already_opponent = self
            .world
            .entities
            .get(initiator)
            .and_then(|e| e.human_data())
            .map(|h| h.opponents.contains(&opponent))
            .unwrap_or(false);

        if !already_opponent {
            // Cross-sector elevation gate: reject if elevation
            // difference exceeds threshold AND entities are in
            // different sectors.  Lives here (not in
            // `can_enter_swordfight_with`) so already-paired fighters
            // can re-enter swordfight after one drifts onto a
            // different-sector elevation.
            {
                let (elev_a, elev_b, sector_a, sector_b) = {
                    let entity_a = self.world.entities.get(initiator);
                    let entity_b = self.world.entities.get(opponent);
                    let (Some(ea), Some(eb)) = (entity_a, entity_b) else {
                        return false;
                    };
                    (
                        ea.position_iface().get_elevation(),
                        eb.position_iface().get_elevation(),
                        ea.element_data().sector(),
                        eb.element_data().sector(),
                    )
                };
                if (elev_a - elev_b).abs() > MAX_ELEVATION_SWORDFIGHT && sector_a != sector_b {
                    tracing::debug!(
                        ?initiator,
                        ?opponent,
                        elev_a,
                        elev_b,
                        "enter_swordfight: cross-sector elevation diff too large"
                    );
                    return false;
                }
            }

            // Distance check: 3D distance must be within both
            // combatants' UBER sword range.
            {
                let dist = entity_distance(&self.world.entities, initiator, opponent);
                let uber_a = self
                    .world
                    .entities
                    .get(initiator)
                    .and_then(|e| get_hth_weapon_id_full(e, &assets.profile_manager))
                    .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                    .map(|p| p.distance[3] as f32)
                    .unwrap_or(70.0);
                let uber_b = self
                    .world
                    .entities
                    .get(opponent)
                    .and_then(|e| get_hth_weapon_id_full(e, &assets.profile_manager))
                    .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                    .map(|p| p.distance[3] as f32)
                    .unwrap_or(70.0);
                if dist > uber_a || dist > uber_b {
                    tracing::debug!(
                        ?initiator,
                        ?opponent,
                        dist,
                        uber_a,
                        uber_b,
                        "enter_swordfight: too far apart"
                    );
                    return false;
                }
            }

            // LOS check: verify line of sight between combatants
            // between upright eye points, matching C++
            // `ComputeEyesPoint(..., RHPOSTURE_UPRIGHT)`.
            {
                let eye_a = self
                    .get_entity(initiator)
                    .and_then(|e| e.compute_eyes_point(Some(Posture::Upright)));
                let eye_b = self
                    .get_entity(opponent)
                    .and_then(|e| e.compute_eyes_point(Some(Posture::Upright)));
                if let (Some(a), Some(b)) = (eye_a, eye_b)
                    && !crate::sight_obstacle::is_reachable_3d(
                        self.sight_obstacles(assets),
                        [a.x, a.y, a.z],
                        [b.x, b.y, b.z],
                        crate::sight_obstacle::SIGHTOBSTACLE_OPAQUE,
                    )
                {
                    tracing::debug!(
                        ?initiator,
                        ?opponent,
                        "enter_swordfight: LOS blocked (3D opaque)"
                    );
                    return false;
                }
            }

            // Single-opponent restriction when hurt.
            // Don't enter if the initiator already has opponents, or if
            // the opponent's principal opponent already has multiple opponents.
            if sword_hurted {
                let initiator_opp_count = self
                    .world
                    .entities
                    .get(initiator)
                    .and_then(|e| e.human_data())
                    .map(|h| h.opponents.len())
                    .unwrap_or(0);
                if initiator_opp_count >= 1 {
                    return false;
                }

                // If the opponent is already fighting and their principal
                // opponent has >1 opponents, don't pile in.
                let principal_opp_id = self
                    .world
                    .entities
                    .get(opponent)
                    .and_then(|e| e.human_data())
                    .and_then(|h| h.opponents.first().copied());

                if let Some(principal_id) = principal_opp_id {
                    let principal_opp_count = self
                        .world
                        .entities
                        .get(principal_id)
                        .and_then(|e| e.human_data())
                        .map(|h| h.opponents.len())
                        .unwrap_or(0);
                    if principal_opp_count > 1 {
                        return false;
                    }
                }
            }

            // Don't enter swordfight with a charging knight.
            let opponent_is_charging_rider = self
                .world
                .entities
                .get(opponent)
                .map(|e| {
                    e.soldier_data().map(|s| s.rider).unwrap_or(false)
                        && e.actor_data()
                            .map(|a| a.action_state == ActionState::MovingFast)
                            .unwrap_or(false)
                })
                .unwrap_or(false);
            if opponent_is_charging_rider {
                return false;
            }
        }

        // Clear step-back flag on swordfight entry.
        if let Some(entity) = self.world.entities.get_mut(initiator)
            && let Some(hd) = entity.human_data_mut()
        {
            hd.last_motion_was_step_back_in_combat = false;
        }

        // Multi-opponent purging.
        let opponent_is_swordfighting = self
            .world
            .entities
            .get(opponent)
            .and_then(|e| e.human_data())
            .map(|h| !h.opponents.is_empty())
            .unwrap_or(false);

        if !opponent_is_swordfighting {
            // Launch an EnterSwordfight sequence element on the
            // opponent so they raise their sword.  (The stop +
            // soldier think from `prepare_to_enter_swordfight`
            // already ran at the top of this function.)
            let mut seq = crate::sequence::Sequence::new();
            let mut elem = crate::sequence::SequenceElement::new_generic(
                1,
                Command::EnterSwordfight,
                Some(opponent),
            );
            elem.set_property(
                crate::sequence::Field::Opponent,
                crate::sequence::FieldValue::Element(initiator),
            );
            elem.set_property(
                crate::sequence::Field::JumplineDestination,
                crate::sequence::FieldValue::Integer(0),
            );
            seq.append_element(elem);
            let reciprocal_sequence = self.launch_sequence(seq);
            let reciprocal_action = self
                .orders
                .sequence_manager
                .take_deferred_owner_action(opponent, reciprocal_sequence, 0)
                .unwrap_or_else(|detail| {
                    panic!(
                        "reciprocal EnterSwordfight instruction for {} failed: {detail}",
                        opponent.index()
                    )
                });
            if let Some(action) = reciprocal_action {
                // `PrepareToEnterSwordFight` launches this element from
                // inside the initiator's Instruct call. Original's active
                // SequenceManager::Go pass instructs it before returning.
                // Put the exact promoted action on the synchronous stream so
                // the outer sequence phase splices it ahead of older work.
                self.orders
                    .sequence_manager
                    .restore_pending_synchronous_actions(vec![action]);
            }
        } else if !already_opponent {
            // Part 1: walk the opponent's existing opponent list.
            // If any of their opponents have >1 opponents themselves,
            // break those fights to make room for the new 1-on-1.
            let opp_opponents: Vec<EntityId> = self
                .world
                .entities
                .get(opponent)
                .and_then(|e| e.human_data())
                .map(|h| h.opponents.clone())
                .unwrap_or_default();

            for ally_id in &opp_opponents {
                let ally_opp_count = self
                    .world
                    .entities
                    .get(*ally_id)
                    .and_then(|e| e.human_data())
                    .map(|h| h.opponents.len())
                    .unwrap_or(0);
                if ally_opp_count > 1 {
                    Self::remove_opponent(&mut self.world.entities, *ally_id, opponent);
                    Self::remove_opponent(&mut self.world.entities, opponent, *ally_id);
                }
            }

            // Part 2: if both sides still have opponents, purge all
            // opponents from the royalist side.
            let initiator_has_opps = self
                .world
                .entities
                .get(initiator)
                .and_then(|e| e.human_data())
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false);
            let opponent_has_opps = self
                .world
                .entities
                .get(opponent)
                .and_then(|e| e.human_data())
                .map(|h| !h.opponents.is_empty())
                .unwrap_or(false);

            if initiator_has_opps && opponent_has_opps {
                let initiator_camp = entity_camp(&self.world.entities, initiator);
                let human_to_purge = if initiator_camp == crate::element::Camp::Royalists {
                    initiator
                } else {
                    opponent
                };

                let purge_opponents: Vec<EntityId> = self
                    .world
                    .entities
                    .get(human_to_purge)
                    .and_then(|e| e.human_data())
                    .map(|h| h.opponents.clone())
                    .unwrap_or_default();

                for opp_id in &purge_opponents {
                    Self::remove_opponent(&mut self.world.entities, *opp_id, human_to_purge);
                    Self::remove_opponent(&mut self.world.entities, human_to_purge, *opp_id);
                }
            }
        }

        // Add opponents.
        tracing::info!(
            ?initiator,
            ?opponent,
            "enter_swordfight: SUCCESS — adding opponents"
        );
        // The aggressor stores `aggressor_jump_line` (their side of
        // the table), the opponent stores the associated paired line
        // on the far side.  When no table fight is involved, both
        // sides store `None`.
        let opponent_jump_line = aggressor_jump_line.and_then(|aggr| {
            self.world
                .fast_grid
                .level
                .jump_lines
                .get(usize::from(aggr))
                .and_then(|jl| jl.associated_line_index)
                .and_then(crate::jump_line::JumpLineIndex::new)
        });
        let opponent_added = Self::add_opponent(
            &mut self.world.entities,
            opponent,
            initiator,
            opponent_jump_line,
        );
        // Original `AddOpponent` owns these side effects and performs them
        // immediately after each *fresh* insertion. In particular, merely
        // re-entering an already-established fight must not reset initiative.
        // Preserve the call order because the first initiative check happens
        // before the reciprocal list entry is installed.
        if opponent_added {
            self.recompute_relative_fighting_ability(opponent, assets);
            self.take_smalltalk_initiative(opponent);
        }
        let initiator_added = Self::add_opponent(
            &mut self.world.entities,
            initiator,
            opponent,
            aggressor_jump_line,
        );
        if initiator_added {
            self.recompute_relative_fighting_ability(initiator, assets);
            self.take_smalltalk_initiative(initiator);
        }

        // Recompute relative fighting ability on both sides after
        // the opponent lists change.
        self.recompute_relative_fighting_ability(initiator, assets);
        self.recompute_relative_fighting_ability(opponent, assets);

        // Cancel pending pathfinder requests and active paths only
        // for entities that are ENTERING combat fresh (not already in
        // a sword or shield state).  This happens via the
        // `prepare_to_enter_swordfight` step, which is only called
        // when the entity wasn't already swordfighting.  Clearing
        // movement for an already-fighting entity would cancel their
        // in-progress walk-away / strafe during combat.
        let initiator_fresh = self
            .world
            .entities
            .get(initiator)
            .and_then(|e| e.actor_data())
            .map(|a| !a.action_state.is_sword() && !a.action_state.is_shield())
            .unwrap_or(true);
        let opponent_fresh = self
            .world
            .entities
            .get(opponent)
            .and_then(|e| e.actor_data())
            .map(|a| !a.action_state.is_sword() && !a.action_state.is_shield())
            .unwrap_or(true);
        // Whenever a movement element is torn down, the failed-path
        // retries also get cleaned out — otherwise stale 100-frame
        // retry entries fire `element_impossible` / hero-speech
        // after the swordfight starts.  The cancel-requests half is
        // a no-op post-pathfinder refactor (sequence-element
        // interruption tears down in-flight requests), so we only
        // retain the failed-path cleanup.
        if initiator_fresh {
            self.orders
                .failed_path_requests
                .retain(|r| r.owner != initiator);
        }
        if opponent_fresh {
            self.orders
                .failed_path_requests
                .retain(|r| r.owner != opponent);
        }

        // `EnterSwordFight` mutates the relationship and initiator PC UI.
        // The ENTER_SWORDFIGHT element that called us owns its pose
        // transition; the reciprocal element does the same for the opponent.
        if let Some(pc) = self
            .world
            .entities
            .get_mut(initiator)
            .and_then(Entity::pc_data_mut)
        {
            if pc.melee_target.is_none() {
                pc.melee_target = Some(opponent);
            }
            pc.disable_all_actions_temp();
        }
        // Set PC melee target.
        Self::set_pc_melee_target(&mut self.world.entities, initiator, opponent);
        Self::set_pc_melee_target(&mut self.world.entities, opponent, initiator);
        // The opponent's `prepare_to_enter_swordfight` think fires
        // at the top of this function; no second dispatch needed
        // here.

        true
    }

    /// Remove this entity from every swordfight relationship.
    ///
    /// This mirrors C++ `RHElementActorHuman::QuitSwordFight`: despite
    /// its name, the method only unlinks opponents and performs PC/AI
    /// bookkeeping. It does not mutate a survivor's action state or
    /// launch the visible lowering-sword transition; the explicit
    /// `Command::QuitSwordfight` dispatcher owns that.
    pub(crate) fn quit_swordfight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        // Collect opponent list first to avoid borrow issues
        let opponents: Vec<EntityId> = self
            .world
            .entities
            .get(entity_id)
            .and_then(|e| e.human_data())
            .map(|h| h.opponents.clone())
            .unwrap_or_default();

        // Remove this entity from each opponent's list
        for opp_id in &opponents {
            Self::remove_opponent(&mut self.world.entities, *opp_id, entity_id);
            // `delete_opponent` refreshes the cached
            // relative-fighting-ability on the surviving opponent so
            // future strike/parry rolls use up-to-date ratios.  No-op
            // when their list is now empty
            // (`compute_relative_fighting_ability(own, 0)` returns
            // 50).
            self.recompute_relative_fighting_ability(*opp_id, assets);
            // If the opponent has no more opponents, mirror the
            // relationship/UI side effects of their recursive C++
            // `QuitSwordFight` call. Do not alter their current action
            // or launch a lowering transition here.
            let opp_count = self
                .world
                .entities
                .get(*opp_id)
                .and_then(|e| e.human_data())
                .map(|h| h.opponents.len())
                .unwrap_or(0);
            if opp_count == 0 {
                if let Some(entity) = self.world.entities.get_mut(*opp_id) {
                    // Re-enable PC actions and clear melee target
                    // for orphaned PCs.  `opp_count == 0` means
                    // this PC's opponents list is empty, so
                    // `is_swordfighting() == false`.
                    // `enable_all_actions_temp` honours the
                    // playable guard and restores `current_action`
                    // from `saved_action` so the PC re-picks the
                    // action it had before entering the
                    // swordfight.
                    if let Some(pc) = entity.pc_data_mut() {
                        pc.melee_target = None;
                        pc.enable_all_actions_temp(false);
                    }
                }
                // When the opponent list becomes empty and the entity
                // is a soldier, pump EventQuitSwordfight through
                // their AI.
                self.dispatch_ai_stimulus(
                    *opp_id,
                    crate::ai::Stimulus::new(crate::ai::StimulusType::EventQuitSwordfight),
                );
            } else if opp_count >= 2 {
                // Re-pick the principal opponent now that the list
                // has changed, so the stale principal pointer doesn't
                // linger on the removed fighter.
                self.choose_principal_opponent(sim, *opp_id);
            }

            // Whenever the survivor is still swordfighting, take
            // smalltalk initiative — fired regardless of whether the
            // principal index moved.  `choose_principal_opponent`
            // above only fires it on a swap, so re-fire here for the
            // ≥1-survivor case.  Re-firing after a swap is a no-op
            // (`take_smalltalk_initiative` simply re-sets the
            // already-true flag).
            if opp_count >= 1 {
                let opp_swordfighting = self
                    .get_entity(*opp_id)
                    .and_then(|e| e.actor_data())
                    .map(|a| a.action_state.is_sword())
                    .unwrap_or(false);
                if opp_swordfighting {
                    self.take_smalltalk_initiative(*opp_id);
                }
            }
        }

        // The post-loop self-cleanup is gated on `!is_dead()`: a
        // dead PC shouldn't have its `disabled_actions_temp`
        // re-enabled, and a dead soldier shouldn't be re-pumped
        // through `think`.
        let entity_is_dead = self
            .get_entity(entity_id)
            .map(|e| e.is_dead())
            .unwrap_or(false);

        // Always clear the entity's own opponent list. Relationship
        // cleanup deliberately leaves its current action and sequence
        // untouched.
        if let Some(entity) = self.world.entities.get_mut(entity_id) {
            if let Some(human) = entity.human_data_mut() {
                human.opponents.clear();
                human.opponent_jump_lines.clear();
            }
            if !entity_is_dead && let Some(pc) = entity.pc_data_mut() {
                pc.melee_target = None;
                // `human.opponents` was cleared just above so
                // `is_swordfighting() == false`;
                // `enable_all_actions_temp` restores
                // `current_action` from `saved_action` if the
                // saved slot is still permitted.
                pc.enable_all_actions_temp(false);
            }
        }

        // When a non-dead soldier voluntarily quits a swordfight,
        // immediately pump EventQuitSwordfight into its own AI so it
        // can re-plan, rather than waiting for the next AI tick.
        if !entity_is_dead && matches!(self.world.entities.get(entity_id), Some(Entity::Soldier(_)))
        {
            self.dispatch_ai_stimulus(
                entity_id,
                crate::ai::Stimulus::new(crate::ai::StimulusType::EventQuitSwordfight),
            );
        }
    }

    /// Set the melee target on a PC entity.
    pub(super) fn set_pc_melee_target(
        entities: &mut crate::entities::Entities,
        pc_id: EntityId,
        opponent_id: EntityId,
    ) {
        if let Some(entity) = entities.get_mut(pc_id)
            && let Some(pc) = entity.pc_data_mut()
        {
            pc.melee_target = Some(opponent_id);
        }
    }

    /// Remove opponents that are too far away, re-evaluate swordfight state.
    ///
    /// Called from the AI tick when soldiers re-evaluate their combat state.
    pub(crate) fn quit_swordfight_with_far_opponents(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) {
        let (opponents, uber_range) = {
            let entity = match self.world.entities.get(entity_id) {
                Some(e) => e,
                None => return,
            };
            let range = get_hth_weapon_id_full(entity, &assets.profile_manager)
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                .map(|p| p.distance[3] as f32) // UBER range
                .unwrap_or(70.0);
            let opps = entity
                .human_data()
                .map(|h| h.opponents.clone())
                .unwrap_or_default();
            (opps, range)
        };

        let mut removed: Vec<EntityId> = Vec::new();
        for opp_id in opponents {
            let dist = entity_distance(&self.world.entities, entity_id, opp_id);
            let opp_uber = self
                .world
                .entities
                .get(opp_id)
                .and_then(|e| get_hth_weapon_id_full(e, &assets.profile_manager))
                .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
                .map(|p| p.distance[3] as f32)
                .unwrap_or(70.0);

            if dist > uber_range && dist > opp_uber {
                Self::remove_opponent(&mut self.world.entities, entity_id, opp_id);
                Self::remove_opponent(&mut self.world.entities, opp_id, entity_id);
                // Recompute the cached relative-fighting-ability on
                // both surviving sides so the next strike / parade
                // roll uses fresh ratios.  (When their list is now
                // empty, `evaluate_opponents` below routes through
                // `quit_swordfight` and resets state anyway.)
                self.recompute_relative_fighting_ability(opp_id, assets);
                self.recompute_relative_fighting_ability(entity_id, assets);
                removed.push(opp_id);
            }
        }
        if removed.is_empty() {
            return;
        }

        // After removing the far opponent from both lists,
        // `evaluate_opponents` runs on the opponent AND on self —
        // dispatches to `quit_swordfight` when the list is empty or
        // `choose_principal_opponent` when two or more remain.
        let remaining = self
            .world
            .entities
            .get(entity_id)
            .and_then(|e| e.human_data())
            .map(|h| h.opponents.len())
            .unwrap_or(0);
        tracing::debug!(
            "quit_swordfight_with_far_opponents: {:?} removed {} far opponents, {} remaining",
            entity_id,
            removed.len(),
            remaining
        );
        for opp_id in &removed {
            self.evaluate_opponents(sim, assets, *opp_id);
        }
        self.evaluate_opponents(sim, assets, entity_id);
    }

    // ─── Experience points ──────────────────────────────────────────

    /// Award sword kill experience to the attacker.
    pub(super) fn award_sword_kill_xp(
        &mut self,
        assets: &LevelAssets,
        attacker_id: EntityId,
        victim_id: EntityId,
    ) {
        // Only PCs can receive XP (they have HumanStatus via Campaign)
        let attacker_is_pc = self
            .get_entity(attacker_id)
            .map(|e| e.kind().is_pc())
            .unwrap_or(false);
        if !attacker_is_pc {
            return;
        }

        let mut xp = combat::SWORD_KILL_EXPERIENCE_POINTS;

        // Bonus if victim was more skilled than attacker.
        let victim_capacity: u32 = self
            .get_entity(victim_id)
            .map(|e| {
                fighting_ability_from_profile(
                    e,
                    &assets.profile_manager,
                    self.control.sim_config.difficulty,
                ) as u32
            })
            .unwrap_or(0);
        let attacker_capacity: u32 = self
            .get_entity(attacker_id)
            .map(|e| {
                fighting_ability_from_profile(
                    e,
                    &assets.profile_manager,
                    self.control.sim_config.difficulty,
                ) as u32
            })
            .unwrap_or(0);

        if victim_capacity > attacker_capacity {
            xp += victim_capacity - attacker_capacity;
        }

        // Apply XP through campaign
        let profile_idx = self
            .get_entity(attacker_id)
            .and_then(|e| match e {
                Entity::Pc(pc) => Some(pc.pc.profile_index),
                _ => None,
            })
            .unwrap_or_default();
        if let Some(campaign) = Some(&mut self.mission_domain.campaign) {
            // The PC experience-add awards a campaign-score bonus
            // whenever the call crosses a 100-XP boundary.
            campaign.add_pc_experience(
                usize::from(profile_idx),
                crate::pc_status::SkillName::HandToHand,
                xp,
            );
            tracing::debug!(
                attacker = ?attacker_id,
                xp,
                "Awarded sword kill XP"
            );
        }
    }

    // ─── PC coma / amulet death-save ────────────────────────────────

    /// Check if a PC should be saved from death by an amulet (coma mechanic).
    ///
    /// If the PC is a VIP, not already in coma, and the campaign has amulets,
    /// the PC survives with 5 HP + max concussion instead of dying.
    ///
    /// Returns `true` if the coma save activated (caller should NOT
    /// proceed with normal death handling).
    pub(super) fn try_pc_coma_save(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        pc_id: EntityId,
        damage: u16,
    ) -> bool {
        let (is_pc, life_points, is_vip, status_idx) = {
            let entity = match self.get_entity(pc_id) {
                Some(e) => e,
                None => return false,
            };
            match entity {
                Entity::Pc(pc) => {
                    let vip = assets
                        .profile_manager
                        .get_character(pc.pc.profile_index)
                        .map(|p| p.vip)
                        .unwrap_or(false);
                    let Some(status_idx) = self.pc_description_index_for_pc_data(&pc.pc) else {
                        return false;
                    };
                    (true, pc.pc.life_points, vip, status_idx)
                }
                _ => return false,
            }
        };
        if !is_pc || damage < life_points as u16 {
            return false;
        }

        // Check if already in coma
        let in_coma = Some(&self.mission_domain.campaign)
            .and_then(|c| c.characters.get(status_idx))
            .map(|desc| desc.status.in_coma)
            .unwrap_or(false);
        if in_coma {
            return false;
        }

        // Check amulets
        let has_amulets = is_vip
            && Some(&self.mission_domain.campaign)
                .map(|c| c.values[crate::campaign::CampaignValue::Amulets] >= 1)
                .unwrap_or(false);
        if !has_amulets {
            return false;
        }

        // Activate coma save
        tracing::info!(entity = ?pc_id, "PC coma save activated — amulet consumed");

        // Set life to 5, max concussion, consume amulet
        if let Some(Entity::Pc(pc)) = self.world.entities.get_mut(pc_id) {
            pc.pc.life_points = 5;
            pc.human.concussion_of_the_brain = combat::CONCUSSION_MAX;
            pc.human.unconscious = true;
            pc.element.set_posture(Posture::Lying);
        }
        if let Some(campaign) = Some(&mut self.mission_domain.campaign) {
            if let Some(desc) = campaign.characters.get_mut(status_idx) {
                desc.status.in_coma = true;
            }
            campaign.values[crate::campaign::CampaignValue::Amulets] -= 1;
        }
        // Play the PC-in-coma jingle once at the coma-transition
        // site (the dominant trigger in the reference is the portrait
        // burn invoked by the messenger when the PC enters coma).
        self.feedback
            .pending_side_effects
            .sounds
            .push(super::SoundCommand::Jingle(crate::sound::Jingle::PcInComa));
        // Wipe the PC's quick-action macro slots so a later coma
        // revive doesn't bring back pre-coma macro bindings.
        for slot in 0..crate::macro_store::NUMBER_OF_QA_MEMORY as u8 {
            self.abort_quick_action(pc_id, slot);
        }
        // Close eyes / stop combat
        if let Some(entity) = self.world.entities.get_mut(pc_id)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.action_state = ActionState::Waiting;
            actor.active_melee.clear();
            actor.clear_path();
        }
        self.quit_swordfight(sim, assets, pc_id);

        // Add unconscious star titbit (event-driven creation).
        self.add_unconscious_star(pc_id);
        true
    }
}
