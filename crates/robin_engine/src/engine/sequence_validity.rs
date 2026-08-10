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
                let opponent_id = match element.get_property(Field::Opponent) {
                    Some(FieldValue::Integer(0)) => {
                        // Intentional Original null-opponent form: this
                        // ENTER_SWORDFIGHT only raises/holds the sword.
                        return true;
                    }
                    Some(FieldValue::Element(id)) => *id,
                    // Do not silently reinterpret a missing or mistyped
                    // required field as the explicit legacy null.
                    _ => return false,
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
                    tracing::trace!(?actor_id, "Take validity failed: missing victim");
                    return false;
                };
                let Some(object) = self.get_entity(victim_id) else {
                    tracing::trace!(
                        ?actor_id,
                        ?victim_id,
                        "Take validity failed: missing object"
                    );
                    return false;
                };
                if !object.is_object() {
                    tracing::trace!(
                        ?actor_id,
                        ?victim_id,
                        kind = ?object.kind(),
                        "Take validity failed: victim is not an object"
                    );
                    return false;
                }
                let Some(obj_data) = object.object_data() else {
                    tracing::trace!(
                        ?actor_id,
                        ?victim_id,
                        "Take validity failed: object payload is missing"
                    );
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
                        let status = self.scroll_status(victim_id);
                        if !object.is_active() || status == ScrollStatus::Taken {
                            tracing::trace!(
                                ?actor_id,
                                ?victim_id,
                                active = object.is_active(),
                                ?status,
                                "Take validity failed: scroll is unavailable"
                            );
                            return false;
                        }
                        if check_position && dist_sq > 900.0 {
                            tracing::trace!(
                                ?actor_id,
                                ?victim_id,
                                dist_sq,
                                "Take validity failed: scroll is out of range"
                            );
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
                assert!(
                    self.scripts.mission.is_some(),
                    "UnlockDoor command validation requires an installed mission script"
                );
                self.script_domains
                    .interactables
                    .doors
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
                let ransom = Some(&self.mission_domain.campaign)
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
                let ransom = Some(&self.mission_domain.campaign)
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
    /// Human and PC `Execute` switches.
    ///
    /// Many `Execute` arms gate the very first frame of a queued sprite
    /// anim on `check_sequence_element_validity(...)` returning true
    /// and early-out with `Aborted` / `Terminated` on failure.  The
    /// Rust animation driver in `engine/animation.rs` runs the sprite
    /// unconditionally, so we run a pre-pass here that walks humans and
    /// marks the failing sequence elements `Impossible` / `Terminated`
    /// before the animation tick advances them.
    ///
    /// Init phase is detected via Actor::Hourglass's selected-order identity,
    /// independently of sprite processing (FrozenAll consumes initialization
    /// even though RHSprite returns before stamping its own order ID).
    ///
    /// Human arms covered:
    /// - `ShootingWithBow`, `ShootingWithBowAnonymous`,
    ///   `ShootingWithBowUp`, and `ShootingWithBowUpAnonymous`:
    ///   `check_position=true`, ABORTED on failure.
    ///
    /// PC-only arms covered:
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
    pub(super) fn pre_tick_human_execute_validity_for(
        &mut self,
        assets: &LevelAssets,
        entity_id: EntityId,
    ) -> bool {
        let Some(entity) = self.world.entities.get(entity_id) else {
            return false;
        };
        if !entity.is_human() {
            return false;
        }
        let actor = entity
            .actor_data()
            .expect("Human Execute validity owner must retain actor data");
        // Inactive (off-map roster) PCs still run Execute and its
        // init-time validity guards: a self-ability launched on a
        // parked PC selects its element, fails the init validity
        // check on the next Execute, and terminates without playing
        // any animation.  Skipping the pre-pass for inactive PCs let
        // the transition play instead, so no active gate here.
        if entity.element_data().posture.is_dead() {
            return false;
        }
        // PC override `Execute` opens with
        // `if (execution_frozen) return InProgress;` — frozen PCs
        // never reach the validity guards. Human::Execute has the same
        // execution-freeze entry guard for non-PC humans.
        if actor.execution_frozen {
            return false;
        }

        let snapshot = self
            .orders
            .sequence_manager
            .current_order_for_actor(entity_id)
            .map(|(s, i, o)| (s, i, o.order_type));
        let Some((seq_id, elem_idx, order_type)) = snapshot else {
            return false;
        };

        let Some((check_position, terminal)) = human_init_validity_arm(order_type, entity.is_pc())
        else {
            return false;
        };

        // Original's guard is `IsInitialisation() == mbNewOrder`, which
        // Actor::Hourglass establishes for this exact Execute call before
        // entering Human::Execute. Do not reconstruct it from a stale or
        // absent historical order ID here: specialized bow owners and
        // restored/test actors can retain a live shot without that history,
        // and re-validating their already-running shoot row would abort it.
        if !actor.execute_order_initialising {
            return false;
        }

        // Look up the element so we can run the per-command
        // validity rule.
        let Some(elem) = self.orders.sequence_manager.get_element(seq_id, elem_idx) else {
            return false;
        };
        if self.check_sequence_element_validity(assets, entity_id, elem, check_position) {
            return false;
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

        // Apply.  `force_drop_carried_corpse_instant` and the
        // sequence-state mutators all need `&mut self`; we drained the
        // entity-iter borrow above so the mutable calls are clean.
        tracing::debug!(
            entity = ?entity_id,
            seq_id = ?seq_id,
            elem_idx,
            terminal = ?terminal,
            "human_execute_validity: Human init-arm validity failed — aborting/terminating"
        );

        // These guards are early returns from RHElementActorHuman::Execute or
        // RHElementActorPC::Execute, so
        // Actor::Hourglass observes and serializes their motion result before
        // it applies the corresponding sequence-state transition.  Merely
        // terminating the Rust element can leave no selected Execute owner
        // below, in which case the previous frame's motion would remain
        // latched indefinitely.
        let motion = match terminal {
            ValidityArmTerminal::Aborted => crate::sprite::MotionState::Aborted,
            ValidityArmTerminal::Terminated
            | ValidityArmTerminal::TerminatedWithDrop { .. }
            | ValidityArmTerminal::TerminatedDropCorpseUnlessDrop => {
                crate::sprite::MotionState::Terminated
            }
        };
        self.world
            .entities
            .get_mut(entity_id)
            .and_then(Entity::actor_data_mut)
            .expect("Human validity owner disappeared before motion-state latch")
            .continuation
            .motion_state = motion;

        match terminal {
            ValidityArmTerminal::Aborted => {
                self.orders
                    .sequence_manager
                    .element_impossible(seq_id, elem_idx);
                // The bow driver is keyed independently from the selected
                // sequence element. Once this Execute guard aborts a shot,
                // the specialized bow tick no longer sees a selected shoot
                // order and therefore cannot clear that runtime latch for
                // us. Only detach the exact shot that was actually made
                // Impossible: NonInterruptable elements deliberately ignore
                // `element_impossible` and must retain their live driver.
                let became_impossible = self
                    .orders
                    .sequence_manager
                    .get_element(seq_id, elem_idx)
                    .is_some_and(|element| {
                        element.state == crate::sequence::SequenceState::Impossible
                    });
                if became_impossible {
                    let actor = self
                        .world
                        .entities
                        .get_mut(entity_id)
                        .and_then(Entity::actor_data_mut)
                        .expect("aborted bow-validity owner lost actor data");
                    if actor.active_shot.sequence_id == Some(seq_id)
                        && actor.active_shot.element_index == elem_idx
                    {
                        actor.active_shot.clear();
                    }
                }
            }
            ValidityArmTerminal::Terminated => {
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            ValidityArmTerminal::TerminatedWithDrop { needs_drop } => {
                if needs_drop {
                    // Instant drop.
                    self.force_drop_carried_corpse_instant(entity_id);
                }
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
            // Unresolved variant — should never reach apply phase
            // because the snapshot loop converts it to
            // `TerminatedWithDrop`.  Defensive log + treat as plain
            // Terminated.
            ValidityArmTerminal::TerminatedDropCorpseUnlessDrop => {
                tracing::warn!(
                    ?entity_id,
                    "human_execute_validity: unresolved TerminatedDropCorpseUnlessDrop"
                );
                self.orders
                    .sequence_manager
                    .element_terminated(seq_id, elem_idx);
            }
        }
        true
    }

    /// Returns true iff the PC is alive, conscious, not netted, not
    /// tied, and — unless `allow_in_buildings` is set — not currently
    /// inside a building sector.  Called from
    /// `check_sequence_element_validity` arms that need to re-gate a
    /// queued command against the PC's current ability to execute,
    /// plus any future per-action enable predicate.
    /// Reads the PC's ammo counter from its exact campaign description since
    /// `PcData` does not store authoritative ammo independently (it lives on
    /// the campaign-level `PcStatus` shared with the save system). Character
    /// profiles are not unique in the campaign table, so `profile_index`
    /// cannot identify this status. Returns `false` if the actor isn't a PC
    /// or its serialized campaign-description identity cannot be resolved.
    fn pc_has_ammo(&self, pc_id: EntityId, action: crate::profiles::Action) -> bool {
        let Some(entity) = self.get_entity(pc_id) else {
            return false;
        };
        let Some(pc) = entity.pc_data() else {
            return false;
        };
        self.pc_description_for_pc_data(pc)
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

/// Human::Execute owns the ordinary/high bow-release arms for PCs, soldiers,
/// and civilians. PC::Execute delegates those orders to Human::Execute, while
/// its other initialization guards remain PC-only.
fn human_init_validity_arm(
    anim: crate::order::OrderType,
    owner_is_pc: bool,
) -> Option<(bool, ValidityArmTerminal)> {
    use crate::order::OrderType as OT;
    match anim {
        OT::ShootingWithBow
        | OT::ShootingWithBowAnonymous
        | OT::ShootingWithBowUp
        | OT::ShootingWithBowUpAnonymous => Some((true, ValidityArmTerminal::Aborted)),
        _ if owner_is_pc => pc_init_validity_arm(anim),
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

    fn bow_execute_fixture() -> (
        EngineInner,
        LevelAssets,
        EntityId,
        EntityId,
        crate::sequence::SequenceId,
    ) {
        use crate::coordinates::{SpriteFrameOffset, SpriteLocalPoint};
        use crate::profiles::{BowProfile, BowShootMode, CharacterProfile, ProfileManager};
        use crate::sprite_script::{NONANIMATION_END, SpriteScript, UNMAPPED};

        let mut campaign = crate::campaign::Campaign::default();
        let mut description = crate::campaign::PcDescription::default();
        // Campaign status lookup validates the serialized description's
        // profile identity before exposing ammo to the PC. A bare default
        // description has no profile and therefore correctly looks corrupt.
        description.character_profile_idx = Some(crate::profiles::CharacterProfileIdx(0));
        description.status.num_arrows = 10;
        campaign.characters.push(description);
        let mut engine = EngineInner::new_with_campaign(campaign);

        let shooter = add_pc(&mut engine);
        let pc = engine
            .get_entity_mut(shooter)
            .and_then(Entity::pc_data_mut)
            .expect("test shooter PC data");
        pc.life_points = 100;
        pc.campaign_description_index = Some(0);

        // Use another PC as the antagonist so this focused fixture exercises
        // the Human out-of-order rule without also depending on NPC camp,
        // civilian, or VIP policy.
        let target = add_pc(&mut engine);
        engine
            .get_entity_mut(target)
            .and_then(Entity::pc_data_mut)
            .expect("test target PC data")
            .life_points = 100;
        let target_element = engine
            .get_entity_mut(target)
            .expect("test target")
            .element_data_mut();
        // Keep the independently stored Original map/ground coordinates
        // coherent. A 1000-unit target is comfortably inside this fixture's
        // 2000-unit normal-shot range and mirrors the established bow
        // lifecycle fixtures.
        target_element.set_position_map(crate::coordinates::MapPoint::new(1000.0, 0.0));
        target_element.set_position(crate::coordinates::WorldPoint3D {
            x: 1000.0,
            y: 0.0,
            z: 0.0,
        });

        let action = crate::order::OrderType::ShootingWithBow;
        let script = SpriteScript {
            action_id: action as u16,
            action_done: 0,
            average_speed: 0.0,
            hotspot: SpriteLocalPoint::ZERO,
            sum_distance: 0,
            frame_ids: vec![1],
            delays: vec![1],
            distances: vec![0],
            offsets: vec![SpriteFrameOffset::ZERO],
            sound_ids: vec![0],
        };
        let mut conversion = vec![UNMAPPED; NONANIMATION_END];
        conversion[action as usize] = 0;
        engine
            .get_entity_mut(shooter)
            .expect("test shooter")
            .element_data_mut()
            .sprite = crate::sprite::Sprite::new(
            std::sync::Arc::new(vec![script; 16]),
            std::sync::Arc::new(conversion),
        );

        let mut profiles = ProfileManager::new();
        profiles.characters.push(CharacterProfile {
            shooting_weapon_id: 1,
            shooting: 100,
            ..CharacterProfile::default()
        });
        profiles.bows.push(BowProfile {
            normal_shoot: BowShootMode {
                range: 2000,
                ..BowShootMode::default()
            },
            ..BowProfile::default()
        });
        let assets = LevelAssets {
            profile_manager: std::sync::Arc::new(profiles),
            ..LevelAssets::new()
        };

        let mut element =
            SequenceElement::new_interaction(1, Command::ShootBow, Some(shooter), Some(target));
        element
            .orders
            .push_back(crate::order::Order::test_new(action, 0.0, 0.0));
        let sequence = engine.orders.sequence_manager.launch_element(element);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(shooter)
            .and_then(Entity::actor_data_mut)
            .expect("test shooter actor data")
            .execute_order_initialising = true;
        (engine, assets, shooter, target, sequence)
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

    #[test]
    fn pc_ammo_validity_uses_campaign_description_identity_not_profile_index() {
        let mut campaign = crate::campaign::Campaign::default();
        let mut wrong_character = crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            ..Default::default()
        };
        wrong_character.status.num_rations = 0;
        let mut actual_character = crate::campaign::PcDescription {
            character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
            ..Default::default()
        };
        actual_character.status.num_rations = 3;
        campaign.characters = vec![wrong_character, actual_character];

        let mut engine = EngineInner::new_with_campaign(campaign);
        let pc_id = add_pc(&mut engine);
        let pc = engine
            .get_entity_mut(pc_id)
            .and_then(Entity::pc_data_mut)
            .expect("test PC data");
        pc.profile_index = crate::profiles::CharacterProfileIdx(0);
        pc.campaign_description_index = Some(1);

        assert!(engine.pc_has_ammo(pc_id, crate::profiles::Action::Eat));
    }

    #[test]
    fn invalid_eating_initialization_latches_execute_terminated_motion() {
        let mut description = crate::campaign::PcDescription::default();
        description.status.num_rations = 1;
        let mut campaign = crate::campaign::Campaign::default();
        campaign.characters.push(description);

        let mut engine = EngineInner::new_with_campaign(campaign);
        let pc_id = add_pc(&mut engine);
        let pc = engine
            .get_entity_mut(pc_id)
            .and_then(Entity::pc_data_mut)
            .expect("test PC data");
        pc.campaign_description_index = Some(0);
        pc.life_points = crate::pc_status::LIFEPOINTS_PC;

        let mut element = SequenceElement::new_generic(1, Command::EatCmd, Some(pc_id));
        element.orders.push_back(crate::order::Order::test_new(
            crate::order::OrderType::Eating,
            0.0,
            0.0,
        ));
        let sequence = engine.orders.sequence_manager.launch_element(element);
        engine
            .orders
            .sequence_manager
            .element_in_progress(sequence, 0);
        engine
            .get_entity_mut(pc_id)
            .and_then(Entity::actor_data_mut)
            .expect("test actor data")
            .execute_order_initialising = true;

        engine.pre_tick_human_execute_validity_for(&LevelAssets::new(), pc_id);

        assert_eq!(
            engine
                .get_entity(pc_id)
                .and_then(Entity::actor_data)
                .expect("test actor data")
                .continuation
                .motion_state,
            crate::sprite::MotionState::Terminated,
            "PC::Execute returns TERMINATED before Actor::Hourglass retires invalid Eating"
        );
    }

    #[test]
    fn healing_new_order_owner_loop_validates_before_ability_effects() {
        enum TargetKind {
            Human(f32),
            Fx(f32),
            SelfHeal,
        }

        for (target_kind, expected_state) in [
            (
                TargetKind::Human(40.0),
                crate::sequence::SequenceState::Terminated,
            ),
            (
                TargetKind::Human(39.999),
                crate::sequence::SequenceState::InProgress,
            ),
            (
                TargetKind::Fx(80.0),
                crate::sequence::SequenceState::InProgress,
            ),
            (
                TargetKind::SelfHeal,
                crate::sequence::SequenceState::InProgress,
            ),
        ] {
            let mut description = crate::campaign::PcDescription {
                character_profile_idx: Some(crate::profiles::CharacterProfileIdx(0)),
                ..Default::default()
            };
            description
                .status
                .set_ammo(crate::profiles::Action::Heal, 1);
            let mut campaign = crate::campaign::Campaign::default();
            campaign.characters.push(description);
            let mut engine = EngineInner::new_with_campaign(campaign);

            let healer = add_pc(&mut engine);
            {
                let healer_entity = engine.get_entity_mut(healer).unwrap();
                let pc = healer_entity.pc_data_mut().unwrap();
                pc.campaign_description_index = Some(0);
                pc.life_points = if matches!(&target_kind, TargetKind::SelfHeal) {
                    50
                } else {
                    100
                };
            }
            let target = match target_kind {
                TargetKind::Human(distance) => {
                    let target = add_pc(&mut engine);
                    let entity = engine.get_entity_mut(target).unwrap();
                    entity.pc_data_mut().unwrap().life_points = 50;
                    entity
                        .element_data_mut()
                        .set_position_map(crate::coordinates::MapPoint::new(distance, 0.0));
                    target
                }
                TargetKind::Fx(distance) => {
                    let mut element = actor_element(ElementKind::Fx);
                    element.set_position_map(crate::coordinates::MapPoint::new(distance, 0.0));
                    engine.add_entity(Entity::Fx(crate::element::ElementFx {
                        element,
                        fx: Default::default(),
                    }))
                }
                TargetKind::SelfHeal => healer,
            };
            let target_life_before = engine
                .get_entity(target)
                .and_then(Entity::pc_data)
                .map(|pc| pc.life_points);

            let mut element =
                SequenceElement::new_interaction(1, Command::HealCmd, Some(healer), Some(target));
            element.orders.push_back(crate::order::Order::test_new(
                crate::order::OrderType::Healing,
                0.0,
                0.0,
            ));
            let sequence = engine.orders.sequence_manager.launch_element(element);
            engine
                .orders
                .sequence_manager
                .element_in_progress(sequence, 0);
            let installed_order = engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .and_then(SequenceElement::current_order)
                .map(|order| crate::element::InstalledActorOrder {
                    order_id: order.order_id,
                    order_type: order.order_type,
                })
                .unwrap();
            let actor = engine
                .get_entity_mut(healer)
                .and_then(Entity::actor_data_mut)
                .unwrap();
            actor.installed_order = Some(installed_order);

            let assets = LevelAssets::new();
            let positions = crate::entities::EntitySlots::filled(engine.world.entities.len(), None);
            engine.tick_actor_owner_envelopes(&crate::sim_rng::test_context(), &assets, &positions);

            assert_eq!(
                engine
                    .orders
                    .sequence_manager
                    .get_element(sequence, 0)
                    .unwrap()
                    .state,
                expected_state
            );
            assert_eq!(
                engine
                    .get_entity(target)
                    .and_then(Entity::pc_data)
                    .map(|pc| pc.life_points),
                target_life_before,
                "Healing validity must run before any ability effect"
            );
            if expected_state == crate::sequence::SequenceState::Terminated {
                assert_eq!(
                    engine
                        .get_entity(healer)
                        .and_then(Entity::actor_data)
                        .unwrap()
                        .continuation
                        .motion_state,
                    crate::sprite::MotionState::Terminated
                );
            } else {
                assert_eq!(
                    engine.actor_order_type(healer),
                    Some(crate::order::OrderType::Healing),
                    "valid FX and self-Heal targets retain canonical Healing"
                );
            }
        }
    }

    #[test]
    fn bow_release_initialization_aborts_when_target_becomes_invalid_during_raise() {
        let (mut engine, assets, shooter, target, sequence) = bow_execute_fixture();
        // The valid-control test below establishes the fixture baseline. This
        // test changes only the target state that became stale during raise.
        engine
            .get_entity_mut(target)
            .and_then(Entity::human_data_mut)
            .expect("test target human data")
            .unconscious = true;
        engine.pre_tick_human_execute_validity_for(&assets, shooter);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .expect("test shot element after abort")
                .state,
            crate::sequence::SequenceState::Impossible,
            "PC bow shot must abort before its shooting row starts",
        );
        assert_eq!(
            engine
                .get_entity(shooter)
                .and_then(Entity::actor_data)
                .expect("test shooter actor data")
                .continuation
                .motion_state,
            crate::sprite::MotionState::Aborted,
        );
    }

    #[test]
    fn aborted_bow_release_clears_matching_active_shot_and_allows_the_next_launch() {
        use crate::bow_shot::{BeginShotResult, begin_bow_shot};
        use crate::movement::ActiveShot;
        use crate::weapons::ShootMode;

        let (mut engine, assets, shooter, target, sequence) = bow_execute_fixture();
        engine
            .get_entity_mut(shooter)
            .and_then(Entity::actor_data_mut)
            .expect("test shooter actor data")
            .active_shot = ActiveShot {
            sequence_id: Some(sequence),
            element_index: 0,
            target: Some(target),
            order_id: Some(std::num::NonZeroU32::new(77).expect("nonzero test order")),
            released: false,
            shoot_mode: Some(ShootMode::Normal),
        };
        engine
            .get_entity_mut(target)
            .and_then(Entity::human_data_mut)
            .expect("test target human data")
            .unconscious = true;

        engine.pre_tick_human_execute_validity_for(&assets, shooter);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .expect("aborted test shot element")
                .state,
            crate::sequence::SequenceState::Impossible,
        );
        assert!(
            !engine
                .get_entity(shooter)
                .and_then(Entity::actor_data)
                .expect("test shooter actor data after abort")
                .active_shot
                .is_active(),
            "execute-time abort must detach its matching bow runtime"
        );

        let next_sequence =
            engine
                .orders
                .sequence_manager
                .launch_element(SequenceElement::new_interaction(
                    1,
                    Command::ShootBow,
                    Some(shooter),
                    Some(target),
                ));
        let result = begin_bow_shot(
            &mut engine.world.entities,
            &mut engine.orders.sequence_manager,
            shooter,
            target,
            next_sequence,
            0,
            false,
            1,
            Some(ShootMode::Normal),
            &mut engine.orders.next_order_id,
        );
        assert_eq!(
            result,
            BeginShotResult::Started,
            "the stale ActiveShot latch must not reject a later bow launch"
        );
    }

    #[test]
    fn non_interruptable_bow_validity_noop_retains_matching_active_shot() {
        use crate::movement::ActiveShot;
        use crate::weapons::ShootMode;

        let (mut engine, assets, shooter, target, sequence) = bow_execute_fixture();
        engine.orders.sequence_manager.set_element_priority(
            sequence,
            0,
            crate::sequence::SequencePriority::NonInterruptable,
        );
        engine
            .get_entity_mut(shooter)
            .and_then(Entity::actor_data_mut)
            .expect("test shooter actor data")
            .active_shot = ActiveShot {
            sequence_id: Some(sequence),
            element_index: 0,
            target: Some(target),
            order_id: Some(std::num::NonZeroU32::new(78).expect("nonzero test order")),
            released: false,
            shoot_mode: Some(ShootMode::Normal),
        };
        engine
            .get_entity_mut(target)
            .and_then(Entity::human_data_mut)
            .expect("test target human data")
            .unconscious = true;

        engine.pre_tick_human_execute_validity_for(&assets, shooter);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .expect("non-interruptable test shot element")
                .state,
            crate::sequence::SequenceState::InProgress,
        );
        assert!(
            engine
                .get_entity(shooter)
                .and_then(Entity::actor_data)
                .expect("test shooter actor data after guarded abort")
                .active_shot
                .is_active(),
            "a blocked Impossible transition must retain its matching bow runtime"
        );
    }

    #[test]
    fn human_bow_validity_arms_cover_ordinary_releases_but_not_leaning_out() {
        use crate::order::OrderType as OT;

        for order in [
            OT::ShootingWithBow,
            OT::ShootingWithBowAnonymous,
            OT::ShootingWithBowUp,
            OT::ShootingWithBowUpAnonymous,
        ] {
            assert!(matches!(
                human_init_validity_arm(order, true),
                Some((true, ValidityArmTerminal::Aborted))
            ));
            assert!(matches!(
                human_init_validity_arm(order, false),
                Some((true, ValidityArmTerminal::Aborted))
            ));
        }

        assert!(human_init_validity_arm(OT::ShootingWithBowLeaningOut, false).is_none());
    }

    #[test]
    fn bow_release_initialization_preserves_valid_target_shots() {
        let (mut engine, assets, shooter, _target, sequence) = bow_execute_fixture();
        engine.pre_tick_human_execute_validity_for(&assets, shooter);

        assert_eq!(
            engine
                .orders
                .sequence_manager
                .get_element(sequence, 0)
                .expect("valid test shot element")
                .state,
            crate::sequence::SequenceState::InProgress,
            "valid PC bow shot must remain in progress",
        );
    }

    #[test]
    fn enter_swordfight_accepts_explicit_null_opponent_but_not_missing_field() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let actor = add_soldier(&mut engine);

        let missing = SequenceElement::new_generic(1, Command::EnterSwordfight, Some(actor));
        assert!(!engine.check_sequence_element_validity(&assets, actor, &missing, true));

        let mut raise_sword =
            SequenceElement::new_generic(1, Command::EnterSwordfight, Some(actor));
        raise_sword.set_property(Field::Opponent, FieldValue::Integer(0));
        raise_sword.set_property(Field::JumplineDestination, FieldValue::Integer(0));
        assert!(engine.check_sequence_element_validity(&assets, actor, &raise_sword, true));
    }

    #[test]
    #[should_panic(expected = "UnlockDoor command validation requires an installed mission script")]
    fn pc_unlock_door_rejects_missing_mission_script() {
        let mut engine = EngineInner::new();
        let assets = LevelAssets::new();
        let actor = add_pc(&mut engine);
        let mut element = SequenceElement::new_generic(1, Command::UnlockDoor, Some(actor));
        element.set_property(Field::Door, FieldValue::DoorId(crate::gate::DoorIndex(0)));

        engine.check_sequence_element_validity(&assets, actor, &element, true);
    }
}
