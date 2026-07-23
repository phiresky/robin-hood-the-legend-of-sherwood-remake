# Original-game parity replay ledger

This is the evergreen record of behavioral changes found through replaying an
instrumented Original game session in the Rust engine. Update it whenever the
trace exposes a new divergence, an implementation changes because of one, or a
regression test closes one. It records logical parity, not incidental identity:
entity IDs may differ when the two worlds can be mapped isomorphically.

## Baseline and contract

- Trace: `original-code/parity-traces/original-demo-rng-baseline.jsonl`
- Mission: `Dem_Lei_MP` / Leicester demo, 25 simulation ticks per second
- Size: 1 header, 1 startup RNG-prefix record, and 1,469 gameplay frames
- Original trace schema: 2
- Inputs: resolved game commands, applied on their recorded simulation frames
- Start point: mission start; menu and raw input-device behavior are out of scope
- Pathfinding: deterministic synchronous A*, while retaining the Original's
  request/processing phase boundary
- Randomness: the Original's filtered gameplay `rand()` values form one global
  draw stream. Rust must consume the same values in the same order and fails at
  the first cursor mismatch. Call-site offsets are diagnostic provenance, not a
  requirement that Rust use identical addresses or function boundaries.
- Comparison: entities are mapped by stable logical traits and creation order;
  compared floats use exact bits. Numeric IDs alone are never treated as a
  behavioral divergence.

The current verified clean prefix is through frame 44. At frame 45 soldier 122
has action state `Bored` (Original discriminant 1) while Rust retains `Waiting`.
The actor is inactive in both worlds; Original still advances its idle Wait
through the waiting-to-bored transition because activity is not an Hourglass
eligibility gate. Rust now executes inactive actors, but the baseline still
finds this frame-45 mismatch with Rust on `WaitingUpright`; audit the remaining
idle-order timing/transition difference. Increase the clean-prefix statement
only after a normal first-divergence run has passed that frame.

## Change ledger

| Status | Area | Trace evidence and Original behavior | Rust change / regression coverage |
| --- | --- | --- | --- |
| Done | Original recorder | A useful comparison needs deterministic state and resolved commands on every tick. | The C++ game writes schema-2 JSONL with frame state, resolved commands, creation order, and RNG batches. Deterministic/synchronous pathfinding is enabled for captures. Original commits: `502a7b3` and `a97c9dd`. |
| Done | Structured Rust frame dump | Broad parity snapshots did not expose transient AI, sequence, movement, and vision state, forcing repeated one-off logging changes. | `original_parity_replay --dump-jsonl` writes stable JSONL records containing the complete serializable engine snapshot, resolved commands, RNG cursor/batch, entity mapping, and parity differences. `--dump-from`, `--dump-through`, and repeatable `--dump-entity kind:index` filters keep targeted captures manageable; omitting the entity filter retains the whole engine. |
| Done | Global RNG replay | Bored/waiting choices affect head direction and later AI behavior; reproducing only a seed is insufficient across different implementations. | Rust can consume the trace's filtered global libc `rand()` draw stream and records typed Rust consumption sites. Startup and every frame assert the exact draw cursor. |
| Done | Isomorphic identity | Original and Rust IDs and hidden startup objects differ, while the logical world is equivalent. | The runner constructs a mission-start entity bijection from kind, stable data, and creation order. Original's hidden 31-object prefix is retained where creation order is itself gameplay state, such as staggered detection. |
| Done | Trace command decoding | Raw mouse/keyboard behavior is not under test. | Recorded resolved commands are translated through the entity bijection; unsupported command values and malformed or non-contiguous traces fail loudly. |
| Done | Enum/state representation | The trace exposed a missing Original discriminant rather than an ID mismatch. | Added the missing discriminant and made the compared logical state representable without substituting fake values. |
| Done | Sequence callbacks and owner boundaries | Original sequence completion, condolence cards, and re-entrant AI callbacks can execute synchronously before an outer dispatch returns. | Sequence effects now preserve callback FIFO order and owner-local synchronous boundaries, including nested condolations. Regression tests cover callback/condolation ordering. |
| Done | `GOTO_DONTSTOP` and waypoint patrol | Patrol setup and continuation differed before visible movement. | Matched Original flag handling, AI callback/lifecycle behavior, and waypoint-route advancement. |
| Done | AI `Move` promotion | A newly launched patrol move must not interrupt the actor's current order before that actor's update slot. | AI moves enter the sequence pipeline and defer owner instruction. Callers that are true synchronous native/condolation boundaries explicitly drain the deferred action; ordinary patrol does not. |
| Done | Path scheduling barrier | Even synchronous Original A* observes `MoveWaiting` for the frame where a request is issued, then exposes `MoveOk` in the next path phase. | All A*-requiring moves enter the queue. Synchronous mode computes and installs the result at the next path barrier instead of bypassing the queue or adding an asynchronous extra frame. |
| Done | Movement animation loops | A walk/run animation row looping does not terminate travel; only `TILL_LAST_FRAME` observes natural animation termination. | `Sprite::perform_motion` keeps ordinary movement alive across animation loops. Unit coverage: `perform_motion_walk_does_not_terminate_when_animation_loops`. |
| Done | Motion start tick | Original initializes a new motion order and advances its animation in the same invocation, making frame-zero distance immediately available. | `perform_motion` performs that first increment while the general action entry point retains its distinct start semantics. Unit coverage: `perform_motion_start_tick_advances_and_emits_frame_zero_distance`. |
| Done | Direction sector classifier | `atan2` plus rounding differed for boundary vectors created by anti-collision. | Ported `SBGeoVector2D::GetSector0to15` as the same f32 half-plane classifier and literal constants. |
| Done | Anti-collision direction | During deviation, Original faces the committed deviation step and rebuilds its normal increment when deviation ends. | Direction/increment are derived after the committed step; deviation recovery invalidates and recomputes the normal goal increment. |
| Done | Mobile anti-collision scope | Original calls `IsBlockedByMobile` only when `GetMobileRepulsiveObjects` reported a mobile intersecting the actor's future box. Layer-wide mobile geometry must not block an unrelated actor's recovery or break-through corridor. | Mobile blocking checks in deviation recovery, ordinary commit, and break-through are gated by the future-box intersection result. |
| Done | Transition anti-collision | Soldier 89 retained `deviated=true` through a running-to-walking transition at frame 25, so frame 26 incorrectly used anti-vibration turning and remained at direction 3. Original sends every nonzero `PerformMotion` distance, including `TILL_LAST_FRAME`, through `UpdatePositionAntiCollision`; it recovered on frame 25 and ordinarily turned to direction 4. | Nonzero movement transitions now use the same anti-collision step and recovery commit as walking motion while retaining their distinct animation-completion semantics. |
| Done | Vision geometry | Rust applied the map's isometric Y correction to the target delta at a point where Original compares raw view-space X/Y, shifting cone membership. At frame 38 Rust then saw soldier 82 from soldier 83 while Original kept looking left. Original applies the correction by scaling the rotated cone-side vectors instead; Rust had omitted that second part and consequently made diagonal cones too wide. | Cone determinants now compare the raw target delta against side vectors whose Y components carry `ASPECT_RATIO`, exactly as `RefreshView` and `IsDetecting` do. The frame-38 soldier geometry has direct regression coverage. Normal eye movement also uses the active PI/8 step and PI/80 iterator constants instead of values from Original's commented old-values block. |
| Done | `Turn`/`FaceTo` during movement | Original turns once toward the retained movement goal, halts/promotes the turn immediately, retains the map goal, and drives the movement-exit animation through ordinary `Execute` even though action state remains `Moving` until that transition completes. | The old-goal turn step precedes halt, the turn sequence is launched eagerly, its requested direction goal is installed, and the retained map goal is restored. Generic animation dispatch admits non-movement walking/running exit transitions while the old moving state is still live. Unit coverage classifies the admitted transition family. |
| Done | Actor idle exit inside movement | Original's bored-to-waiting animation always uses base `PerformAction`, including when the order is stored in a movement element; only particular movement animation arms branch on `IsMovement()` and use `PerformMotion`. | Generic animation execution no longer rejects an order solely because its command/element is movement-shaped. The live animation-arm catalog remains the owner selector, and the universal base-actor idle state effects now apply to PCs as well as NPCs. Regression coverage verifies a PC changes from `Bored` to `Waiting` on bored-exit completion. |
| Done | Patrol `GoToSpeed` close-point gate | At frame 24 Rust changed soldier 92 from patrol-running to waiting and issued `Turn`; Original downgraded it to patrol-walking and retained `MoveOk`. `RHArtificialIntelligence::GoTo` only synthesizes `EventReachPoint` within five units when the current animation is an idle wait. | `go_to_speed`, which represents the same Original overload, now applies the idle-animation, likes-to-sit, and special-action gates before setting `already_on_point`. Regression coverage exercises both ordinary and speed variants, including a nearby running actor that must still queue movement. |
| Done | Re-entrant move after condolation | Soldier 94's Turn completion advances `DefaultGotoRouteTurn` to `DefaultEnroute` and launches the next patrol move during the sequence-manager pass. Original's active `Hourglass` loop instructs that newly registered Move before returning; Rust left it queued and exposed `Wait` for frame 26. | Generic condolation dispatch now promotes and synchronously instructs only the card owner's re-entrant Move before `Ready()` resumes. Other owners retain their FIFO positions, preserving the earlier deferred-patrol ordering fix. |
| Done | Scripted waypoint owner boundary | At frame 27 soldier 104's Turn condolence advances path 13 to scripted waypoint 0. Original synchronously calls `Officier_jaloux__0___8000024c::ReachPoint`, consumes `Rand(2)` (the odd result skips the optional animation), fires `EventAfterScriptGoOn`, and launches the next patrol Move before returning. Rust deferred the waypoint VM until the next frame. | Waypoint callbacks now have an owner-specific drain used inside both condolation paths, followed by same-stack `EventAfterScriptGoOn` and owner Move promotion. The compatibility global drain delegates to the same owner-local implementation without interleaving other NPCs. |
| Done | Deviated arrival ordering | At frame 31 soldier 89 was deviated, blocked, and within ten units of its goal. Rust's pre-movement `IsGoalReached` emulation took the blocked proximity shortcut and snapped to the waypoint. Original first runs `UpdatePositionAntiCollision`, which commits a recovery step and rebuilds the increment; only then does `IsGoalReached` remain false. Soldier90's mismatch was downstream because its later owner slot observed soldier89's incorrect snapped position. | Pre-movement crossed-waypoint retirement is suppressed while deviated, preserving the Original anti-collision-before-arrival order. Replay verifies both soldiers' exact positions and soldier89's rebuilt direction goal. |
| Done | Post-step intermediate arrival | At frame 32 soldier 89 has crossed its non-final waypoint only after committing the anti-collision step. Original runs `IsGoalReached` after `PerformMotion` and retires the waypoint in that owner slot; Rust only had the pre-step arrival check and retained the old patrol goal. | Non-final zero-tolerance movement now performs the post-step arrival check and pops the intermediate order at the same owner boundary. A worktree refactor is generalizing the shared final/tolerance/door arrival tail and its regression coverage. |
| Done | Running patrol member `FaceTo` boundary | At frame 32 soldiers 95 and 97 receive a near-point-backwards `FaceTo` from their chief before their own creation slots. Original `Stop` rewrites the live run into a running-to-waiting transition, lets that slot commit its six-unit frame, then instructs the already-computed Turn in the sequence-manager phase. Eager Rust instruction skipped the transition movement; partially deferring it recomputed facing from the later position and lost the retained patrol goal. | Standalone Turns produced by cross-owner patrol coordination defer instruction only for a running member. The direction is frozen at `FaceTo` time, the movement goal is carried through and restored at Turn instruction, and walking/mixed-order patrol cases retain their existing synchronous behavior. Replay matches positions, directions, goals, commands, and the follow-on duplicate Turn through frame 37. |
| Done | Cached movement increments and transition arrival | At frame 39 Rust advanced soldier 96 but left soldier 98 at the end of their run-to-walk transitions; Original did the reverse. The branch was determined by tiny signed dot products. Rust renormalized the remaining goal vector every ordinary frame instead of using `GetIncrementMap`, drifting the chief's historical patrol positions and the later formation targets. Rust also omitted `RHMOTIONMETHOD_TILL_LAST_FRAME`'s per-step arrival snap/zeroing and treated a merely-near destination as exactly equal when initializing the copied continuation. | Ordinary motion now uses the increment cached by order initialization and rebuilt by anti-collision, the 16-sector helper uses Original's literal float table, and non-deflecting anti-collision avoids a subtract/add round trip. Transition motion checks arrival after every committed step, zeros both increment representations, and snaps exactly under Original's deviation/tolerance gates. Fresh non-transition motion only short-circuits on exact point equality. This selects the correct continuation for soldiers 96/98 and retires it at the correct owner boundary, matching through frame 44. |
| In progress | Inactive actor Hourglass | At frame 45 inactive soldier 122 changes from `Waiting` to `Bored` in Original while Rust leaves its idle sequence frozen. `RHEngine::Hourglass` calls every element's virtual Hourglass without testing `IsActive`; `RHElementActor::Hourglass` still installs a missing Wait and executes the current order. The active flag controls world/render presence and actor anti-collision, not sequence time. | Inactive actors now receive lazy Wait initialization and generic sprite Execute at their normal creation-order slot. Regression coverage verifies both gates. The baseline still diverges at frame 45 with Rust on `WaitingUpright`, so the remaining idle-order start/completion timing or generated transition chain must be audited before marking this row done. |
| In progress | Frame-38 idle-look frontier | Soldier 83 is in `LookLeft` / AI substate 20 in Original but `Wait` / substate 21 in Rust. | Audit the patrol or idle-action continuation and its exact owner/RNG boundary from the now-clean frame-37 state. |
| Open | Remaining trace | Passing an early prefix does not establish parity for later player interaction, combat, AI, effects, or mission scripting. | Continue first-divergence repair until all 1,469 frames pass, then run `--scan-all` as a second check and add further captures for behavioral coverage. |

## Workflow

Build once, then use the first-divergence run for iteration:

```sh
cargo build --example original_parity_replay
ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
  target/debug/examples/original_parity_replay \
  original-code/parity-traces/original-demo-rng-baseline.jsonl
```

After the first-divergence run is clean, collect the first occurrence of every
remaining compared-field mismatch with:

```sh
ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
  target/debug/examples/original_parity_replay --scan-all \
  original-code/parity-traces/original-demo-rng-baseline.jsonl
```

For interactive inspection, start the headless runner paused with its local
HTTP endpoint:

```sh
ROBINHOOD_DATA_DIR=datadirs/demo_leicester_linux \
  target/debug/examples/original_parity_replay \
  --http-server 17640 --start-paused \
  original-code/parity-traces/original-demo-rng-baseline.jsonl
```

While it is running, `GET /engine-dump` returns the complete current engine,
`GET /state` reports the frame, and the existing controls advance the Original
trace while retaining its recorded commands and RNG stream:

```sh
curl -s http://127.0.0.1:17640/engine-dump
curl -s -X POST -H 'content-type: application/json' \
  -d '{"n":1}' http://127.0.0.1:17640/step-forward
curl -s -X POST -H 'content-type: application/json' \
  -d '{"frame":45}' http://127.0.0.1:17640/go-to-frame
```

Forward steps reply only after all requested frames match. A divergence halts
advancement and makes the in-flight step fail, but leaves `/engine-dump` and the
other inspection endpoints alive at the divergent frame. Backward stepping is
deliberately unsupported: restart the runner and use forward `go-to-frame` so
every inspected state is rebuilt from the mission-start trace contract.

For every newly fixed divergence:

1. Identify the first causal mismatch, not merely the largest downstream diff.
2. Confirm the behavior in `original-code` and record the relevant Original
   function or rule in this ledger or the code comment.
3. Implement the smallest general behavioral correction; do not special-case a
   recorded frame, actor, or ID.
4. Add focused regression coverage where practical.
5. Re-run from mission start because an earlier fix can change all later state.
6. Update the clean prefix, ledger status, and remaining first divergence here.

## Coverage limits

A clean baseline proves exact parity only for the state fields serialized by the
recorder and the behaviors exercised by this session. When a divergence depends
on unrecorded state, extend the neutral trace schema rather than guessing from a
downstream symptom. New recordings should keep the same resolved-command,
mission-start, synchronous-path, global-RNG-stream contract unless this document
explicitly introduces and motivates another profile.
