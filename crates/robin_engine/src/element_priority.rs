//! `DeterminePriority` chain used by the sequence system to decide which
//! commands can interrupt which.
//!
//! ## Virtual-dispatch chain
//!
//! Conceptually a base `Actor` priority function with subclass overrides
//! falling through to the parent:
//!
//! ```text
//! Actor
//! └── Human
//!     ├── PC
//!     └── NPC
//!         ├── Soldier
//!         └── Civilian
//! ```
//!
//! All subclass overrides short-circuit if `elem.priority != NotYetSet`.
//!
//! This module flattens the chain into `determine_priority` keyed on
//! [`ElementKind`]. The `ctx` struct supplies the few actor-state bits
//! (`is_dead`, `is_unconscious`) that the `Human::WAIT` branch needs.

use crate::element::Command;
use crate::element_kinds::ElementKind;
use crate::sequence::{MoveFlags, SequenceElement, SequenceElementData, SequencePriority};

/// Actor state needed to resolve priority for a sequence element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorPriorityContext {
    pub kind: ElementKind,
    pub is_dead: bool,
    pub is_unconscious: bool,
}

impl ActorPriorityContext {
    pub fn new(kind: ElementKind, is_dead: bool, is_unconscious: bool) -> Self {
        Self {
            kind,
            is_dead,
            is_unconscious,
        }
    }
}

/// Resolve a sequence element's priority using the virtual-dispatch chain.
///
/// Short-circuits if `elem.priority != NotYetSet` — every subclass override
/// short-circuits at the top before consulting its own table.
pub fn determine_priority(ctx: ActorPriorityContext, elem: &SequenceElement) -> SequencePriority {
    if elem.priority != SequencePriority::NotYetSet {
        return elem.priority;
    }

    match ctx.kind {
        ElementKind::ActorPc => pc_branch(ctx, elem),
        ElementKind::ActorSoldier => soldier_branch(ctx, elem),
        ElementKind::ActorCivilian => civilian_branch(ctx, elem),
        // Non-actor element kinds should never reach `determine_priority`
        // (only actors own sequence elements). Fall back to the generic
        // actor branch so any stray element still resolves to something
        // sensible rather than defaulting to `Normal` unconditionally.
        _ => actor_branch(elem),
    }
}

// ─────────────────────────────────────────────────────────────────────
// PC override
// ─────────────────────────────────────────────────────────────────────
fn pc_branch(ctx: ActorPriorityContext, elem: &SequenceElement) -> SequencePriority {
    match elem.command {
        Command::Fall => SequencePriority::NonInterruptable,

        Command::CrouchUp | Command::CrouchDown => SequencePriority::Preference,

        Command::Jump
        | Command::ClimbUpOnShoulders
        | Command::ClimbDownFromShoulders
        | Command::EnterHelpingClimb
        | Command::LeaveHelpingClimb
        | Command::EnterListen
        | Command::LeaveListen
        | Command::EnterBeggar
        | Command::LeaveBeggar => SequencePriority::NonInterruptable,

        Command::Take
        | Command::DropAmmo
        | Command::DropAle
        | Command::EatCmd
        | Command::WhistleCmd
        | Command::HealCmd
        | Command::ThrowApple
        | Command::ThrowStone
        | Command::ThrowPurse
        | Command::ThrowWaspNest
        | Command::ThrowNet
        | Command::HideBehindShield
        | Command::UseLever
        | Command::HitTarget
        | Command::HandleTarget
        | Command::TakeTarget
        | Command::Pay
        | Command::TieCmd
        | Command::StrangleCmd
        | Command::TakeCorpse
        | Command::DropCorpse => SequencePriority::Normal,

        Command::Teleport | Command::SpeakHeroReachDestination | Command::SpeakVipsAreForRobin => {
            SequencePriority::None
        }

        _ => human_branch(ctx, elem),
    }
}

// ─────────────────────────────────────────────────────────────────────
// Soldier override
// ─────────────────────────────────────────────────────────────────────
fn soldier_branch(ctx: ActorPriorityContext, elem: &SequenceElement) -> SequencePriority {
    match elem.command {
        Command::ReceiveWaspSting => SequencePriority::Preference,

        Command::Take
        | Command::StartMenace
        | Command::StopMenace
        | Command::StopSleep
        | Command::GatherSoldiers
        | Command::DrinkAle
        | Command::EnterLeisure => SequencePriority::Normal,

        _ => npc_branch(ctx, elem),
    }
}

// ─────────────────────────────────────────────────────────────────────
// Civilian override
// ─────────────────────────────────────────────────────────────────────
fn civilian_branch(ctx: ActorPriorityContext, elem: &SequenceElement) -> SequencePriority {
    match elem.command {
        Command::ReceivePurse => SequencePriority::Normal,
        _ => npc_branch(ctx, elem),
    }
}

// ─────────────────────────────────────────────────────────────────────
// NPC override
// ─────────────────────────────────────────────────────────────────────
fn npc_branch(ctx: ActorPriorityContext, elem: &SequenceElement) -> SequencePriority {
    match elem.command {
        Command::FlyDoor => SequencePriority::Ko,

        Command::DrinkWhisky
        | Command::KickLow
        | Command::LookDown
        | Command::SitDown
        | Command::Point
        | Command::Untie
        | Command::SendDone
        | Command::LookLeft
        | Command::LookRight
        | Command::LeanOut
        | Command::BeggarShowFace
        | Command::EnterLeisure => SequencePriority::Normal,

        Command::EnterAttentiveMode
        | Command::LeaveAttentiveMode
        | Command::LeaveAttentiveModeOfficer => SequencePriority::PostponeEverythingButInjuries,

        _ => human_branch(ctx, elem),
    }
}

// ─────────────────────────────────────────────────────────────────────
// Human override
// ─────────────────────────────────────────────────────────────────────
fn human_branch(ctx: ActorPriorityContext, elem: &SequenceElement) -> SequencePriority {
    match elem.command {
        Command::RaiseBow
        | Command::LowerBow
        | Command::LowerBowLeanOut
        | Command::ShootBow
        | Command::ShootBowOnce
        | Command::SearchCmd
        | Command::SwordstrikeDown
        | Command::WakeUp
        | Command::HitCmd
        | Command::RaiseShield
        | Command::LowerShield
        | Command::RaiseShieldInstantly => SequencePriority::Normal,

        Command::SwordstrikeSmalltalkLeft
        | Command::SwordstrikeSmalltalkRight
        | Command::ParrySmalltalkLeft
        | Command::ParrySmalltalkRight
        | Command::Provoke => SequencePriority::Wait,

        Command::StopParrySword | Command::ParrySword | Command::ParrySwordLow => {
            SequencePriority::Preference
        }

        c if c.is_swordstrike() => SequencePriority::Preference,

        Command::SwordstrikeTired => SequencePriority::Injury,

        Command::EnterSwordfight
        | Command::QuitSwordfight
        | Command::ParryShield
        | Command::EquipBow
        | Command::EquipBowDown
        | Command::UnequipBow => SequencePriority::PostponeEverythingButInjuries,

        Command::ReceiveSwordDamage
        | Command::ReceiveArrowDamage
        | Command::ReceiveStoneDamage
        | Command::ReceiveHitDamage
        | Command::ReceiveDamage
        | Command::ReceiveMobileDamage
        | Command::ReceiveNet => SequencePriority::Injury,

        Command::GetKilledAtBottom => SequencePriority::Lethal,

        Command::Wait => {
            if ctx.is_dead {
                SequencePriority::Lethal
            } else if ctx.is_unconscious {
                SequencePriority::Ko
            } else {
                actor_branch(elem)
            }
        }

        _ => actor_branch(elem),
    }
}

// ─────────────────────────────────────────────────────────────────────
// Actor base
// ─────────────────────────────────────────────────────────────────────
fn actor_branch(elem: &SequenceElement) -> SequencePriority {
    match elem.command {
        Command::StopAll => SequencePriority::Preference,
        Command::PassDoor => SequencePriority::NonInterruptable,
        Command::CrossRailroad => SequencePriority::Preference,
        Command::Wait | Command::Freeze => SequencePriority::None,

        Command::WaitTimer
        | Command::Generic
        | Command::UnlockDoor
        | Command::Turn
        | Command::TurnFast
        | Command::TurnElement
        | Command::ChangePosition
        | Command::Seek
        | Command::PlayAnim
        | Command::PlayAnimLoop
        | Command::PlayAnimFreeze
        | Command::PlayAnimFrozen
        | Command::WaitFreeLift
        | Command::StandUp
        | Command::AssertPosition => SequencePriority::Normal,

        Command::Move => {
            if movement_flags(elem).contains(MoveFlags::MAP) {
                // Movements out of the map are not interruptable.
                SequencePriority::NonInterruptable
            } else {
                SequencePriority::Normal
            }
        }

        Command::BodyCheck => SequencePriority::Injury,

        // Anything reaching the base default is unexpected: panic in
        // debug, fall back to None in release so we don't corrupt the sim.
        other => {
            debug_assert!(
                false,
                "DeterminePriority: unhandled command {other:?} reached actor_branch default",
            );
            SequencePriority::None
        }
    }
}

/// Extract movement flags from a sequence element, or empty if the
/// element is not a movement element. Panics in debug builds if called
/// on a non-movement element with a `Move` command.
fn movement_flags(elem: &SequenceElement) -> MoveFlags {
    match &elem.data {
        SequenceElementData::Movement { flags, .. } => *flags,
        _ => {
            debug_assert!(
                elem.command != Command::Move,
                "Move command without Movement data (element id {})",
                elem.id,
            );
            MoveFlags::empty()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::EntityId;
    use crate::order::OrderType;
    use crate::sequence::SequenceElement;

    fn make_elem(command: Command) -> SequenceElement {
        SequenceElement::new(1, command, Some(EntityId::from_raw(1)))
    }

    fn pc_ctx() -> ActorPriorityContext {
        ActorPriorityContext::new(ElementKind::ActorPc, false, false)
    }
    fn soldier_ctx() -> ActorPriorityContext {
        ActorPriorityContext::new(ElementKind::ActorSoldier, false, false)
    }
    fn civilian_ctx() -> ActorPriorityContext {
        ActorPriorityContext::new(ElementKind::ActorCivilian, false, false)
    }

    #[test]
    fn short_circuit_on_preset_priority() {
        let mut elem = make_elem(Command::Move);
        elem.priority = SequencePriority::Script;
        assert_eq!(
            determine_priority(pc_ctx(), &elem),
            SequencePriority::Script
        );
    }

    #[test]
    fn pc_fall_is_non_interruptable() {
        let elem = make_elem(Command::Fall);
        assert_eq!(
            determine_priority(pc_ctx(), &elem),
            SequencePriority::NonInterruptable,
        );
    }

    #[test]
    fn pc_take_is_normal() {
        let elem = make_elem(Command::Take);
        assert_eq!(
            determine_priority(pc_ctx(), &elem),
            SequencePriority::Normal
        );
    }

    #[test]
    fn pc_teleport_is_none() {
        let elem = make_elem(Command::Teleport);
        assert_eq!(determine_priority(pc_ctx(), &elem), SequencePriority::None);
    }

    #[test]
    fn soldier_wasp_sting_is_preference() {
        let elem = make_elem(Command::ReceiveWaspSting);
        assert_eq!(
            determine_priority(soldier_ctx(), &elem),
            SequencePriority::Preference,
        );
    }

    #[test]
    fn soldier_gather_is_normal() {
        let elem = make_elem(Command::GatherSoldiers);
        assert_eq!(
            determine_priority(soldier_ctx(), &elem),
            SequencePriority::Normal,
        );
    }

    #[test]
    fn civilian_receive_purse_is_normal() {
        let elem = make_elem(Command::ReceivePurse);
        assert_eq!(
            determine_priority(civilian_ctx(), &elem),
            SequencePriority::Normal,
        );
    }

    #[test]
    fn human_receive_damage_is_injury() {
        let elem = make_elem(Command::ReceiveDamage);
        assert_eq!(
            determine_priority(soldier_ctx(), &elem),
            SequencePriority::Injury,
        );
    }

    #[test]
    fn human_swordstrike_is_preference() {
        let elem = make_elem(Command::SwordstrikeThrustC);
        assert_eq!(
            determine_priority(soldier_ctx(), &elem),
            SequencePriority::Preference,
        );
    }

    #[test]
    fn human_pass_door_is_non_interruptable() {
        let elem = make_elem(Command::PassDoor);
        assert_eq!(
            determine_priority(soldier_ctx(), &elem),
            SequencePriority::NonInterruptable,
        );
    }

    #[test]
    fn actor_move_with_map_flag_is_non_interruptable() {
        let mut elem = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(EntityId::from_raw(1)),
            OrderType::WaitingUpright,
        );
        if let SequenceElementData::Movement { flags, .. } = &mut elem.data {
            *flags |= MoveFlags::MAP;
        }
        assert_eq!(
            determine_priority(civilian_ctx(), &elem),
            SequencePriority::NonInterruptable,
        );
    }

    #[test]
    fn actor_move_without_map_flag_is_normal() {
        let elem = SequenceElement::new_movement(
            1,
            Command::Move,
            Some(EntityId::from_raw(1)),
            OrderType::WaitingUpright,
        );
        assert_eq!(
            determine_priority(civilian_ctx(), &elem),
            SequencePriority::Normal,
        );
    }

    #[test]
    fn human_wait_dead_is_lethal() {
        let elem = make_elem(Command::Wait);
        let ctx = ActorPriorityContext::new(ElementKind::ActorSoldier, true, false);
        assert_eq!(determine_priority(ctx, &elem), SequencePriority::Lethal);
    }

    #[test]
    fn human_wait_unconscious_is_ko() {
        let elem = make_elem(Command::Wait);
        let ctx = ActorPriorityContext::new(ElementKind::ActorSoldier, false, true);
        assert_eq!(determine_priority(ctx, &elem), SequencePriority::Ko);
    }

    #[test]
    fn human_wait_alive_falls_through_to_actor_none() {
        let elem = make_elem(Command::Wait);
        assert_eq!(
            determine_priority(soldier_ctx(), &elem),
            SequencePriority::None,
        );
    }

    #[test]
    fn pc_jump_is_non_interruptable() {
        let elem = make_elem(Command::Jump);
        assert_eq!(
            determine_priority(pc_ctx(), &elem),
            SequencePriority::NonInterruptable,
        );
    }

    #[test]
    fn npc_fly_door_is_ko() {
        let elem = make_elem(Command::FlyDoor);
        assert_eq!(
            determine_priority(soldier_ctx(), &elem),
            SequencePriority::Ko,
        );
    }

    #[test]
    fn npc_enter_attentive_is_postpone_but_injuries() {
        let elem = make_elem(Command::EnterAttentiveMode);
        assert_eq!(
            determine_priority(soldier_ctx(), &elem),
            SequencePriority::PostponeEverythingButInjuries,
        );
    }
}
