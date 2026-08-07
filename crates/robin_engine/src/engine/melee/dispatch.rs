//! Sword/parry/shield command dispatch entry points.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::element::{ActionState, Command, EntityId};
use crate::engine::sequence_runtime::OwnerActionBarrier;
use crate::sequence::SequenceElementData;
use crate::weapons::SwordStrike;

impl EngineInner {
    // ─── Sword strike dispatch (sequence-driven) ────────────────────

    /// Dispatch a sword strike command from the sequence system.
    ///
    /// Called when an `InstructOwner` action delivers a strike command
    /// (e.g. `SwordstrikeThrustA`) to an actor. The resulting sequence order
    /// is the complete runtime identity of the strike, as in Original.
    ///
    /// Handles the `SwordstrikeThrustA..I` strike commands.
    pub(crate) fn dispatch_sword_strike(
        &mut self,
        _sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        target: EntityId,
        strike: SwordStrike,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        // Validate attacker
        let owner_ok = self
            .get_entity(owner)
            .map(|e| e.is_human() && !e.is_dead())
            .unwrap_or(false);
        if !owner_ok {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return;
        }

        // Validate target
        let target_ok = self
            .get_entity(target)
            .map(|e| e.is_human() && !e.is_dead())
            .unwrap_or(false);
        if !target_ok {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return;
        }

        if strike == SwordStrike::A {
            if can_enter_swordfight_with(
                &self.world.entities,
                owner,
                target,
                &assets.profile_manager,
                &self.world.fast_grid,
            ) {
                self.set_as_new_principal_opponent(assets, owner, target);
                self.set_as_new_principal_opponent(assets, target, owner);
            } else {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                return;
            }
        }

        let anim = strike_to_animation(strike);
        // Read target position for the animation order
        let (tx, ty) = self
            .get_entity(target)
            .map(|e| {
                (
                    e.element_data().position_map().x,
                    e.element_data().position_map().y,
                )
            })
            .unwrap_or((0.0, 0.0));

        if let Some(entity) = self.world.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.clear_path();
        }

        // RHElementActorHuman::Translate stores the target as the order's
        // pAntagonist. Execute derives both the target and strike type from
        // this selected order; there is no parallel melee state object.
        let mut order = crate::order::Order::new(anim, tx, ty, self.orders.allocate_order_id());
        order.target_actor = Some(target.index());
        order.antagonist = Some(target);
        order.compute_direction = false;
        self.orders
            .sequence_manager
            .push_order_on(seq_id, elem_idx, order);

        self.orders
            .sequence_manager
            .element_in_progress(seq_id, elem_idx);

        tracing::debug!(
            attacker = ?owner,
            target = ?target,
            ?strike,
            "Sword strike dispatched"
        );
    }

    // ─── Enter / quit swordfight ────────────────────────────────────

    /// Dispatch an EnterSwordfight command.
    ///
    /// Establishes the fight relationship and queues the transition into the
    /// sword pose.  Execute owns the action-state and facing changes.
    pub(in crate::engine) fn dispatch_enter_swordfight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        opponent: Option<EntityId>,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        {
            let Some(entity) = self.world.entities.get_mut(owner) else {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                return OwnerActionBarrier::Skip;
            };
            if entity.is_dead() || entity.human_data().map(|h| h.unconscious).unwrap_or(true) {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                return OwnerActionBarrier::Skip;
            }
        }

        // Table swordfight positioning: when entering a swordfight
        // whose opponent sits in a different sector, walk the
        // jump-line graph to find a free slot among any fighters
        // already engaged from our side.  If the jump line already has
        // 3+ fighters on our side, interrupt.  Otherwise launch a
        // movement element to nudge ourselves to the free slot before
        // raising the sword.
        let swordfight_prepared = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .is_some_and(is_swordfight_prepared);
        if !swordfight_prepared
            && let Some(opp) = opponent
            && opp != owner
            && let Some(jl_idx) = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| e.get_property(crate::sequence::Field::JumplineDestination))
                .and_then(|v| match v {
                    crate::sequence::FieldValue::LineId(id) if id.get() != 0 => Some(*id),
                    _ => None,
                })
        {
            match self.try_launch_table_swordfight_move(owner, opp, jl_idx.get()) {
                TableFightMove::Abort => {
                    self.orders
                        .sequence_manager
                        .element_impossible(seq_id, elem_idx);
                    return OwnerActionBarrier::Skip;
                }
                TableFightMove::Launched => {
                    let element = self
                        .orders
                        .sequence_manager
                        .get_element_mut(seq_id, elem_idx)
                        .unwrap_or_else(|| {
                            panic!(
                                "EnterSwordfight element {seq_id:?}:{elem_idx} disappeared \
                                 while preparing table movement"
                            )
                        });
                    mark_swordfight_prepared(element);
                }
                TableFightMove::Ok => {}
            }
        }

        let owner_action_state = self
            .world
            .entities
            .get(owner)
            .and_then(|entity| entity.actor_data())
            .map(|actor| actor.action_state)
            .unwrap_or(ActionState::Waiting);
        let transition = match owner_action_state {
            ActionState::WaitingSword => None,
            ActionState::Menacing => Some(crate::order::OrderType::TransitionMenacingWaitingSword),
            _ => Some(crate::order::OrderType::TransitionRaisingSword),
        };
        // The EnterSwordfight sequence element carries the
        // opponent (set by apply_enter_swordfight when the player
        // clicked a sword-target, or by the AI side on reciprocal
        // entry).  Run the full `enter_swordfight` engine path so both
        // entities get added to each other's opponent lists and the
        // cursor / is_selected_pc_swordfighting flag flips on.
        // Without this, action_state changed to WaitingSword but the
        // opponents list stayed empty — the cursor kept showing the
        // non-combat pointer and no strikes were possible.
        if let Some(opp) = opponent
            && opp != owner
        {
            // Re-read JumplineDestination from the element so it lands
            // in the opponent list as the aggressor's table-swordfight
            // line.
            let aggressor_jl = self
                .orders
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| e.get_property(crate::sequence::Field::JumplineDestination))
                .and_then(|v| match v {
                    crate::sequence::FieldValue::LineId(id) if id.get() != 0 => Some(*id),
                    _ => None,
                });
            self.enter_swordfight_with_jump_line(sim, assets, owner, opp, false, aggressor_jl);
        }
        if let Some(transition) = transition {
            let id = self.orders.allocate_order_id();
            let mut order = crate::order::Order::new(transition, 0.0, 0.0, id);
            if let Some(opp) = opponent.filter(|opp| *opp != owner) {
                order = order.with_antagonist(opp);
            }
            self.orders
                .sequence_manager
                .push_order_on(seq_id, elem_idx, order);
        }
        if transition.is_some() {
            self.orders
                .sequence_manager
                .element_in_progress(seq_id, elem_idx);
            OwnerActionBarrier::Reach
        } else {
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
            // SetState(Terminated) synchronously sends the condolence card.
            // If that callback changes mpSequenceElement, Actor::Instruct
            // returns before its accepted-motion/order epilogue.
            OwnerActionBarrier::Skip
        }
    }

    /// Handle the table-swordfight position check on entering a
    /// cross-sector swordfight.  Returns `Abort` when the line is
    /// oversubscribed or the slot is unreachable, `Launched` when a
    /// movement element was enqueued, `Ok` otherwise (no move needed).
    ///
    pub(super) fn try_launch_table_swordfight_move(
        &mut self,
        owner: EntityId,
        opp: EntityId,
        jl_idx: u32,
    ) -> TableFightMove {
        let (owner_sector, owner_pos, owner_layer, owner_move_box) = {
            let Some(e) = self.get_entity(owner) else {
                return TableFightMove::Abort;
            };
            let Some(sector) = e.element_data().sector() else {
                return TableFightMove::Ok;
            };
            let pos = e.element_data().position_map();
            let layer = e.element_data().layer();
            let mb = *e.position_iface().get_move_box();
            (i16::from(sector), pos, layer, mb)
        };
        let opp_sector = match self.get_entity(opp).and_then(|e| e.element_data().sector()) {
            Some(s) => i16::from(s),
            None => return TableFightMove::Ok,
        };
        // Same-sector fights skip the positioning entirely.
        if owner_sector == opp_sector {
            return TableFightMove::Ok;
        }

        let table_count =
            number_of_table_swordfight_opponents(&self.world.entities, opp, owner_sector);
        // No existing fighters from our side → no slotting needed; the
        // caller's pre-move (`apply_table_swordfight`) already placed us.
        if table_count == 0 {
            return TableFightMove::Ok;
        }
        if table_count >= 3 {
            return TableFightMove::Abort;
        }

        let jump_line = match self.world.fast_grid.level.jump_lines.get(jl_idx as usize) {
            Some(jl) => jl.clone(),
            None => return TableFightMove::Abort,
        };

        let Some(new_pos) = find_position_for_table_swordfight(
            &self.world.entities,
            owner_pos,
            owner_sector,
            owner,
            opp,
            &jump_line,
        ) else {
            return TableFightMove::Abort;
        };

        // MaxNorm == max(|dx|, |dy|); matches the 1-unit dead-zone
        // below which the position is considered already reached.
        let dx = new_pos.x - owner_pos.x;
        let dy = new_pos.y - owner_pos.y;
        if dx.abs().max(dy.abs()) < 1.0 {
            return TableFightMove::Ok;
        }

        if !self.world.fast_grid.is_straight_movement_authorized(
            owner_pos,
            new_pos,
            owner_layer,
            &owner_move_box,
        ) {
            return TableFightMove::Abort;
        }

        // Launch the positioning move as a standalone element — it
        // runs in parallel with the rest of the ENTER_SWORDFIGHT
        // dispatch, then falls through to `EnterSwordFight`.
        let mut move_elem = crate::sequence::SequenceElement::new_movement(
            1,
            crate::element::Command::Move,
            Some(owner),
            crate::order::OrderType::WalkingUpright,
        );
        if let crate::sequence::SequenceElementData::Movement {
            destination,
            flags,
            tolerance,
            line_id,
            ..
        } = &mut move_elem.data
        {
            *destination = crate::coordinates::MapPoint {
                x: new_pos.x,
                y: new_pos.y,
            };
            // STRAIGHT: go in a line, no gates.  LINE + `line_id`
            // plumb the jump-line goal so downstream arrival code can
            // snap to line tolerance.
            *flags |= crate::sequence::MoveFlags::STRAIGHT | crate::sequence::MoveFlags::LINE;
            *line_id = crate::jump_line::JumpLineIndex::new(jl_idx);
            *tolerance = 0.0;
        }
        move_elem.priority = crate::sequence::SequencePriority::PostponeEverythingButInjuries;
        self.launch_element(move_elem);
        TableFightMove::Launched
    }

    /// Dispatch a QuitSwordfight command.
    ///
    /// Transitions the entity out of sword-fighting action state.
    pub(in crate::engine) fn dispatch_quit_swordfight(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let queue_lower = self
            .world
            .entities
            .get(owner)
            .and_then(Entity::actor_data)
            .is_some_and(|actor| actor.action_state.is_sword());

        // A higher-priority explicit quit can postpone an independently
        // selected sword movement. Original mutates that surviving movement's
        // logical action before it is translated again after lowering; leaving
        // it as `*_WITH_SWORD` would make the resumed Execute immediately quit
        // a second time because the relationship is now empty.
        let postponed = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .unwrap_or_else(|| {
                panic!(
                    "dispatch_quit_swordfight: missing element ({seq_id:?}, {elem_idx}) \
                     for {owner:?}"
                )
            })
            .cross_postponed;
        if let Some((postponed_sequence, postponed_index)) = postponed {
            self.rewrite_sword_movement_for_fight_exit(
                postponed_sequence,
                postponed_index,
                owner,
                false,
            );
        }

        // The explicit command owns the visible transition. Relationship
        // cleanup itself must not lower the sword, and action state stays
        // sword-ready until the transition order actually starts.
        self.quit_swordfight(sim, assets, owner);
        if queue_lower {
            let id = self.orders.allocate_order_id();
            self.orders.sequence_manager.push_order_on(
                seq_id,
                elem_idx,
                crate::order::Order::new(
                    crate::order::OrderType::TransitionLoweringSword,
                    0.0,
                    0.0,
                    id,
                ),
            );
            self.orders
                .sequence_manager
                .element_in_progress(seq_id, elem_idx);
            OwnerActionBarrier::Reach
        } else {
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
            OwnerActionBarrier::Skip
        }
    }

    // ─── Parry ──────────────────────────────────────────────────────

    /// Dispatch a ParrySword command.
    pub(in crate::engine) fn dispatch_parry_sword(
        &mut self,
        owner: EntityId,
        low: bool,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let Some(entity) = self.world.entities.get(owner) else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        let Some(actor) = entity.actor_data() else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };

        if !matches!(
            actor.action_state,
            ActionState::WaitingSword
                | ActionState::MovingSword
                | ActionState::MovingFastSword
                | ActionState::ParryingSword
                | ActionState::ParryingSwordLow
        ) {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        }

        if matches!(
            actor.action_state,
            ActionState::ParryingSword | ActionState::ParryingSwordLow
        ) {
            // Original terminates either parry command immediately when any
            // parade is already active. It does not append another hold
            // order, even when the requested low/normal variant differs.
            // Actor::Instruct has already assigned this incoming element to
            // mpSequenceElement while Translate performs that zero-frame
            // termination. Keep the same selected identity through the
            // synchronous condolence snapshot so Actor::SendCondolationCard
            // clears the selected order and PositionGoalMap before releasing
            // any postponed predecessor.
            self.orders
                .sequence_manager
                .begin_instruct_callback(owner, seq_id, elem_idx);
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
            self.orders
                .sequence_manager
                .end_instruct_callback(owner, seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        }

        let transition = if low {
            crate::order::OrderType::TransitionWaitingSwordParryingSwordLow
        } else {
            crate::order::OrderType::TransitionWaitingSwordParryingSword
        };
        let id = self.orders.allocate_order_id();
        self.orders.sequence_manager.push_order_on(
            seq_id,
            elem_idx,
            crate::order::Order::new(transition, 0.0, 0.0, id),
        );

        let hold = if low {
            crate::order::OrderType::ParryingLowSword
        } else {
            crate::order::OrderType::ParryingSword
        };
        let id = self.orders.allocate_order_id();
        self.orders.sequence_manager.push_order_on(
            seq_id,
            elem_idx,
            crate::order::Order::new(hold, 0.0, 0.0, id),
        );
        self.orders
            .sequence_manager
            .element_in_progress(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }

    /// Dispatch a StopParrySword command.
    pub(in crate::engine) fn dispatch_stop_parry(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        let Some(entity) = self.world.entities.get(owner) else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        let Some(actor) = entity.actor_data() else {
            self.orders
                .sequence_manager
                .element_impossible(seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        };
        if !matches!(
            actor.action_state,
            ActionState::ParryingSword | ActionState::ParryingSwordLow
        ) {
            // As in the ParrySword early-exit above, Translate terminates the
            // already-selected incoming element rather than an unrelated
            // queued command.
            self.orders
                .sequence_manager
                .begin_instruct_callback(owner, seq_id, elem_idx);
            self.orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
            self.orders
                .sequence_manager
                .end_instruct_callback(owner, seq_id, elem_idx);
            return OwnerActionBarrier::Skip;
        }

        let id = self.orders.allocate_order_id();
        self.orders.sequence_manager.push_order_on(
            seq_id,
            elem_idx,
            crate::order::Order::new(
                crate::order::OrderType::TransitionParryingSwordWaitingSword,
                0.0,
                0.0,
                id,
            ),
        );
        self.orders
            .sequence_manager
            .element_in_progress(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }
}

// ─── Shield commands ────────────────────────────────────────────────

/// Shield command translation against only entity state, sequence state, and
/// order-id allocation.
///
/// Original provenance: `RHElementActorHuman::Translate` in
/// `RHelementactorhuman.cpp:2018-2054` appends each shield animation directly,
/// while `RHElementActorPC::Translate` in `RHelementactorpc.cpp:3015-3071`
/// synchronously launches a follow-up `SEEK` when an already-shielding PC gets
/// a refreshed danger/protectee command. The follow-up is returned so the
/// sequence-phase owner can launch it through the normal Instruct path before
/// performing the after-action synchronous splice.
pub(crate) struct ShieldCommandContext<'a> {
    entities: &'a mut crate::entities::Entities,
    sequence_manager: &'a mut crate::sequence::SequenceManager,
    next_order_id: &'a mut u32,
}

impl<'a> ShieldCommandContext<'a> {
    pub(crate) fn new(
        entities: &'a mut crate::entities::Entities,
        sequence_manager: &'a mut crate::sequence::SequenceManager,
        next_order_id: &'a mut u32,
    ) -> Self {
        Self {
            entities,
            sequence_manager,
            next_order_id,
        }
    }

    /// Dispatch one shield command and return an owned follow-up that must be
    /// launched synchronously before the sequence phase splices pending work.
    pub(crate) fn dispatch(
        &mut self,
        owner: EntityId,
        command: Command,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> Option<crate::sequence::SequenceElement> {
        match command {
            Command::RaiseShield => self.dispatch_raise_shield(owner, seq_id, elem_idx),
            Command::RaiseShieldInstantly => {
                self.dispatch_raise_shield_instantly(owner, seq_id, elem_idx);
                None
            }
            Command::LowerShield => {
                self.dispatch_lower_shield(owner, seq_id, elem_idx);
                None
            }
            Command::ParryShield => {
                self.dispatch_parry_shield(owner, seq_id, elem_idx);
                None
            }
            _ => unreachable!("non-shield command passed to shield command context"),
        }
    }

    /// Dispatch a RaiseShield command.
    ///
    /// If already holding shield, terminates immediately. Otherwise
    /// transitions to `HoldingShield` and queues the raising animation.
    fn dispatch_raise_shield(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> Option<crate::sequence::SequenceElement> {
        // Read danger point for facing direction.
        // Supports both Interaction data (player-issued: antagonist
        // entity position) and Generic data (AI-issued: shield danger
        // point + ShieldProtected target).  We stamp the per-PC
        // `shield_danger_point` and the bidirectional protection link
        // below.
        //
        // The read happens BEFORE the action-state branch, so an
        // "already holding shield" actor still gets its danger point
        // and protection link refreshed by the new command.
        let (danger_pt, danger_pt3d, danger_layer, new_protected) = self
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| match &e.data {
                crate::sequence::SequenceElementData::Interaction { antagonist } => {
                    let pt = antagonist.and_then(|id| {
                        self.entities
                            .get(id)
                            .map(|e| e.element_data().position_map())
                    });
                    (pt, None, None, None)
                }
                crate::sequence::SequenceElementData::Generic { properties } => {
                    use crate::sequence::{Field, FieldValue};

                    let (pt2d, pt3d) = read_shield_danger_point(properties);
                    // Picked layer travels with the danger point.  The
                    // C++ original (RHelementactorpc.cpp:3022) reads
                    // RHFIELD_SHIELD_DANGER_POINT_LAYER and feeds it to
                    // the danger-point titbit so the indicator renders
                    // on the chosen map layer rather than the PC's own.
                    let layer = match properties.get(&Field::ShieldDangerPointLayer) {
                        Some(FieldValue::Integer(v)) => Some(*v as u16),
                        _ => None,
                    };
                    let prot = match properties.get(&Field::ShieldProtected) {
                        Some(FieldValue::Element(id)) => Some(*id),
                        _ => None,
                    };
                    (pt2d, pt3d, layer, prot)
                }
                _ => (None, None, None, None),
            })
            .unwrap_or((None, None, None, None));

        // Stamp the per-PC shield danger point when the Generic
        // property carries a non-zero point. Leave it zero-initialised
        // otherwise; `sync_danger_point_titbits` skips zero danger
        // points so no titbit is created in that case.
        //
        // The layer is always overwritten so a stale player-picked
        // layer from a previous raise can't leak into a follow-up
        // AI-issued raise (which omits the layer property and thus
        // gets `None` here, falling back to the PC's own layer in
        // `sync_danger_point_titbits`).
        if let Some(pt3d) = danger_pt3d
            && (pt3d.x != 0.0 || pt3d.y != 0.0 || pt3d.z != 0.0)
            && let Some(entity) = self.entities.get_mut(owner)
            && let Some(pc) = entity.pc_data_mut()
        {
            pc.shield_danger_point = pt3d;
            pc.shield_danger_point_layer = danger_layer.unwrap_or(0);
        }
        // Only call `SetShieldProtected` when the Generic property is
        // non-null.
        if let Some(prot) = new_protected {
            if let Some(pc) = self.entities.get_mut(owner).and_then(Entity::pc_data_mut) {
                pc.shield_protected = Some(prot);
            }
        }

        // Action-state branch.  If already shielding (HOLDING_SHIELD or
        // MOVING_SHIELD), terminate the RAISE_SHIELD element — only
        // the danger-point/protected updates above are wanted — and,
        // when a protectee is set, launch a fresh SEEK so the
        // protector follows the protectee.  WALKING_UPRIGHT with
        // tolerance 50 when the danger point is zero; tolerance 0 +
        // SEEK_SHIELD when a danger point is set.
        let action_state = self
            .entities
            .get(owner)
            .and_then(|e| e.actor_data())
            .map(|a| a.action_state);
        match action_state {
            Some(ActionState::HoldingShield) | Some(ActionState::MovingShield) => {
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                let protected_now = self
                    .entities
                    .get(owner)
                    .and_then(|e| e.pc_data())
                    .and_then(|pc| pc.shield_protected);
                if let Some(target) = protected_now {
                    let danger_zero = self
                        .entities
                        .get(owner)
                        .and_then(|e| e.pc_data())
                        .map(|pc| {
                            pc.shield_danger_point.x == 0.0
                                && pc.shield_danger_point.y == 0.0
                                && pc.shield_danger_point.z == 0.0
                        })
                        .unwrap_or(true);
                    let mut seek = crate::sequence::SequenceElement::new_movement(
                        1,
                        Command::Seek,
                        Some(owner),
                        crate::order::OrderType::WalkingUpright,
                    );
                    if let SequenceElementData::Movement {
                        element,
                        tolerance,
                        flags,
                        ..
                    } = &mut seek.data
                    {
                        *element = Some(target);
                        if danger_zero {
                            *tolerance = 50.0;
                            *flags |= crate::sequence::MoveFlags::SEEK;
                        } else {
                            *tolerance = 0.0;
                            *flags |= crate::sequence::MoveFlags::SEEK
                                | crate::sequence::MoveFlags::SEEK_SHIELD;
                        }
                    }
                    return Some(seek);
                }
                return None;
            }
            Some(s) if s.is_sword() || s.is_bow() => {
                // Defensive gate (must be Waiting / Alerted / holding
                // shield): the transition machine should already have
                // rejected this, but terminate cleanly if it slips
                // through.
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                return None;
            }
            None => {
                self.sequence_manager.element_impossible(seq_id, elem_idx);
                return None;
            }
            _ => {} // Waiting, Bored, ParryingShield, etc. — proceed.
        }

        let mut started = false;
        if let Some(entity) = self.entities.get_mut(owner) {
            // The PC override faces the picked danger point. Enemy NPCs use
            // the human-base RaiseShield implementation instead: their AI
            // has already called Focus, and the non-directional shield order
            // must preserve that direction goal.
            if entity.is_pc()
                && let Some(pt) = danger_pt
            {
                let owner_pos = entity.element_data().position_map();
                let dx = pt.x - owner_pos.x;
                let dy = pt.y - owner_pos.y;
                if dx != 0.0 || dy != 0.0 {
                    let goal = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
                    if entity.actor_data().is_some() {
                        entity.position_iface_mut().set_direction(
                            crate::position_interface::Direction::from_raw(goal as i32),
                        );
                    }
                }
            }

            if let Some(actor) = entity.actor_data_mut() {
                // Don't set HoldingShield immediately — the animation
                // tick will set it when the raising animation
                // completes (on MotionState::Done →
                // SetStates(Upright, HoldingShield)).
                actor.clear_path();
                actor.shield_face_point = danger_pt;
                started = true;
            }
            entity.set_posture(Posture::Upright);
        }
        if started {
            // Push the order onto the element so `do_next_order` sees
            // an exhaustion when the animation terminates.  The
            // shield-arm `dispatch_arm_completion` entry in
            // `engine/animation.rs` gates advance on TERMINATED only
            // so the side-effect `SetStates(Upright, HoldingShield)`
            // on Done doesn't also pop the order mid-play.
            self.push_order(seq_id, elem_idx, crate::order::OrderType::RaisingShield);
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
        } else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        }
        None
    }

    /// Dispatch a RaiseShieldInstantly command.
    ///
    /// Sets `HoldingShield` immediately without a raising animation.
    fn dispatch_raise_shield_instantly(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        if let Some(entity) = self.entities.get_mut(owner) {
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::HoldingShield;
                actor.clear_path();
            }
            entity.set_posture(Posture::Upright);
        }
        self.push_order(seq_id, elem_idx, crate::order::OrderType::WaitingShield);
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }

    /// Dispatch a LowerShield command.
    ///
    /// Transitions out of shield state to `Waiting` with a lowering animation.
    fn dispatch_lower_shield(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let mut started = false;
        if let Some(entity) = self.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
            && actor.action_state.is_shield()
        {
            // Don't set Waiting immediately — the animation tick
            // will set it when the lowering animation completes (on
            // MotionState::Done → SetStates(Upright, Waiting)).
            // The sprite-anim fallback to TRANSITION_LOWERING_SWORD
            // when the actor has no LOWERING_SHIELD anim is applied
            // by the animation driver — the *order's* action stays
            // LOWERING_SHIELD, only the played sprite differs.
            actor.shield_face_point = None;
            started = true;
        }
        if started {
            // The order's animation field is `LOWERING_SHIELD`; the
            // sprite-anim fallback to `TRANSITION_LOWERING_SWORD`
            // happens at perform_action time only.  The shield-arm
            // `dispatch_arm_completion` entry gates advance on
            // TERMINATED only so Done fires the action-state flip
            // without retiring the order.
            self.push_order(seq_id, elem_idx, crate::order::OrderType::LoweringShield);
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
        } else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        }
    }

    /// Dispatch a ParryShield command.
    ///
    /// Transitions to `ParryingShield` from a shield-holding state.
    fn dispatch_parry_shield(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let mut started = false;
        if let Some(entity) = self.entities.get_mut(owner)
            && let Some(actor) = entity.actor_data_mut()
        {
            // Requires the actor to currently be holding the shield.  The
            // entry transition does not itself move the actor into
            // `ParryingShield`: the actor keeps holding the shield for the
            // whole parry animation and only reports parrying once that
            // animation completes.
            if actor.action_state == ActionState::HoldingShield
                || actor.action_state == ActionState::ParryingShield
            {
                started = true;
            }
        }
        if started {
            // Order action is `PARRYING_SHIELD`; the sprite-anim
            // fallback to `PARRYING_SWORD` happens at perform_action
            // time only.  The shield-arm `dispatch_arm_completion`
            // entry gates advance on TERMINATED only so the parry
            // sprite plays all the way through before the side-effect
            // handler returns to HoldingShield.
            self.push_order(seq_id, elem_idx, crate::order::OrderType::ParryingShield);
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
        } else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        }
    }

    fn push_order(
        &mut self,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
        order_type: crate::order::OrderType,
    ) {
        let id = crate::order::alloc_order_id(self.next_order_id);
        // All four Original shield translators explicitly disable direction
        // recomputation (`RHelementactorhuman.cpp:2018-2054`).  These are
        // posture-local animations: facing is controlled by Focus/the shield
        // danger point before translation, and selecting the new order must
        // not derive a fresh goal from its zero-valued destination.
        let mut order = crate::order::Order::new(order_type, 0.0, 0.0, id);
        order.compute_direction = false;
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
    }
}

impl EngineInner {
    // ─── Receive damage dispatch ────────────────────────────────────

    /// Complete Actor::Instruct's accepted-empty-order path for damage.
    ///
    /// Original publishes `RHMOTION_IN_PROGRESS`, discovers that Translate
    /// produced no current order, clears `mpSequenceElement`, and only then
    /// terminates the accepted element. Its condolence card therefore cannot
    /// clear a movement goal belonging to the element that will resume next.
    fn terminate_accepted_empty_damage(
        &mut self,
        victim_id: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        self.world
            .entities
            .get_mut(victim_id)
            .and_then(Entity::actor_data_mut)
            .expect("accepted empty damage lost its actor")
            .continuation
            .motion_state = crate::sprite::MotionState::InProgress;
        self.orders.sequence_manager.set_translating_element(None);
        self.orders
            .sequence_manager
            .element_terminated(seq_id, elem_idx);
        OwnerActionBarrier::Reach
    }

    /// Dispatch a receive-damage command from the sequence system.
    ///
    /// Reads damage data from the sequence element, applies it to the
    /// victim, and handles death/KO transitions.  Handles
    /// `ReceiveSwordDamage`, `ReceiveDamage`, `ReceiveHitDamage`, etc.
    pub(in crate::engine) fn dispatch_receive_damage(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        victim_id: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) -> OwnerActionBarrier {
        // Read damage data from the sequence element
        let elem = match self.orders.sequence_manager.get_element(seq_id, elem_idx) {
            Some(e) => e,
            None => return OwnerActionBarrier::Skip,
        };
        let command = elem.command;

        let (
            origin,
            projectile,
            damage,
            concussion,
            sword_strike,
            sword_profile_idx,
            is_harder_hit,
        ) = match &elem.data {
            SequenceElementData::Damage {
                origin,
                projectile,
                damage,
                concussion,
                sword_strike,
                sword_profile_idx,
                is_harder_hit,
            } => (
                *origin,
                projectile.or_else(|| {
                    elem.legacy_v48
                        .as_ref()
                        .and_then(|legacy| legacy.damage_arrow)
                }),
                *damage,
                *concussion,
                *sword_strike,
                *sword_profile_idx,
                *is_harder_hit,
            ),
            _ => {
                tracing::warn!(
                    ?victim_id,
                    ?command,
                    "dispatch_receive_damage: element is not Damage"
                );
                return self.terminate_accepted_empty_damage(victim_id, seq_id, elem_idx);
            }
        };

        // Apply damage based on command type.  Per-command `apply_*`
        // functions are responsible for applying the civilian-with-
        // attached-scroll immunity check: nets land, everything else
        // is a no-op on a scroll-carrying beggar.
        match command {
            Command::ReceiveSwordDamage => {
                // Sword damage is the one receive-damage command gated on
                // the element owner still being active.  A victim that has
                // left the world (walked into a building, been removed)
                // between the strike landing in the sweep queue and the
                // sequence manager dispatching this element takes no damage
                // at all — the element is terminated without rolling the
                // two protection draws.
                let owner_active = match self.get_entity(victim_id) {
                    Some(e) => e.is_active(),
                    None => {
                        tracing::warn!(
                            ?victim_id,
                            "dispatch_receive_damage: sword damage victim is gone"
                        );
                        false
                    }
                };
                if !owner_active {
                    self.orders
                        .sequence_manager
                        .element_terminated(seq_id, elem_idx);
                    return OwnerActionBarrier::Skip;
                }
                #[cfg(test)]
                let test_life_before = self
                    .get_entity(victim_id)
                    .and_then(super::damage::test_human_life_points)
                    .expect("sword damage test victim is human");
                self.apply_sword_damage(
                    sim,
                    assets,
                    victim_id,
                    origin,
                    sword_strike,
                    sword_profile_idx,
                    (seq_id, elem_idx),
                );
                #[cfg(test)]
                if let (Some(attacker_id), Some(strike)) = (origin, sword_strike) {
                    super::damage::record_test_sword_damage_observation(
                        self,
                        victim_id,
                        attacker_id,
                        strike,
                        test_life_before,
                    );
                }
                // ExecuteFallingPushed / ExecuteRolling marks the
                // damage element NonInterruptable directly when those
                // anims start.  Here, `queue_damage_anim` does the
                // equivalent inline `set_element_priority` call when
                // the falling/rolling order is pushed onto the
                // element — no separate propagation step is needed.
            }
            Command::ReceiveDamage | Command::ReceiveMobileDamage => {
                self.apply_generic_damage(
                    sim,
                    assets,
                    victim_id,
                    damage,
                    concussion,
                    (seq_id, elem_idx),
                );
            }
            Command::ReceiveArrowDamage | Command::ReceiveStoneDamage => {
                // `HitHuman` checks arrow hurtability before it registers
                // this element. Do not repeat that check here: the
                // intervening EventGetArrow callback is allowed to change
                // the victim's AI state before damage executes.
                let victim_active = self
                    .get_entity(victim_id)
                    .is_some_and(|victim| victim.element_data().active);
                if !victim_active {
                    self.orders
                        .sequence_manager
                        .element_terminated(seq_id, elem_idx);
                    return OwnerActionBarrier::Skip;
                }
                self.apply_piercing_damage(
                    sim,
                    assets,
                    victim_id,
                    damage,
                    concussion,
                    command == Command::ReceiveArrowDamage,
                    (seq_id, elem_idx),
                );

                if command == Command::ReceiveArrowDamage {
                    // Original performs these after ReceivePiercingDamage /
                    // TranslateArrowDamage, while executing the deferred
                    // damage element. The projectile remains retained as a
                    // tombstone, exactly like the Original element pointer.
                    if self
                        .get_entity(victim_id)
                        .is_some_and(|victim| get_life_points(victim) <= 0)
                    {
                        let shooter = origin.expect("arrow damage element has no shooter");
                        self.award_bow_kill_xp(shooter);
                    }
                    if let Some(projectile_id) = projectile {
                        let direction = match self.get_entity(projectile_id) {
                            Some(crate::element::Entity::Projectile(projectile)) => {
                                projectile.projectile.flight_direction as i16
                            }
                            Some(other) => panic!(
                                "arrow damage projectile {projectile_id:?} is {:?}",
                                other.kind()
                            ),
                            None => panic!(
                                "arrow damage projectile {projectile_id:?} disappeared before dispatch"
                            ),
                        };
                        self.get_entity_mut(victim_id)
                            .expect("arrow damage victim disappeared after damage")
                            .element_data_mut()
                            .set_direction_instantly(direction ^ 8);
                    }
                }
            }
            Command::ReceiveHitDamage => {
                if self
                    .get_entity(victim_id)
                    .is_some_and(|victim| victim.element_data().posture == Posture::Lying)
                {
                    // Human::Translate(RECEIVE_HIT_DAMAGE) calls SetState
                    // directly when the victim is already lying. That
                    // changes mpSequenceElement and bypasses Instruct's
                    // accepted motion/order epilogue.
                    self.orders
                        .sequence_manager
                        .element_terminated(seq_id, elem_idx);
                    return OwnerActionBarrier::Skip;
                }
                self.apply_hit_damage(
                    sim,
                    assets,
                    victim_id,
                    origin,
                    concussion,
                    is_harder_hit,
                    (seq_id, elem_idx),
                );
            }
            Command::ReceiveNet => {
                self.apply_net(victim_id);
            }
            _ => {
                tracing::warn!(
                    ?command,
                    "dispatch_receive_damage: unhandled damage command"
                );
            }
        }

        // DoNextOrder boot: if the damage handler pushed any orders
        // (the sword-damage path pushes simpleHit / standup /
        // BeingStunnedSword), let the element keep running so
        // `do_next_order` chains through on each MotionState::Terminated.
        // Order ids are stamped at construction time (`Order::new`
        // requires `NonZeroU32`), so no batch fixup is needed here.
        // Otherwise terminate now.
        let order_count = self
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.orders.len())
            .unwrap_or(0);
        if order_count > 0 && self.get_entity(victim_id).is_some() {
            self.orders
                .sequence_manager
                .element_in_progress(seq_id, elem_idx);
            return OwnerActionBarrier::Reach;
        }
        self.terminate_accepted_empty_damage(victim_id, seq_id, elem_idx)
    }
}

fn is_swordfight_prepared(element: &crate::sequence::SequenceElement) -> bool {
    matches!(
        element.get_property(crate::sequence::Field::SwordfightPrepared),
        Some(crate::sequence::FieldValue::Bool(true))
    )
}

fn mark_swordfight_prepared(element: &mut crate::sequence::SequenceElement) {
    let crate::sequence::SequenceElementData::Generic { properties } = &mut element.data else {
        panic!("EnterSwordfight preparation requires a generic sequence element");
    };
    properties.insert(
        crate::sequence::Field::SwordfightPrepared,
        crate::sequence::FieldValue::Bool(true),
    );
}

fn read_shield_danger_point(
    properties: &std::collections::HashMap<crate::sequence::Field, crate::sequence::FieldValue>,
) -> (
    Option<crate::coordinates::MapPoint>,
    Option<crate::coordinates::WorldPoint3D>,
) {
    use crate::sequence::{Field, FieldValue};

    match properties.get(&Field::ShieldDangerPoint) {
        Some(FieldValue::Point3D { x, y, z }) => (
            Some(crate::coordinates::MapPoint::new(*x, *y)),
            Some(crate::coordinates::WorldPoint3D {
                x: *x,
                y: *y,
                z: *z,
            }),
        ),
        Some(FieldValue::GeoPoint2D { x, y }) => (
            Some(crate::coordinates::MapPoint::new(*x, *y)),
            Some(crate::coordinates::WorldPoint3D {
                x: *x,
                y: *y,
                z: 0.0,
            }),
        ),
        _ => (None, None),
    }
}

#[cfg(test)]
mod swordfight_preparation_tests {
    use super::{is_swordfight_prepared, mark_swordfight_prepared};
    use crate::element::Command;
    use crate::sequence::{Field, FieldValue, SequenceElement};

    #[test]
    fn table_preparation_marker_is_persistent_and_legacy_missing_means_false() {
        let mut element = SequenceElement::new_generic(1, Command::EnterSwordfight, None);
        assert!(!is_swordfight_prepared(&element));

        element.set_property(Field::SwordfightPrepared, FieldValue::Bool(false));
        assert!(!is_swordfight_prepared(&element));

        mark_swordfight_prepared(&mut element);
        assert!(is_swordfight_prepared(&element));
        assert!(matches!(
            element.get_property(Field::SwordfightPrepared),
            Some(FieldValue::Bool(true))
        ));
    }
}

#[cfg(test)]
mod shield_order_tests {
    use super::ShieldCommandContext;
    use crate::entities::Entities;
    use crate::order::OrderType;
    use crate::sequence::{Sequence, SequenceElement, SequenceManager};
    use crate::{element::Command, sequence::SequenceState};

    #[test]
    fn translated_shield_orders_never_recompute_facing() {
        for order_type in [
            OrderType::RaisingShield,
            OrderType::WaitingShield,
            OrderType::LoweringShield,
            OrderType::ParryingShield,
        ] {
            let mut sequence_manager = SequenceManager::new();
            let mut sequence = Sequence::new();
            sequence.append_element(SequenceElement::new_generic(1, Command::Wait, None));
            let sequence_id = sequence_manager.launch_sequence(sequence);
            let mut entities = Entities::new();
            let mut next_order_id = 1;

            ShieldCommandContext {
                entities: &mut entities,
                sequence_manager: &mut sequence_manager,
                next_order_id: &mut next_order_id,
            }
            .push_order(sequence_id, 0, order_type);

            let element = sequence_manager
                .get_element(sequence_id, 0)
                .expect("shield test sequence element");
            assert_eq!(element.state, SequenceState::Todo);
            assert_eq!(element.orders.len(), 1);
            assert_eq!(element.orders[0].order_type, order_type);
            assert!(!element.orders[0].compute_direction);
        }
    }
}
