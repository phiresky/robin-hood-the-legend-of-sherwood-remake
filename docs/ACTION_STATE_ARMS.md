# Per-arm `action_state` semantics

Every actor animation arm decides for itself whether it stamps a new
`(posture, action_state)` pair, and on which motion state. Neighbouring arms of
the same family routinely disagree — a walk transition settles on
`Done | Terminated`, a walk loop stamps on `Start`, and several hold animations
stamp unconditionally on every tick. This document records the rules that are
easy to get wrong; it is not a full transcription of the five execute switches.

Getting an arm wrong shows up in parity sweeps as `state:actor.action_state`
with a stale value: the actor carries whatever the *previous* arm stamped into
frames that should already have settled.

## The class that owns an arm decides who receives it

The single most common source of a stale action state is implementing an arm
in the wrong dispatch tier. The original engine's execute switches form an
inheritance chain, and an arm implemented by a base class is inherited by every
subclass unless that subclass overrides the same animation:

| Tier | Reaches |
| --- | --- |
| actor | every actor, PCs included |
| human | PC, soldier, civilian |
| NPC | soldier, civilian |
| PC / soldier / civilian | that subclass only |

An arm that belongs to the human tier but is written into a soldier-only
handler leaves PCs and civilians with a stale state, and the divergence only
surfaces when a PC happens to reach that animation. The knock-out holds
(`BeingUnconscious`, `BeingUnconsciousSword`, `BeingUnconsciousBow`) are human
tier: their `Start` stamps `Lying` plus `Waiting` / `WaitingSword` /
`AimingWithBow`. Written soldier-only, a knocked-out PC kept the `MovingSword`
or `Bored` state it carried into the blow for the whole unconscious period.

## Where the stamp is applied

The Rust side has three sites, chosen by the dispatch category an order is
bound to in the actor-execute catalog (`engine/tick.rs`):

| Category | Stamp site |
| --- | --- |
| `Movement` | `movement_execute_state_effect` in `engine/movement.rs` — a `(order, motion) -> (posture, action_state)` table, applied through `movement_state_effects` with the `Start`-survival gates |
| `GenericAnimation` and universal arms | `apply_active_animation_start_state_side_effect` in `engine/animation.rs`, applied immediately |
| soldier-only / NPC-only arms | `apply_soldier_execute_side_effects` / `apply_npc_execute_side_effects` |

`arm_is_always_consumed` is unrelated: it decides whether the sequence element
advances, not whether the arm stamps a state. A hold animation can be in that
list and still owe a `Start` stamp.

## Movement transitions

Every walk/run/crouch speed-and-posture transition settles on
`Done | Terminated`, never on `Start` — `Start` only carries an assertion in
the original. The full crouch/upright family:

| Transition | Settles on `Done \| Terminated` |
| --- | --- |
| `TransitionWaitingCrouchedWalkingCrouched` | `Crouched`, `Moving` |
| `TransitionWalkingCrouchedWaitingCrouched` | `Crouched`, `Waiting` |
| `TransitionWalkingUprightWalkingCrouched` | `Crouched`, `Moving` |
| `TransitionRunningUprightWalkingCrouched` | `Crouched`, `Moving` |
| `TransitionWalkingCrouchedWalkingUpright` | `Upright`, `Moving` |
| `TransitionWalkingCrouchedRunningUpright` | `Upright`, `MovingFast` |

The four upright-crouch cross transitions were missing from the movement table,
so a PC leaving a crouch walk kept `Moving` where the original had already
promoted it to `MovingFast`.

## Corpse carry

The carry family is PC-only. Its states are stamped by three arms, and the
idle hold is the one that resets the carrier:

| Arm | Motion | Result |
| --- | --- | --- |
| `TransitionWaitingUprightCarryingCorpse` | `Done` | `CarryingCorpse`, `Waiting` |
| `WaitingWithCorpse` | `Start` | `CarryingCorpse`, `Waiting` |
| `WalkingWithCorpse` | `Start` | `CarryingCorpse`, `Moving` |
| `WalkingWithCorpse` | `Terminated` | `CarryingCorpse`, `Waiting` |
| `TransitionCarryingCorpseWaitingUpright` | — | no stamp; the drop transition inherits whatever the hold left |

Because the drop transition stamps nothing, a missing `WaitingWithCorpse`
`Start` propagates: the carrier keeps `Moving` through the idle hold, through
the drop, and into whatever command follows (`WhistleCmd`, `EnterHelpingClimb`).
Note also that a seek-mode corpse walk consumes `Start`, so the walk arm often
never stamps `Moving` at all and the hold is the only thing setting the state.

## Arms that stamp on every tick

A few arms stamp unconditionally rather than on a motion state.
`SleepingUpright` does, and so does the PC shield walk `WalkingWithShield`: it
stamps `(Upright, MovingShield)` before looking at the motion state at all, then
overwrites it with `(Upright, HoldingShield)` on `Terminated`. Implementing
these as `Start`-only leaves the state stale for any actor that adopts the
animation mid-flight.

TODO: `WalkingWithShield` has no `action_state` stamp on the Rust side at all —
neither the every-tick `MovingShield` nor the `Terminated` `HoldingShield`. No
trace in the corpus isolated it, so it is recorded here rather than guessed at.
