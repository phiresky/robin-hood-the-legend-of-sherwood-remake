# Per-arm `Turn()` semantics

Every actor animation arm decides for itself whether the body keeps rotating
toward `direction_goal` while the arm's sprite action plays. There is no rule
that can be derived from the animation family: neighbouring arms of the same
family routinely disagree, and several arms are overridden per actor subclass
with different answers. This document is the authoritative table.

Getting an arm wrong shows up in parity sweeps as `state:direction` off by ±1
with `state:sprite_row` following it, usually with `state:direction_goal`
matching — one rotation step landing on a different frame.

## Where the step is applied

Turning is not applied in one place; it follows the dispatch category an order
is bound to in the actor-execute catalog (`engine/tick.rs`):

| Category | Turn site |
| --- | --- |
| `Movement` | `engine/movement.rs` — the ordinary motion branch turns unconditionally before `perform_motion`; the fast climb loop turns again before its second call |
| `GenericAnimation` | `engine/animation.rs` — the `needs_turn` set, gated by `turn_arm_condition_holds` |
| `Ability` | `abilities.rs` — the per-`AbilityKind` `turning` set in `tick_ability`, plus the inline `Listen` block |
| `Bow` | `bow_shot.rs` — `is_shoot_order` turns and freezes the first frame |
| `Melee`, `Beggar`, `WaitingSword` | their own dispatch modules |

## Arms that turn

Ordering matters: an arm that turns *after* its sprite call stamps the row of
the pre-turn direction, which `actor_action_row` handles.

### Movement (all turn, no per-arm exceptions)

Walk/run/crouch transitions, `WalkingUpright`, `RunningUpright`,
`WalkingStairs`, `RunningStairs`, `WalkingCrouched`, `WalkingWithCorpse`,
`WalkingCarryingOnShoulders`, `WalkingWithSword` / `RunningWithSword`, the shield
and sword strafe/backward walks, every wall and ladder climb and its entry/exit
transition (including the `*Alerted` and `*Fast` variants), the jump take-off /
landing transitions, `JumpingDown`, and `TransitionHelpingClimbingDown`.

`PassingDoor`, `RefreshingSeek`, `WaitingFreeLift` and `Freezing` do not reach
the motion path's turn.

### Generic animation

Unconditional: `TransitionLoweringSword`, `ParryingSword`, the four
`Striking*Smalltalk` and four `Parrying*Smalltalk` arms, `StrikingDownSword`,
`FallingLadderWall`, `RaisingShield`, `WaitingShield`, `Rolling`, `TakingNet`,
`Taking`, `TakingCrouched`, `TakingTarget`, `DroppingAmmo`,
`DroppingAmmoCrouched`, `DroppingAle`, `DroppingAleCrouched`, `UsingLever`,
`DrinkingAle`, `HittingTarget`, `HandlingTarget`, `UnlockingDoor`,
`UnlockingTrap`, `Searching`, `SearchingCrouched`, `WaitingHelpingClimbing`,
`WaitingCarryingOnShoulders`,
`TransitionWaitingCarryingOnShouldersWaitingUpright`, `FallingShoulders`,
`TransitionCrouchingUp`, `TransitionCrouchingDown`, `GettingFreeFromWasp`,
`Pointing`, and the PC beggar transition/idle family.

Conditional (`turn_arm_condition_holds`):

| Arm | Condition |
| --- | --- |
| `TransitionRaisingSword` | always for human/PC; soldiers only with an order antagonist |
| `StandingUpSword` | human/PC only — the soldier override replays the sprite without turning |
| `ExtractingArrowSword` | only while swordfighting |

Ordering and multiplicity exceptions:

* `StandingUpSword` turns *after* its sprite call, so the played row is the
  pre-turn direction.
* `Turning` when an attentive soldier substitutes `TurningAlerted` likewise
  stamps the pre-turn row.
* `RaisingShield` on a **PC** turns twice per tick: the PC override turns and
  then delegates to the human arm, which turns again.
* `Taking`, `TakingCrouched`, `TakingTarget`, `DroppingAmmo`,
  `DroppingAmmoCrouched`, `HittingTarget` and `HandlingTarget` freeze the first
  sprite frame for as long as the turn is still in progress.

### Abilities

Turning: `Hit`, `Heal`, `Pay`, `Tie`, `Eat`, `ClimbOnShoulders`, `ThrowApple`,
`ThrowStone`, `ThrowPurse`, `ThrowWaspNest`, `ThrowNet`, and all three `Listen`
phases.

Freezing the first frame while turning: `Hit`, `Pay`, and the five throws.
`Heal`, `Tie`, `Eat`, `ClimbOnShoulders` and `Listen` advance their action
regardless.

Not turning: `Carry` (`TransitionWaitingUprightCarryingCorpse`), `Drop`
(`TransitionCarryingCorpseWaitingUpright`), `Whistle`,
`ClimbDownFromShoulders`, `ReceivePurse`.

### Bow

`ShootingWithBow`, `ShootingWithBowUp`, their `*Anonymous` variants and
`ShootingWithBowLeaningOut` turn and freeze the first frame while turning. The
equip/load/raise/lower transitions and the `AimingWithBow*` idles do not turn.

### Melee / soldier

`TransitionCharging` turns before its motion call. `WaitingSword` turns
unconditionally for humans and PCs but only while swordfighting for soldiers.

## Arms that explicitly do **not** turn

These are the traps — each sits next to an arm that does turn:

* `ParryingLowSword` (but `ParryingSword` turns)
* `StandingUpBow` and `StandingUp` (but `StandingUpSword` turns)
* `TransitionWaitingUprightHelpingClimbing` and
  `TransitionHelpingClimbingWaitingUpright` (but the `WaitingHelpingClimbing`
  idle between them turns)
* `TransitionWaitingUprightCarryingCorpse` and
  `TransitionCarryingCorpseWaitingUpright` (but
  `TransitionWaitingCarryingOnShouldersWaitingUpright` turns)
* `LoweringShield` and `ParryingShield` (but `RaisingShield` and `WaitingShield`
  turn)
* `WaitingOnShoulders` and `ClimbingDownFromShoulders` (but
  `ClimbingUpOnShoulders` turns)
* `Whistling` (but `Eating`, `Searching` and `Healing` turn)
* `WaitingCape`, `WaitingHidden` and their exit transitions
* the whole death / unconsciousness / falling-hit family, `WakingUp`,
  `Provoking`, `BeingTied`, `WriggleUnderNet`, `LyingStuckUnderNet`
* the soldier idle/look/menace/sleep/lean-out family
* every civilian arm
