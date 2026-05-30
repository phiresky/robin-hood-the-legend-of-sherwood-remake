//! Push effects, falls, rolls, multi-target strikes.
//!
//! Extracted from the original `melee.rs` mega-file.

use super::*;
use crate::combat::{self};
use crate::element::{ActionState, Entity, EntityId, EyeStatus, Posture};
use crate::profiles::WeaponThrustKind;
use crate::weapons::SwordStrike;

impl EngineInner {
    // ─── Push / stumble effects ─────────────────────────────────────

    /// Apply push-back movement and posture-aware falling animation to a
    /// victim from a push/circle/charge strike.
    ///
    /// Moves the victim away from the attacker, selects the correct
    /// falling-pushed animation based on posture/action state, and queues
    /// standup + stunned animations as needed.
    ///
    /// Returns `true` if the push handled the post-damage state transition
    /// (i.e. the caller should skip the regular hit reaction anim and
    /// `handle_post_damage`).
    /// Apply push effect with per-frame flight animation.
    ///
    /// Look up a lift sector's low-entry point via the cached
    /// `lowest_door_index` on the `GridSector`.
    ///
    /// Returns `lowest_door.point_out`.  The cache is populated at
    /// level load by `initialize_motion_from_level_data` (see the
    /// "Cache lowest / highest door per lift sector" pass), so this
    /// method is O(1) — no runtime door-list scan.
    pub(super) fn find_lift_low_entry(&self, lift_sector: u16) -> Option<(f32, f32, u16)> {
        let grid_idx = *self
            .fast_grid
            .level
            .sector_number_map
            .get(&crate::sector::SectorNumber::new(lift_sector as i16))?;
        let gs = self.fast_grid.level.sectors.get(grid_idx)?;
        let door_idx = gs.lowest_door_index?;
        let game_host = self.mission_script.as_ref()?.game_host()?;
        let door = game_host.doors.get(door_idx as usize)?;
        Some((door.point_out.0, door.point_out.1, door.layer_out))
    }

    /// Translate a push applied to an entity on a ladder or wall:
    /// play the `FallingLadderWall` animation and flight the entity
    /// to the ladder's low entry point.
    pub(crate) fn translate_ladder_wall_fall(
        &mut self,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        let (victim_pos, victim_sector) = match self.get_entity(victim_id) {
            Some(e) => (e.element_data().position_map(), e.element_data().sector()),
            None => return,
        };

        // Destination is the ladder's low entry point.  If we can't
        // locate it, leave the victim in place — the animation still
        // plays so the visual feedback is correct.
        let low_entry = victim_sector.and_then(|s| self.find_lift_low_entry(u16::from(s)));

        // The sector should be a lift.  Log a warning if not rather
        // than crashing — we fall through to the safe path.
        if low_entry.is_none() {
            tracing::warn!(
                entity = ?victim_id,
                sector = ?victim_sector,
                "translate_ladder_wall_fall: no lowest door found for lift sector"
            );
        }

        // Compute flight tick count from the FallingLadderWall
        // sprite.  Uses the sum of per-frame delays rather than raw
        // frame count.
        let frames = {
            let from_sprite = self
                .get_entity(victim_id)
                .map(|e| e.sprite())
                .map(|s| s.total_ticks_for_anim(OrderType::FallingLadderWall))
                .unwrap_or(0);
            if from_sprite > 1 { from_sprite } else { 12 }
        };

        // Free the lift occupancy so other actors can climb it.
        // Uses the `active_lift` marker that was set when the victim
        // entered the lift via WaitFreeLift.
        let active_lift = self
            .get_entity(victim_id)
            .and_then(|e| e.actor_data())
            .and_then(|a| a.active_lift);
        if let Some(lift) = active_lift {
            if let Some(grid_idx) = self
                .fast_grid
                .level
                .sector_number_map
                .get(&crate::sector::SectorNumber::new(lift.sector_number as i16))
                .copied()
            {
                let st = self.fast_grid.lift_state_mut(grid_idx as u32);
                st.occupants = st.occupants.saturating_sub(1);
                if st.occupants == 0 {
                    st.occupied_upwards = false;
                    st.occupied_downwards = false;
                    st.wait_time = 0;
                }
            }
            // Clear the marker so the actor isn't credited with an
            // occupancy slot they no longer hold.
            if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize)
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.active_lift = None;
            }
        }

        // Insert FallingLadderWall onto the damage element.  Posture
        // transition (Upright/Lying/Dead per alive/unconscious/dead)
        // is applied via `apply_falling_completion_side_effect` on
        // MotionState::Terminated.
        self.queue_damage_anim(victim_id, damage_element, OrderType::FallingLadderWall);
        // ActiveFlight is independent of animation state and is set
        // unconditionally so the victim is carried to the ladder's low
        // entry point.
        if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize)
            && let Some(actor) = entity.actor_data_mut()
        {
            {
                if let Some((gx, gy, _)) = low_entry {
                    let dx = gx - victim_pos.x;
                    let dy = gy - victim_pos.y;
                    if dx.abs() > 0.01 || dy.abs() > 0.01 {
                        actor.active_flight = Some(crate::element::ActiveFlight {
                            increment_x: dx / frames as f32,
                            increment_y: dy / frames as f32,
                            goal_x: gx,
                            goal_y: gy,
                            frames_remaining: frames,
                            // Ladder/wall fall: domino effect is
                            // not invoked.
                            antagonist: None,
                            ..Default::default()
                        });
                    }
                }
            }
        }

        tracing::debug!(
            entity = ?victim_id,
            ?low_entry,
            "Ladder/wall fall translated"
        );
    }

    /// Dispatch `Command::Fall` to an actor.
    ///
    /// Runs on the actor that the sub-sequence targets — i.e.
    /// whichever side of a broken carry is NOT the one currently
    /// handling a damage element.  Posture dictates which fall
    /// animation plays:
    ///
    /// - `OnShoulders`  → `FallingShoulders` (NonInterruptable,
    ///   carrier pointer cleared).
    /// - `CarryingOnShoulders` / `HelpingToClimb` →
    ///   `FallingBackUpright` (carrier wobble, carried pointer
    ///   cleared).
    /// - Anything else → no-op with a warning.
    ///
    /// The sequence element is marked terminated as soon as combat_anim
    /// is in place — the animation plays out via the per-frame combat
    /// animation tick and the sprite-completion callback transitions
    /// posture to `Lying` / `DeadBack` at the end. This matches the
    /// semantics of every other combat-animation dispatch in the port
    /// (priority lives on the actor, not the element).
    pub(crate) fn dispatch_fall(
        &mut self,
        owner: EntityId,
        seq_id: crate::sequence::SequenceId,
        elem_idx: usize,
    ) {
        let posture = self
            .get_entity(owner)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();

        // Pick the fall animation by current posture and insert it
        // as an order on the element.  The order is consumed by
        // `do_next_order`; per-anim posture transitions are applied
        // by `apply_falling_completion_side_effect`.
        let fall_anim = match posture {
            Posture::OnShoulders => Some(OrderType::FallingShoulders),
            Posture::CarryingOnShoulders | Posture::HelpingToClimb => {
                Some(OrderType::FallingBackUpright)
            }
            _ => {
                // The reference asserts here.  We log a warning and
                // still queue a stumble.
                tracing::warn!(
                    entity = ?owner,
                    ?posture,
                    "dispatch_fall: called on non-shoulder posture"
                );
                Some(OrderType::FallingBackUpright)
            }
        };

        // Clearing the carrier link sets the direction goal to the
        // carrier's direction first.  Capture the carrier's direction
        // before dropping the back-reference so we can rewrite the
        // falling actor's goal afterward.
        let carrier_dir = if matches!(posture, Posture::OnShoulders) {
            self.get_entity(owner)
                .and_then(|e| e.human_data())
                .and_then(|h| h.carrier)
                .and_then(|cid| self.get_entity(cid))
                .map(|c| c.element_data().direction())
        } else {
            None
        };

        // Side-state cleanup: clear carrier/carried pointers and
        // unfreeze the carried actor before the order insertion.
        if let Some(Some(entity)) = self.entities.get_mut(owner.index() as usize) {
            match posture {
                Posture::OnShoulders => {
                    if let Some(human) = entity.human_data_mut() {
                        human.carrier = None;
                    }
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.execution_frozen = false;
                    }
                    // Mirror the carrier-clear direction-goal rewrite.
                    if let Some(d) = carrier_dir {
                        entity.element_data_mut().set_direction_goal(d);
                    }
                }
                Posture::CarryingOnShoulders | Posture::HelpingToClimb => {
                    if let Some(pc) = entity.pc_data_mut() {
                        pc.carried = None;
                    }
                }
                _ => {
                    if let Some(pc) = entity.pc_data_mut() {
                        pc.carried = None;
                    }
                }
            }
        }

        if let Some(anim) = fall_anim {
            // Mark NonInterruptable for OnShoulders only — carrier
            // wobble runs at normal priority.
            if matches!(posture, Posture::OnShoulders) {
                self.sequence_manager.set_element_priority(
                    seq_id,
                    elem_idx,
                    crate::sequence::SequencePriority::NonInterruptable,
                );
            }
            self.push_new_order(seq_id, elem_idx, anim, 0.0, 0.0);
            self.sequence_manager.element_in_progress(seq_id, elem_idx);
        } else {
            self.sequence_manager.element_terminated(seq_id, elem_idx);
        }
    }

    /// Translate damage that lands on a PC mid-carry.
    ///
    /// Routes every damage type that strikes a `CarryingOnShoulders`,
    /// `OnShoulders`, or `HelpingToClimb` PC through the
    /// shoulder-fall path instead of the normal sword / hit / push
    /// handler.
    ///
    /// The victim side of the carry drives its own fall via
    /// `combat_anim` (the current damage element is already being
    /// dispatched, so we don't launch a second element for it).  The
    /// *partner* side gets a fresh `Command::Fall` sequence element
    /// launched through `SequenceManager::launch_element`.  That
    /// element is dispatched by `dispatch_fall` next tick and
    /// terminates once its combat_anim is in place.
    ///
    /// Posture-to-side mapping:
    /// - **OnShoulders** (victim is carried): victim plays
    ///   `FallingShoulders` (NonInterruptable), Fall command is
    ///   launched at the carrier so the carrier stumbles.
    /// - **CarryingOnShoulders / HelpingToClimb** (victim is the
    ///   carrier): victim plays a `FallingBackUpright` stumble;
    ///   Fall command is launched at the carried so the carried
    ///   falls.
    ///
    /// ## Known gap
    ///
    /// - The victim's fall animation runs via `combat_anim` rather
    ///   than as an order on the current damage element, because the
    ///   damage elements don't support multi-order chains.  The
    ///   sprite-completion callback in `animation.rs` flips posture
    ///   to `Lying` / `DeadBack` when the sprite terminates.
    pub(super) fn translate_shoulder_damage(
        &mut self,
        assets: &LevelAssets,
        victim_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        // Say the hurt expression.  In the reference, the PC
        // shoulder-damage override short-circuits before the
        // base-class hit-damage path that calls SayOuch, so PC
        // shoulder hits skip HERO_HURT; NPCs reach SayOuch via their
        // own hit-damage path.  Our unified damage path routes NPCs
        // on a PC's shoulders through this helper, so keeping the
        // call here covers the NPC-victim case at the cost of a
        // minor PC-side over-trigger when an already-ouched PC on
        // shoulders gets hit again.  The callers
        // (`apply_generic_damage`, `apply_piercing_damage`, etc.)
        // skip their own `say_ouch` when routing here to avoid the
        // double-trigger compounding further.
        self.say_ouch(assets, victim_id, None);

        // Read posture + carrier/carried relationships.
        let (posture, carrier_id, carried_id) = {
            let v = match self.get_entity(victim_id) {
                Some(e) => e,
                None => return,
            };
            let posture = v.element_data().posture;
            let carrier = v.human_data().and_then(|h| h.carrier);
            let carried = v.pc_data().and_then(|p| p.carried);
            (posture, carrier, carried)
        };

        // Partner receives a Fall sub-sequence — determined by which
        // side of the carry the victim is on. Own-side animation is
        // set directly further down because the damage element is
        // already dispatching this function.
        let partner_for_fall: Option<EntityId> = match posture {
            Posture::OnShoulders => carrier_id,
            Posture::CarryingOnShoulders | Posture::HelpingToClimb => carried_id,
            _ => None,
        };

        // Pick the victim's fall animation by posture: OnShoulders
        // → FallingShoulders (NonInterruptable); the carrier side
        // (CarryingOnShoulders / HelpingToClimb) → FallingBackUpright
        // stumble at normal priority.
        let (anim, priority) = match posture {
            Posture::OnShoulders => (
                Some(OrderType::FallingShoulders),
                crate::sequence::SequencePriority::NonInterruptable,
            ),
            Posture::CarryingOnShoulders | Posture::HelpingToClimb => (
                Some(OrderType::FallingBackUpright),
                crate::sequence::SequencePriority::NotYetSet,
            ),
            _ => {
                tracing::warn!(
                    entity = ?victim_id,
                    ?posture,
                    "translate_shoulder_damage called on non-shoulder posture"
                );
                (None, crate::sequence::SequencePriority::NotYetSet)
            }
        };

        // Clearing the carrier link rewrites the direction goal to
        // the carrier's direction first.  Capture the carrier's
        // direction now.
        let carrier_dir = if matches!(posture, Posture::OnShoulders) {
            carrier_id
                .and_then(|cid| self.get_entity(cid))
                .map(|c| c.element_data().direction())
        } else {
            None
        };

        // Side-state cleanup before queuing the animation.
        if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize) {
            match posture {
                Posture::OnShoulders => {
                    if let Some(human) = entity.human_data_mut() {
                        human.carrier = None;
                    }
                    if let Some(actor) = entity.actor_data_mut() {
                        actor.execution_frozen = false;
                    }
                    if let Some(d) = carrier_dir {
                        entity.element_data_mut().set_direction_goal(d);
                    }
                }
                Posture::CarryingOnShoulders | Posture::HelpingToClimb => {
                    if let Some(pc) = entity.pc_data_mut() {
                        pc.carried = None;
                    }
                    // The reference does NOT mutate the carrier's
                    // posture at translate time — the
                    // FallingBackUpright execute handler is the one
                    // that flips it.  We flip eagerly because
                    // `queue_damage_anim` + the sprite-completion
                    // callback in `animation.rs` expect the posture
                    // to reflect the post-fall state.  Nothing reads
                    // the posture between this translate call and
                    // the subsequent execute pass, so the
                    // one-tick-earlier flip is behaviourally
                    // equivalent.
                    entity.set_posture(Posture::Upright);
                }
                _ => {}
            }
        }

        if let Some(fall_anim) = anim {
            // Set priority to NonInterruptable BEFORE inserting the
            // FallingShoulders order — at translate time, not execute
            // time.  The FallingShoulders execute path *asserts* the
            // priority is already NonInterruptable.  We set it
            // eagerly here for the OnShoulders branch; the
            // carrier-wobble branch inherits the parent damage
            // element's priority.
            if matches!(
                priority,
                crate::sequence::SequencePriority::NonInterruptable
            ) {
                let (dseq, didx) = damage_element;
                self.sequence_manager
                    .set_element_priority(dseq, didx, priority);
            }
            self.queue_damage_anim(victim_id, damage_element, fall_anim);
        }

        // Launch the Fall sub-sequence for the partner.
        if let Some(partner_id) = partner_for_fall {
            let elem = crate::sequence::SequenceElement::new(
                1,
                crate::element::Command::Fall,
                Some(partner_id),
            );
            let partner_seq_id = self.launch_element(elem);
            // For the CarryingOnShoulders / HelpingToClimb branches
            // the partner (the *carried* body) also receives a roll
            // translation on its new Fall sequence element, so a body
            // dropped onto a slope rolls away from the collapse.  The
            // OnShoulders branch launches Fall on the *carrier* and
            // does not roll, so we gate on posture.
            if matches!(
                posture,
                Posture::CarryingOnShoulders | Posture::HelpingToClimb
            ) {
                self.try_queue_roll(assets, partner_id, (partner_seq_id, 0));
            }
        }

        // After appending the fall order, attempt a roll on the
        // victim's own damage element so a shoulder-damaged actor
        // landing on a slope rolls instead of stopping at the final
        // fall frame.  `try_queue_roll` is a no-op on flat terrain.
        self.try_queue_roll(assets, victim_id, damage_element);

        tracing::debug!(
            entity = ?victim_id,
            ?posture,
            partner = ?partner_for_fall,
            "Shoulder damage translated"
        );
    }

    /// Computes a strike-type-specific flight vector, validates the
    /// destination against walkable terrain, and sets up an
    /// `ActiveFlight` that advances the victim position each frame
    /// over the animation duration.
    pub(super) fn apply_push_effect(
        &mut self,
        assets: &LevelAssets,
        victim_id: EntityId,
        attacker_id: EntityId,
        push: &PushStrikeInfo,
        damage_result: combat::SwordDamageResult,
        damage_element: (crate::sequence::SequenceId, usize),
    ) -> bool {
        // NonInterruptable guard: if the victim is already playing a
        // non-interruptable sequence (an earlier falling-pushed /
        // rolling / ladder-wall fall), refuse to start a fresh push
        // on top of it.  We check the priority of the victim's
        // current InProgress sequence element.
        let current_priority = self
            .current_sequence_element_for_actor(victim_id)
            .and_then(|(s, i)| self.sequence_manager.get_element(s, i))
            .map(|e| e.priority)
            .unwrap_or_default();
        if current_priority.is_non_interruptable()
            // Skip the guard for the damage element we're currently
            // dispatching — its NonInterruptable priority is on the
            // current element being processed, not a *prior* one.
            && Some(damage_element)
                != self.current_sequence_element_for_actor(victim_id)
        {
            tracing::debug!(
                victim = ?victim_id,
                attacker = ?attacker_id,
                "apply_push_effect: victim sequence is non-interruptable, skipping push visual"
            );
            // Still counted as "handled" so the caller's damage-path
            // branching stays consistent.
            return true;
        }

        // SayOuch.  Push visuals follow an already-resolved damage
        // apply; pass `None` so HERO_HURT still fires as before.
        self.say_ouch(assets, victim_id, None);

        // Shoulder-posture victims route through
        // `translate_shoulder_damage` before falling through to the
        // base-class push-damage path.
        let victim_posture = self
            .get_entity(victim_id)
            .map(|e| e.element_data().posture)
            .unwrap_or_default();
        if matches!(
            victim_posture,
            Posture::OnShoulders | Posture::CarryingOnShoulders | Posture::HelpingToClimb
        ) {
            self.translate_shoulder_damage(assets, victim_id, damage_element);
            return true;
        }

        // CarryingCorpse arm — drop the corpse instantly and fall
        // through to the base-class push-damage path which runs the
        // push flight machinery below.  Re-read the posture
        // afterwards so the downstream ladder/wall + flight
        // selection sees the carrier's new Upright posture.
        let victim_posture = if victim_posture == Posture::CarryingCorpse {
            self.force_drop_carried_corpse_instant(victim_id);
            self.get_entity(victim_id)
                .map(|e| e.element_data().posture)
                .unwrap_or_default()
        } else {
            victim_posture
        };

        // Entities on a ladder/wall get the ladder-fall variant
        // instead of the normal push flight.
        if matches!(victim_posture, Posture::OnLadder | Posture::OnWall) {
            self.translate_ladder_wall_fall(victim_id, damage_element);
            return true;
        }

        // Gather positions and attacker direction.
        let (
            attacker_pos,
            attacker_dir,
            victim_pos,
            victim_z,
            victim_layer,
            victim_sector,
            victim_is_rider,
            victim_move_box,
        ) = {
            let apos = self
                .get_entity(attacker_id)
                .map(|e| e.element_data().position_map())
                .unwrap_or_default();
            let adir = self
                .get_entity(attacker_id)
                .map(|e| e.element_data().direction())
                .unwrap_or(0);
            let vpos = self
                .get_entity(victim_id)
                .map(|e| e.element_data().position_map())
                .unwrap_or_default();
            let vz = self
                .get_entity(victim_id)
                .map(|e| e.position_iface().get_elevation())
                .unwrap_or(0.0);
            let vlayer = self
                .get_entity(victim_id)
                .map(|e| e.element_data().layer())
                .unwrap_or(0);
            let vsector = self
                .get_entity(victim_id)
                .and_then(|e| e.element_data().sector());
            let vis_rider = self
                .get_entity(victim_id)
                .map(|e| matches!(e, Entity::Soldier(s) if s.soldier.rider))
                .unwrap_or(false);
            let vmbox = self
                .get_entity(victim_id)
                .map(|e| e.position_iface())
                .map(|p| *p.get_move_box())
                .unwrap_or_else(|| {
                    crate::coordinates::MoveBox::from_corners(
                        crate::coordinates::MapVec::new(-5.0, -5.0),
                        crate::coordinates::MapVec::new(5.0, 5.0),
                    )
                });
            (apos, adir, vpos, vz, vlayer, vsector, vis_rider, vmbox)
        };

        // Compute flight vector based on strike type.
        let attacker_dir_vec = sector_to_vector_iso(attacker_dir as u16, ASPECT_RATIO);
        let (mut flight_x, mut flight_y) = if push.strike == SwordStrike::Charge {
            // Charge uses attacker direction × charge repulsion.
            (
                attacker_dir_vec.0 * push.repulsion as f32,
                attacker_dir_vec.1 * push.repulsion as f32,
            )
        } else {
            match push.kind {
                WeaponThrustKind::PushAside => {
                    // Attacker direction × (max_distance - proximity).
                    let dx = victim_pos.x - attacker_pos.x;
                    let dy = victim_pos.y - attacker_pos.y;
                    let proximity = (dx * attacker_dir_vec.0 + dy * attacker_dir_vec.1).abs();
                    let push_dist = (push.max_distance - proximity).max(0.0);
                    (
                        attacker_dir_vec.0 * push_dist,
                        attacker_dir_vec.1 * push_dist,
                    )
                }
                WeaponThrustKind::TrueCircle | WeaponThrustKind::FalseCircle => {
                    // Radial (victim - attacker), normalised ×
                    // (max_dist - proximity).
                    let dx = victim_pos.x - attacker_pos.x;
                    let dy = victim_pos.y - attacker_pos.y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist < 0.01 {
                        (0.0, 0.0)
                    } else {
                        let push_dist = (push.max_distance - dist).max(0.0);
                        (dx / dist * push_dist, dy / dist * push_dist)
                    }
                }
                _ => {
                    // Fallback: radial push by repulsion
                    let dx = victim_pos.x - attacker_pos.x;
                    let dy = victim_pos.y - attacker_pos.y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist < 0.01 {
                        (0.0, 0.0)
                    } else {
                        (
                            dx / dist * push.repulsion as f32,
                            dy / dist * push.repulsion as f32,
                        )
                    }
                }
            }
        };

        // Set victim facing opposite to flight direction.
        if flight_x.abs() > 0.01 || flight_y.abs() > 0.01 {
            let flight_sector =
                crate::position_interface::vector_to_sector_0_to_15(flight_x, flight_y);
            let facing = (flight_sector + 8) % 16;
            if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize) {
                entity.element_data_mut().set_direction_instantly(facing);
            }
        }

        // "Horses cannot fly!"
        if victim_is_rider {
            flight_x = 0.0;
            flight_y = 0.0;
        }

        // Validate destination with `is_straight_movement_authorized`
        // using a nested-gate fallback: try 100%; on fail try 50%,
        // then if 50% reaches additionally try 75% (pick whichever);
        // if 50% blocked drop to 25%; final fallback is the minimal
        // goal (no displacement, bOK = false).  Then verify the
        // chosen point is inside the minimal-goal sector's polygon
        // and revert to minimal goal on failure.
        let (goal_x, goal_y) = {
            let pt_start = victim_pos;
            let try_pt = |frac: f32| {
                crate::coordinates::MapPoint::new(
                    victim_pos.x + flight_x * frac,
                    victim_pos.y + flight_y * frac,
                )
            };
            let authorized = |pt_try: crate::coordinates::MapPoint| {
                self.fast_grid.is_straight_movement_authorized(
                    pt_start,
                    pt_try,
                    victim_layer,
                    &victim_move_box,
                )
            };

            let mut chosen: Option<crate::coordinates::MapPoint> = None;
            // 100%
            let pt_full = try_pt(1.0);
            if authorized(pt_full) {
                chosen = Some(pt_full);
            } else {
                // 50% gate
                let pt_half = try_pt(0.5);
                if authorized(pt_half) {
                    // 50% reaches: try 75% on top, else stay at 50%.
                    let pt_three_quarter = try_pt(0.75);
                    if authorized(pt_three_quarter) {
                        chosen = Some(pt_three_quarter);
                    } else {
                        chosen = Some(pt_half);
                    }
                } else {
                    // 50% blocked: drop to 25% (no upward retry to 75%).
                    let pt_quarter = try_pt(0.25);
                    if authorized(pt_quarter) {
                        chosen = Some(pt_quarter);
                    }
                    // else fall through to minimal goal (bOK = false).
                }
            }

            // If the chosen goal isn't inside the minimal-goal
            // sector's polygon, revert to the minimal goal.
            if let Some(pt) = chosen {
                let inside_polygon = victim_sector
                    .and_then(|s| {
                        self.fast_grid
                            .level
                            .sector_number_map
                            .get(&crate::sector::SectorNumber::new(u16::from(s) as i16))
                            .copied()
                    })
                    .and_then(|idx| self.fast_grid.level.sectors.get(idx))
                    .map(|gs| gs.contains_point(pt))
                    // Without a known sector, trust the
                    // `is_straight_movement_authorized` result.
                    .unwrap_or(true);
                if inside_polygon {
                    (pt.x, pt.y)
                } else {
                    (victim_pos.x, victim_pos.y)
                }
            } else {
                (victim_pos.x, victim_pos.y)
            }
        };

        // Compute flight tick count from the falling-pushed sprite.
        // `total_ticks_for_anim` returns the sum of per-frame delays
        // so the per-frame increment is paced to match how long the
        // sprite actually plays.
        let flight_frames = {
            let victim = self.get_entity(victim_id);
            let posture = victim.map(|e| e.element_data().posture).unwrap_or_default();
            let action = victim
                .and_then(|e| e.actor_data())
                .map(|a| a.action_state)
                .unwrap_or_default();
            let anims = select_push_damage_animations(posture, action);
            let falling_anim = anims.map(|a| a.falling);
            let from_sprite = falling_anim
                .and_then(|anim| {
                    victim
                        .map(|e| e.sprite())
                        .map(|s| s.total_ticks_for_anim(anim))
                })
                .unwrap_or(0);
            if from_sprite > 1 { from_sprite } else { 8u16 }
        };

        // Resolve the goal's projection-area obstacle (if any) and
        // derive `goal_z`.  When no projection area covers the
        // chosen flight goal the destination sits on flat ground
        // (z = 0).  We use the victim's current sector here — push
        // flights are intra-sector by default (the goal sector is
        // copied from the minimal-goal sector at validation time).
        let (goal_obstacle, goal_z) = match victim_sector {
            Some(sh) => match self.get_projection_area_index(
                assets,
                sh.get(),
                victim_layer,
                crate::coordinates::MapPoint::new(goal_x, goal_y),
            ) {
                Some(obs_idx) => {
                    let z = self
                        .sight_obstacles(assets)
                        .get(obs_idx as usize)
                        .map(|obs| obs.compute_top_z(goal_x, goal_y))
                        .unwrap_or(0.0);
                    (crate::position_interface::ObstacleHandle::new(obs_idx), z)
                }
                None => (None, 0.0),
            },
            None => (None, 0.0),
        };

        // Set up ActiveFlight on the victim.
        let total_dx = goal_x - victim_pos.x;
        let total_dy = goal_y - victim_pos.y;
        let total_dz = goal_z - victim_z;
        if flight_frames > 0
            && (total_dx.abs() > 0.01 || total_dy.abs() > 0.01 || total_dz.abs() > 0.01)
        {
            let inc_x = total_dx / flight_frames as f32;
            let inc_y = total_dy / flight_frames as f32;
            let inc_z = total_dz / flight_frames as f32;
            if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize)
                && let Some(actor) = entity.actor_data_mut()
            {
                actor.active_flight = Some(crate::element::ActiveFlight {
                    increment_x: inc_x,
                    increment_y: inc_y,
                    goal_x,
                    goal_y,
                    frames_remaining: flight_frames,
                    // The push-flight tick unconditionally invokes
                    // `apply_domino_effect` with the attacker each
                    // frame.
                    antagonist: Some(attacker_id),
                    increment_z: inc_z,
                    goal_z,
                    goal_layer: victim_layer,
                    goal_sector: victim_sector,
                    obstacle: goal_obstacle,
                });
            }
        }

        // Read victim state for animation selection
        let (posture, action_state, is_dead, is_unconscious, concussion) = {
            let victim = match self.get_entity(victim_id) {
                Some(e) => e,
                None => return false,
            };
            let posture = victim.element_data().posture;
            let action = victim
                .actor_data()
                .map(|a| a.action_state)
                .unwrap_or_default();
            let dead = victim.is_dead();
            let unconscious = victim.human_data().map(|h| h.unconscious).unwrap_or(false);
            let conc = victim
                .human_data()
                .map(|h| h.concussion_of_the_brain)
                .unwrap_or(0);
            (posture, action, dead, unconscious, conc)
        };

        // Rider-dead special case: when the victim is both a rider
        // and already dead, override the falling animation to
        // `DyingUpright` and bypass the posture switch entirely.
        let push_anims = if victim_is_rider && is_dead {
            Some(PushDamageAnimations {
                falling: crate::order::OrderType::DyingUpright,
                standing_up: None,
                stunned: None,
            })
        } else {
            // Select posture-aware push animation
            select_push_damage_animations(posture, action_state)
        };

        if let Some(anims) = push_anims {
            // The falling sequence is marked non-interruptable so
            // incoming damage can't replace it mid-flight; the
            // falling, optional standup, and optional stunned
            // animations are inserted as chained orders on the
            // damage element so `do_next_order` plays them in
            // sequence.
            self.queue_damage_anim(victim_id, damage_element, anims.falling);
            if !is_dead
                && !is_unconscious
                && damage_result.contains(combat::SwordDamageResult::STUNNING_DAMAGE)
            {
                let (dseq, didx) = damage_element;
                if let Some(standup) = anims.standing_up {
                    self.push_new_order(dseq, didx, standup, 0.0, 0.0);
                }
                if let Some(stunned) = anims.stunned
                    && concussion > STUNNING_THRESHOLD
                {
                    self.push_new_order(dseq, didx, stunned, 0.0, 0.0);
                }
            }

            // Handle death/KO side effects.  Push-specific work
            // lives in this helper; the unconscious transition is
            // centralised in `set_concussion`.
            if is_dead {
                // Simplified death handling for push — sets posture, quits swordfight
                let is_pc =
                    if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize) {
                        entity.set_posture(Posture::Dead);
                        if let Some(actor) = entity.actor_data_mut() {
                            if actor.action_state.is_sword()
                                || actor.action_state == ActionState::Menacing
                            {
                                actor.action_state = ActionState::Waiting;
                            }
                            actor.active_melee.clear();
                            actor.clear_path();
                        }
                        if let Some(npc) = entity.npc_data_mut() {
                            crate::ai_vision::set_view_status(npc, EyeStatus::DieOrGetUnconscious);
                            npc.alerted = false;
                        }
                        entity.kind().is_pc()
                    } else {
                        false
                    };
                self.quit_swordfight(assets, victim_id);
                if is_pc {
                    // Run the PC kill cascade (gang removal, trumpet,
                    // new-PC stat decrement, macro burn, dead_pc
                    // gate) for the push-fatal path so it matches the
                    // damage-element death path.
                    self.apply_pc_kill_cascade(assets, victim_id);
                }
            } else if is_unconscious {
                let attacker_is_pc = self
                    .get_entity(attacker_id)
                    .map(|e| e.kind().is_pc())
                    .unwrap_or(false);
                self.apply_knockout_side_effects(assets, victim_id, attacker_is_pc, false);
            } else if concussion > STUNNING_THRESHOLD && !posture.is_lying() {
                // Stumble to lying from concussion
                if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize) {
                    entity.set_posture(Posture::Lying);
                }
            }

            self.try_queue_roll(assets, victim_id, damage_element);

            tracing::debug!(
                victim = ?victim_id,
                attacker = ?attacker_id,
                repulsion = push.repulsion,
                ?posture,
                falling_anim = ?anims.falling,
                "Push effect applied (posture-aware)"
            );
            true // push handled everything
        } else {
            // No falling animation (already lying/dead/carried).
            if is_dead {
                let is_pc =
                    if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize) {
                        if let Some(actor) = entity.actor_data_mut() {
                            actor.active_melee.clear();
                            actor.clear_path();
                        }
                        if let Some(npc) = entity.npc_data_mut() {
                            crate::ai_vision::set_view_status(npc, EyeStatus::DieOrGetUnconscious);
                            npc.alerted = false;
                        }
                        entity.kind().is_pc()
                    } else {
                        false
                    };
                self.quit_swordfight(assets, victim_id);
                if is_pc {
                    self.apply_pc_kill_cascade(assets, victim_id);
                }
                if let Some(Some(entity)) = self.entities.get_mut(victim_id.index() as usize) {
                    entity.set_posture(Posture::Dead);
                }
            }
            if is_unconscious {
                let attacker_is_pc = self
                    .get_entity(attacker_id)
                    .map(|e| e.kind().is_pc())
                    .unwrap_or(false);
                self.apply_knockout_side_effects(assets, victim_id, attacker_is_pc, true);
            }
            tracing::debug!(
                victim = ?victim_id,
                attacker = ?attacker_id,
                repulsion = push.repulsion,
                "Push effect applied (no animation — already down)"
            );
            true // still handled by push path
        }
    }

    // ─── Rolling on slopes ──────────────────────────────────────────

    /// Compute the top-plane normal for a sight obstacle from its three
    /// defining points.  Returns `[nx, ny, nz]`.
    ///
    pub(super) fn obstacle_top_normal(obstacle: &crate::sight_obstacle::SightObstacle) -> [f32; 3] {
        let [p0, p1, p2] = obstacle.top_plane_points;
        // Two edge vectors
        let u = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let v = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        // Cross product
        let nx = u[1] * v[2] - u[2] * v[1];
        let ny = u[2] * v[0] - u[0] * v[2];
        let nz = u[0] * v[1] - u[1] * v[0];
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        if len < 1e-6 {
            return [0.0, 0.0, 1.0]; // flat
        }
        let inv = 1.0 / len;
        // Ensure normal points upward (positive Z)
        if nz * inv >= 0.0 {
            [nx * inv, ny * inv, nz * inv]
        } else {
            [-nx * inv, -ny * inv, -nz * inv]
        }
    }

    /// Check if the entity is on a slope steep enough to roll.
    ///
    /// Returns the obstacle's top-plane normal if rolling is needed
    /// (normal.z <= 0.76, i.e. ~cos(40°)).
    pub(super) fn get_roll_normal(
        &self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) -> Option<[f32; 3]> {
        let obstacle_idx = self
            .get_entity(entity_id)?
            .element_data()
            .obstacle_index()?;

        let obstacle = self
            .sight_obstacles(assets)
            .get(usize::from(obstacle_idx))?;

        let normal = Self::obstacle_top_normal(obstacle);
        // Roll is needed when normal.z <= 0.76 (~cos(40°)).
        if normal[2] <= 0.76 {
            Some(normal)
        } else {
            None
        }
    }

    /// Compute a roll destination point from the obstacle normal.
    ///
    /// The roll direction is the downhill direction derived from the surface
    /// normal projected onto the ground plane, scaled by 100 map units.
    /// When `check_increment` is true, the function also refuses to return a
    /// destination whose roll direction opposes the entity's current
    /// movement increment (used by `update_roll` to stop rolling against the
    /// slope when the entity has been redirected).
    ///
    pub(super) fn find_roll_point(
        &self,
        entity_id: EntityId,
        normal: [f32; 3],
        check_increment: bool,
    ) -> Option<crate::coordinates::MapPoint> {
        // Use the lying-posture move-box, not the actor's *current*
        // move-box: at call time (post-fall or mid-roll) the posture
        // is typically not yet Lying, so the live box has the wrong
        // shape for picking a roll landing.
        const BOX_LYING_X: f32 = 10.0;
        const BOX_LYING_Y: f32 = 5.0;
        let entity = self.get_entity(entity_id)?;
        // Only humans can roll; gate on actor presence (drops the
        // pre-existing actor check while keeping the same coverage).
        entity.actor_data()?;
        let pos = entity.element_data().position_map();
        let layer = entity.element_data().layer();

        // Compute roll direction from normal.
        let mut rx = normal[0];
        let mut ry = normal[1];
        let mut rz = normal[2];

        if rz > 0.0 {
            rz -= 1.0 / rz;
        } else {
            rz -= 100.0;
        }

        // Apply aspect ratio to Y.
        ry *= combat::ASPECT_RATIO;

        // Scale by 100.
        rx *= 100.0;
        ry *= 100.0;
        rz *= 100.0;

        let push_map = crate::coordinates::MapVec::from_world_xyz(rx, ry, rz);

        // If the entity is already moving against the roll direction
        // (dot product negative), refuse to redirect it.
        if check_increment && let Some(pi) = Some(entity.position_iface()) {
            let inc = pi.get_increment_map();
            if inc.x * push_map.x + inc.y * push_map.y < 0.0 {
                return None;
            }
        }

        let dest_x = pos.x + push_map.x;
        let dest_y = pos.y + push_map.y;

        // Build a lying-posture box at the roll destination and call
        // `find_authorized_position_straight`, which **mutates the
        // box** by iteratively pushing it off intersecting motion
        // lines (two 50-iter passes).  The roll endpoint is the
        // *adjusted* box centre, not `pos + direction`.
        let mut dest_box = crate::coordinates::MapBBox::from_corners(
            crate::coordinates::MapPoint::new(dest_x - BOX_LYING_X, dest_y - BOX_LYING_Y),
            crate::coordinates::MapPoint::new(dest_x + BOX_LYING_X, dest_y + BOX_LYING_Y),
        );
        let pt_start = pos;
        if !self
            .fast_grid
            .find_authorized_position_straight(&mut dest_box, pt_start, layer)
        {
            return None;
        }
        Some(dest_box.center())
    }

    /// Queue a rolling animation after a fall if the entity is on a steep slope.
    ///
    pub(super) fn try_queue_roll(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
        damage_element: (crate::sequence::SequenceId, usize),
    ) {
        let normal = match self.get_roll_normal(assets, entity_id) {
            Some(n) => n,
            None => return,
        };
        let dest = match self.find_roll_point(entity_id, normal, false) {
            Some(d) => d,
            None => return,
        };

        // Append a Rolling order onto the active sequence element
        // with `point_destination_2d = dest`.  Also set `pending_roll`
        // on the actor — `apply_rolling_start_side_effect` reads it
        // on MotionState::Start to install `active_flight` toward the
        // destination (Order doesn't carry the dest separately from
        // `active_flight`'s per-frame increments).
        if let Some(Some(entity)) = self.entities.get_mut(entity_id.index() as usize)
            && let Some(actor) = entity.actor_data_mut()
        {
            actor.pending_roll = Some(dest);
            tracing::debug!(
                entity = ?entity_id,
                ?dest,
                "Rolling queued after fall on slope"
            );
        }
        let (dseq, didx) = damage_element;
        let mut roll_order =
            crate::order::Order::new(OrderType::Rolling, dest.x, dest.y, self.alloc_order_id());
        roll_order.compute_direction = false;
        self.sequence_manager.push_order_on(dseq, didx, roll_order);
    }

    // ─── Multi-target strike execution ──────────────────────────────

    /// Execute a sword strike that may hit multiple targets.
    ///
    /// Dispatches to the appropriate hit-detection method based on strike kind:
    /// - Straight: single-target distance check
    /// - Lateral: angular arc sweep
    /// - Push: rectangular area
    /// - Circle/half-circle: wide angular sweep
    ///
    /// Returns the list of victims actually hit.
    pub(super) fn execute_multi_target_strike(
        &mut self,
        assets: &LevelAssets,
        attacker_id: EntityId,
        strike: SwordStrike,
        profile_idx: Option<u32>,
        warn_ai: bool,
    ) -> Vec<EntityId> {
        let profile = profile_idx
            .and_then(|idx| assets.profile_manager.get_hth_weapon(idx))
            .cloned();
        let profile = match profile {
            Some(p) => p,
            None => return Vec::new(),
        };

        let attacker_pos = self
            .get_entity(attacker_id)
            .map(|e| {
                let p = e.element_data().position_map();
                (p.x, p.y)
            })
            .unwrap_or((0.0, 0.0));
        let attacker_dir = self
            .get_entity(attacker_id)
            .map(|e| e.element_data().direction())
            .unwrap_or(0);

        let thrust = &profile.thrusts[strike as usize];
        let min_dist = thrust.minimal_distance as f32;
        let max_dist = thrust.maximal_distance as f32;
        let kind = thrust.kind;

        let obstacles = crate::sight_obstacle::ObstacleList {
            static_obstacles: assets.static_sight_obstacles.as_slice(),
            dynamic_obstacles: &self.dynamic_sight_obstacles,
            static_active: &self.static_sight_obstacle_active,
        };

        match kind {
            WeaponThrustKind::Straight | WeaponThrustKind::Assault => {
                // Single-target: check principal opponent / original target only
                // For sequence-driven strikes, the target is already known
                Vec::new() // handled by the existing single-target path
            }
            WeaponThrustKind::Lateral => {
                let dir_angle = sector_to_angle(attacker_dir);
                let strike_dir = thrust.direction;
                let (begin_sector, end_sector) = match strike_dir {
                    crate::profiles::WeaponThrustDirection::RightToLeft => {
                        let initial = dir_angle
                            + profile.thrusts[strike as usize].initial_angle as f32
                                * std::f32::consts::PI
                                / 180.0;
                        let final_a = dir_angle
                            - profile.thrusts[strike as usize].final_angle as f32
                                * std::f32::consts::PI
                                / 180.0;
                        (angle_to_sector(final_a), angle_to_sector(initial))
                    }
                    _ => {
                        let initial = dir_angle
                            - profile.thrusts[strike as usize].initial_angle as f32
                                * std::f32::consts::PI
                                / 180.0;
                        let final_a = dir_angle
                            + profile.thrusts[strike as usize].final_angle as f32
                                * std::f32::consts::PI
                                / 180.0;
                        (angle_to_sector(initial), angle_to_sector(final_a))
                    }
                };
                collect_arc_victims(
                    &self.entities,
                    attacker_id,
                    attacker_pos,
                    min_dist,
                    max_dist,
                    begin_sector,
                    end_sector,
                    &assets.profile_manager,
                    &self.fast_grid,
                    obstacles,
                )
            }
            WeaponThrustKind::PushAside => {
                let (dir_x, dir_y) = sector_to_direction(attacker_dir);
                let half_width = thrust.repulsion as f32 / 2.0;
                let attacker_elevation = self
                    .get_entity(attacker_id)
                    .map(|e| e.position_iface().get_elevation())
                    .unwrap_or(0.0);
                collect_push_victims(
                    &self.entities,
                    &PushStrikeParams {
                        attacker_id,
                        attacker_pos,
                        attacker_elevation,
                        dir_x,
                        dir_y,
                        min_distance: min_dist,
                        max_distance: max_dist,
                        half_width,
                    },
                    &assets.profile_manager,
                    &self.fast_grid,
                    obstacles,
                )
            }
            WeaponThrustKind::TrueHalfCircle | WeaponThrustKind::FalseHalfCircle => {
                // Half circle: ±90° from facing direction
                let dir_angle = sector_to_angle(attacker_dir);
                let strike_dir = thrust.direction;
                let (begin_sector, end_sector) = match strike_dir {
                    crate::profiles::WeaponThrustDirection::RightToLeft => {
                        let initial = dir_angle
                            + profile.thrusts[strike as usize].initial_angle as f32
                                * std::f32::consts::PI
                                / 180.0;
                        let final_a = initial - std::f32::consts::PI;
                        (angle_to_sector(final_a), angle_to_sector(initial))
                    }
                    _ => {
                        let initial = dir_angle
                            - profile.thrusts[strike as usize].initial_angle as f32
                                * std::f32::consts::PI
                                / 180.0;
                        let final_a = initial + std::f32::consts::PI;
                        (angle_to_sector(initial), angle_to_sector(final_a))
                    }
                };
                collect_arc_victims(
                    &self.entities,
                    attacker_id,
                    attacker_pos,
                    min_dist,
                    max_dist,
                    begin_sector,
                    end_sector,
                    &assets.profile_manager,
                    &self.fast_grid,
                    obstacles,
                )
            }
            WeaponThrustKind::TrueCircle | WeaponThrustKind::FalseCircle => {
                // Full circle: all directions.  When invoked from the
                // WarnForStrike phase, extend the max distance for
                // walking-with-sword enemies (the warn-AI branch of
                // circle-strike victim collection).
                if warn_ai {
                    collect_circle_warn_victims(
                        &self.entities,
                        attacker_id,
                        attacker_pos,
                        attacker_dir,
                        max_dist,
                        thrust.rotation_angle,
                        &assets.profile_manager,
                        &self.fast_grid,
                        obstacles,
                    )
                } else {
                    collect_arc_victims(
                        &self.entities,
                        attacker_id,
                        attacker_pos,
                        min_dist,
                        max_dist,
                        0,
                        15,
                        &assets.profile_manager,
                        &self.fast_grid,
                        obstacles,
                    )
                }
            }
        }
    }
}
