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
//! Explicit `match`es on `ElementKind` select shared actor/human behavior
//! and the specialized NPC, PC, and soldier responses.
//!
//! # Scope
//!
//! [`EngineInner::generate_transition`] is the public entry point and is
//! wired into the live instruct/sequence pipeline: it runs from sequence
//! arbitration and phase handling as well as script synchronisation, so
//! transitions here are exercised in normal gameplay in addition to this
//! module's unit tests.

use crate::element::{ActionState, Command, EntityId, Posture};
use crate::element_kinds::{
    ChangePostureFlags as CP, ElementKind, EnterActionStateFlags as EA, ExitActionStateFlags as EX,
};
use crate::order::OrderType;
use crate::sequence::{SequenceElementData, SequenceId};
use serde::{Deserialize, Serialize};

use super::{EngineInner, LevelAssets};

/// Invalid transition state is not a gameplay refusal (for example trying to
/// crouch an actor that is already crouched). Keep that distinction until the
/// legacy sequence pipeline's bool boundary, where invalid state is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
enum TransitionError {
    #[error("transition owner {0:?} is missing")]
    MissingOwner(EntityId),
    #[error("transition element {seq_id:?}/{elem_idx} is missing")]
    MissingElement { seq_id: SequenceId, elem_idx: usize },
    #[error("transition element owner {actual:?} does not match {expected:?}")]
    OwnerMismatch {
        expected: EntityId,
        actual: Option<EntityId>,
    },
}

/// Identity only: never caches posture, action state, or other observations
/// across callbacks. Revalidate against live state after each mutation stage.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct TransitionTarget {
    owner: EntityId,
    seq_id: SequenceId,
    elem_idx: usize,
}

impl TransitionTarget {
    fn validate(self, engine: &EngineInner) -> Result<(), TransitionError> {
        engine
            .get_entity(self.owner)
            .ok_or(TransitionError::MissingOwner(self.owner))?;
        let elem = engine
            .orders
            .sequence_manager
            .get_element(self.seq_id, self.elem_idx)
            .ok_or(TransitionError::MissingElement {
                seq_id: self.seq_id,
                elem_idx: self.elem_idx,
            })?;
        if elem.owner != Some(self.owner) {
            return Err(TransitionError::OwnerMismatch {
                expected: self.owner,
                actual: elem.owner,
            });
        }
        Ok(())
    }

    fn stage(
        self,
        engine: &mut EngineInner,
        run: impl FnOnce(&mut EngineInner) -> bool,
    ) -> Result<bool, TransitionError> {
        self.validate(engine)?;
        let allowed = run(engine);
        // Even a refusing callback may have invalidated the target. Do not
        // silently classify that as an ordinary impossible command.
        self.validate(engine)?;
        Ok(allowed)
    }
}

fn transition_element(
    engine: &EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
) -> &crate::sequence::SequenceElement {
    engine
        .orders
        .sequence_manager
        .get_element(seq_id, elem_idx)
        .expect("validated transition element must remain present within a stage")
}

fn transition_owner(engine: &EngineInner, owner: EntityId) -> &crate::element::Entity {
    engine
        .get_entity(owner)
        .expect("validated transition owner must remain present within a stage")
}

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
// Transition-flag calculation
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
                    "transition generation: movement command with no movement_action; using no flags"
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
                    tracing::warn!(?other, "transition generation: unhandled movement action");
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
                Some(DT::LiftHigh) | Some(DT::LiftLow) => {
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
                Some(DT::LiftHighCrenel) => {
                    // Original-game actor transition flags omit
                    // DOOR_LIFT_HIGH_CRENEL from its PassDoor switch even
                    // though Translate handles the door as a high lift. Keep
                    // every transition flag empty here: the translated wall
                    // choreography owns its own crouch/climb animations.
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
                "transition generation: unhandled command — no flags set"
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
        EnterHelpingClimb | EnterBeggar | EnterListen | EnterCloak => {
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
        | UseLever | UnlockDoor | HitTarget | HandleTarget | TakeTarget | Pay | TieCmd | Untie
        | StrangleCmd | WhistleCmd => (EX::MUST_BE_WAITING, CP::MUST_BE_UPRIGHT, EA::empty()),
        HideBehindShield => (
            EX::MUST_BE_WAITING,
            CP::MUST_BE_UPRIGHT | CP::CAN_BE_CROUCHED,
            EA::empty(),
        ),
        // Rust-authored AI-controlled heroes reuse the enemy NPC overview AI.
        // Original never dispatched these commands to a PC, so apply the
        // soldier transition contract explicitly for that extension.
        LookLeft | LookRight => (
            EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED,
            CP::MUST_BE_UPRIGHT,
            EA::empty(),
        ),
        LeanOut => (
            EX::MUST_BE_WAITING | EX::CAN_BE_ALERTED,
            CP::MUST_BE_UPRIGHT | CP::CAN_BE_LEANING_OUT,
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

/// Top-level transition-flag dispatch keyed on [`ElementKind`].
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
    let id = engine.orders.allocate_order_id();
    let order = crate::order::Order::new(anim, 0.0, 0.0, id);
    engine
        .orders
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
    let id = engine.orders.allocate_order_id();
    let mut order = crate::order::Order::new(anim, 0.0, 0.0, id);
    order.compute_direction = false;
    engine
        .orders
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
    engine
        .orders
        .sequence_manager
        .get_element_mut(seq_id, elem_idx)
        .expect("validated transition element must remain present within a stage")
        .posture_after_transition = p;
}

fn set_action_state_after(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    a: ActionState,
) {
    engine
        .orders
        .sequence_manager
        .get_element_mut(seq_id, elem_idx)
        .expect("validated transition element must remain present within a stage")
        .action_state_after_transition = a;
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
    let elem = engine
        .orders
        .sequence_manager
        .get_element(seq_id, elem_idx)?;

    let movement_action = match &elem.data {
        SequenceElementData::Movement { action, .. } => Some(*action),
        _ => None,
    };
    let gate_id = match &elem.data {
        SequenceElementData::Movement { gate_id, .. } => *gate_id,
        _ => None,
    };

    // Interaction antagonist: landed net-element detection for player `TAKE`.
    // Inventory bonus nets are bonus elements in the original game and fall
    // through the ordinary-object branch, which permits a crouched pickup.
    let antagonist_is_net = match &elem.data {
        SequenceElementData::Interaction {
            antagonist: Some(antagonist),
        } => engine
            .get_entity(*antagonist)
            .is_some_and(|entity| matches!(entity, crate::element::Entity::Net(_))),
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
            let door = engine.scripts.mission.as_ref().and_then(|_| {
                engine
                    .script_domains
                    .interactables
                    .doors
                    .get(usize::from(idx))
            });
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
        force_crouched,
        door_type,
        door_lift_kind,
        antagonist_is_net,
    })
}

// ===========================================================================
// Action transitions
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
    let posture = entity.element_data().posture();
    let Some(actor) = entity.actor_data() else {
        tracing::warn!(?owner, "make_action_transition: owner has no actor data");
        return false;
    };
    let action_state = actor.action_state;

    if action_state == ActionState::Waiting {
        return true;
    }

    let command = transition_element(engine, seq_id, elem_idx).command;
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
                        engine
                            .orders
                            .sequence_manager
                            .element_terminated(seq_id, elem_idx);
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
                    "upright action transition: unhandled action state; ignoring"
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
                tracing::warn!("crouched action transition: MovingFast not expected; skipping");
            }
            other => {
                tracing::warn!(?other, "crouched action transition: unhandled action state");
            }
        },
        _ => {}
    }

    true
}

fn make_action_transition_human(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EX,
) -> bool {
    let Some(entity) = engine.get_entity(owner) else {
        tracing::warn!(?owner, "make_action_transition_human: entity gone");
        return false;
    };
    let Some(actor) = entity.actor_data() else {
        tracing::warn!(
            ?owner,
            "make_action_transition_human: owner has no actor data"
        );
        return false;
    };
    let action_state = actor.action_state;
    let posture = entity.element_data().posture();
    let is_anonymous_archer = posture == Posture::AnonymousArcher;

    match action_state {
        ActionState::AimingWithBow => {
            if flags.contains(EX::MUST_BE_WAITING) && !flags.contains(EX::CAN_BE_AIMING_BOW) {
                // Original-game unequip-bow translation: unload then unequip,
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
                    // Original-game unequip-bow translation: unload then unequip.
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
                // Original reaches this through
                // Translate the existing element into a quit-swordfight command,
                // whose translation leaves swordfight immediately after
                // queuing the lowering animation. Relationship removal and
                // AI callbacks therefore happen at transition-generation
                // time, not when the animation eventually starts.
                engine.quit_swordfight(sim, assets, owner);
                // Return directly to TransitionTarget::stage: it revalidates
                // the live target after these synchronous AI callbacks, before
                // the posture stage reads it again.
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
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EX,
) -> bool {
    let entity = transition_owner(engine, owner);
    let attentive = entity.enemy_ai().is_some_and(|ai| ai.attentive);
    let action_state = entity
        .actor_data()
        .expect("soldier has actor data")
        .action_state;

    // attentive && MUST_BE_WAITING && !CAN_BE_ALERTED → leave-attentive
    // transition.  This routes through the same translate arm as an
    // explicit LeaveAttentiveMode command: the transition animation only
    // plays when the element's stamped posture is upright.  Any other
    // posture (e.g. on a ladder mid door-pass) terminates the element
    // outright and silently drops the attentive pose instead of queueing
    // an animation that would fire much later.
    if attentive && flags.contains(EX::MUST_BE_WAITING) && !flags.contains(EX::CAN_BE_ALERTED) {
        let (posture_after, command) = engine
            .orders
            .sequence_manager
            .get_element(seq_id, elem_idx)
            .map(|e| (e.posture_after_transition, Some(e.command)))
            .expect("validated transition element must remain present within a stage");
        tracing::trace!(
            ?owner,
            ?seq_id,
            elem_idx,
            ?command,
            ?flags,
            ?posture_after,
            "soldier leave-attentive exit transition"
        );
        if posture_after == crate::element::Posture::Upright {
            push_anim_order_no_dir(
                engine,
                seq_id,
                elem_idx,
                OrderType::TransitionWaitingAlertedWaitingUpright,
            );
        } else {
            engine
                .orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
            if let Some(enemy) = engine
                .get_entity_mut(owner)
                .and_then(crate::element::Entity::enemy_ai_mut)
            {
                enemy.attentive = false;
            }
        }
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
                // The original game translates unequip-bow here, so preserve
                // the unload frame before unequipping.
                push_unequip_bow_transition_orders(engine, seq_id, elem_idx, false);
                set_action_state_after(engine, seq_id, elem_idx, ActionState::Waiting);
            } else {
                set_action_state_after(engine, seq_id, elem_idx, ActionState::AimingWithBow);
            }
        }
        return true;
    }

    make_action_transition_human(engine, sim, assets, seq_id, elem_idx, owner, flags)
}

fn make_action_transition_pc(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EX,
) -> bool {
    if flags.contains(EX::MUST_BE_LISTENING) {
        // PC requires ListeningState; refuse if the scheduled state
        // doesn't already match.
        let action_state_after =
            transition_element(engine, seq_id, elem_idx).action_state_after_transition;
        if action_state_after != ActionState::Listening {
            return false;
        }
    }
    make_action_transition_human(engine, sim, assets, seq_id, elem_idx, owner, flags)
}

fn dispatch_make_action_transition(
    engine: &mut EngineInner,
    sim: &crate::sim_rng::SimulationContext,
    assets: &LevelAssets,
    seq_id: SequenceId,
    elem_idx: usize,
    owner: EntityId,
    flags: EX,
) -> bool {
    let kind = transition_owner(engine, owner).kind();
    match kind {
        ElementKind::ActorPc => {
            make_action_transition_pc(engine, sim, assets, seq_id, elem_idx, owner, flags)
        }
        ElementKind::ActorSoldier => {
            make_action_transition_soldier(engine, sim, assets, seq_id, elem_idx, owner, flags)
        }
        ElementKind::ActorCivilian => {
            make_action_transition_human(engine, sim, assets, seq_id, elem_idx, owner, flags)
        }
        _ => make_action_transition_actor(engine, seq_id, elem_idx, owner, flags),
    }
}

// ===========================================================================
// Posture transitions
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
    _owner: EntityId,
    flags: CP,
) -> bool {
    let posture_after = transition_element(engine, seq_id, elem_idx).posture_after_transition;
    let command = transition_element(engine, seq_id, elem_idx).command;

    if flags.contains(CP::MUST_BE_UPRIGHT) {
        return match posture_after {
            Posture::Upright => true,
            Posture::Crouched => {
                if !flags.contains(CP::CAN_BE_CROUCHED) {
                    if command == Command::CrouchDown {
                        // Do not crouch down twice!  Forward
                        // `MSG_STATURE_CHANGE_END` so stature-HUD
                        // listeners clear their latch.
                        tracing::debug!("posture transition: CROUCH_DOWN from Crouched — refused");
                        engine.orders.messenger.send(crate::messenger::Message::new(
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
                    // The original game translates stand-up as an in-place
                    // animation (direction computation disabled) chosen from
                    // the post-transition action state.
                    let action_state_after =
                        transition_element(engine, seq_id, elem_idx).action_state_after_transition;
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
                // Shipping saves can
                // legitimately retain a command on a tied actor, so parity
                // with the release game requires rejecting that command
                // rather than aborting the whole simulation.
                tracing::debug!(
                    ?other,
                    ?seq_id,
                    elem_idx,
                    ?command,
                    "upright posture requirement: rejecting unhandled posture"
                );
                false
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
                        "posture transition: CROUCH_UP from Upright — refused (double-crouch)"
                    );
                    engine.orders.messenger.send(crate::messenger::Message::new(
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
                tracing::debug!(
                    ?other,
                    ?seq_id,
                    elem_idx,
                    ?command,
                    "crouched posture requirement: rejecting unhandled posture"
                );
                false
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
    let posture = transition_owner(engine, owner).element_data().posture();

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
    let posture = transition_owner(engine, owner).element_data().posture();

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
    let posture = transition_owner(engine, owner).element_data().posture();

    if posture == Posture::LeaningOut {
        if flags.contains(CP::MUST_BE_UPRIGHT) && !flags.contains(CP::CAN_BE_LEANING_OUT) {
            push_anim_order_no_dir(
                engine,
                seq_id,
                elem_idx,
                OrderType::TransitionLeaningOutWaitingAlerted,
            );
            set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
        }

        // Original-game soldier posture transitions own the complete
        // LEANING_OUT arm.  In particular LEAN_OUT itself carries both
        // MUST_BE_UPRIGHT and CAN_BE_LEANING_OUT, so delegating that case
        // to the base actor would feed a soldier-only posture into the
        // base switch (and hit its unhandled-state assertion).
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
        let posture_after = transition_element(engine, seq_id, elem_idx).posture_after_transition;
        return posture_after == Posture::CarryingCorpse;
    }

    if flags.contains(CP::MUST_BE_UPRIGHT) {
        let posture_after = transition_element(engine, seq_id, elem_idx).posture_after_transition;
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
                            "posture transition: CarryingCorpse has no carried entity"
                        );
                        return false;
                    }
                    // Original always translates this posture change into the
                    // corpse-exit order first.  ENTER_SWORDFIGHT is special
                    // only when that order reaches its first Execute: the PC
                    // arm drops synchronously and returns TERMINATED there
                    // during the action. Doing the drop while
                    // generating transitions skips the registered animation
                    // boundary and advances the owner a manager phase early.
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
            Posture::Cloaked => {
                push_anim_order_no_dir(
                    engine,
                    seq_id,
                    elem_idx,
                    OrderType::TransitionWaitingCapeWaitingUpright,
                );
                set_posture_after(engine, seq_id, elem_idx, Posture::Upright);
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
        let posture_after = transition_element(engine, seq_id, elem_idx).posture_after_transition;
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
    let kind = transition_owner(engine, owner).kind();
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
// Final action transitions
// ===========================================================================

fn make_final_action_transition_actor(
    engine: &mut EngineInner,
    seq_id: SequenceId,
    elem_idx: usize,
    flags: EA,
) -> bool {
    let elem = transition_element(engine, seq_id, elem_idx);
    let (posture_after, action_after) = (
        elem.posture_after_transition,
        elem.action_state_after_transition,
    );

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
                other => tracing::warn!(?other, "final moving transition: unhandled action"),
            },
            other => tracing::warn!(?other, "final moving transition: unhandled posture"),
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
                other => tracing::warn!(?other, "final fast-movement transition: unhandled action"),
            },
            other => tracing::warn!(?other, "final fast-movement transition: unhandled posture"),
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
        .orders
        .sequence_manager
        .get_element(seq_id, elem_idx)
        .map(|e| (e.action_state_after_transition, e.owner))
        .expect("validated transition element must remain present within a stage");

    // Equip-bow terminates the sequence element early if the live
    // action state is already one of the aiming states — used to dedup
    // redundant equip-bow commands that target a Waiting
    // action_state_after_transition while the entity is already aiming.
    let entity = transition_owner(
        engine,
        owner.expect("human transition element requires an owner"),
    );
    let live_action_state = entity
        .actor_data()
        .expect("human transition owner requires actor data")
        .action_state;
    let already_aiming = matches!(
        live_action_state,
        ActionState::AimingWithBow | ActionState::AimingWithBowUp | ActionState::AimingWithBowDown
    );

    // Equip-bow: anonymous archers use the _ANONYMOUS animation
    // variants.
    let is_anonymous = entity.element_data().posture() == Posture::AnonymousArcher;
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
                "final bow-aiming transition: unhandled subsequent action"
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
                "final raised-bow transition: unhandled subsequent action"
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
    // The original game tests the soldier's current attentive pose here, not
    // the desired attentiveness target. The distinction is observable
    // when a queued LeaveAttentiveMode is translated after its preceding
    // enter transition has completed: will-be is already false, while the
    // actor is actually attentive and needs no redundant enter animation.
    let attentive = transition_owner(engine, owner)
        .enemy_ai()
        .is_some_and(|enemy| enemy.attentive);

    if flags.contains(EA::MUST_BE_ALERTED) && !attentive {
        // Enter-attentive-mode transition, routed through the same
        // translate arm as an explicit EnterAttentiveMode command: the
        // waiting→alerted transition animation only plays when the
        // element's stamped posture is upright.  Any other posture
        // terminates the element and flips the attentive pose silently.
        let posture_after = transition_element(engine, seq_id, elem_idx).posture_after_transition;
        if posture_after == crate::element::Posture::Upright {
            push_anim_order_no_dir(
                engine,
                seq_id,
                elem_idx,
                OrderType::TransitionWaitingUprightWaitingAlerted,
            );
        } else {
            engine
                .orders
                .sequence_manager
                .element_terminated(seq_id, elem_idx);
            if let Some(enemy) = engine
                .get_entity_mut(owner)
                .and_then(crate::element::Entity::enemy_ai_mut)
            {
                enemy.attentive = true;
            }
        }
        return true;
    }

    if flags.contains(EA::MUST_BE_BORED) && attentive {
        // Attentive soldiers never go bored; no-op.
        return true;
    }

    if flags.contains(EA::MUST_BE_AIMING_BOW_DOWN) {
        let action_after =
            transition_element(engine, seq_id, elem_idx).action_state_after_transition;
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
                "final lowered-bow transition: unhandled subsequent action"
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
    let kind = transition_owner(engine, owner).kind();
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
// Transition generation (public entry point)
// ===========================================================================

impl EngineInner {
    /// Generate any transition orders needed before `(seq, elem)`'s
    /// real command logic runs.
    ///
    /// Returns `false` when the transition is impossible (caller should
    /// mark the element `Impossible`) and `true` otherwise. Invalid state is
    /// reported separately before adapting to the sequence pipeline's bool API.
    pub(crate) fn generate_transition(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> bool {
        match self.try_generate_transition(sim, assets, owner, seq_id, elem_idx) {
            Ok(allowed) => allowed,
            Err(error) => {
                tracing::error!(?owner, ?seq_id, elem_idx, %error, "invalid transition target");
                false
            }
        }
    }

    fn try_generate_transition(
        &mut self,
        sim: &crate::sim_rng::SimulationContext,
        assets: &LevelAssets,
        owner: EntityId,
        seq_id: SequenceId,
        elem_idx: usize,
    ) -> Result<bool, TransitionError> {
        let target = TransitionTarget {
            owner,
            seq_id,
            elem_idx,
        };
        target.validate(self)?;
        let entity = transition_owner(self, owner);
        let actor_posture = entity.element_data().posture();
        // Non-actor elements can carry commands too, but have no action-state
        // machine. Waiting is their explicit initial state, not a missing-owner
        // fallback; actor-specific dispatch below requires actual actor data.
        let actor_action_state = entity
            .actor_data()
            .map_or(ActionState::Waiting, |a| a.action_state);
        let elem = self
            .orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .expect("target was just validated");
        if elem.posture_after_transition == Posture::Undefined {
            elem.posture_after_transition = actor_posture;
            elem.action_state_after_transition = actor_action_state;
        }

        let ctx = build_ctx(self, owner, seq_id, elem_idx).expect("target was just validated");

        let (exit_flags, mut change_flags, enter_flags) = get_transition_flags(&ctx);
        // The reusable cape has only a stationary idle row. Any actor action
        // other than waiting or the cloak transition itself first plays the
        // shipped cape-to-upright strip and reveals the wearer. This includes
        // damage and hostile interactions whose Original transition flags are
        // deliberately empty.
        if actor_posture == Posture::Cloaked && crate::cloak::command_breaks_cloak(ctx.command) {
            change_flags |= CP::MUST_BE_UPRIGHT;
        }

        if !target.stage(self, |engine| {
            dispatch_make_action_transition(
                engine, sim, assets, seq_id, elem_idx, owner, exit_flags,
            )
        })? {
            return Ok(false);
        }

        if !target.stage(self, |engine| {
            dispatch_make_posture_transition(engine, seq_id, elem_idx, owner, change_flags)
        })? {
            return Ok(false);
        }

        if !target.stage(self, |engine| {
            dispatch_make_final_action_transition(engine, seq_id, elem_idx, owner, enter_flags)
        })? {
            return Ok(false);
        }

        // Stamp the transition-order count to the current order list
        // length so subsequent code can distinguish transition orders
        // from the orders queued by the command itself.
        self.orders
            .sequence_manager
            .get_element_mut(seq_id, elem_idx)
            .expect("target was just revalidated")
            .initialize_transition_orders();
        Ok(true)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
#[path = "transitions/tests.rs"]
mod tests;
