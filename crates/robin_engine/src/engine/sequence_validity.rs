//! Pre-launch re-validation for queued sequence elements.
//!
//! Centralises the per-command switch invoked from many `Execute` /
//! `Instruct` sites to reject elements that were valid when queued but
//! have since become impossible (target walked into a building, got
//! blipped, lay unconscious, etc.).  Rust previously had only three
//! ad-hoc gap-acknowledgement sites (QA replay stale-entity, bow-target
//! comment, Pay comment) — this module provides the full per-command
//! gate and wires it into the per-tick sequence-element pickup path.

use crate::element::{Command, Entity, EntityId, ObjectType, Posture};
use crate::engine::{EngineInner, LevelAssets};
use crate::sequence::{Field, FieldValue, SequenceElement, SequenceElementData};

use super::input::BowTarget;
use super::scroll_reveal::ScrollStatus;

impl EngineInner {
    /// Re-validate a queued sequence element against the current world
    /// state.  Returns `false` if the element should be marked
    /// [`crate::sequence::SequenceState::Impossible`] before dispatch.
    ///
    /// Returns `true` for any command the switch doesn't cover: the
    /// reference default arm aborts with an `assert(false)` that is
    /// only tripped in dev builds, so shipped behaviour effectively
    /// accepts unknown commands.  Accepting here keeps the pre-filter
    /// from blocking commands that wouldn't be routed through this
    /// function at all.
    ///
    /// `check_position` — most callers pass `true`; post-seek
    /// re-validation passes `false` because the seek has already
    /// closed the distance.
    pub fn check_sequence_element_validity(
        &self,
        assets: &LevelAssets,
        actor_id: EntityId,
        element: &SequenceElement,
        check_position: bool,
    ) -> bool {
        let Some(actor) = self.get_entity(actor_id) else {
            return false;
        };

        match element.command {
            // ── WakeUp ──────────────────────────────────────────
            Command::WakeUp => {
                if !check_position {
                    return true;
                }
                let Some(victim) = interaction_victim(self, element) else {
                    return false;
                };
                let action_distance = match actor
                    .sprite()
                    .action_distance(crate::order::OrderType::WakingUp)
                {
                    Ok(distance) => distance + 20.0,
                    Err(err) => {
                        tracing::warn!(
                            ?actor_id,
                            error = %err,
                            "check_sequence_element_validity: missing WakingUp action distance"
                        );
                        return false;
                    }
                };
                square_distance(actor, victim) <= action_distance * action_distance
            }

            // ── Strangle / Hit ──────────────────────────────────
            Command::StrangleCmd | Command::HitCmd => {
                let Some(victim) = interaction_victim(self, element) else {
                    return false;
                };
                if !victim.is_human() {
                    return false;
                }
                if actor.is_pc() {
                    let in_building = self
                        .entity_building_sector(victim.element_data().sector())
                        .is_some();
                    let is_civilian = victim.is_civilian();
                    let is_vip = self.is_entity_vip(assets, victim);
                    let is_rider = victim.soldier_data().map(|s| s.rider).unwrap_or(false);
                    let out_of_order = is_human_out_of_order(victim);
                    let camp_ok = victim.camp() == crate::element::Camp::Lacklandists;
                    let hit_civilian_ok = element.command == Command::HitCmd || !is_civilian;
                    if in_building
                        || victim.element_data().blipped
                        || !camp_ok
                        || is_vip
                        || is_rider
                        || out_of_order
                        || !hit_civilian_ok
                    {
                        return false;
                    }
                }
                if !check_position {
                    return true;
                }
                square_distance(actor, victim) <= 1600.0
            }

            // ── EnterSwordfight ─────────────────────────────────
            Command::EnterSwordfight => {
                let Some(opponent_id) =
                    element.get_property(Field::Opponent).and_then(|v| match v {
                        FieldValue::Element(id) => Some(*id),
                        _ => None,
                    })
                else {
                    return false;
                };
                let Some(opponent) = self.get_entity(opponent_id) else {
                    return false;
                };
                if actor.is_pc() && opponent.element_data().blipped {
                    return false;
                }
                let opp_is_rider = opponent.soldier_data().map(|s| s.rider).unwrap_or(false);
                if opp_is_rider
                    && opponent.actor_data().map(|a| a.action_state)
                        == Some(crate::element::ActionState::MovingFast)
                {
                    return false;
                }
                let actor_vip = self.is_entity_vip(assets, actor);
                let opp_vip = self.is_entity_vip(assets, opponent);
                let actor_is_robin = is_entity_robin(actor);
                let opp_is_robin = is_entity_robin(opponent);
                if actor.is_soldier() && actor_vip && !opp_is_robin {
                    return false;
                }
                if opponent.is_soldier() && opp_vip && !actor_is_robin {
                    return false;
                }
                true
            }

            // ── Trivially-valid fight transitions ───────────────
            Command::QuitSwordfight
            | Command::RaiseShield
            | Command::RaiseShieldInstantly
            | Command::ParrySword => true,

            // ── ShootBow / ShootBowOnce ─────────────────────────
            Command::ShootBow | Command::ShootBowOnce => {
                if self.is_climbing_or_inside_building(actor_id) {
                    return false;
                }
                let Some(victim_id) = interaction_victim_id(element) else {
                    return false;
                };
                let Some(victim) = self.get_entity(victim_id) else {
                    return false;
                };
                if actor.is_pc() {
                    if victim.element_data().blipped {
                        return false;
                    }
                    if victim.is_human() && is_human_out_of_order(victim) {
                        return false;
                    }
                    if victim.is_npc() {
                        let is_royalist = victim.camp() == crate::element::Camp::Royalists;
                        let is_civilian = victim.is_civilian();
                        let is_vip = self.is_entity_vip(assets, victim);
                        if is_royalist || is_civilian || is_vip {
                            return false;
                        }
                    }
                    if !self.can_pc_execute_commands(actor_id, false) {
                        return false;
                    }
                }
                if is_human_out_of_order(actor) {
                    return false;
                }
                let (status, _) = self.can_shoot_with_bow_at(assets, actor_id, victim_id);
                status == BowTarget::Valid
            }

            // ── SwordstrikeDown ─────────────────────────────────
            Command::SwordstrikeDown => {
                // CanExecuteCommands(allow_in_buildings=true,
                // check_position) — first arg is unconditionally true.
                if actor.is_pc() && !self.can_pc_execute_commands(actor_id, true) {
                    return false;
                }
                let Some(victim) = interaction_victim(self, element) else {
                    return false;
                };
                if actor.is_pc() {
                    let vip = self.is_entity_vip(assets, victim);
                    let is_dead = victim.is_dead();
                    let unconscious = victim.human_data().map(|h| h.unconscious).unwrap_or(false);
                    let lying = victim.element_data().posture == Posture::Lying;
                    let tied = victim.element_data().posture == Posture::Tied;
                    if !victim.is_active()
                        || victim.element_data().blipped
                        || is_dead
                        || !unconscious
                        || (!lying && !tied)
                        || !victim.is_soldier()
                        || vip
                    {
                        return false;
                    }
                }
                if !check_position {
                    return true;
                }
                square_distance(actor, victim) <= 2025.0
            }

            // ── Take ─────────────────────────────────────────────
            Command::Take => {
                let Some(victim_id) = interaction_victim_id(element) else {
                    return false;
                };
                let Some(object) = self.get_entity(victim_id) else {
                    return false;
                };
                if !object.is_object() {
                    return false;
                }
                let Some(obj_data) = object.object_data() else {
                    return false;
                };

                let dist_sq = square_distance(actor, object);
                if !actor.is_pc() {
                    return match obj_data.object_type {
                        ObjectType::Net => {
                            object.is_active() && (!check_position || dist_sq <= 4900.0)
                        }
                        ObjectType::Purse => {
                            !obj_data.taken && (!check_position || dist_sq <= 900.0)
                        }
                        ObjectType::Coin => {
                            object.is_active() && (!check_position || dist_sq <= 900.0)
                        }
                        _ => false,
                    };
                }

                match obj_data.object_type {
                    ObjectType::Net => {
                        if !object.is_active() {
                            return false;
                        }
                        if check_position && dist_sq > 4900.0 {
                            return false;
                        }
                        true
                    }
                    ObjectType::BonusNet if actor.is_pc() => {
                        if !object.is_active() {
                            return false;
                        }
                        if check_position && dist_sq > 4900.0 {
                            return false;
                        }
                        true
                    }
                    // Scrolls track their own status field rather than
                    // the generic `taken` flag.
                    ObjectType::Scroll => {
                        if !object.is_active()
                            || self.scroll_status(victim_id) == ScrollStatus::Taken
                        {
                            return false;
                        }
                        if check_position && dist_sq > 900.0 {
                            return false;
                        }
                        true
                    }
                    ObjectType::Purse | ObjectType::BonusPurse if actor.is_pc() => {
                        if obj_data.taken {
                            return false;
                        }
                        if check_position && dist_sq > 900.0 {
                            return false;
                        }
                        true
                    }
                    ObjectType::Purse => {
                        if obj_data.taken {
                            return false;
                        }
                        if check_position && dist_sq > 900.0 {
                            return false;
                        }
                        true
                    }
                    _ => {
                        if !object.is_active() {
                            return false;
                        }
                        if actor.is_pc()
                            && !super::commands::is_pc_takable(self, assets, object, actor_id)
                        {
                            return false;
                        }
                        if check_position && dist_sq > 900.0 {
                            return false;
                        }
                        true
                    }
                }
            }

            // ── Search ──────────────────────────────────────────
            Command::SearchCmd => {
                // PC override: CanExecuteCommands(true, !check_position) —
                // first arg (allow_in_buildings) is unconditionally true.
                if actor.is_pc() && !self.can_pc_execute_commands(actor_id, true) {
                    return false;
                }
                let Some(victim) = interaction_victim(self, element) else {
                    return false;
                };
                if victim.element_data().blipped || !victim.is_active() {
                    return false;
                }
                if victim.is_human() && !is_human_out_of_order(victim) {
                    return false;
                }
                if !check_position {
                    return true;
                }
                // The PC override widens the SEARCH range to 3600
                // SquareNorm; the human-base arm uses 1600.  Branch by
                // actor type so PC search keeps the wider reach.
                let max_sq = if actor.is_pc() { 3600.0 } else { 1600.0 };
                square_distance(actor, victim) < max_sq
            }

            // ── Move / MoveOk / MoveWaiting / CrouchDown / CrouchUp
            //    All five gate solely on
            //    CanExecuteCommands(allow_in_buildings=true,
            //                       check_position=true).
            Command::Move
            | Command::MoveOk
            | Command::MoveWaiting
            | Command::CrouchDown
            | Command::CrouchUp => {
                if actor.is_pc() && !self.can_pc_execute_commands(actor_id, true) {
                    return false;
                }
                true
            }

            // ── Seek ────────────────────────────────────────────
            // CanExecuteCommands(true, true) plus the target-active
            // check.  Post-seek re-validation (check_position=false)
            // still applies the gate.
            Command::Seek => {
                if actor.is_pc() && !self.can_pc_execute_commands(actor_id, true) {
                    return false;
                }
                let target_id = match &element.data {
                    SequenceElementData::Movement { element, .. } => *element,
                    _ => None,
                };
                if let Some(id) = target_id
                    && let Some(target) = self.get_entity(id)
                    && !target.is_active()
                {
                    return false;
                }
                true
            }

            // ── Whistle ─────────────────────────────────────────
            // CanExecuteCommands(true, false) — allow_in_buildings=true.
            Command::WhistleCmd => {
                if actor.is_pc() && !self.can_pc_execute_commands(actor_id, true) {
                    return false;
                }
                true
            }

            // ── EnterHelpingClimb ───────────────────────────────
            // CanExecuteCommands() && IsActionEnabled(HelpToClimb) —
            // defaults to allow_in_buildings=false.
            Command::EnterHelpingClimb => {
                if actor.is_pc() && !self.can_pc_execute_commands(actor_id, false) {
                    return false;
                }
                is_pc_action_enabled(assets, actor, crate::profiles::Action::HelpToClimb)
            }

            // ── EnterBeggar ─────────────────────────────────────
            // CanExecuteCommands() && IsActionEnabled(Beggar).
            Command::EnterBeggar => {
                if actor.is_pc() && !self.can_pc_execute_commands(actor_id, false) {
                    return false;
                }
                is_pc_action_enabled(assets, actor, crate::profiles::Action::Beggar)
            }

            // ── PC-only arms ────────────────────────────────────
            //
            // The PC override layers extra per-command gates on top of
            // the human base.

            // ── Jump ────────────────────────────────────────────
            // Reject when posture != OnShoulders, the source line's
            // upward jump-height > 0, AND the source line is flagged
            // helper-needed.  Jump height is `assoc.z_a - src.z_a`.
            Command::JumpCmd => {
                let Some(src_idx) = element
                    .get_property(crate::sequence::Field::JumplineSource)
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::LineId(id) => Some(*id),
                        crate::sequence::FieldValue::Integer(id) => {
                            crate::jump_line::JumpLineIndex::new(*id)
                        }
                        _ => None,
                    })
                else {
                    return false;
                };
                let Some(src_line) = self
                    .world
                    .fast_grid
                    .level
                    .jump_lines
                    .get(usize::from(src_idx))
                else {
                    return false;
                };
                let jump_height = src_line
                    .associated_line_index
                    .and_then(|i| self.world.fast_grid.level.jump_lines.get(i as usize))
                    .map(|dst| dst.z_a - src_line.z_a)
                    .unwrap_or(0.0);
                if actor.element_data().posture != Posture::OnShoulders
                    && jump_height > 0.0
                    && src_line.helper_needed
                {
                    return false;
                }
                true
            }

            // ── TakeCorpse ──────────────────────────────────────
            // Reject when actor is out-of-order, the corpse is gone,
            // not dead/unconscious, in a non-corpse posture, already
            // carried by someone else, or further than 40-MaxNorm away.
            Command::TakeCorpse => {
                if !actor.is_pc() {
                    return true;
                }
                if is_human_out_of_order(actor) {
                    return false;
                }
                let Some(corpse) = interaction_victim(self, element) else {
                    return false;
                };
                let posture = corpse.element_data().posture;
                let posture_ok = matches!(
                    posture,
                    Posture::Lying | Posture::Dead | Posture::DeadBack | Posture::Tied
                );
                let unconscious = corpse.human_data().is_some_and(|h| h.unconscious);
                let dead = corpse.is_dead();
                let carrier = corpse.human_data().and_then(|h| h.carrier);
                let carrier_ok = match carrier {
                    None => true,
                    Some(c) => c == actor_id,
                };
                if !corpse.is_active() || (!unconscious && !dead) || !posture_ok || !carrier_ok {
                    return false;
                }
                if check_position && max_norm_distance(actor, corpse) >= 40.0 {
                    return false;
                }
                true
            }

            // ── DropCorpse ──────────────────────────────────────
            // Carrier slot must be occupied — non-PC actors fall
            // through to base behaviour (return true).
            Command::DropCorpse => {
                if !actor.is_pc() {
                    return true;
                }
                actor.pc_data().and_then(|p| p.carried).is_some()
            }

            // ── ClimbUpOnShoulders ──────────────────────────────
            // CanExecute + carrier posture==HelpingToClimb +
            // current_action==HelpToClimb + 40-MaxNorm + ceiling
            // headroom (`can_carry_on_shoulders`).
            Command::ClimbUpOnShoulders => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, false) {
                    return false;
                }
                let Some(carrier) = interaction_victim(self, element) else {
                    return false;
                };
                if !carrier.is_pc() {
                    return false;
                }
                if carrier.element_data().posture != Posture::HelpingToClimb
                    || carrier.pc_data().map(|p| p.current_action)
                        != Some(crate::profiles::Action::HelpToClimb)
                {
                    return false;
                }
                if check_position && max_norm_distance(actor, carrier) >= 40.0 {
                    return false;
                }
                let carrier_pos_3d = carrier.element_data().position();
                let obstacles = self.sight_obstacles(assets);
                if !crate::abilities::can_carry_on_shoulders(carrier_pos_3d, obstacles) {
                    return false;
                }
                true
            }

            // ── ClimbDownFromShoulders ──────────────────────────
            // Carrier slot occupied AND posture == OnShoulders.
            Command::ClimbDownFromShoulders => {
                if !actor.is_pc() {
                    return true;
                }
                let has_carrier = actor.human_data().and_then(|h| h.carrier).is_some();
                let on_shoulders = actor.element_data().posture == Posture::OnShoulders;
                has_carrier && on_shoulders
            }

            // ── Eat ─────────────────────────────────────────────
            // CanExecuteCommands(true, false) && AmmoAmount(Eat) > 0
            // && life_points < LIFEPOINTS_PC.
            Command::EatCmd => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, true) {
                    return false;
                }
                if !self.pc_has_ammo(actor_id, crate::profiles::Action::Eat) {
                    return false;
                }
                actor
                    .pc_data()
                    .map(|p| p.life_points < crate::pc_status::LIFEPOINTS_PC)
                    .unwrap_or(false)
            }

            // ── DropAle ─────────────────────────────────────────
            // CanExecuteCommands(!check_position, !check_position) —
            // when `check_position` is false, the PC may be in a
            // building/on a lift; when true, the PC must be outdoors.
            Command::DropAle => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, !check_position) {
                    return false;
                }
                self.pc_has_ammo(actor_id, crate::profiles::Action::Ale)
            }

            // ── UnlockDoor ──────────────────────────────────────
            // CanExecuteCommands(true, true) && door.is_unlockable().
            Command::UnlockDoor => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, true) {
                    return false;
                }
                let door_id = element
                    .get_property(crate::sequence::Field::Door)
                    .and_then(|v| match v {
                        crate::sequence::FieldValue::DoorId(id) => Some(*id),
                        crate::sequence::FieldValue::Integer(id) => {
                            Some(crate::gate::DoorIndex(*id))
                        }
                        _ => None,
                    });
                let Some(id) = door_id else {
                    return false;
                };
                let Some(host) = self.scripts.mission.as_ref().and_then(|s| s.game_host()) else {
                    return false;
                };
                host.doors
                    .get(usize::from(id))
                    .map(|d| d.is_unlockable())
                    .unwrap_or(false)
            }

            // ── Tie ─────────────────────────────────────────────
            // CanExecuteCommands(true, !check_position) +
            // antagonist `unconscious && posture == Lying` +
            // (check_position == false || dist² <= 1600).
            Command::TieCmd => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, true) {
                    return false;
                }
                let Some(victim) = interaction_victim(self, element) else {
                    return false;
                };
                let unconscious = victim.human_data().is_some_and(|h| h.unconscious);
                let lying = victim.element_data().posture == Posture::Lying;
                if !unconscious || !lying {
                    return false;
                }
                if !check_position {
                    return true;
                }
                square_distance(actor, victim) <= 1600.0
            }

            // ── HitTarget / HandleTarget ────────────────────────
            // CanExecuteCommands(!check_position, !check_position) +
            // target.is_active() + dist² <= 1600 (when checking).
            Command::HitTarget | Command::HandleTarget => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, !check_position) {
                    return false;
                }
                let Some(target) = interaction_victim(self, element) else {
                    return false;
                };
                if !target.is_active() {
                    return false;
                }
                if !check_position {
                    return true;
                }
                square_distance(actor, target) <= 1600.0
            }

            // ── DropAmmo ────────────────────────────────────────
            // CanExecuteCommands() — defaults (false, false).
            Command::DropAmmo => {
                if !actor.is_pc() {
                    return true;
                }
                self.can_pc_execute_commands(actor_id, false)
            }

            // ── EnterListen ─────────────────────────────────────
            // CanExecuteCommands(true, false).
            Command::EnterListen => {
                if !actor.is_pc() {
                    return true;
                }
                self.can_pc_execute_commands(actor_id, true)
            }

            // ── Heal ────────────────────────────────────────────
            // CanExecute(true, false) + Ammo(Heal)>0; for human
            // victim: 0 < life < LIFEPOINTS_PC and (when checking)
            // SquareNorm < 1600.  Non-human victims (FX targets) pass
            // once ammo is available.
            Command::HealCmd => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, true) {
                    return false;
                }
                if !self.pc_has_ammo(actor_id, crate::profiles::Action::Heal) {
                    return false;
                }
                let Some(victim) = interaction_victim(self, element) else {
                    return false;
                };
                if victim.is_human() {
                    let life = victim
                        .pc_data()
                        .map(|p| p.life_points)
                        .or_else(|| victim.npc_data().map(|n| n.life_points))
                        .unwrap_or(0);
                    if life <= 0 || life >= crate::abilities::LIFEPOINTS_PC {
                        return false;
                    }
                    if !check_position {
                        return true;
                    }
                    square_distance(actor, victim) < 1600.0
                } else {
                    // FX target — falls through to `return true`.
                    true
                }
            }

            // ── ThrowApple ──────────────────────────────────────
            // CanExecute + Ammo(Apple)>0 + (non-human OR
            // (!blipped && Soldier && camp != Royalists &&
            // !is_out_of_order)) + is_in_range_for_projectile.
            Command::ThrowApple => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, false) {
                    return false;
                }
                if !self.pc_has_ammo(actor_id, crate::profiles::Action::Apple) {
                    return false;
                }
                let Some(victim_id) = interaction_victim_id(element) else {
                    return false;
                };
                let Some(victim) = self.get_entity(victim_id) else {
                    return false;
                };
                let target_ok = if victim.is_human() {
                    !victim.element_data().blipped
                        && victim.is_soldier()
                        && victim.camp() != crate::element::Camp::Royalists
                        && !is_human_out_of_order(victim)
                } else {
                    true
                };
                if !target_ok {
                    return false;
                }
                self.is_in_range_for_projectile(
                    assets,
                    actor_id,
                    victim.element_data().position_map(),
                    crate::profiles::Action::Apple,
                    Some(victim_id),
                )
            }

            // ── ThrowStone ──────────────────────────────────────
            // Same as apple but also rejects VIP human targets.
            Command::ThrowStone => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, false) {
                    return false;
                }
                if !self.pc_has_ammo(actor_id, crate::profiles::Action::Stone) {
                    return false;
                }
                let Some(victim_id) = interaction_victim_id(element) else {
                    return false;
                };
                let Some(victim) = self.get_entity(victim_id) else {
                    return false;
                };
                let target_ok = if victim.is_human() {
                    !victim.element_data().blipped
                        && victim.is_soldier()
                        && victim.camp() != crate::element::Camp::Royalists
                        && !is_human_out_of_order(victim)
                        && !self.is_entity_vip(assets, victim)
                } else {
                    true
                };
                if !target_ok {
                    return false;
                }
                self.is_in_range_for_projectile(
                    assets,
                    actor_id,
                    victim.element_data().position_map(),
                    crate::profiles::Action::Stone,
                    Some(victim_id),
                )
            }

            // ── HideBehindShield ────────────────────────────────
            // CanExecute + holder.is_holding_shield + (holder's
            // shield_protected is None or this actor).
            Command::HideBehindShield => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, false) {
                    return false;
                }
                let Some(holder) = interaction_victim(self, element) else {
                    return false;
                };
                if !holder.is_pc() {
                    return false;
                }
                let holding = holder
                    .actor_data()
                    .map(|a| a.action_state.is_shield())
                    .unwrap_or(false);
                let shield_protected = holder.pc_data().and_then(|p| p.shield_protected);
                holding && (shield_protected.is_none() || shield_protected == Some(actor_id))
            }

            // ── ThrowPurse ──────────────────────────────────────
            // CanExecute + Ammo(Purse)>0 + ransom budget covers a
            // full purse (COINS_PER_PURSE * COIN_VALUE) +
            // is_in_range_for_projectile against the stored 3D
            // target, then a hypothetical-trajectory simulation that
            // rejects when the resting projectile would have no valid
            // layer.
            Command::ThrowPurse => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, false) {
                    return false;
                }
                if !self.pc_has_ammo(actor_id, crate::profiles::Action::Purse) {
                    return false;
                }
                let ransom = self
                    .mission_domain
                    .campaign
                    .as_ref()
                    .map(|c| c.get_value(crate::campaign::CampaignValue::Ransom))
                    .unwrap_or(0);
                let purse_cost = (crate::inventory::COINS_PER_PURSE as i32)
                    * (crate::inventory::COIN_VALUE as i32);
                if ransom < purse_cost {
                    return false;
                }
                let Some(target_3d) =
                    read_target_point_3d(element, crate::sequence::Field::PurseTarget)
                else {
                    return false;
                };
                if !self.is_in_range_for_projectile(
                    assets,
                    actor_id,
                    crate::coordinates::MapPoint {
                        x: target_3d.x,
                        y: target_3d.y,
                    },
                    crate::profiles::Action::Purse,
                    None,
                ) {
                    return false;
                }
                self.purse_trajectory_lands_on_layer(assets, actor, target_3d)
            }

            // ── ThrowWaspNest ───────────────────────────────────
            // CanExecute + Ammo(WaspNest)>0 + is_in_range_for_projectile.
            Command::ThrowWaspNest => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, false) {
                    return false;
                }
                if !self.pc_has_ammo(actor_id, crate::profiles::Action::WaspNest) {
                    return false;
                }
                let Some(target_2d) =
                    read_target_point_2d(element, crate::sequence::Field::WaspNestTarget)
                else {
                    return false;
                };
                self.is_in_range_for_projectile(
                    assets,
                    actor_id,
                    crate::coordinates::MapPoint {
                        x: target_2d.x,
                        y: target_2d.y,
                    },
                    crate::profiles::Action::WaspNest,
                    None,
                )
            }

            // ── ThrowNet ────────────────────────────────────────
            // CanExecute + Ammo(Net)>0 + is_in_range_for_projectile.
            Command::ThrowNet => {
                if !actor.is_pc() {
                    return true;
                }
                if !self.can_pc_execute_commands(actor_id, false) {
                    return false;
                }
                if !self.pc_has_ammo(actor_id, crate::profiles::Action::Net) {
                    return false;
                }
                let Some(target_2d) =
                    read_target_point_2d(element, crate::sequence::Field::NetTarget)
                else {
                    return false;
                };
                self.is_in_range_for_projectile(
                    assets,
                    actor_id,
                    crate::coordinates::MapPoint {
                        x: target_2d.x,
                        y: target_2d.y,
                    },
                    crate::profiles::Action::Net,
                    None,
                )
            }

            // ── Pay ─────────────────────────────────────────────
            // ransom_value >= BEGGAR_SALARY + dist² <= 2025
            // (when check_position is true).
            Command::Pay => {
                if !actor.is_pc() {
                    return true;
                }
                let ransom = self
                    .mission_domain
                    .campaign
                    .as_ref()
                    .map(|c| c.get_value(crate::campaign::CampaignValue::Ransom))
                    .unwrap_or(0);
                if ransom < crate::engine::BEGGAR_SALARY {
                    return false;
                }
                if !check_position {
                    return true;
                }
                let Some(victim) = interaction_victim(self, element) else {
                    return false;
                };
                square_distance(actor, victim) <= 2025.0
            }

            // Commands the switch doesn't cover — see doc-comment.
            _ => true,
        }
    }

    /// Per-arm `check_sequence_element_validity` pre-tick gate for the
    /// PC `Execute` switch.
    ///
    /// Many `Execute` arms gate the very first frame of a queued sprite
    /// anim on `check_sequence_element_validity(...)` returning true
    /// and early-out with `Aborted` / `Terminated` on failure.  The
    /// Rust animation driver in `engine/animation.rs` runs the sprite
    /// unconditionally, so we run a pre-pass here that walks PCs and
    /// marks the failing sequence elements `Impossible` / `Terminated`
    /// before the animation tick advances them.
    ///
    /// Init phase is detected via `sprite.last_processed_order_id`:
    /// the flag is set only on the very first tick a fresh order is
    /// ever ticked through `Execute`.
    ///
    /// Arms covered:
    /// - `Taking` / `TakingCrouched`: `check_position=true`,
    ///   ABORTED on failure.
    /// - `Eating`: `check_position=false`, TERMINATED.
    /// - `Searching` / `SearchingCrouched`: `check_position=true`,
    ///   TERMINATED.
    /// - `Healing`: `check_position=true`, TERMINATED.
    /// - `TransitionWaitingUprightHelpingClimbing` and
    ///   `TransitionHelpingClimbingWaitingUpright`:
    ///   `check_position=true`, TERMINATED.
    /// - `TransitionWaitingUprightCarryingCorpse`:
    ///   `check_position=true`, ABORTED.
    /// - `TransitionCarryingCorpseWaitingUpright`:
    ///   `check_position=true`, TERMINATED — and when the driving
    ///   command is not `DropCorpse`, additionally drops the carried
    ///   body instantaneously.  The `EnterSwordfight` shortcut isn't
    ///   reached here because `make_posture_transition_pc` already
    ///   short-circuits it before the order is queued.
    /// - All `IsInitialisation()` jump-init arms:
    ///   `check_position=true`, ABORTED.  These arms assert
    ///   NON_INTERRUPTABLE priority, which makes `element_impossible`
    ///   a silent no-op via the priority guard at
    ///   `sequence.rs::element_impossible` — matching shipping
    ///   behaviour where the same NI guard blocks the cascade.
    pub(super) fn pre_tick_pc_execute_validity(&mut self, assets: &LevelAssets) {
        // Snapshot (entity_id, seq_id, elem_idx, terminal, fresh-order)
        // tuples for PCs whose front order is in init phase and whose
        // current arm has a validity rule.  Snapshotting is necessary
        // because `check_sequence_element_validity` takes `&self` and
        // calling it inside the entity loop would conflict with the
        // sequence-manager mutations below.
        struct Pending {
            entity_id: EntityId,
            seq_id: crate::sequence::SequenceId,
            elem_idx: usize,
            terminal: ValidityArmTerminal,
        }
        let mut pending: Vec<Pending> = Vec::new();

        for (entity_id, entity) in self.world.entities.pcs() {
            if entity.element.posture.is_dead() {
                continue;
            }
            if !entity.element.active {
                continue;
            }
            // PC override `Execute` opens with
            // `if (execution_frozen) return InProgress;` — frozen PCs
            // never reach the validity guards.
            if entity.actor.execution_frozen {
                continue;
            }

            let snapshot = self
                .orders
                .sequence_manager
                .current_order_for_actor(entity_id)
                .map(|(s, i, o)| (s, i, o.order_type, o.order_id.get()));
            let Some((seq_id, elem_idx, order_type, order_id)) = snapshot else {
                continue;
            };

            let Some((check_position, terminal)) = pc_init_validity_arm(order_type) else {
                continue;
            };

            // Init-phase detection.  Sprite's
            // `last_processed_order_id` is stamped on the first
            // `perform_action` call for a given order; before that, it
            // holds the previous order's id (or `u32::MAX` for a
            // fresh sprite).
            let last_processed = entity.element.sprite.last_processed_order_id;
            if last_processed == order_id {
                continue;
            }

            // Look up the element so we can run the per-command
            // validity rule.
            let Some(elem) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
                continue;
            };
            if self.check_sequence_element_validity(assets, entity_id.into(), elem, check_position)
            {
                continue;
            }

            // Resolve the special `TransitionCarryingCorpseWaitingUpright`
            // drop-on-fail case here while we still have `elem` in scope.
            let terminal = match terminal {
                ValidityArmTerminal::TerminatedDropCorpseUnlessDrop => {
                    let needs_drop = !matches!(elem.command, Command::DropCorpse);
                    ValidityArmTerminal::TerminatedWithDrop { needs_drop }
                }
                other => other,
            };

            pending.push(Pending {
                entity_id: entity_id.into(),
                seq_id,
                elem_idx,
                terminal,
            });
        }

        // Apply.  `force_drop_carried_corpse_instant` and the
        // sequence-state mutators all need `&mut self`; we drained the
        // entity-iter borrow above so the mutable calls are clean.
        for p in pending {
            tracing::debug!(
                entity = ?p.entity_id,
                seq_id = ?p.seq_id,
                elem_idx = p.elem_idx,
                terminal = ?p.terminal,
                "pc_execute_validity: PC init-arm validity failed — aborting/terminating"
            );
            match p.terminal {
                ValidityArmTerminal::Aborted => {
                    self.orders
                        .sequence_manager
                        .element_impossible(p.seq_id, p.elem_idx);
                }
                ValidityArmTerminal::Terminated => {
                    self.orders
                        .sequence_manager
                        .element_terminated(p.seq_id, p.elem_idx);
                }
                ValidityArmTerminal::TerminatedWithDrop { needs_drop } => {
                    if needs_drop {
                        // Instant drop.
                        self.force_drop_carried_corpse_instant(p.entity_id);
                    }
                    self.orders
                        .sequence_manager
                        .element_terminated(p.seq_id, p.elem_idx);
                }
                // Unresolved variant — should never reach apply phase
                // because the snapshot loop converts it to
                // `TerminatedWithDrop`.  Defensive log + treat as plain
                // Terminated.
                ValidityArmTerminal::TerminatedDropCorpseUnlessDrop => {
                    tracing::warn!(
                        ?p.entity_id,
                        "pc_execute_validity: unresolved TerminatedDropCorpseUnlessDrop"
                    );
                    self.orders
                        .sequence_manager
                        .element_terminated(p.seq_id, p.elem_idx);
                }
            }
        }
    }

    /// Returns true iff the PC is alive, conscious, not netted, not
    /// tied, and — unless `allow_in_buildings` is set — not currently
    /// inside a building sector.  Called from
    /// `check_sequence_element_validity` arms that need to re-gate a
    /// queued command against the PC's current ability to execute,
    /// plus any future per-action enable predicate.
    /// Reads the PC's ammo counter from
    /// `campaign.characters[profile_index].status` since `PcData` does
    /// not store ammo directly (it lives on the campaign-level
    /// `PcStatus` shared with the save system).  Returns `false` if
    /// the actor isn't a PC, the campaign isn't loaded, or the PC
    /// description isn't found.
    fn pc_has_ammo(&self, pc_id: EntityId, action: crate::profiles::Action) -> bool {
        let Some(entity) = self.get_entity(pc_id) else {
            return false;
        };
        let Some(pc) = entity.pc_data() else {
            return false;
        };
        let Some(campaign) = self.mission_domain.campaign.as_ref() else {
            return false;
        };
        campaign
            .characters
            .get(usize::from(pc.profile_index))
            .map(|d| d.status.get_ammo(action) > 0)
            .unwrap_or(false)
    }

    pub fn can_pc_execute_commands(&self, pc_id: EntityId, allow_in_buildings: bool) -> bool {
        let Some(entity) = self.get_entity(pc_id) else {
            return false;
        };
        let Some(human) = entity.human_data() else {
            return false;
        };
        if entity.is_dead() {
            return false;
        }
        if human.unconscious {
            return false;
        }
        if human.stuck_under_nets_counter > 0 {
            return false;
        }
        if entity.element_data().posture == Posture::Tied {
            return false;
        }
        if !allow_in_buildings
            && self
                .entity_building_sector(entity.element_data().sector())
                .is_some()
        {
            return false;
        }
        true
    }

    /// Trajectory-layer validity for `ThrowPurse`.
    ///
    /// Build the launch source (`compute_hand_point` for direction
    /// `ThrowingPurse`, posture `Upright`), simulate the ballistic
    /// trajectory with `APEX_PURSE`/`MASS_PURSE`, then resolve the
    /// landing footprint against the fast-find grid.  The projectile
    /// has no valid layer unless the landing point is inside a motion
    /// sector reachable from the world (ground, or a projection-area
    /// top); we emulate that by accepting only when
    /// `resolve_projectile_landing` finds a containing motion area
    /// and isn't blocked by a motion obstacle.
    fn purse_trajectory_lands_on_layer(
        &self,
        assets: &LevelAssets,
        actor: &Entity,
        target: crate::coordinates::WorldPoint3D,
    ) -> bool {
        let actor_ground = actor.ground_position();
        let direction = crate::position_interface::vector_to_sector_0_to_15_iso(
            target.x - actor_ground.x,
            target.y - actor_ground.y,
        );
        let Some(source) = actor.compute_hand_point_for_posture(
            direction,
            crate::order::OrderType::ThrowingPurse,
            Posture::Upright,
        ) else {
            return false;
        };
        let direction_vec = crate::coordinates::WorldVec3D {
            x: target.x - source.x,
            y: target.y - source.y,
            z: target.z - source.z,
        };
        let velocity = crate::bow_shot::compute_initial_throw_velocity(
            direction_vec,
            crate::bow_shot::APEX_PURSE,
            crate::bow_shot::MASS_PURSE,
            0,
            None,
        );
        let obstacle_check = crate::bow_shot::TrajectoryObstacleCheck {
            fast_find_grid: &self.world.fast_grid,
            layer: actor.element_data().layer(),
            sight_obstacles: self.sight_obstacles(assets),
            water_zones: Some(&assets.water_zones),
        };
        let trajectory = crate::bow_shot::compute_trajectory_ballistic(
            source,
            velocity,
            crate::bow_shot::MASS_PURSE,
            false,
            Some(&obstacle_check),
        );
        let Some(landing) = trajectory.last() else {
            return false;
        };
        let resolution = self
            .world
            .fast_grid
            .resolve_projectile_landing(landing.position.to_map(), self.sight_obstacles(assets));
        resolution.sector.is_some() && !resolution.blocked_by_motion_obstacle
    }
}

/// Failure terminal for an init-phase PC validity arm.
///
/// Distinguishes the `Aborted` (→ `SetState(Impossible)`) vs
/// `Terminated` (→ `DoNextOrder`) early-outs, plus the special
/// `TransitionCarryingCorpseWaitingUpright` arm whose default branch
/// additionally calls `DropCorpse(12, true)` when the driving command
/// isn't `DropCorpse` before terminating.
#[derive(Debug, Clone, Copy)]
pub(super) enum ValidityArmTerminal {
    /// Equivalent of `RHMOTION_ABORTED` → `element_impossible`.  Note:
    /// jump-init arms assert NON_INTERRUPTABLE priority, which makes
    /// the cascade a silent no-op; this matches the
    /// `element_impossible` priority guard.
    Aborted,
    /// Equivalent of `RHMOTION_TERMINATED` → `element_terminated`.
    Terminated,
    /// `TransitionCarryingCorpseWaitingUpright`: mark the element
    /// terminated; if the driving command is not `DropCorpse`, also
    /// drop the carried body instantly.  The unresolved variant is
    /// converted to [`Self::TerminatedWithDrop`] inside the validity
    /// pre-pass once the element's command has been read.
    TerminatedDropCorpseUnlessDrop,
    /// Resolved version of `TerminatedDropCorpseUnlessDrop` once we
    /// know whether the body needs to be dropped.
    TerminatedWithDrop { needs_drop: bool },
}

/// Map a PC sprite-anim order type to its init-phase validity arm, if
/// any.  Returns `Some((check_position, terminal))` for the arms that
/// gate on `check_sequence_element_validity` in their init branch.
/// Returns `None` for arms without a validity guard or whose guard
/// fires at a non-init motion state (those aren't covered by the init
/// pre-pass).
pub(super) fn pc_init_validity_arm(
    anim: crate::order::OrderType,
) -> Option<(bool, ValidityArmTerminal)> {
    use crate::order::OrderType as OT;
    match anim {
        // ── Taking / TakingCrouched ────────────────────────────
        // Aborted on validity failure.
        OT::Taking | OT::TakingCrouched => Some((true, ValidityArmTerminal::Aborted)),

        // ── Eating ─────────────────────────────────────────────
        // Init-phase guard with check_position=false; Terminated.
        OT::Eating => Some((false, ValidityArmTerminal::Terminated)),

        // ── Searching / SearchingCrouched ──────────────────────
        // Terminated on validity failure.
        OT::Searching | OT::SearchingCrouched => Some((true, ValidityArmTerminal::Terminated)),

        // ── Healing ────────────────────────────────────────────
        // Terminated on validity failure.
        OT::Healing => Some((true, ValidityArmTerminal::Terminated)),

        // ── HelpingClimb transitions ───────────────────────────
        // Terminated on validity failure.
        OT::TransitionWaitingUprightHelpingClimbing
        | OT::TransitionHelpingClimbingWaitingUpright => {
            Some((true, ValidityArmTerminal::Terminated))
        }

        // ── TransitionWaitingUprightCarryingCorpse ─────────────
        // Aborted on validity failure.
        OT::TransitionWaitingUprightCarryingCorpse => Some((true, ValidityArmTerminal::Aborted)),

        // ── TransitionCarryingCorpseWaitingUpright ─────────────
        // Default-branch validity check: if it fails, drop instantly
        // (unless command is DropCorpse) and TERMINATE.  The
        // EnterSwordfight shortcut is short-circuited before the
        // order is even queued by `make_posture_transition_pc`'s
        // `Posture::CarryingCorpse` arm.
        OT::TransitionCarryingCorpseWaitingUpright => {
            Some((true, ValidityArmTerminal::TerminatedDropCorpseUnlessDrop))
        }

        // ── Jump-init arms ─────────────────────────────────────
        // All five init-phase jump arms gate on validity with ABORTED
        // on failure.  These elements are NON_INTERRUPTABLE (each arm
        // asserts the priority); the cascade is silently dropped by
        // `element_impossible`'s priority guard, matching shipping
        // behaviour.
        OT::TransitionWaitingOnShouldersJumpingUp
        | OT::TransitionWaitingOnShouldersJumpingLong
        | OT::TransitionWaitingUprightJumpingUp
        | OT::TransitionWaitingCrouchedJumpingDown
        | OT::TransitionWaitingUprightJumpingLong
        | OT::TransitionWaitingSwordJumpingLongSword => Some((true, ValidityArmTerminal::Aborted)),

        _ => None,
    }
}

// ─── Local helpers ─────────────────────────────────────────────

fn interaction_victim_id(element: &SequenceElement) -> Option<EntityId> {
    match &element.data {
        SequenceElementData::Interaction { antagonist } => *antagonist,
        _ => None,
    }
}

fn interaction_victim<'a>(
    engine: &'a EngineInner,
    element: &SequenceElement,
) -> Option<&'a Entity> {
    engine.get_entity(interaction_victim_id(element)?)
}

/// Out-of-order test for a human entity — life ≤ 0 or unconscious.
/// Callers filter on `is_human()` upstream so a missing `human_data` /
/// life field is a true "non-human, not out-of-order" answer rather
/// than a skipped check.
fn is_human_out_of_order(entity: &Entity) -> bool {
    let Some(human) = entity.human_data() else {
        return false;
    };
    let life = entity
        .pc_data()
        .map(|p| p.life_points)
        .or_else(|| entity.npc_data().map(|n| n.life_points))
        .unwrap_or(0);
    life <= 0 || human.unconscious
}

/// Whether the action is currently enabled for the PC's toolbar.
/// Consulted by `check_sequence_element_validity` for
/// `EnterHelpingClimb` / `EnterBeggar`.  Reads the PC's
/// `disabled_actions` / `disabled_actions_temp` at the profile action
/// slot. The action enum value is not the portrait slot.
/// Non-PC actors don't have the toolbar — return true so the generic
/// command path isn't blocked.
fn is_pc_action_enabled(
    assets: &LevelAssets,
    entity: &Entity,
    action: crate::profiles::Action,
) -> bool {
    let Some(pc) = entity.pc_data() else {
        return true;
    };
    let Some(profile) = assets.profile_manager.get_character(pc.profile_index) else {
        return false;
    };
    let Some(idx) = crate::inventory::find_action_slot(profile, action) else {
        return true;
    };
    let disabled = pc.disabled_actions.get(idx).copied().unwrap_or(false)
        || pc.disabled_actions_temp.get(idx).copied().unwrap_or(false);
    !disabled
}

fn is_entity_robin(entity: &Entity) -> bool {
    // The Robin slot lives at index 0 of the campaign character table.
    entity
        .pc_data()
        .map(|pc| u32::from(pc.profile_index) == 0)
        .unwrap_or(false)
}

fn square_distance(a: &Entity, b: &Entity) -> f32 {
    let pa = a.element_data().position_map();
    let pb = b.element_data().position_map();
    let dx = pa.x - pb.x;
    let dy = pa.y - pb.y;
    dx * dx + dy * dy
}

/// 2D Chebyshev (max-norm) distance between two entities.
fn max_norm_distance(a: &Entity, b: &Entity) -> f32 {
    let pa = a.element_data().position_map();
    let pb = b.element_data().position_map();
    (pa.x - pb.x).abs().max((pa.y - pb.y).abs())
}

/// Read a 3D-or-2D target point from a generic sequence element's
/// property map and project it to 2D for the projectile range check.
/// The thrown-projectile target fields are normally stored as
/// `Point3D`; the Rust port may also see them as `GeoPoint2D` in older
/// saves.  The Z is dropped: `is_in_range_for_projectile` re-derives
/// Z from the projection-area obstacles.
fn read_target_point_2d(
    element: &SequenceElement,
    field: crate::sequence::Field,
) -> Option<crate::coordinates::MapPoint> {
    match element.get_property(field)? {
        crate::sequence::FieldValue::Point3D { x, y, .. } => {
            Some(crate::coordinates::MapPoint { x: *x, y: *y })
        }
        crate::sequence::FieldValue::GeoPoint2D { x, y } => {
            Some(crate::coordinates::MapPoint { x: *x, y: *y })
        }
        _ => None,
    }
}

/// Read a target point preserving Z.  `GeoPoint2D`-shaped fields are
/// lifted with `z = 0.0` (matches the spawn-side behaviour at
/// engine/combat.rs ThrowPurseDone, which lifts the stored target the
/// same way).
fn read_target_point_3d(
    element: &SequenceElement,
    field: crate::sequence::Field,
) -> Option<crate::coordinates::WorldPoint3D> {
    match element.get_property(field)? {
        crate::sequence::FieldValue::Point3D { x, y, z } => {
            Some(crate::coordinates::WorldPoint3D {
                x: *x,
                y: *y,
                z: *z,
            })
        }
        crate::sequence::FieldValue::GeoPoint2D { x, y } => {
            Some(crate::coordinates::WorldPoint3D {
                x: *x,
                y: *y,
                z: 0.0,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{
        ActorPc, ActorSoldier, ElementBonus, ElementData, ElementKind, ObjectData,
    };

    fn actor_element(kind: ElementKind) -> ElementData {
        let mut element = ElementData {
            kind,
            active: true,
            ..ElementData::default()
        };
        element.set_position_map(crate::coordinates::MapPoint::new(0.0, 0.0));
        element
    }

    fn object_element(object_type: ObjectType) -> Entity {
        let mut element = ElementData {
            kind: ElementKind::ObjectBonus,
            active: true,
            ..ElementData::default()
        };
        element.set_position_map(crate::coordinates::MapPoint::new(10.0, 0.0));
        Entity::Bonus(ElementBonus {
            element,
            object: ObjectData {
                object_type,
                ..ObjectData::default()
            },
        })
    }

    fn add_pc(engine: &mut EngineInner) -> EntityId {
        engine.add_entity(Entity::Pc(ActorPc {
            element: actor_element(ElementKind::ActorPc),
            actor: Default::default(),
            human: Default::default(),
            pc: Default::default(),
        }))
    }

    fn add_soldier(engine: &mut EngineInner) -> EntityId {
        engine.add_entity(Entity::Soldier(ActorSoldier {
            element: actor_element(ElementKind::ActorSoldier),
            actor: Default::default(),
            human: Default::default(),
            npc: Default::default(),
            soldier: Default::default(),
        }))
    }

    fn take_valid_for(actor_is_pc: bool, object_type: ObjectType) -> bool {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let actor = if actor_is_pc {
            add_pc(&mut engine)
        } else {
            add_soldier(&mut engine)
        };
        let object = engine.add_entity(object_element(object_type));
        let element = SequenceElement::new_interaction(1, Command::Take, Some(actor), Some(object));

        engine.check_sequence_element_validity(&assets, actor, &element, true)
    }

    #[test]
    fn non_pc_take_accepts_only_original_runtime_object_types() {
        for object_type in [ObjectType::Net, ObjectType::Purse, ObjectType::Coin] {
            assert!(
                take_valid_for(false, object_type),
                "non-PC TAKE should accept {object_type:?}"
            );
        }

        for object_type in [
            ObjectType::BonusNet,
            ObjectType::BonusPurse,
            ObjectType::BonusAle,
            ObjectType::Ale,
        ] {
            assert!(
                !take_valid_for(false, object_type),
                "non-PC TAKE should reject {object_type:?}"
            );
        }
    }

    #[test]
    fn pc_take_still_accepts_bonus_pickups() {
        for object_type in [
            ObjectType::BonusNet,
            ObjectType::BonusPurse,
            ObjectType::BonusAle,
        ] {
            assert!(
                take_valid_for(true, object_type),
                "PC TAKE should still accept {object_type:?}"
            );
        }
    }
}
