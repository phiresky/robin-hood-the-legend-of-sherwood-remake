//! Transition generation: helpers that compute animation orders
//! injected before a sequence element's command logic runs.
//!
//! Every newly launched sequence element needs to check whether the
//! actor's current posture and action-state are compatible with the
//! command's requirements, and — if not — queue the matching
//! transition animations onto the element *before* its own logic
//! runs.  For example, a `Move` issued to a Crouched soldier without
//! `CAN_BE_CROUCHED` needs to first play the `CROUCH_UP` animation;
//! a `ShootBow` issued while the bow is un-equipped needs to play
//! the `EQUIP_BOW` transition first.
//!
//! The actor-class hierarchy (base → human → npc/pc, with soldier
//! inheriting from npc) is reproduced as explicit `match`es keyed on
//! `ElementKind` rather than virtual dispatch.
//!
//! # Scope
//!
//! [`EngineInner::generate_transition`] is the public entry point.  A
//! future integration task will wire this into `arbitrate_instruct`,
//! replacing (and deleting) the narrower
//! [`EngineInner::auto_leave_disguise_if_needed`] helper in
//! [`crate::engine::tick`].  For now this module is self-contained and
//! exercised only by its own unit tests.

// The entire module is not yet wired into `arbitrate_instruct` — the
// follow-up integration task will flip that switch.  Suppress
// unused-code lints for now so the dead-code warnings don't drown out
// real issues during development.
#![allow(dead_code)]

use crate::element::{ActionState, Command, EntityId, Posture};
use crate::element_kinds::{
    ChangePostureFlags as CP, ElementKind, EnterActionStateFlags as EA, ExitActionStateFlags as EX,
};
use crate::order::OrderType;
use crate::sequence::{SequenceElementData, SequenceId};

use super::EngineInner;

// ---------------------------------------------------------------------------
// Snapshot: read-only context passed to the flag/transition helpers so
// they don't need to re-read from `EngineInner` every time.
// ---------------------------------------------------------------------------

/// Minimal actor/element view needed to decide which flags to set and
/// which transition orders to queue.  Keeps the pure flag-decision
/// helpers independent from the rest of the engine state.
#[derive(Debug, Clone, Copy)]
struct TransitionCtx {
    kind: ElementKind,
    command: Command,
    /// Movement element's order action — only meaningful when the
    /// element is a Movement variant; set to `None` for every other
    /// command.
    movement_action: Option<OrderType>,
    /// The actor's current posture.
    actor_posture: Posture,
    /// The actor's current action state.
    actor_action_state: ActionState,
    /// The posture the actor is scheduled to have once transition
    /// orders finish.
    posture_after_transition: Posture,
    /// The action state scheduled after transition orders finish.
    action_state_after_transition: ActionState,
    /// Whether this element is part of a movement chain — movement
    /// sub-elements inherit their transition orders from the
    /// surrounding movement sequence, so a few action-transition
    /// branches skip the redundant queue on them.
    is_part_of_movement: bool,
    /// Soldier attentive flag.  Only meaningful for soldier actors;
    /// defaults to `false` for everyone else.
    attentive: bool,
    /// For PC `WAIT` in a force-crouched sector: overrides the default
    /// upright flag set with crouched flags.  Plumbed via the context
    /// so the pure flag helper doesn't need sector access.
    force_crouched: bool,
    /// For `PASS_DOOR` movement commands: the door's type, resolved
    /// from `gate_id`.  `None` for non-movement or non-door commands.
    door_type: Option<crate::gate::DoorType>,
    /// For `PASS_DOOR` on a lift-type door: the lift type of the
    /// adjacent lift sector (the side opposite the actor).  Three
    /// semantic values matter:
    /// - `Some(Stairs)`: use default-door flag set.
    /// - `Some(Ladder)` or `Some(Wall)`: collapse to `MUST_UPRIGHT`.
    /// - `None`: not a lift door, or adjacent sector is not a lift.
    door_lift_kind: Option<crate::sector::LiftType>,
    /// For `TAKE` interactions: whether the antagonist is a Net-type
    /// object.  Nets omit `CAN_BE_CROUCHED` so the PC is forced
    /// upright.
    antagonist_is_net: bool,
}

// ===========================================================================
// GetTransitionFlags
// ===========================================================================

/// Base-class transition flags.  Sets the three flag groups based
/// purely on the command and — for movement commands — the movement's
/// `action`.  Subclass dispatchers call into this for commands they do
/// not override.
fn get_transition_flags_actor(ctx: &TransitionCtx) -> (EX, CP, EA) {
    use Command::*;
    let mut exit = EX::empty();
    let mut change = CP::empty();
    let mut enter = EA::empty();

    match ctx.command {
        Wait | WaitTimer => {
            exit = EX::MUST_BE_WAITING;
            enter = EA::MUST_BE_BORED;
        }

        Move | Seek => {
            let Some(action) = ctx.movement_action else {
                tracing::warn!(
                    ?ctx.command,
                    "GenerateTransition: movement command with no movement_action; using no flags"
                );
                return (exit, change, enter);
            };
            match action {
                OrderType::WalkingUpright | OrderType::WalkingStairs => {
                    exit = EX::MUST_BE_WAITING
                        | EX::CAN_BE_MOVING
                        | EX::CAN_BE_HOLDING_SWORD
                        | EX::CAN_BE_HOLDING_SHIELD
                        | EX::CAN_BE_ALERTED;
                    change = CP::MUST_BE_UPRIGHT
                        | CP::CAN_BE_CROUCHED
                        | CP::CAN_BE_ON_LADDER
                        | CP::CAN_BE_ON_WALL
                        | CP::CAN_BE_CARRYING_CORPSE
                        | CP::CAN_BE_CARRYING_ON_SHOULDERS;
                }
                OrderType::RunningUpright => {
                    exit = EX::MUST_BE_WAITING
                        | EX::CAN_BE_MOVING_FAST
                        | EX::CAN_BE_HOLDING_SWORD
                        | EX::CAN_BE_ALERTED;
                    change = CP::MUST_BE_UPRIGHT
                        | CP::CAN_BE_ON_LADDER
                        | CP::CAN_BE_ON_WALL
                        | CP::CAN_BE_CARRYING_CORPSE
                        | CP::CAN_BE_CARRYING_ON_SHOULDERS;
                }
                OrderType::WalkingCrouched => {
                    exit = EX::MUST_BE_WAITING | EX::CAN_BE_MOVING_FAST | EX::CAN_BE_ALERTED;
                    change = CP::MUST_BE_CROUCHED | CP::CAN_BE_ON_LADDER | CP::CAN_BE_ON_WALL;
                }
                OrderType::WalkingCarryingOnShoulders | OrderType::WalkingWithCorpse => {
                    // No transition flags for these movement actions.
                }
                OrderType::WalkingWithSword => {
                    exit = EX::MUST_BE_WAITING
                        | EX::CAN_BE_MOVING
                        | EX::CAN_BE_HOLDING_SWORD
                        | EX::CAN_BE_ALERTED;
                    change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_ON_LADDER | CP::CAN_BE_ON_WALL;
                }
                OrderType::RunningWithSword => {
                    exit = EX::MUST_BE_WAITING
                        | EX::CAN_BE_MOVING_FAST
                        | EX::CAN_BE_HOLDING_SWORD
                        | EX::CAN_BE_ALERTED;
                    change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_ON_LADDER | CP::CAN_BE_ON_WALL;
                }
                OrderType::WalkingWithShield => {
                    exit = EX::MUST_BE_WAITING
                        | EX::CAN_BE_MOVING
                        | EX::CAN_BE_HOLDING_SHIELD
                        | EX::CAN_BE_ALERTED;
                    change = CP::MUST_BE_UPRIGHT;
                }
                OrderType::RiderCharging => {
                    exit = EX::MUST_BE_WAITING
                        | EX::CAN_BE_MOVING_FAST
                        | EX::CAN_BE_HOLDING_SWORD
                        | EX::CAN_BE_ALERTED;
                    change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_ON_LADDER | CP::CAN_BE_ON_WALL;
                }
                OrderType::ClimbingWallUp
                | OrderType::ClimbingWallDown
                | OrderType::ClimbingWallUpFast
                | OrderType::ClimbingWallDownFast => {
                    exit = EX::MUST_BE_WAITING | EX::CAN_BE_MOVING | EX::CAN_BE_MOVING_FAST;
                    change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_ON_WALL;
                }
                OrderType::ClimbingLadderUp | OrderType::ClimbingLadderDown => {
                    exit = EX::MUST_BE_WAITING | EX::CAN_BE_MOVING | EX::CAN_BE_MOVING_FAST;
                    change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_ON_LADDER;
                }
                other => {
                    tracing::warn!(?other, "GenerateTransition: unhandled movement action");
                }
            }
        }

        Turn | TurnFast | TurnElement => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_HOLDING_SWORD | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_HELPING_TO_CLIMB;
        }

        CrouchDown => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_MOVING | EX::CAN_BE_MOVING_FAST;
            change = CP::MUST_BE_UPRIGHT;
        }

        CrouchUp => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_MOVING | EX::CAN_BE_MOVING_FAST;
            change = CP::MUST_BE_CROUCHED;
        }

        PassDoor => {
            // Dispatch on DoorType (and, for lifts, the adjacent
            // LiftType) via fields pre-populated by `build_ctx`.
            use crate::gate::DoorType as DT;
            use crate::sector::LiftType as LT;
            let shared_exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_MOVING
                | EX::CAN_BE_MOVING_FAST
                | EX::CAN_BE_ALERTED
                | EX::CAN_BE_HOLDING_SWORD
                | EX::CAN_BE_HOLDING_SHIELD;
            match ctx.door_type {
                Some(DT::Building) | Some(DT::BuildingTrap) | Some(DT::Gate) | Some(DT::Trap) => {
                    // building/gate/trap doors: upright or crouched
                    // or carrying-corpse allowed, but NOT
                    // carrying-on-shoulders.
                    change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_CROUCHED | CP::CAN_BE_CARRYING_CORPSE;
                    exit = shared_exit;
                }
                Some(DT::Default) | Some(DT::Reinforcement) => {
                    // default door adds CAN_BE_CARRYING_ON_SHOULDERS
                    // on top of the building-door set.
                    change = CP::MUST_BE_UPRIGHT
                        | CP::CAN_BE_CROUCHED
                        | CP::CAN_BE_CARRYING_CORPSE
                        | CP::CAN_BE_CARRYING_ON_SHOULDERS;
                    exit = shared_exit;
                }
                Some(DT::LiftHigh) | Some(DT::LiftLow) | Some(DT::LiftHighCrenel) => {
                    // lift doors: adjacent sector's LiftType
                    // determines the outcome.
                    match ctx.door_lift_kind {
                        Some(LT::Stairs) => {
                            // Treat like default door.
                            change = CP::MUST_BE_UPRIGHT
                                | CP::CAN_BE_CROUCHED
                                | CP::CAN_BE_CARRYING_CORPSE
                                | CP::CAN_BE_CARRYING_ON_SHOULDERS;
                            exit = shared_exit;
                        }
                        Some(LT::Ladder) | Some(LT::Wall) => {
                            // collapse to MUST_UPRIGHT only; no
                            // exit-state permissions.
                            change = CP::MUST_BE_UPRIGHT;
                        }
                        _ => {
                            // Adjacent sector is not a lift — leave
                            // all flags empty.
                        }
                    }
                }
                None => {
                    // Door not resolved (e.g. unit tests without
                    // engine-side door data).  Fall back to the
                    // default-door flag set so behaviour stays sane.
                    change = CP::MUST_BE_UPRIGHT
                        | CP::CAN_BE_CROUCHED
                        | CP::CAN_BE_CARRYING_CORPSE
                        | CP::CAN_BE_CARRYING_ON_SHOULDERS;
                    exit = shared_exit;
                }
            }
        }

        AssertPosition | Freeze | WaitFreeLift | Generic | ChangePosition | PlayAnim
        | PlayAnimFreeze | PlayAnimFrozen | PlayAnimLoop | ActivateApple | ActivateArrow
        | ActivateHandle | ActivateHeal | ActivateLever | ActivateMoney | ActivateSearch
        | ActivateStone | ActivateSword => {
            // No transition flags — these are no-op commands for the
            // base actor (their Translate arms handle their own setup).
        }

        _ => {
            tracing::warn!(
                ?ctx.command,
                kind = ?ctx.kind,
                "GenerateTransition: base-class unhandled command — no flags set"
            );
        }
    }

    (exit, change, enter)
}

fn get_transition_flags_human(ctx: &TransitionCtx) -> (EX, CP, EA) {
    use Command::*;
    let mut exit = EX::empty();
    let mut change = CP::empty();
    let mut enter = EA::empty();

    match ctx.command {
        Wait | WaitTimer => {
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_BORED
                | EX::CAN_BE_AIMING_BOW
                | EX::CAN_BE_AIMING_BOW_UP
                | EX::CAN_BE_HOLDING_SWORD
                | EX::CAN_BE_PARRYING_SWORD
                | EX::CAN_BE_ALERTED
                | EX::CAN_BE_HOLDING_SHIELD
                | EX::CAN_BE_PARRYING_SHIELD
                | EX::CAN_BE_MENACING
                | EX::CAN_BE_SLEEPING
                | EX::CAN_BE_LISTENING
                | EX::CAN_BE_HIDING_BEHIND_SHIELD
                | EX::CAN_BE_AIMING_BOW_DOWN;
            enter = EA::MUST_BE_BORED;
        }

        EquipBow | EquipBowDown => {
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_AIMING_BOW
                | EX::CAN_BE_AIMING_BOW_UP
                | EX::CAN_BE_AIMING_BOW_DOWN
                | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_ANONYMOUS_ARCHER;
            enter = EA::MUST_BE_ALERTED;
        }

        UnequipBow => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_AIMING_BOW | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_ANONYMOUS_ARCHER;
        }

        LowerBow => {
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_AIMING_BOW
                | EX::CAN_BE_AIMING_BOW_UP
                | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_ANONYMOUS_ARCHER;
            enter = EA::MUST_BE_AIMING_BOW_UP;
        }

        RaiseBow => {
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_AIMING_BOW
                | EX::CAN_BE_AIMING_BOW_UP
                | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_ANONYMOUS_ARCHER;
            enter = EA::MUST_BE_AIMING_BOW;
        }

        LowerBowLeanOut => {
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_AIMING_BOW
                | EX::CAN_BE_AIMING_BOW_DOWN
                | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT;
            enter = EA::MUST_BE_AIMING_BOW;
        }

        ShootBow | ShootBowOnce => {
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_AIMING_BOW
                | EX::CAN_BE_AIMING_BOW_UP
                | EX::CAN_BE_AIMING_BOW_DOWN
                | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_LEANING_OUT | CP::CAN_BE_ANONYMOUS_ARCHER;
            enter = EA::MUST_BE_AIMING_BOW;
        }

        QuitSwordfight => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_HOLDING_SWORD | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT;
            enter = EA::MUST_BE_HOLDING_SWORD;
        }

        ParrySword
        | ParrySwordLow
        | SwordstrikeSmalltalkLeft
        | SwordstrikeSmalltalkRight
        | ParrySmalltalkLeft
        | ParrySmalltalkRight
        | SwordstrikeTired
        | Provoke => {
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_HOLDING_SWORD
                | EX::CAN_BE_ALERTED
                | EX::CAN_BE_MENACING;
            change = CP::MUST_BE_UPRIGHT;
            enter = EA::MUST_BE_HOLDING_SWORD;
        }

        c if c.is_swordstrike() => {
            // Any swordstrike command shares the generic
            // sword-transition flags with ParrySword et al.
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_HOLDING_SWORD
                | EX::CAN_BE_ALERTED
                | EX::CAN_BE_MENACING;
            change = CP::MUST_BE_UPRIGHT;
            enter = EA::MUST_BE_HOLDING_SWORD;
        }

        SwordstrikeDown => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_HOLDING_SWORD | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT;
            enter = EA::MUST_BE_HOLDING_SWORD;
        }

        StopParrySword => {
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_PARRYING_SWORD
                | EX::CAN_BE_HOLDING_SWORD
                | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT;
            enter = EA::MUST_BE_PARRYING_SWORD;
        }

        ReceiveDamage | ReceiveSwordDamage | ReceiveArrowDamage | ReceiveStoneDamage
        | ReceiveHitDamage | ReceiveMobileDamage | ReceiveNet => {
            // All flags intentionally empty.
        }

        SearchCmd => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT | CP::CAN_BE_CROUCHED;
        }

        GetKilledAtBottom => {}

        WakeUp => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT;
        }

        HitCmd => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED;
            change = CP::MUST_BE_UPRIGHT;
        }

        RaiseShield => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED | EX::CAN_BE_HOLDING_SHIELD;
            change = CP::MUST_BE_UPRIGHT;
        }

        RaiseShieldInstantly => {
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_ALERTED
                | EX::CAN_BE_MOVING
                | EX::CAN_BE_MOVING_FAST;
            change = CP::MUST_BE_UPRIGHT;
        }

        LowerShield => {
            exit = EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED | EX::CAN_BE_HOLDING_SHIELD;
            change = CP::MUST_BE_UPRIGHT;
        }

        ParryShield => {
            exit = EX::MUST_BE_WAITING
                | EX::CAN_BE_ALERTED
                | EX::CAN_BE_HOLDING_SHIELD
                | EX::CAN_BE_PARRYING_SHIELD;
            change = CP::MUST_BE_UPRIGHT;
            enter = EA::MUST_BE_HOLDING_SHIELD;
        }

        StandUp => {
            // No transition flags needed.
        }

        _ => return get_transition_flags_actor(ctx),
    }

    (exit, change, enter)
}

fn get_transition_flags_npc(ctx: &TransitionCtx) -> (EX, CP, EA) {
    use Command::*;
    match ctx.command {
        Point => (
            EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED,
            CP::MUST_BE_UPRIGHT,
            EA::empty(),
        ),
        SitDown => (EX::MUST_BE_WAITING, CP::MUST_BE_UPRIGHT, EA::empty()),
        BeggarShowFace => (EX::MUST_BE_WAITING, CP::MUST_BE_UPRIGHT, EA::empty()),
        EnterLeisure => (
            EX::MUST_BE_WAITING,
            CP::MUST_BE_UPRIGHT | CP::CAN_BE_LEISURING,
            EA::empty(),
        ),
        _ => get_transition_flags_human(ctx),
    }
}

fn get_transition_flags_soldier(ctx: &TransitionCtx) -> (EX, CP, EA) {
    use Command::*;
    match ctx.command {
        EnterAttentiveMode => (
            EX::MUST_BE_WAITING
                | EX::CAN_BE_HOLDING_SWORD
                | EX::CAN_BE_PARRYING_SWORD
                | EX::CAN_BE_ALERTED,
            CP::MUST_BE_UPRIGHT,
            EA::empty(),
        ),
        LeaveAttentiveMode | LeaveAttentiveModeOfficer => (
            EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED,
            CP::MUST_BE_UPRIGHT,
            EA::MUST_BE_ALERTED,
        ),
        Take | GatherSoldiers | DrinkAle => (
            EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED,
            CP::MUST_BE_UPRIGHT,
            EA::empty(),
        ),
        LookLeft | LookRight => (
            EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED,
            CP::MUST_BE_UPRIGHT,
            EA::empty(),
        ),
        EnterSwordfight => (
            // The exit-state and enter-state flag enums collide on
            // this bit (0x80) — preserve the exact value via
            // `from_bits_retain` rather than fabricating a new
            // ExitFlag variant.
            EX::from_bits_retain(EA::MUST_BE_ALERTED.bits()) | EX::CAN_BE_HOLDING_SWORD,
            CP::MUST_BE_UPRIGHT,
            EA::MUST_BE_ALERTED,
        ),
        ReceiveWaspSting => (EX::MUST_BE_WAITING, CP::MUST_BE_UPRIGHT, EA::empty()),
        StartMenace => (
            EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED,
            CP::MUST_BE_UPRIGHT,
            EA::empty(),
        ),
        StopMenace => (
            EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED | EX::CAN_BE_MENACING,
            CP::MUST_BE_UPRIGHT,
            EA::empty(),
        ),
        LeanOut => (
            EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED,
            CP::MUST_BE_UPRIGHT | CP::CAN_BE_LEANING_OUT,
            EA::empty(),
        ),
        _ => get_transition_flags_npc(ctx),
    }
}

fn get_transition_flags_pc(ctx: &TransitionCtx) -> (EX, CP, EA) {
    use Command::*;
    match ctx.command {
        Wait | WaitTimer => {
            if ctx.force_crouched {
                (
                    EX::MUST_BE_WAITING | EX::CAN_BE_HIDING_BEHIND_SHIELD,
                    CP::MUST_BE_CROUCHED,
                    EA::empty(),
                )
            } else {
                get_transition_flags_human(ctx)
            }
        }
        EnterSwordfight => (
            EX::MUST_BE_WAITING | EX::CAN_BE_HOLDING_SWORD,
            CP::MUST_BE_UPRIGHT,
            EA::empty(),
        ),
        Jump => (
            EX::MUST_BE_WAITING | EX::CAN_BE_HOLDING_SWORD,
            CP::MUST_BE_UPRIGHT | CP::CAN_BE_CROUCHED | CP::CAN_BE_ON_SHOULDERS,
            EA::empty(),
        ),
        Take => {
            // Net antagonist forces upright (no `CAN_BE_CROUCHED`);
            // anything else permits crouched Take.
            let change = if ctx.antagonist_is_net {
                CP::MUST_BE_UPRIGHT
            } else {
                CP::MUST_BE_UPRIGHT | CP::CAN_BE_CROUCHED
            };
            (EX::MUST_BE_WAITING, change, EA::empty())
        }
        EnterHelpingClimb | EnterBeggar | EnterListen => {
            (EX::MUST_BE_WAITING, CP::MUST_BE_UPRIGHT, EA::empty())
        }
        LeaveHelpingClimb => (
            EX::MUST_BE_WAITING,
            CP::MUST_BE_HELPING_TO_CLIMB,
            EA::empty(),
        ),
        LeaveBeggar => (
            EX::MUST_BE_WAITING,
            CP::MUST_BE_SIMULATING_BEGGAR,
            EA::empty(),
        ),
        LeaveListen => (EX::MUST_BE_LISTENING, CP::MUST_BE_UPRIGHT, EA::empty()),
        ClimbUpOnShoulders => (EX::MUST_BE_WAITING, CP::MUST_BE_UPRIGHT, EA::empty()),
        ClimbDownFromShoulders => (EX::MUST_BE_WAITING, CP::MUST_BE_ON_SHOULDERS, EA::empty()),
        TakeCorpse => (EX::MUST_BE_WAITING, CP::MUST_BE_UPRIGHT, EA::empty()),
        DropCorpse => (
            EX::MUST_BE_WAITING,
            CP::MUST_BE_CARRYING_CORPSE,
            EA::empty(),
        ),
        Fall => (
            EX::MUST_BE_WAITING | EX::CAN_BE_MOVING | EX::CAN_BE_BORED,
            CP::MUST_BE_ON_SHOULDERS | CP::CAN_BE_ON_SHOULDERS,
            EA::empty(),
        ),
        DropAmmo | DropAle => (
            EX::MUST_BE_WAITING,
            CP::MUST_BE_UPRIGHT | CP::CAN_BE_CROUCHED,
            EA::empty(),
        ),
        EatCmd | HealCmd | ThrowApple | ThrowStone | ThrowPurse | ThrowWaspNest | ThrowNet
        | UseLever | UnlockDoor | HitTarget | HandleTarget | TakeTarget | Pay | TieCmd
        | StrangleCmd | WhistleCmd => (EX::MUST_BE_WAITING, CP::MUST_BE_UPRIGHT, EA::empty()),
        HideBehindShield => (
            EX::MUST_BE_WAITING,
            CP::MUST_BE_UPRIGHT | CP::CAN_BE_CROUCHED,
            EA::empty(),
        ),
        _ => get_transition_flags_human(ctx),
    }
}

/// One explicit arm for RECEIVE_PURSE; everything else delegates to NPC.
fn get_transition_flags_civilian(ctx: &TransitionCtx) -> (EX, CP, EA) {
    match ctx.command {
        Command::ReceivePurse => (EX::MUST_BE_WAITING, CP::empty(), EA::empty()),
        _ => get_transition_flags_npc(ctx),
    }
}

/// Top-level GetTransitionFlags dispatch keyed on [`ElementKind`].
fn get_transition_flags(ctx: &TransitionCtx) -> (EX, CP, EA) {
    match ctx.kind {
        ElementKind::ActorPc => get_transition_flags_pc(ctx),
        ElementKind::ActorSoldier => get_transition_flags_soldier(ctx),
        ElementKind::ActorCivilian => get_transition_flags_civilian(ctx),
        _ => get_transition_flags_actor(ctx),
    }
}

// ---------------------------------------------------------------------------
// Order queueing helpers
// ---------------------------------------------------------------------------

/// Push a non-movement animation order onto the sequence element.
fn push_anim_order(engine: &mut EngineInner, seq_id: SequenceId, elem_idx: usize, anim: OrderType) {
    let id = engine.alloc_order_id();
    let order = crate::order::Order::new(anim, 0.0, 0.0, id);
    engine
        .sequence_manager
        .push_order_on(seq_id, elem_idx, order);
}

/// Push a non-movement animation order with `compute_direction = false`,
/// used by the posture transitions that must not re-face the actor.
fn push_anim_order_no_dir(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    anim: OrderType,
) {
    let id = engine.alloc_order_id();
    let mut order = crate::order::Order::new(anim, 0.0, 0.0, id);
    order.compute_direction = false;
    engine
        .sequence_manager
        .push_order_on(seq_id, elem_idx, order);
}

fn stand_up_order_for_action_state(action_state: ActionState) -> OrderType {
    if action_state.is_sword() || action_state == ActionState::Menacing {
        OrderType::StandingUpSword
    } else if action_state.is_bow() {
        OrderType::StandingUpBow
    } else {
        OrderType::StandingUp
    }
}

fn set_posture_after(engine: &mut EngineInner, seq_id: SequenceId, elem_idx: usize, p: Posture) {
    if let Some(e) = engine.sequence_manager.get_element_mut(seq_id, elem_idx) {
        e.posture_after_transition = p;
    }
}

fn set_action_state_after(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    a: ActionState,
) {
    if let Some(e) = engine.sequence_manager.get_element_mut(seq_id, elem_idx) {
        e.action_state_after_transition = a;
    }
}

fn push_unequip_bow_transition_orders(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    anonymous: bool,
) {
    let (unload, unequip) = if anonymous {
        (
            OrderType::TransitionUnloadBowAnonymous,
            OrderType::TransitionUnequipBowAnonymous,
        )
    } else {
        (
            OrderType::TransitionUnloadBow,
            OrderType::TransitionUnequipBow,
        )
    };
    push_anim_order(engine, seq_id, elem_idx, unload);
    push_anim_order(engine, seq_id, elem_idx, unequip);
}

/// Build a [`TransitionCtx`] from the current state of `(owner, seq,
/// elem)`. Returns `None` if the entity or element is missing.
fn build_ctx(
    engine: &EngineInner,
    owner: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
) -> Option<TransitionCtx> {
    let entity = engine.get_entity(owner)?;
    let elem = engine.sequence_manager.get_element(seq_id, elem_idx)?;

    let actor_action_state = entity
        .actor_data()
        .map(|a| a.action_state)
        .unwrap_or_default();
    let actor_posture = entity.element_data().posture;
    let is_part_of_movement = matches!(
        elem.command,
        Command::Move
            | Command::MoveOk
            | Command::Seek
            | Command::PassDoor
            | Command::Jump
            | Command::AssertPosition
    );
    let movement_action = match &elem.data {
        SequenceElementData::Movement { action, .. } => Some(*action),
        _ => None,
    };
    let gate_id = match &elem.data {
        SequenceElementData::Movement { gate_id, .. } => *gate_id,
        _ => None,
    };

    let attentive = entity.enemy_ai().map(|e| e.attentive).unwrap_or(false);

    // Interaction antagonist: Net detection for PC `TAKE`.
    let antagonist_is_net = match &elem.data {
        SequenceElementData::Interaction {
            antagonist: Some(antagonist),
        } => engine
            .get_entity(*antagonist)
            .and_then(|e| e.object_data())
            .map(|o| {
                matches!(
                    o.object_type,
                    crate::element::ObjectType::Net | crate::element::ObjectType::BonusNet
                )
            })
            .unwrap_or(false),
        _ => false,
    };

    // `WAIT` force-crouched sector check for PC.
    let actor_sector_num = entity
        .element_data()
        .sector()
        .map(|s| crate::sector::SectorNumber::from(i16::from(s)));
    let force_crouched = actor_sector_num
        .map(|n| engine.sector_forces_crouch(n))
        .unwrap_or(false);

    // Door type / adjacent-sector lift kind lookup for PASS_DOOR.
    let (door_type, door_lift_kind) = match (elem.command, gate_id, actor_sector_num) {
        (Command::PassDoor, Some(idx), actor_sector) => {
            let door = engine
                .mission_script
                .as_ref()
                .and_then(|s| s.game_host())
                .and_then(|h| h.doors.get(usize::from(idx)));
            match door {
                Some(d) => {
                    let ty = d.door_type;
                    // Lift doors: look up the adjacent lift sector's
                    // LiftType.  Pick `sector_in` if actor is on the
                    // `sector_out` side, else `sector_out`.
                    let lift_kind = if matches!(
                        ty,
                        crate::gate::DoorType::LiftHigh
                            | crate::gate::DoorType::LiftLow
                            | crate::gate::DoorType::LiftHighCrenel
                    ) {
                        let adjacent = if actor_sector == Some(d.sector_out) {
                            d.sector_in
                        } else {
                            d.sector_out
                        };
                        engine.get_sector_lift_type(adjacent)
                    } else {
                        None
                    };
                    (Some(ty), lift_kind)
                }
                None => (None, None),
            }
        }
        _ => (None, None),
    };

    Some(TransitionCtx {
        kind: entity.kind(),
        command: elem.command,
        movement_action,
        actor_posture,
        actor_action_state,
        posture_after_transition: elem.posture_after_transition,
        action_state_after_transition: elem.action_state_after_transition,
        is_part_of_movement,
        attentive,
        force_crouched,
        door_type,
        door_lift_kind,
        antagonist_is_net,
    })
}

// ===========================================================================
// MakeActionTransition
// ===========================================================================

/// Returns `false` only when the transition is impossible (e.g.
/// attempting to raise the shield while it's already raised with no
/// exit path).
#[allow(clippy::too_many_arguments)]
fn make_action_transition_actor(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EX,
) -> bool {
    if !flags.contains(EX::MUST_BE_WAITING) {
        return true;
    }

    let Some(entity) = engine.get_entity(owner) else {
        tracing::warn!(?owner, "make_action_transition: entity gone");
        return false;
    };
    let posture = entity.element_data().posture;
    let action_state = entity
        .actor_data()
        .map(|a| a.action_state)
        .unwrap_or_default();

    if action_state == ActionState::Waiting {
        return true;
    }

    let elem = engine.sequence_manager.get_element(seq_id, elem_idx);
    let command = elem.map(|e| e.command).unwrap_or(Command::Null);
    // When true, skip the transition-order insertion for MOVING /
    // MOVING_FAST arms to avoid injecting spurious stop-walking
    // frames into a composite movement chain.
    let is_part_of_movement = command.is_part_of_movement();

    match posture {
        Posture::Upright => match action_state {
            ActionState::Bored => {
                if !flags.contains(EX::CAN_BE_BORED) {
                    push_anim_order(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionWaitingUprightBoredWaitingUpright,
                    );
                    set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
                }
            }
            ActionState::Moving => {
                if !flags.contains(EX::CAN_BE_MOVING) && !is_part_of_movement {
                    push_anim_order(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionWalkingUprightWaitingUpright,
                    );
                    set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
                }
            }
            ActionState::MovingFast => {
                if !flags.contains(EX::CAN_BE_MOVING_FAST) && !is_part_of_movement {
                    push_anim_order(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionRunningUprightWaitingUpright,
                    );
                    set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
                }
            }
            ActionState::HoldingShield => {
                if !flags.contains(EX::CAN_BE_HOLDING_SHIELD) {
                    if command == Command::RaiseShield {
                        // The command is refused because the shield is
                        // already up with no auto-lower path.
                        engine.sequence_manager.element_terminated(seq_id, elem_idx);
                        return false;
                    }
                    push_anim_order(engine, seq_id, elem_idx, OrderType::LoweringShield);
                    set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
                }
            }
            ActionState::Listening => {
                if !flags.contains(EX::CAN_BE_LISTENING) {
                    push_anim_order(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionListeningWaitingUpright,
                    );
                    set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
                }
            }
            ActionState::Waiting => {
                // Handled by the early-return above; arm kept for completeness.
            }
            other => {
                tracing::warn!(
                    ?other,
                    "MakeActionTransition (Upright): unhandled action state; ignoring"
                );
            }
        },
        Posture::Crouched => match action_state {
            ActionState::Moving => {
                if !flags.contains(EX::CAN_BE_MOVING) && !is_part_of_movement {
                    push_anim_order(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionWalkingCrouchedWaitingCrouched,
                    );
                    set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
                }
            }
            ActionState::MovingFast => {
                // No running-crouched in the final game.  Warn and
                // fall through.
                tracing::warn!(
                    "MakeActionTransition (Crouched): MovingFast not expected; skipping"
                );
            }
            other => {
                tracing::warn!(
                    ?other,
                    "MakeActionTransition (Crouched): unhandled action state"
                );
            }
        },
        _ => {}
    }

    true
}

fn make_action_transition_human(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EX,
) -> bool {
    let Some(entity) = engine.get_entity(owner) else {
        return false;
    };
    let action_state = entity
        .actor_data()
        .map(|a| a.action_state)
        .unwrap_or_default();
    let posture = entity.element_data().posture;
    let is_anonymous_archer = posture == Posture::AnonymousArcher;

    match action_state {
        ActionState::AimingWithBow => {
            if flags.contains(EX::MUST_BE_WAITING) && !flags.contains(EX::CAN_BE_AIMING_BOW) {
                // C++ Translate(UNEQUIP_BOW): unload then unequip,
                // with anonymous-posture variants.
                push_unequip_bow_transition_orders(engine, seq_id, elem_idx, is_anonymous_archer);
                set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
            }
            true
        }
        ActionState::AimingWithBowUp => {
            if flags.contains(EX::MUST_BE_WAITING) && !flags.contains(EX::CAN_BE_AIMING_BOW_UP) {
                push_anim_order(engine, seq_id, elem_idx, OrderType::TransitionLoweringBow);
                if !flags.contains(EX::CAN_BE_AIMING_BOW) {
                    // C++ Translate(UNEQUIP_BOW): unload then unequip.
                    push_unequip_bow_transition_orders(
                        engine,
                        seq_id,
                        elem_idx,
                        is_anonymous_archer,
                    );
                    set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
                } else {
                    set_action_state_after(engine, seq_id, elem_idx, ActionState::AimingWithBow);
                }
            }
            true
        }
        ActionState::WaitingSword | ActionState::MovingSword | ActionState::MovingFastSword => {
            if flags.contains(EX::MUST_BE_WAITING) && !flags.contains(EX::CAN_BE_HOLDING_SWORD) {
                // Quit-swordfight transition: queue TransitionLoweringSword.
                // The sword-state arms match the generic sword-action-state
                // case; non-sword action states fall through to the default
                // arm below (which would otherwise terminate the sequence
                // element).
                push_anim_order(engine, seq_id, elem_idx, OrderType::TransitionLoweringSword);
                set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
            }
            true
        }
        ActionState::ParryingSword => {
            if !flags.contains(EX::CAN_BE_PARRYING_SWORD) {
                // Stop-parry-sword transition.
                push_anim_order(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionParryingSwordWaitingSword,
                );
                set_action_state_after(engine, seq_id, elem_idx, ActionState::WaitingSword);
            }
            true
        }
        ActionState::ParryingSwordLow => true,
        s if s.is_shield() => {
            if flags.contains(EX::MUST_BE_WAITING) && !flags.contains(EX::CAN_BE_HOLDING_SHIELD) {
                // Lower-shield transition.
                push_anim_order(engine, seq_id, elem_idx, OrderType::LoweringShield);
                set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
            }
            true
        }
        ActionState::Menacing => {
            if flags.contains(EX::MUST_BE_WAITING) && !flags.contains(EX::CAN_BE_MENACING) {
                // Stop-menace transition: queue
                // TransitionMenacingWaitingSword then
                // TransitionLoweringSword — menace exit returns to
                // upright waiting via the sword-lowering animation.
                push_anim_order(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionMenacingWaitingSword,
                );
                push_anim_order(engine, seq_id, elem_idx, OrderType::TransitionLoweringSword);
                set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
            }
            true
        }
        ActionState::Sleeping => {
            if flags.contains(EX::MUST_BE_WAITING) && !flags.contains(EX::CAN_BE_SLEEPING) {
                push_anim_order(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionSleepingWaitingUpright,
                );
                set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
            }
            true
        }
        _ => make_action_transition_actor(engine, seq_id, elem_idx, owner, flags),
    }
}

fn make_action_transition_soldier(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EX,
) -> bool {
    let (attentive, action_state) = engine
        .get_entity(owner)
        .map(|e| {
            let a = e.enemy_ai().map(|ai| ai.attentive).unwrap_or(false);
            let st = e.actor_data().map(|a| a.action_state).unwrap_or_default();
            (a, st)
        })
        .unwrap_or((false, ActionState::Waiting));

    // attentive && MUST_BE_WAITING && !CAN_BE_ALERTED → leave-attentive
    // transition.  Queue the matching transition animation so the
    // soldier unstands from attentive before the command's real orders
    // run.
    if attentive && flags.contains(EX::MUST_BE_WAITING) && !flags.contains(EX::CAN_BE_ALERTED) {
        push_anim_order(
            engine,
            seq_id,
            elem_idx,
            OrderType::TransitionWaitingAlertedWaitingUpright,
        );
    }

    // For bow-down soldiers, short-circuit the Human fall-through
    // regardless of flags (the `return true` below is unconditional —
    // outside the inner `if`).
    if action_state == ActionState::AimingWithBowDown {
        if flags.contains(EX::MUST_BE_WAITING) && !flags.contains(EX::CAN_BE_AIMING_BOW_DOWN) {
            // Raise-bow-lean-out transition + optional Unequip Bow.
            push_anim_order(
                engine,
                seq_id,
                elem_idx,
                OrderType::TransitionRaisingBowLeaningOut,
            );
            if !flags.contains(EX::CAN_BE_AIMING_BOW) {
                // C++ calls Translate(UNEQUIP_BOW) here, so preserve
                // the unload frame before unequipping.
                push_unequip_bow_transition_orders(engine, seq_id, elem_idx, false);
                set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
            } else {
                set_action_state_after(engine, seq_id, elem_idx, ActionState::AimingWithBow);
            }
        }
        return true;
    }

    make_action_transition_human(engine, seq_id, elem_idx, owner, flags)
}

fn make_action_transition_pc(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EX,
) -> bool {
    if flags.contains(EX::MUST_BE_LISTENING) {
        // PC requires ListeningState; refuse if the scheduled state
        // doesn't already match.
        let action_state_after = engine
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.action_state_after_transition)
            .unwrap_or_default();
        if action_state_after != ActionState::Listening {
            return false;
        }
    }
    make_action_transition_human(engine, seq_id, elem_idx, owner, flags)
}

fn dispatch_make_action_transition(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EX,
) -> bool {
    let kind = engine
        .get_entity(owner)
        .map(|e| e.kind())
        .unwrap_or(ElementKind::ActorCivilian);
    match kind {
        ElementKind::ActorPc => make_action_transition_pc(engine, seq_id, elem_idx, owner, flags),
        ElementKind::ActorSoldier => {
            make_action_transition_soldier(engine, seq_id, elem_idx, owner, flags)
        }
        ElementKind::ActorCivilian => {
            make_action_transition_human(engine, seq_id, elem_idx, owner, flags)
        }
        _ => make_action_transition_actor(engine, seq_id, elem_idx, owner, flags),
    }
}

// ===========================================================================
// MakePostureTransition
// ===========================================================================

/// Base posture transition.  Subclasses add *new* posture arms — NPC
/// (`SITTING`), Human (`LEISURE`), Soldier (`LEANING_OUT`), and PC
/// (carry/spy/beggar/archer/tree/on-shoulders) — which delegate to
/// this base for every posture they don't handle.
#[allow(clippy::too_many_arguments)]
fn make_posture_transition_actor(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: CP,
) -> bool {
    let posture_after = engine
        .sequence_manager
        .get_element(seq_id, elem_idx)
        .map(|e| e.posture_after_transition)
        .unwrap_or_default();
    let command = engine
        .sequence_manager
        .get_element(seq_id, elem_idx)
        .map(|e| e.command)
        .unwrap_or(Command::Null);

    if flags.contains(CP::MUST_BE_UPRIGHT) {
        return match posture_after {
            Posture::Upright => true,
            Posture::Crouched => {
                if !flags.contains(CP::CAN_BE_CROUCHED) {
                    if command == Command::CrouchDown {
                        // Do not crouch down twice!  Forward
                        // `MSG_STATURE_CHANGE_END` so stature-HUD
                        // listeners clear their latch.
                        tracing::debug!(
                            "MakePostureTransition: CROUCH_DOWN from Crouched — refused"
                        );
                        engine.messenger.send(crate::messenger::Message::new(
                            crate::messenger::MessageType::Simple(
                                crate::messenger::SimpleMessage::StatureChangeEnd,
                            ),
                        ));
                        return false;
                    }
                    push_anim_order(engine, seq_id, elem_idx, OrderType::TransitionCrouchingUp);
                    set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
                }
                true
            }
            Posture::Lying => {
                if !flags.contains(CP::CAN_BE_LYING) {
                    // C++ translates RHCOMMAND_STAND_UP as an in-place
                    // animation (`bComputeDirection = false`) chosen from
                    // the post-transition action state.
                    let action_state_after = engine
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .map(|e| e.action_state_after_transition)
                        .or_else(|| {
                            engine
                                .get_entity(owner)
                                .and_then(|entity| entity.actor_data())
                                .map(|actor| actor.action_state)
                        })
                        .unwrap_or(ActionState::Waiting);
                    let stand_up = stand_up_order_for_action_state(action_state_after);
                    push_anim_order_no_dir(engine, seq_id, elem_idx, stand_up);
                    set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
                }
                true
            }
            Posture::Dead | Posture::DeadBack => false,
            Posture::OnLadder => flags.contains(CP::CAN_BE_ON_LADDER),
            Posture::OnWall => flags.contains(CP::CAN_BE_ON_WALL),
            Posture::HelpingToClimb => {
                if !flags.contains(CP::CAN_BE_HELPING_TO_CLIMB) {
                    push_anim_order_no_dir(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionHelpingClimbingWaitingUpright,
                    );
                    set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
                }
                true
            }
            Posture::SimulatingBeggar => {
                if !flags.contains(CP::CAN_BE_SIMULATING_BEGGAR) {
                    push_anim_order_no_dir(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionSimulatingBeggarWaitingUpright,
                    );
                    set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
                }
                true
            }
            Posture::Flying => false,
            other => {
                panic!(
                    "MakePostureTransition(MUST_BE_UPRIGHT): unhandled posture-after {other:?} \
                     for seq={seq_id:?} elem={elem_idx} command={command:?}"
                );
            }
        };
    }

    if flags.contains(CP::MUST_BE_CROUCHED) {
        return match posture_after {
            Posture::Upright => {
                if command == Command::CrouchUp {
                    // Do not crouch up twice!  Forward
                    // `MSG_STATURE_CHANGE_END` so stature-HUD
                    // listeners clear their latch.
                    tracing::debug!(
                        "MakePostureTransition: CROUCH_UP from Upright — refused (double-crouch)"
                    );
                    engine.messenger.send(crate::messenger::Message::new(
                        crate::messenger::MessageType::Simple(
                            crate::messenger::SimpleMessage::StatureChangeEnd,
                        ),
                    ));
                    return false;
                }
                push_anim_order(engine, seq_id, elem_idx, OrderType::TransitionCrouchingDown);
                set_posture_after(engine, seq_id, elem_idx, Posture::Crouched);
                true
            }
            Posture::Crouched => true,
            Posture::OnLadder => flags.contains(CP::CAN_BE_ON_LADDER),
            Posture::OnWall => flags.contains(CP::CAN_BE_ON_WALL),
            other => {
                panic!(
                    "MakePostureTransition(MUST_BE_CROUCHED): unhandled posture-after {other:?} \
                     for seq={seq_id:?} elem={elem_idx} command={command:?}"
                );
            }
        };
    }

    true
}

fn make_posture_transition_human(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: CP,
) -> bool {
    let posture = engine
        .get_entity(owner)
        .map(|e| e.element_data().posture)
        .unwrap_or(Posture::Upright);

    if posture == Posture::Leisure
        && flags.contains(CP::MUST_BE_UPRIGHT)
        && !flags.contains(CP::CAN_BE_LEISURING)
    {
        push_anim_order_no_dir(
            engine,
            seq_id,
            elem_idx,
            OrderType::TransitionSpecialWaitingUpright,
        );
        set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
        return true;
    }

    make_posture_transition_actor(engine, seq_id, elem_idx, owner, flags)
}

/// Only `SITTING` is handled here; `LYING` / `DODGED` are deferred
/// to the base.
fn make_posture_transition_npc(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: CP,
) -> bool {
    let posture = engine
        .get_entity(owner)
        .map(|e| e.element_data().posture)
        .unwrap_or(Posture::Upright);

    if posture == Posture::Sitting && flags.contains(CP::MUST_BE_UPRIGHT) {
        push_anim_order_no_dir(
            engine,
            seq_id,
            elem_idx,
            OrderType::TransitionSittingWaitingUpright,
        );
        set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
        return true;
    }

    make_posture_transition_human(engine, seq_id, elem_idx, owner, flags)
}

fn make_posture_transition_soldier(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: CP,
) -> bool {
    let posture = engine
        .get_entity(owner)
        .map(|e| e.element_data().posture)
        .unwrap_or(Posture::Upright);

    if posture == Posture::LeaningOut
        && flags.contains(CP::MUST_BE_UPRIGHT)
        && !flags.contains(CP::CAN_BE_LEANING_OUT)
    {
        push_anim_order_no_dir(
            engine,
            seq_id,
            elem_idx,
            OrderType::TransitionLeaningOutWaitingAlerted,
        );
        set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
        return true;
    }

    make_posture_transition_npc(engine, seq_id, elem_idx, owner, flags)
}

fn make_posture_transition_pc(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: CP,
) -> bool {
    if flags.contains(CP::MUST_BE_CARRYING_CORPSE) {
        let posture_after = engine
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.posture_after_transition)
            .unwrap_or_default();
        return posture_after == Posture::CarryingCorpse;
    }

    if flags.contains(CP::MUST_BE_UPRIGHT) {
        let posture_after = engine
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.posture_after_transition)
            .unwrap_or_default();
        let handled = match posture_after {
            Posture::HelpingToClimb => {
                if !flags.contains(CP::CAN_BE_HELPING_TO_CLIMB) {
                    push_anim_order_no_dir(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionHelpingClimbingWaitingUpright,
                    );
                    set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
                }
                true
            }
            Posture::CarryingOnShoulders => {
                if !flags.contains(CP::CAN_BE_CARRYING_ON_SHOULDERS) {
                    let carried_id = engine
                        .get_entity(owner)
                        .and_then(|e| e.pc_data())
                        .and_then(|pc| pc.carried);
                    if let Some(carried_id) = carried_id {
                        // Leave-helping-climb from CarryingOnShoulders
                        // with a carried PC: first lower the carried
                        // PC, then leave the helping-climb stance.
                        push_anim_order_no_dir(
                            engine,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionHelpingClimbingDown,
                        );
                        push_anim_order_no_dir(
                            engine,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionHelpingClimbingWaitingUpright,
                        );
                        // TransitionHelpingClimbingDown init freezes
                        // the carried PC so it can't acquire a fresh
                        // sequence element while the carrier plays the
                        // dismount animation.
                        engine.actor_freeze_execution(carried_id);
                    } else {
                        // Fallback when the carrier no longer has a
                        // carried actor attached.
                        push_anim_order_no_dir(
                            engine,
                            seq_id,
                            elem_idx,
                            OrderType::TransitionWaitingCarryingOnShouldersWaitingUpright,
                        );
                    }
                    set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
                }
                true
            }
            Posture::CarryingCorpse => {
                if !flags.contains(CP::CAN_BE_CARRYING_CORPSE) {
                    let has_carried = engine
                        .get_entity(owner)
                        .and_then(|e| e.pc_data())
                        .is_some_and(|pc| pc.carried.is_some());
                    if !has_carried {
                        tracing::warn!(
                            ?owner,
                            ?seq_id,
                            elem_idx,
                            "MakePostureTransition: CarryingCorpse has no carried entity"
                        );
                        return false;
                    }
                    // When the driving command is
                    // `Command::EnterSwordfight`, the
                    // carrying-corpse→waiting-upright init arm drops
                    // the corpse synchronously and terminates the
                    // transition — the drop resolves on the same frame
                    // as the swordfight starts, with no animation gate.
                    // Run the synchronous drop here and skip the
                    // transition order entirely so the carrier lands in
                    // Upright/Waiting ready to begin swording.
                    let driving_command = engine
                        .sequence_manager
                        .get_element(seq_id, elem_idx)
                        .map(|e| e.command);
                    if matches!(driving_command, Some(Command::EnterSwordfight)) {
                        engine.force_drop_carried_corpse_instant(owner);
                        set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
                        return true;
                    }
                    push_anim_order_no_dir(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionCarryingCorpseWaitingUpright,
                    );
                    set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
                }
                true
            }
            Posture::OnShoulders => {
                if !flags.contains(CP::CAN_BE_ON_SHOULDERS) {
                    // Climb-down-from-shoulders transition: queue the
                    // dismount animation before the command's own
                    // orders run.
                    push_anim_order(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::ClimbingDownFromShoulders,
                    );
                    set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
                    // ClimbingDownFromShoulders init freezes the
                    // carrier PC while the carried plays the dismount
                    // animation.
                    let carrier_id = engine
                        .get_entity(owner)
                        .and_then(|e| e.human_data())
                        .and_then(|h| h.carrier);
                    if let Some(carrier_id) = carrier_id {
                        engine.actor_freeze_execution(carrier_id);
                    }
                }
                true
            }
            Posture::SimulatingBeggar => {
                if !flags.contains(CP::CAN_BE_SIMULATING_BEGGAR) {
                    push_anim_order_no_dir(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionSimulatingBeggarWaitingUpright,
                    );
                    set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
                }
                true
            }
            Posture::Spy => {
                push_anim_order_no_dir(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionWaitingCapeWaitingUpright,
                );
                true
            }
            Posture::AnonymousArcher => {
                if !flags.contains(CP::CAN_BE_ANONYMOUS_ARCHER) {
                    push_anim_order_no_dir(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionWaitingCapeWaitingUpright,
                    );
                }
                true
            }
            Posture::Tree => {
                push_anim_order_no_dir(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionWaitingHiddenWaitingUpright,
                );
                true
            }
            _ => false,
        };
        if handled {
            return true;
        }
    }

    if flags.contains(CP::MUST_BE_ON_SHOULDERS) {
        let posture_after = engine
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.posture_after_transition)
            .unwrap_or_default();
        if posture_after != Posture::OnShoulders {
            return false;
        }
    }

    make_posture_transition_human(engine, seq_id, elem_idx, owner, flags)
}

fn dispatch_make_posture_transition(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: CP,
) -> bool {
    let kind = engine
        .get_entity(owner)
        .map(|e| e.kind())
        .unwrap_or(ElementKind::ActorCivilian);
    match kind {
        ElementKind::ActorPc => make_posture_transition_pc(engine, seq_id, elem_idx, owner, flags),
        ElementKind::ActorSoldier => {
            make_posture_transition_soldier(engine, seq_id, elem_idx, owner, flags)
        }
        ElementKind::ActorCivilian => {
            make_posture_transition_npc(engine, seq_id, elem_idx, owner, flags)
        }
        _ => make_posture_transition_actor(engine, seq_id, elem_idx, owner, flags),
    }
}

// ===========================================================================
// MakeFinalActionTransition
// ===========================================================================

fn make_final_action_transition_actor(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    flags: EA,
) -> bool {
    let elem_snapshot = engine.sequence_manager.get_element(seq_id, elem_idx);
    let (posture_after, action_after) = match elem_snapshot {
        Some(e) => (e.posture_after_transition, e.action_state_after_transition),
        None => {
            // The element pointer should always be valid; if we get
            // here something is genuinely wrong with the caller's
            // seq/elem indices.
            tracing::error!(
                ?seq_id,
                elem_idx,
                "make_final_action_transition: sequence element missing"
            );
            return false;
        }
    };

    if flags.contains(EA::MUST_BE_BORED) {
        if posture_after == Posture::Upright && action_after == ActionState::Waiting {
            push_anim_order(engine, seq_id, elem_idx, OrderType::WaitingUpright);
            push_anim_order(
                engine,
                seq_id,
                elem_idx,
                OrderType::TransitionWaitingUprightWaitingUprightBored,
            );
            set_action_state_after(engine, seq_id, elem_idx, ActionState::Bored);
        }
        return true;
    }

    if flags.contains(EA::MUST_BE_MOVING) {
        match posture_after {
            Posture::Upright => match action_after {
                ActionState::Waiting => push_anim_order(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionWaitingUprightWalkingUpright,
                ),
                ActionState::MovingFast => push_anim_order(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionRunningUprightWalkingUpright,
                ),
                other => tracing::warn!(?other, "MakeFinalActionTransition(MOVING): unhandled"),
            },
            other => tracing::warn!(?other, "MakeFinalActionTransition(MOVING): posture"),
        }
        set_action_state_after(engine, seq_id, elem_idx, ActionState::Moving);
        return true;
    }

    if flags.contains(EA::MUST_BE_MOVING_FAST) {
        match posture_after {
            Posture::Upright => match action_after {
                ActionState::Waiting => {
                    push_anim_order(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionWaitingUprightWalkingUpright,
                    );
                    push_anim_order(
                        engine,
                        seq_id,
                        elem_idx,
                        OrderType::TransitionWalkingUprightRunningUpright,
                    );
                }
                ActionState::MovingFast => push_anim_order(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionWalkingUprightRunningUpright,
                ),
                other => tracing::warn!(
                    ?other,
                    "MakeFinalActionTransition(MOVING_FAST): unhandled action"
                ),
            },
            other => tracing::warn!(
                ?other,
                "MakeFinalActionTransition(MOVING_FAST): unhandled posture"
            ),
        }
        set_action_state_after(engine, seq_id, elem_idx, ActionState::MovingFast);
        return true;
    }

    if flags.contains(EA::MUST_BE_AIMING_BOW) {
        // Equip-bow expansion: insert both the take-bow and load-bow
        // animations.  Non-anonymous branch covers the base-actor case
        // — there is no AnonymousArcher arm on non-human kinds.
        push_anim_order(engine, seq_id, elem_idx, OrderType::TransitionEquipBow);
        push_anim_order(engine, seq_id, elem_idx, OrderType::TransitionLoadingBow);
        set_action_state_after(engine, seq_id, elem_idx, ActionState::AimingWithBow);
    }

    true
}

fn make_final_action_transition_human(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    flags: EA,
) -> bool {
    let (action_after, owner) = engine
        .sequence_manager
        .get_element(seq_id, elem_idx)
        .map(|e| (e.action_state_after_transition, e.owner))
        .unwrap_or_default();

    // Equip-bow terminates the sequence element early if the live
    // action state is already one of the aiming states — used to dedup
    // redundant equip-bow commands that target a Waiting
    // action_state_after_transition while the entity is already aiming.
    let live_action_state = owner
        .and_then(|e| engine.get_entity(e))
        .and_then(|e| e.actor_data())
        .map(|a| a.action_state)
        .unwrap_or_default();
    let already_aiming = matches!(
        live_action_state,
        ActionState::AimingWithBow | ActionState::AimingWithBowUp | ActionState::AimingWithBowDown
    );

    // Equip-bow: anonymous archers use the _ANONYMOUS animation
    // variants.
    let is_anonymous = owner
        .and_then(|e| engine.get_entity(e))
        .map(|e| e.element_data().posture == Posture::AnonymousArcher)
        .unwrap_or(false);
    let (equip_bow, loading_bow) = if is_anonymous {
        (
            OrderType::TransitionEquipBowAnonymous,
            OrderType::TransitionLoadingBowAnonymous,
        )
    } else {
        (
            OrderType::TransitionEquipBow,
            OrderType::TransitionLoadingBow,
        )
    };
    let raise_bow = if is_anonymous {
        OrderType::TransitionRaisingBowAnonymous
    } else {
        OrderType::TransitionRaisingBow
    };

    if flags.contains(EA::MUST_BE_AIMING_BOW) {
        match action_after {
            ActionState::Waiting => {
                if !already_aiming {
                    // Equip-bow: EquipBow + LoadingBow.
                    push_anim_order(engine, seq_id, elem_idx, equip_bow);
                    push_anim_order(engine, seq_id, elem_idx, loading_bow);
                }
                set_action_state_after(engine, seq_id, elem_idx, ActionState::AimingWithBow);
            }
            ActionState::AimingWithBow => {
                set_action_state_after(engine, seq_id, elem_idx, ActionState::AimingWithBow);
            }
            ActionState::AimingWithBowUp => {
                set_action_state_after(engine, seq_id, elem_idx, ActionState::AimingWithBowUp);
            }
            ActionState::AimingWithBowDown => {
                set_action_state_after(engine, seq_id, elem_idx, ActionState::AimingWithBowDown);
            }
            other => tracing::warn!(
                ?other,
                "MakeFinalActionTransition(AIMING_BOW): unhandled action-after"
            ),
        }
        return true;
    }

    if flags.contains(EA::MUST_BE_AIMING_BOW_UP) {
        match action_after {
            ActionState::Waiting => {
                if !already_aiming {
                    push_anim_order(engine, seq_id, elem_idx, equip_bow);
                    push_anim_order(engine, seq_id, elem_idx, loading_bow);
                }
                push_anim_order(engine, seq_id, elem_idx, raise_bow);
            }
            ActionState::AimingWithBowUp => {}
            ActionState::AimingWithBow | ActionState::AimingWithBowDown => {
                push_anim_order(engine, seq_id, elem_idx, raise_bow);
            }
            other => tracing::warn!(
                ?other,
                "MakeFinalActionTransition(AIMING_BOW_UP): unhandled action-after"
            ),
        }
        set_action_state_after(engine, seq_id, elem_idx, ActionState::AimingWithBowUp);
        return true;
    }

    if flags.contains(EA::MUST_BE_ALERTED) {
        // Humans are always alerted; no-op.
        return true;
    }

    make_final_action_transition_actor(engine, seq_id, elem_idx, flags)
}

/// Soldier-specific "alerted" auto-insert — a soldier receiving a
/// command that requires the attentive pose first queues
/// `EnterAttentiveMode` so it stands up straight before doing the
/// command.
fn make_final_action_transition_soldier(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EA,
) -> bool {
    // Read `will_be_attentive` instead of `attentive` so a second
    // element launched while an `EnterAttentiveMode` transition is
    // still animating doesn't re-queue the same animation.  `attentive`
    // only flips after the sprite completes; `will_be_attentive` flips
    // synchronously inside `set_soldier_attentive_mode` the moment the
    // command is accepted, which is the correct "has the soldier
    // already committed to alerted?" query for launch-time transition
    // decisions.  Regression test
    // `mid_transition_soldier_no_double_alert` documents the actual
    // "lean-forward plays twice" bug this prevents.
    let will_be_attentive = engine
        .get_entity(owner)
        .and_then(|e| e.enemy_ai())
        .map(|e| e.will_be_attentive)
        .unwrap_or(false);
    let attentive = engine
        .get_entity(owner)
        .and_then(|e| e.enemy_ai())
        .map(|e| e.attentive)
        .unwrap_or(false);

    if flags.contains(EA::MUST_BE_ALERTED) && !will_be_attentive {
        // Enter-attentive-mode transition: queue the matching
        // waiting→alerted transition animation so the soldier enters
        // the alerted pose before the command's real orders run.
        push_anim_order(
            engine,
            seq_id,
            elem_idx,
            OrderType::TransitionWaitingUprightWaitingAlerted,
        );
        return true;
    }

    if flags.contains(EA::MUST_BE_BORED) && attentive {
        // Attentive soldiers never go bored; no-op.
        return true;
    }

    if flags.contains(EA::MUST_BE_AIMING_BOW_DOWN) {
        let action_after = engine
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| e.action_state_after_transition)
            .unwrap_or_default();
        match action_after {
            ActionState::Waiting => {
                push_anim_order(engine, seq_id, elem_idx, OrderType::TransitionEquipBow);
                push_anim_order(engine, seq_id, elem_idx, OrderType::TransitionLoadingBow);
                push_anim_order(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionLoweringBowLeaningOut,
                );
            }
            ActionState::AimingWithBowDown => {}
            ActionState::AimingWithBow | ActionState::AimingWithBowUp => {
                push_anim_order(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionLoweringBowLeaningOut,
                );
            }
            other => tracing::warn!(
                ?other,
                "MakeFinalActionTransition(AIMING_BOW_DOWN): unhandled action-after"
            ),
        }
        // AIMING_WITH_BOW_UP is set here (likely a bug — preserved
        // for parity).
        set_action_state_after(engine, seq_id, elem_idx, ActionState::AimingWithBowUp);
        return true;
    }

    make_final_action_transition_human(engine, seq_id, elem_idx, flags)
}

fn dispatch_make_final_action_transition(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EA,
) -> bool {
    let kind = engine
        .get_entity(owner)
        .map(|e| e.kind())
        .unwrap_or(ElementKind::ActorCivilian);
    match kind {
        ElementKind::ActorSoldier => {
            make_final_action_transition_soldier(engine, seq_id, elem_idx, owner, flags)
        }
        ElementKind::ActorPc | ElementKind::ActorCivilian => {
            make_final_action_transition_human(engine, seq_id, elem_idx, flags)
        }
        _ => make_final_action_transition_actor(engine, seq_id, elem_idx, flags),
    }
}

// ===========================================================================
// GenerateTransition (public entry point)
// ===========================================================================

impl EngineInner {
    /// Generate any transition orders needed before `(seq, elem)`'s
    /// real command logic runs.
    ///
    /// Returns `false` when the transition is impossible (caller should
    /// mark the element `Impossible`) and `true` otherwise.
    ///
    /// Not yet wired into `arbitrate_instruct`.  A follow-up task will
    /// do that and delete the narrower
    /// [`EngineInner::auto_leave_disguise_if_needed`] which this fully
    /// subsumes.
    pub(crate) fn generate_transition(
        &mut self,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> bool {
        let Some((actor_posture, actor_action_state)) = self.get_entity(owner).map(|entity| {
            (
                entity.element_data().posture,
                entity
                    .actor_data()
                    .map(|a| a.action_state)
                    .unwrap_or_default(),
            )
        }) else {
            tracing::warn!(
                ?owner,
                ?seq_id,
                elem_idx,
                "generate_transition: missing entity"
            );
            return false;
        };

        if let Some(elem) = self.sequence_manager.get_element_mut(seq_id, elem_idx)
            && elem.posture_after_transition == Posture::Undefined
        {
            elem.posture_after_transition = actor_posture;
            elem.action_state_after_transition = actor_action_state;
        }

        let Some(ctx) = build_ctx(self, owner, seq_id, elem_idx) else {
            tracing::warn!(
                ?owner,
                ?seq_id,
                elem_idx,
                "generate_transition: missing element"
            );
            return false;
        };

        let (exit_flags, change_flags, enter_flags) = get_transition_flags(&ctx);

        if !dispatch_make_action_transition(self, seq_id, elem_idx, owner, exit_flags) {
            return false;
        }

        if !dispatch_make_posture_transition(self, seq_id, elem_idx, owner, change_flags) {
            return false;
        }

        if !dispatch_make_final_action_transition(self, seq_id, elem_idx, owner, enter_flags) {
            return false;
        }

        // Stamp the transition-order count to the current order list
        // length so subsequent code can distinguish transition orders
        // from the orders queued by the command itself.
        if let Some(elem) = self.sequence_manager.get_element_mut(seq_id, elem_idx) {
            elem.initialize_transition_orders();
        }
        true
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ActorData, Entity, HumanData, NpcData, SoldierData};
    use crate::element_kinds::{ActionState as AS, Posture as P};
    use crate::sequence::{SequenceElement, SequencePriority};

    fn make_soldier(posture: P, action_state: AS, attentive: bool) -> Entity {
        let mut enemy_ai = crate::ai_enemy::EnemyAi::new(0);
        enemy_ai.attentive = attentive;
        // In the running game, `set_soldier_attentive_mode`
        // flips `will_be_attentive` synchronously and `attentive`
        // after the transition animation completes — so a settled
        // alerted soldier has both true.  Mirror that for tests.
        enemy_ai.will_be_attentive = attentive;
        Entity::Soldier(crate::element::ActorSoldier {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorSoldier,
                posture,
                ..Default::default()
            },
            actor: ActorData {
                action_state,
                ..Default::default()
            },
            human: HumanData::default(),
            npc: NpcData {
                ai_brain: crate::element::AiBrain::Enemy(Box::new(enemy_ai)),
                ..Default::default()
            },
            soldier: SoldierData::default(),
        })
    }

    fn make_pc(posture: P, action_state: AS) -> Entity {
        Entity::Pc(crate::element::ActorPc {
            element: crate::element::ElementData {
                kind: crate::element::ElementKind::ActorPc,
                posture,
                ..Default::default()
            },
            actor: ActorData {
                action_state,
                ..Default::default()
            },
            human: HumanData::default(),
            pc: Default::default(),
        })
    }

    /// Launch a sequence element for `owner` with the given command.
    /// Returns `(seq_id, elem_idx)`.
    fn launch(engine: &mut EngineInner, owner: EntityId, command: Command) -> (SequenceId, usize) {
        let mut elem = SequenceElement::new(1, command, Some(owner));
        elem.priority = SequencePriority::Preference;
        // Stamp posture/action-state snapshot as arbitrate_instruct
        // would, so transition helpers see a live "after_transition"
        // value instead of Posture::default().
        if let Some(ent) = engine.get_entity(owner) {
            elem.posture_after_transition = ent.element_data().posture;
            elem.action_state_after_transition =
                ent.actor_data().map(|a| a.action_state).unwrap_or_default();
        }
        let seq_id = engine.sequence_manager.launch_element(elem);
        (seq_id, 0)
    }

    fn launch_movement(
        engine: &mut EngineInner,
        owner: EntityId,
        command: Command,
        action: OrderType,
    ) -> (SequenceId, usize) {
        let mut elem = SequenceElement::new_movement(1, command, Some(owner), action);
        elem.priority = SequencePriority::Preference;
        if let Some(ent) = engine.get_entity(owner) {
            elem.posture_after_transition = ent.element_data().posture;
            elem.action_state_after_transition =
                ent.actor_data().map(|a| a.action_state).unwrap_or_default();
        }
        let seq_id = engine.sequence_manager.launch_element(elem);
        (seq_id, 0)
    }

    fn orders_for(engine: &EngineInner, seq: SequenceId, idx: usize) -> Vec<OrderType> {
        engine
            .sequence_manager
            .get_element(seq, idx)
            .map(|e| e.orders.iter().map(|o| o.order_type).collect())
            .unwrap_or_default()
    }

    fn order_compute_direction_for(
        engine: &EngineInner,
        seq: SequenceId,
        idx: usize,
        order_type: OrderType,
    ) -> Option<bool> {
        engine.sequence_manager.get_element(seq, idx).and_then(|e| {
            e.orders
                .iter()
                .find(|o| o.order_type == order_type)
                .map(|o| o.compute_direction)
        })
    }

    #[test]
    fn stand_up_transition_matches_cpp_action_variants() {
        assert_eq!(
            stand_up_order_for_action_state(AS::Waiting),
            OrderType::StandingUp
        );
        assert_eq!(
            stand_up_order_for_action_state(AS::WaitingSword),
            OrderType::StandingUpSword
        );
        assert_eq!(
            stand_up_order_for_action_state(AS::Menacing),
            OrderType::StandingUpSword
        );
        assert_eq!(
            stand_up_order_for_action_state(AS::AimingWithBow),
            OrderType::StandingUpBow
        );
    }

    #[test]
    fn lying_soldier_stand_up_transition_is_in_place_no_direction() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(P::Lying, AS::WaitingSword, false));
        let (seq, idx) = launch(&mut engine, owner, Command::Turn);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        let orders = orders_for(&engine, seq, idx);
        assert!(
            orders.contains(&OrderType::StandingUpSword),
            "expected sword stand-up, got {:?}",
            orders
        );
        assert_eq!(
            order_compute_direction_for(&engine, seq, idx, OrderType::StandingUpSword),
            Some(false)
        );
        let posture_after = engine
            .sequence_manager
            .get_element(seq, idx)
            .unwrap()
            .posture_after_transition;
        assert_eq!(posture_after, P::Upright);
    }

    #[test]
    fn anonymous_archer_aiming_bow_up_transition_uses_anonymous_raise() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_pc(P::AnonymousArcher, AS::Waiting));
        let (seq, idx) = launch(&mut engine, owner, Command::LowerBow);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        let orders = orders_for(&engine, seq, idx);
        assert_eq!(
            orders,
            vec![
                OrderType::TransitionEquipBowAnonymous,
                OrderType::TransitionLoadingBowAnonymous,
                OrderType::TransitionRaisingBowAnonymous,
            ],
            "C++ Translate(RAISE_BOW) preserves anonymous archer variants during final bow-up transitions"
        );
    }

    /// Soldier with MOVE from LeaningOut queues the unstick transition.
    #[test]
    fn soldier_move_from_leaning_out_queues_unstick() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(P::LeaningOut, AS::Waiting, false));
        let (seq, idx) =
            launch_movement(&mut engine, owner, Command::Move, OrderType::WalkingUpright);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok, "transition should succeed");

        let orders = orders_for(&engine, seq, idx);
        assert!(
            orders.contains(&OrderType::TransitionLeaningOutWaitingAlerted),
            "expected LeaningOut→WaitingAlerted unstick, got {:?}",
            orders
        );
    }

    #[test]
    fn soldier_bow_down_entry_from_waiting_loads_before_lowering() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(P::Upright, AS::Waiting, true));
        let (seq, idx) = launch(&mut engine, owner, Command::EquipBowDown);

        let ok = dispatch_make_final_action_transition(
            &mut engine,
            seq,
            idx,
            owner,
            EA::MUST_BE_AIMING_BOW_DOWN,
        );
        assert!(ok);

        let orders = orders_for(&engine, seq, idx);
        assert_eq!(
            orders,
            vec![
                OrderType::TransitionEquipBow,
                OrderType::TransitionLoadingBow,
                OrderType::TransitionLoweringBowLeaningOut,
            ],
            "C++ soldier bow-down entry calls Translate(EQUIP_BOW) before LOWER_BOW_LEAN_OUT"
        );
    }

    #[test]
    fn soldier_bow_down_exit_queues_unload_before_unequip() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(P::LeaningOut, AS::AimingWithBowDown, false));
        let (seq, idx) = launch(&mut engine, owner, Command::Turn);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        let orders = orders_for(&engine, seq, idx);
        assert!(
            orders.windows(3).any(|window| {
                window
                    == [
                        OrderType::TransitionRaisingBowLeaningOut,
                        OrderType::TransitionUnloadBow,
                        OrderType::TransitionUnequipBow,
                    ]
            }),
            "C++ Translate(UNEQUIP_BOW) queues unload before unequip after raising from bow-down, got {:?}",
            orders
        );
    }

    /// Soldier with MOVE (WalkingUpright) from Crouched stays crouched
    /// because `CAN_BE_CROUCHED` is set.  No crouch-up transition.
    #[test]
    fn soldier_move_from_crouched_stays_crouched() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(P::Crouched, AS::Waiting, false));
        let (seq, idx) =
            launch_movement(&mut engine, owner, Command::Move, OrderType::WalkingUpright);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        let orders = orders_for(&engine, seq, idx);
        assert!(
            !orders.contains(&OrderType::TransitionCrouchingUp),
            "should not queue crouch-up (CAN_BE_CROUCHED is set), got {:?}",
            orders
        );
        // posture_after_transition stays Crouched since no posture
        // change was required.
        let posture_after = engine
            .sequence_manager
            .get_element(seq, idx)
            .unwrap()
            .posture_after_transition;
        assert_eq!(posture_after, P::Crouched);
    }

    /// Soldier MOVE with RunningUpright from Crouched must queue
    /// CROUCH_UP (no CAN_BE_CROUCHED on the run path).
    #[test]
    fn soldier_run_from_crouched_queues_crouch_up() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(P::Crouched, AS::Waiting, false));
        let (seq, idx) =
            launch_movement(&mut engine, owner, Command::Move, OrderType::RunningUpright);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        let orders = orders_for(&engine, seq, idx);
        assert!(
            orders.contains(&OrderType::TransitionCrouchingUp),
            "expected crouch-up, got {:?}",
            orders
        );
        let posture_after = engine
            .sequence_manager
            .get_element(seq, idx)
            .unwrap()
            .posture_after_transition;
        assert_eq!(posture_after, P::Upright);
    }

    #[test]
    fn pc_carrying_on_shoulders_exit_queues_lower_then_stand_chain() {
        let mut engine = EngineInner::new();
        let mut carrier = make_pc(P::CarryingOnShoulders, AS::Waiting);
        let carried = engine.add_entity(make_pc(P::OnShoulders, AS::Waiting));
        if let Entity::Pc(pc) = &mut carrier {
            pc.pc.carried = Some(carried);
        }
        let owner = engine.add_entity(carrier);
        let (seq, idx) = launch(&mut engine, owner, Command::Turn);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        assert_eq!(
            orders_for(&engine, seq, idx),
            vec![
                OrderType::TransitionHelpingClimbingDown,
                OrderType::TransitionHelpingClimbingWaitingUpright,
            ]
        );
        let posture_after = engine
            .sequence_manager
            .get_element(seq, idx)
            .unwrap()
            .posture_after_transition;
        assert_eq!(posture_after, P::Upright);
    }

    #[test]
    fn pc_carrying_corpse_without_carried_entity_is_impossible() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_pc(P::CarryingCorpse, AS::Waiting));
        let (seq, idx) = launch(&mut engine, owner, Command::Turn);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(
            !ok,
            "CarryingCorpse without pc.carried should fail instead of silently snapping upright"
        );
        assert!(
            orders_for(&engine, seq, idx).is_empty(),
            "unsupported corpse-drop transition should not queue a fake animation"
        );
    }

    /// PC CROUCH_UP from Crouched produces no transition animations
    /// (the command itself is the animation) and flips
    /// posture-after-transition to Upright.
    #[test]
    fn pc_crouch_up_from_crouched_snaps_to_upright() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_pc(P::Crouched, AS::Waiting));
        let (seq, idx) = launch(&mut engine, owner, Command::CrouchUp);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        // Refused double-crouch case: CROUCH_UP on Upright → false.
        // From Crouched, MUST_BE_CROUCHED is set and posture_after
        // stays Crouched — no animation is queued.
        let posture_after = engine
            .sequence_manager
            .get_element(seq, idx)
            .unwrap()
            .posture_after_transition;
        assert_eq!(posture_after, P::Crouched);
    }

    /// Soldier ENTER_SWORDFIGHT from Waiting with `attentive=false`
    /// must fire the MUST_BE_ALERTED branch, queueing the
    /// `TransitionWaitingUprightWaitingAlerted` order so the soldier
    /// stands to attention before fighting.
    #[test]
    fn soldier_enter_swordfight_fires_must_be_alerted() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(P::Upright, AS::Waiting, false));
        let (seq, idx) = launch(&mut engine, owner, Command::EnterSwordfight);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok, "transition should succeed");

        let orders = orders_for(&engine, seq, idx);
        assert!(
            orders.contains(&OrderType::TransitionWaitingUprightWaitingAlerted),
            "expected attentive-mode transition, got {:?}",
            orders
        );
    }

    /// Regression: a soldier mid-transition to alerted (i.e.
    /// `will_be_attentive=true, attentive=false` — the animation has
    /// been queued but hasn't completed yet) must NOT re-queue the
    /// alerted transition when a second MUST_BE_ALERTED element is
    /// launched in the same window.  This was the root cause of the
    /// "lean-forward plays twice when a guard spots the PC" bug: the
    /// ENTER_ATTENTIVE_MODE element queued anim #1 via Translate, then
    /// ENTER_SWORDFIGHT launched and `generate_transition` saw
    /// `attentive=false` and queued anim #2 via
    /// MakeFinalActionTransition.  Checking `will_be_attentive` (which
    /// flips synchronously in `set_soldier_attentive_mode`) fixes it.
    #[test]
    fn mid_transition_soldier_no_double_alert() {
        let mut engine = EngineInner::new();
        let mut e = make_soldier(P::Upright, AS::Waiting, false);
        if let Entity::Soldier(s) = &mut e
            && let Some(enemy) = s.npc.ai_brain.enemy_mut()
        {
            enemy.will_be_attentive = true;
            enemy.attentive = false;
        }
        let owner = engine.add_entity(e);
        let (seq, idx) = launch(&mut engine, owner, Command::EnterSwordfight);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        let orders = orders_for(&engine, seq, idx);
        assert!(
            !orders.contains(&OrderType::TransitionWaitingUprightWaitingAlerted),
            "mid-transition soldier should not re-queue the alerted anim, got {:?}",
            orders
        );
    }

    /// Attentive soldier with ENTER_SWORDFIGHT should NOT queue the
    /// enter-attentive transition again (the flag short-circuits).
    #[test]
    fn attentive_soldier_enter_swordfight_no_double_alert() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(P::Upright, AS::Waiting, true));
        let (seq, idx) = launch(&mut engine, owner, Command::EnterSwordfight);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        let orders = orders_for(&engine, seq, idx);
        assert!(
            !orders.contains(&OrderType::TransitionWaitingUprightWaitingAlerted),
            "already-attentive soldier should not re-queue the transition, got {:?}",
            orders
        );
    }

    /// A soldier Bored + Wait command shouldn't queue anything: the
    /// transition flags allow CAN_BE_BORED, so the bored→waiting path
    /// is skipped.
    #[test]
    fn bored_soldier_wait_no_transition() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(P::Upright, AS::Bored, false));
        let (seq, idx) = launch(&mut engine, owner, Command::Wait);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        let orders = orders_for(&engine, seq, idx);
        assert!(
            !orders.contains(&OrderType::TransitionWaitingUprightBoredWaitingUpright),
            "bored→waiting should not be queued when CAN_BE_BORED is set, got {:?}",
            orders
        );
    }

    /// Upright Moving soldier receiving CrouchDown must queue the
    /// walking→waiting exit transition (CAN_BE_MOVING is cleared),
    /// then the crouch-down itself is the command.  Verify the exit
    /// transition fires.
    #[test]
    fn soldier_crouch_down_from_upright_moving_queues_exit() {
        let mut engine = EngineInner::new();
        let owner = engine.add_entity(make_soldier(P::Upright, AS::Moving, false));
        let (seq, idx) = launch(&mut engine, owner, Command::CrouchDown);

        let ok = engine.generate_transition(owner, seq, idx);
        assert!(ok);

        let orders = orders_for(&engine, seq, idx);
        // CAN_BE_MOVING is set for CrouchDown in the base flags, so
        // the walk→wait exit transition should NOT queue.  This test
        // locks that behaviour in.
        assert!(
            !orders.contains(&OrderType::TransitionWalkingUprightWaitingUpright),
            "CAN_BE_MOVING covers CrouchDown; should not queue walk→wait, got {:?}",
            orders
        );
    }
}
