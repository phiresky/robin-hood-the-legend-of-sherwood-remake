//! Sword/parry/shield command dispatch entry points.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::element::{ActionState, Command, EntityId, Posture};
use crate::movement::ActiveMelee;
use crate::sequence::SequenceElementData;
use crate::weapons::SwordStrike;

impl EngineInner {
    // ─── Sword strike dispatch (sequence-driven) ────────────────────

    /// Dispatch a sword strike command from the sequence system.
    ///
    /// Called when an `InstructOwner` action delivers a strike command
    /// (e.g. `SwordstrikeThrustA`) to an actor. Sets up `ActiveMelee`
    /// on the attacker and marks the sequence element in-progress.
    ///
    /// Handles the `SwordstrikeThrustA..I` strike commands.
    pub(crate) fn dispatch_sword_strike(
        &mut self,
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
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        }

        // Validate target
        let target_ok = self
            .get_entity(target)
            .map(|e| e.is_human() && !e.is_dead())
            .unwrap_or(false);
        if !target_ok {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        }

        if strike == SwordStrike::A {
            if can_enter_swordfight_with(
                &self.entities,
                owner,
                target,
                &assets.profile_manager,
                &self.fast_grid,
            ) {
                self.set_as_new_principal_opponent(assets, owner, target);
                self.set_as_new_principal_opponent(assets, target, owner);
            } else {
                self.sequence_manager.element_impossible(seq_id, elem_idx);
                return;
            }
        }

        // Face the target
        let dir = direction_to(&self.entities, owner, target);
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

        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) {
            entity.element_data_mut().set_direction_instantly(dir);
            if let Some(actor) = entity.actor_data_mut() {
                actor.active_melee = ActiveMelee::new(target, strike, Some(seq_id), elem_idx);
                actor.action_state = ActionState::WaitingSword;
                actor.clear_path();
            }
        }

        // Push the strike animation order via `push_order_with_id` so
        // the stamped `order_id` the sprite pipeline sees matches the
        // id stored on `active_melee`.  Mirror the id onto
        // `active_melee` so `tick_melee_strikes` can pass it to
        // `sprite.perform_action`; otherwise the sprite's
        // `last_processed_order_id` would thrash between the
        // animation driver (reading `order.order_id`) and the melee
        // tick (reading `active_melee.order_id`), wedging the strike
        // at `MotionState::Start`.
        let mut order = crate::order::Order::new(anim, tx, ty, self.alloc_order_id());
        order.target_actor = Some(target.0);
        order.compute_direction = false;
        let order_id = order.order_id;
        self.sequence_manager.push_order_on(seq_id, elem_idx, order);
        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.active_melee.order_id = Some(order_id);
        }

        self.sequence_manager.element_in_progress(seq_id, elem_idx);

        // Warn potential victims so they can auto-parry, and
        // dispatch EventSwordstrike to NPC AI for the
        // consider-to-begin-parade path.
        let owner_weapon = self
            .get_entity(owner)
            .and_then(|e| get_hth_weapon_id_full(e, &assets.profile_manager));
        // WarnForStrike phase: collect victims with the
        // warn-AI-extended tolerance so the circle strike's
        // enemy-walking envelope is included.
        let victims = self.execute_multi_target_strike(assets, owner, strike, owner_weapon, true);
        if !victims.is_empty() {
            self.warn_for_strike(assets, owner, &victims, strike);
        } else {
            self.warn_for_strike(assets, owner, &[target], strike);
        }

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
    /// Transitions the entity into sword-fighting action state.
    pub(crate) fn dispatch_enter_swordfight(
        &mut self,
        assets: &LevelAssets,
        owner: EntityId,
        opponent: Option<EntityId>,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        {
            let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) else {
                self.sequence_manager.element_impossible(seq_id, elem_idx);
                return;
            };
            if entity.is_dead() || entity.human_data().map(|h| h.unconscious).unwrap_or(true) {
                self.sequence_manager.element_impossible(seq_id, elem_idx);
                return;
            }
        }

        // Table swordfight positioning: when entering a swordfight
        // whose opponent sits in a different sector, walk the
        // jump-line graph to find a free slot among any fighters
        // already engaged from our side.  If the jump line already has
        // 3+ fighters on our side, interrupt.  Otherwise launch a
        // movement element to nudge ourselves to the free slot before
        // raising the sword.
        if let Some(opp) = opponent
            && opp != owner
            && let Some(jl_idx) = self
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
                    self.sequence_manager.element_impossible(seq_id, elem_idx);
                    return;
                }
                TableFightMove::Ok | TableFightMove::Launched => {}
            }
        }

        // Pre-fetch opponent position before borrowing owner mutably,
        // so the `SetDirection` toward opponent on raising-sword
        // initialisation can be applied at order-launch time.
        let opponent_for_facing = opponent.filter(|opp| *opp != owner);
        let opp_pos = opponent_for_facing
            .and_then(|opp| self.get_entity(opp))
            .map(|e| e.element_data().position_map());

        let queue_raise = {
            let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) else {
                self.sequence_manager.element_impossible(seq_id, elem_idx);
                return;
            };
            // Queue transition animation based on current action state.
            let needs_raise = entity
                .actor_data()
                .map(|a| !a.action_state.is_sword())
                .unwrap_or(false);
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::WaitingSword;
            }
            entity.set_posture(Posture::Upright);
            // TRANSITION_RAISING_SWORD initialisation sets the
            // direction goal toward the opponent; the goal is then
            // pursued each frame by `turn()`.  We set the goal here at
            // order-launch time so the per-tick `turn()` call in
            // `engine/animation.rs` rotates the body toward the
            // opponent during the raise.
            if needs_raise && let Some(tp) = opp_pos {
                let me_pos = entity.element_data().position_map();
                let dx = tp.x - me_pos.x;
                let dy = tp.y - me_pos.y;
                let dir = crate::position_interface::vector_to_sector_0_to_15_iso(dx, dy);
                entity.element_data_mut().set_direction_goal(dir);
            }
            needs_raise
        };
        if queue_raise {
            // Queue raising-sword transition animation onto the owning element.
            let id = self.alloc_order_id();
            let mut order = crate::order::Order::new(
                crate::order::OrderType::TransitionRaisingSword,
                0.0,
                0.0,
                id,
            );
            // The raising-sword order carries the antagonist (opponent)
            // so per-tick logic and callers inspecting the order can
            // see them.
            if let Some(opp) = opponent_for_facing {
                order = order.with_antagonist(opp);
            }
            self.sequence_manager.push_order_on(seq_id, elem_idx, order);
        }
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
                .sequence_manager
                .get_element(seq_id, elem_idx)
                .and_then(|e| e.get_property(crate::sequence::Field::JumplineDestination))
                .and_then(|v| match v {
                    crate::sequence::FieldValue::LineId(id) if id.get() != 0 => Some(*id),
                    _ => None,
                });
            self.enter_swordfight_with_jump_line(assets, owner, opp, false, aggressor_jl);
        }
        self.sequence_manager.element_terminated(seq_id, elem_idx);
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
            let pos = e.element_data().position_map().to_geo();
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

        let table_count = number_of_table_swordfight_opponents(&self.entities, opp, owner_sector);
        // No existing fighters from our side → no slotting needed; the
        // caller's pre-move (`apply_table_swordfight`) already placed us.
        if table_count == 0 {
            return TableFightMove::Ok;
        }
        if table_count >= 3 {
            return TableFightMove::Abort;
        }

        let jump_line = match self.fast_grid.level.jump_lines.get(jl_idx as usize) {
            Some(jl) => jl.clone(),
            None => return TableFightMove::Abort,
        };

        let Some(new_pos) = find_position_for_table_swordfight(
            &self.entities,
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

        if !self.fast_grid.is_straight_movement_authorized(
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
            *destination = crate::element::Point2D {
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
    pub(crate) fn dispatch_quit_swordfight(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let queue_lower = if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
            && let Some(actor) = entity.actor_data_mut()
        {
            // Queue lowering-sword animation if in sword state.
            let was_sword = actor.action_state.is_sword();
            if was_sword {
                actor.action_state = ActionState::Waiting;
            }
            actor.active_melee.clear();
            was_sword
        } else {
            false
        };
        if queue_lower {
            let id = self.alloc_order_id();
            self.sequence_manager.push_order_on(
                seq_id,
                elem_idx,
                crate::order::Order::new(
                    crate::order::OrderType::TransitionLoweringSword,
                    0.0,
                    0.0,
                    id,
                ),
            );
        }
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }

    // ─── Parry ──────────────────────────────────────────────────────

    /// Dispatch a ParrySword command.
    pub(crate) fn dispatch_parry_sword(
        &mut self,
        owner: EntityId,
        low: bool,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let Some(Some(entity)) = self.entities.get(owner.0 as usize) else {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        };
        let Some(actor) = entity.actor_data() else {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        };

        if !matches!(
            actor.action_state,
            ActionState::WaitingSword
                | ActionState::MovingSword
                | ActionState::MovingFastSword
                | ActionState::ParryingSword
                | ActionState::ParryingSwordLow
        ) {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        }

        if !matches!(
            actor.action_state,
            ActionState::ParryingSword | ActionState::ParryingSwordLow
        ) {
            let transition = if low {
                crate::order::OrderType::TransitionWaitingSwordParryingSwordLow
            } else {
                crate::order::OrderType::TransitionWaitingSwordParryingSword
            };
            let id = self.alloc_order_id();
            self.sequence_manager.push_order_on(
                seq_id,
                elem_idx,
                crate::order::Order::new(transition, 0.0, 0.0, id),
            );
        }

        let hold = if low {
            crate::order::OrderType::ParryingLowSword
        } else {
            crate::order::OrderType::ParryingSword
        };
        let id = self.alloc_order_id();
        self.sequence_manager.push_order_on(
            seq_id,
            elem_idx,
            crate::order::Order::new(hold, 0.0, 0.0, id),
        );
        self.sequence_manager.element_in_progress(seq_id, elem_idx);
    }

    /// Dispatch a StopParrySword command.
    pub(crate) fn dispatch_stop_parry(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let Some(Some(entity)) = self.entities.get(owner.0 as usize) else {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        };
        let Some(actor) = entity.actor_data() else {
            self.sequence_manager.element_impossible(seq_id, elem_idx);
            return;
        };
        if !matches!(
            actor.action_state,
            ActionState::ParryingSword | ActionState::ParryingSwordLow
        ) {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
            return;
        }

        let id = self.alloc_order_id();
        self.sequence_manager.push_order_on(
            seq_id,
            elem_idx,
            crate::order::Order::new(
                crate::order::OrderType::TransitionParryingSwordWaitingSword,
                0.0,
                0.0,
                id,
            ),
        );
        self.sequence_manager.element_in_progress(seq_id, elem_idx);
    }

    // ─── Shield commands ────────────────────────────────────────────

    /// Dispatch a RaiseShield command.
    ///
    /// If already holding shield, terminates immediately. Otherwise
    /// transitions to `HoldingShield` and queues the raising animation.
    pub(crate) fn dispatch_raise_shield(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
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
                            .get(id.0 as usize)
                            .and_then(|s| s.as_ref())
                            .map(|e| e.element_data().position_map())
                    });
                    (pt, None, None, None)
                }
                crate::sequence::SequenceElementData::Generic { properties } => {
                    use crate::sequence::{Field, FieldValue};
                    let (pt2d, pt3d) = match properties.get(&Field::ShieldDangerPoint) {
                        Some(FieldValue::Point3D { x, y, z }) => (
                            Some(crate::coordinates::MapPoint { x: *x, y: *y }),
                            Some(crate::element::Point3D {
                                x: *x,
                                y: *y,
                                z: *z,
                            }),
                        ),
                        Some(FieldValue::Point2D { x, y }) => (
                            Some(crate::coordinates::MapPoint { x: *x, y: *y }),
                            Some(crate::element::Point3D {
                                x: *x,
                                y: *y,
                                z: 0.0,
                            }),
                        ),
                        _ => (None, None),
                    };
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
            && let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
            && let Some(pc) = entity.pc_data_mut()
        {
            pc.shield_danger_point = pt3d;
            pc.shield_danger_point_layer = danger_layer.unwrap_or(0);
        }
        // Only call `SetShieldProtected` when the Generic property is
        // non-null.
        if let Some(prot) = new_protected {
            self.set_shield_protected(owner, Some(prot));
        }

        // Action-state branch.  If already shielding (HOLDING_SHIELD or
        // MOVING_SHIELD), terminate the RAISE_SHIELD element — only
        // the danger-point/protected updates above are wanted — and,
        // when a protectee is set, launch a fresh SEEK so the
        // protector follows the protectee.  WALKING_UPRIGHT with
        // tolerance 50 when the danger point is zero; tolerance 0 +
        // SEEK_SHIELD when a danger point is set.
        let action_state = self
            .get_entity(owner)
            .and_then(|e| e.actor_data())
            .map(|a| a.action_state);
        match action_state {
            Some(ActionState::HoldingShield) | Some(ActionState::MovingShield) => {
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                let protected_now = self
                    .get_entity(owner)
                    .and_then(|e| e.pc_data())
                    .and_then(|pc| pc.shield_protected);
                if let Some(target) = protected_now {
                    let danger_zero = self
                        .get_entity(owner)
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
                    self.launch_element(seek);
                }
                return;
            }
            Some(s) if s.is_sword() || s.is_bow() => {
                // Defensive gate (must be Waiting / Alerted / holding
                // shield): the transition machine should already have
                // rejected this, but terminate cleanly if it slips
                // through.
                self.sequence_manager.element_terminated(seq_id, elem_idx);
                return;
            }
            None => {
                self.sequence_manager.element_impossible(seq_id, elem_idx);
                return;
            }
            _ => {} // Waiting, Bored, ParryingShield, etc. — proceed.
        }

        let mut started = false;
        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) {
            // Face toward danger point if available.  Sets the
            // direction *goal*; the per-tick `turn()` (in the
            // order-driven animation handler) interpolates toward it
            // one step per frame.
            if let Some(pt) = danger_pt {
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
            self.push_new_order(
                seq_id,
                elem_idx,
                crate::order::OrderType::RaisingShield,
                0.0,
                0.0,
            );
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
        } else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        }
    }

    /// Dispatch a RaiseShieldInstantly command.
    ///
    /// Sets `HoldingShield` immediately without a raising animation.
    pub(crate) fn dispatch_raise_shield_instantly(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize) {
            if let Some(actor) = entity.actor_data_mut() {
                actor.action_state = ActionState::HoldingShield;
                actor.clear_path();
            }
            entity.set_posture(Posture::Upright);
        }
        self.push_new_order(
            seq_id,
            elem_idx,
            crate::order::OrderType::WaitingShield,
            0.0,
            0.0,
        );
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }

    /// Dispatch a LowerShield command.
    ///
    /// Transitions out of shield state to `Waiting` with a lowering animation.
    pub(crate) fn dispatch_lower_shield(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let mut started = false;
        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
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
            self.push_new_order(
                seq_id,
                elem_idx,
                crate::order::OrderType::LoweringShield,
                0.0,
                0.0,
            );
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
        } else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        }
    }

    /// Dispatch a ParryShield command.
    ///
    /// Transitions to `ParryingShield` from a shield-holding state.
    pub(crate) fn dispatch_parry_shield(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let mut started = false;
        if let Some(Some(entity)) = self.entities.get_mut(owner.0 as usize)
            && let Some(actor) = entity.actor_data_mut()
        {
            // Requires the actor to currently be holding the shield.
            if actor.action_state == ActionState::HoldingShield
                || actor.action_state == ActionState::ParryingShield
            {
                actor.action_state = ActionState::ParryingShield;
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
            self.push_new_order(
                seq_id,
                elem_idx,
                crate::order::OrderType::ParryingShield,
                0.0,
                0.0,
            );
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
        } else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        }
    }

    // ─── Receive damage dispatch ────────────────────────────────────

    /// Dispatch a receive-damage command from the sequence system.
    ///
    /// Reads damage data from the sequence element, applies it to the
    /// victim, and handles death/KO transitions.  Handles
    /// `ReceiveSwordDamage`, `ReceiveDamage`, `ReceiveHitDamage`, etc.
    pub(crate) fn dispatch_receive_damage(
        &mut self,
        assets: &LevelAssets,
        victim_id: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        // Read damage data from the sequence element
        let elem = match self.sequence_manager.get_element(seq_id, elem_idx) {
            Some(e) => e,
            None => return,
        };
        let command = elem.command;

        let (origin, damage, concussion, sword_strike, sword_profile_idx, is_harder_hit) =
            match &elem.data {
                SequenceElementData::Damage {
                    origin,
                    damage,
                    concussion,
                    sword_strike,
                    sword_profile_idx,
                    is_harder_hit,
                } => (
                    *origin,
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
                    self.sequence_manager.element_terminated(seq_id, elem_idx);
                    return;
                }
            };

        // Apply damage based on command type.  Per-command `apply_*`
        // functions are responsible for applying the civilian-with-
        // attached-scroll immunity check: nets land, everything else
        // is a no-op on a scroll-carrying beggar.
        match command {
            Command::ReceiveSwordDamage => {
                self.apply_sword_damage(
                    assets,
                    victim_id,
                    origin,
                    sword_strike,
                    sword_profile_idx,
                    (seq_id, elem_idx),
                );
                // ExecuteFallingPushed / ExecuteRolling marks the
                // damage element NonInterruptable directly when those
                // anims start.  Here, `queue_damage_anim` does the
                // equivalent inline `set_element_priority` call when
                // the falling/rolling order is pushed onto the
                // element — no separate propagation step is needed.
            }
            Command::ReceiveDamage | Command::ReceiveMobileDamage => {
                self.apply_generic_damage(
                    assets,
                    victim_id,
                    damage,
                    concussion,
                    (seq_id, elem_idx),
                );
            }
            Command::ReceiveArrowDamage | Command::ReceiveStoneDamage => {
                // Spy and Tree postures grant arrow invulnerability.
                if command == Command::ReceiveArrowDamage {
                    let posture = self
                        .entities
                        .get(victim_id.0 as usize)
                        .and_then(|s| s.as_ref())
                        .map(|e| e.element_data().posture)
                        .unwrap_or(Posture::Upright);
                    if !posture.is_hurtable_by_arrow() {
                        tracing::debug!(
                            ?victim_id,
                            ?posture,
                            "arrow blocked: target in stealth posture"
                        );
                        self.sequence_manager.element_terminated(seq_id, elem_idx);
                        return;
                    }
                }
                self.apply_piercing_damage(
                    assets,
                    victim_id,
                    damage,
                    concussion,
                    command == Command::ReceiveArrowDamage,
                    (seq_id, elem_idx),
                );
            }
            Command::ReceiveHitDamage => {
                self.apply_hit_damage(
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
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.orders.len())
            .unwrap_or(0);
        if order_count > 0 && self.get_entity(victim_id).is_some() {
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
            return;
        }
        self.sequence_manager.element_terminated(seq_id, elem_idx);
    }
}
