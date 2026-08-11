# External parity lanes 31–40

Work from baseline `3c48d625b` or a later integrated descendant. The objective is
to advance ten current, distinct frontiers without reopening completed lanes
21–30 or colliding with coordinator-owned work.

Run each actionable lane in its own branch/worktree, then merge reviewed
commits into `external-parity-31-40-integration`. A source-proven no-fix,
already-cleared, or trace-blocked result is valid and should not create a
commit.

## Global rules

- Read `AGENTS.md`; never use `git stash` and never use `/tmp`. Put diagnostics
  in a lane-specific repo-local directory and remove them before handoff.
- Always consult `./original-code` before changing behavior.
- Reproduce the representative on the lane baseline before editing. The frame
  below is the latest screened frontier, not permission to assume its cause.
- Record the exact first boundary, every differing field or RNG site, the
  responsible logical entity, and the Original and Rust call owners.
- Do not infer a fix from serialized output alone. If decisive Original
  transient state is unavailable, report no-fix instead of guessing.
- Never consume or skip RNG merely to align the stream. Prove the earlier state
  or cadence gate that explains a missing or extra draw.
- Never force a recorded field, add an epsilon comparator, suppress a path
  request, or return a fake default for missing required state.
- Prefer one narrow production boundary plus focused regression coverage.
  Re-run the representative and at least one nearby, independently selected
  control that reaches beyond the changed owner.
- Build and run separately. Do not pipe or redirect Cargo output, do not run
  clippy, and do not rebuild an optimized runner concurrently with another
  lane.
- Run focused tests, `cargo fmt --all -- --check`, and `git diff --check`.
- Commit only lane-owned production/tests. Do not edit campaign, archive,
  retirement, or this prompt from a lane branch.
- Search `docs/parity-task-archive/`, `docs/PARITY_CAMPAIGN_STATE.md`, lanes
  21–30, and current coordinator branches before editing. Stop if another
  owner has already claimed the same source boundary.

## Exclusions and dispositions

Do not substitute any of these for a lane:

- randomguy Profile 004 Restart replay 007, nicouzouf Savegame 069 replay 001,
  schema-14 Savegame 075 replay 008, old nicouzouf Savegame 041 replay 007,
  and linux3 Profile 002 Restart replay 004 now match exact EOF;
- old nicouzouf Savegame 022 replay 005 stops at a transient anti-collision
  boundary whose Original owner is not serialized;
- old nicouzouf Savegame 045 replay 014 was retired by Task 497 because its
  old record omits resolved click-versus-gesture seek-distance provenance;
- old SuN Savegame 017 replay 013 was retired by Task 488 because its recorded
  projectile membership contradicts current Original source and geometry;
- old SuN Savegame 034 replays 010 and 009 now match exact EOF after Task507's
  cached-3D takeoff fix in commit `9913b66cd` and are completed controls, not
  replacement work;
- old linux2 Profile 002 Savegame 024 replay 007 now matches exact EOF on the
  current integrated runner and is a completed exclusion, not topology or
  out-of-bounds replacement work;
- schema-14 linux2 Profile 002 Savegame 002 replay 009 now matches exact EOF
  after Task500 commit `f31e42a77`; schema-14 Cyrdach Profile 156 Savegame 001
  replay 009 remains its exact-EOF control. Both are completed exclusions;
- old nicouzouf Profile 001 Savegame 045 replay 003 and its Savegame 041
  replay-007 control now match exact EOF after commit `e553adb78`; Task484's
  projectile-water work and the later Soldier58 shield-pair cleanup are both
  complete and immutable;
- old nicouzouf Profile 001 Savegame 010 replays 012 and 011 now match exact
  EOF after Task533 commit `5361c9f00`; its Projectile106 motion-obstacle
  sector topology is complete and immutable, not replacement work;
- old linux2 Profile 002 Savegame 018 replay 009 and schema-14 linux2
  Savegame 017 replay 013 now match exact EOF after Task527 commit
  `2338dc115`; both rider-charge boundaries are complete and immutable;
- old linux2 Profile 002 Savegame 042 replays 004 and 003 now match exact EOF
  after Task541's world-ground domino fix in commit `9704d0bef`; their former
  combat-alert cascade is complete and immutable;
- old linux2 Profile 002 Continue replay 004 and schema-14 Cyrdach Profile 156
  Savegame 001 replay 009 now match exact EOF after Task543 commit `f6fbd2674`;
  their dead-damage lifecycle and exact-EOF control are completed exclusions;
- old SuN1Sh1nE Profile 004 Savegame 032 replays 012 and 011 now match exact
  EOF after Task539 commit `ddabda8ef`; their retained-circle victim-effect
  boundary is complete and immutable;
- schema-14 linux3 Profile 003 and all families explicitly owned by active
  coordinator sessions remain excluded.

Lane 32 must retain Task500 commit `f31e42a77` or a descendant. Task500's
alert-distance fix and the completed
Save002/Cyrdach replay-009 pair are immutable for this wave. Lane 32 owns only
the distinct SuN Save024 replay-004 RNG frontier described below.

Lane 35 must start from `a8153cfdf` or a descendant. Tasks 502 and 527 have
completed its former rider-charge family; Lane 35 now owns only the distinct
Cyrdach Save015 replay-005 direction/row frontier described below.

Lane 36 must start from `f6fbd2674` or a later integrated descendant and retain
Task542 commit `37fea7ac4`. Task542's reciprocal combat-neighbour death cleanup
and its isolated frame-406 clearance are complete and immutable for this wave.
Lane 36 owns only the distinct earlier Soldier110 facing/row frontier described
below.

Lane 40 must start from commit `233012233` or a later production descendant
and retain Task544's recursive after-script boundary. SuN Save013 replay-002
and schema-14 Nescafe Profile 001 Savegame 001 replay-009 now match exact EOF;
they are completed exclusions, not replacement work. Its still-earlier
Save045/Soldier58 frontier and Save041 control are likewise complete and
immutable. Lane 40 now owns only the distinct linux2 ExQuickSave patrol-
coordinate boundary described below.

## Lane 31 — SuN Save004 hidden strike-proposal history (read-only/blocked)

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_004/replay-003-session-0001.jsonl.zst`

**This lane is read-only and blocked from production edits.** On the optimized
`5361c9f00` runner the first serialized difference is after frame 16553 on
Pc344: Original has animation 76 / `ParrySword`, while Rust has animation 67 /
`SwordstrikeThrustA`. That split is downstream of an earlier hidden
`WarnForStrike` / `ProposeGoodSwordStrike` history difference, not evidence
that the final selection formula is wrong.

The boundary draw is aligned, and Will's strike A is source-valid for the
serialized principal Soldier111: its range, damage, skill gate, and timing all
admit the strike. Rust reaches the callback with strike-A boredom 10 and
therefore selects A. The loaded save starts Pc344's boredom at 40, while
Original's recorded Parry at this geometry requires a pre-call strike-A
boredom of at least 40. The trace does not serialize Original's live boredom
or the earlier transient proposal-call cardinality, so it cannot locate the
first excess or missing proposal that produced the final split.

Reproduce the boundary and investigate only if earlier instrumented history is
available that records every Pc344 warning/proposal call, its attacker,
principal opponent, RNG draw, boredom before/after, and installed or discarded
result from the loaded save through frame 16553. Otherwise return an explicit
evidence-blocked no-fix/no-commit report. Do not change strike geometry,
damage, skill/timing gates, principal selection, or aligned RNG; do not force
Parry, suppress strike A, or special-case the final frame. The cleared
frame-15812 replacement-goal and frame-16062 position boundaries remain
controls and are not owners of this hidden-history divergence.

This lane is distinct from Lane32's Save024 frame-911 RNG-cardinality batch:
Lane31 reaches a serialized Pc344 command split with the RNG stream still
aligned, but owns only the earlier hidden proposal-history search. Exchange
the exact combat actor and source callsite if either investigation converges,
and stop the later lane rather than implementing the same melee defect twice.

## Lane 32 — SuN Save024 melee RNG cardinality

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-004-session-0001.jsonl.zst`

On the last screened post-Task507 optimized runner, this retained canonical
trace first fails during Original frame 911. Rust consumes draws 6940–6945 at
`MacroRand`, `SwordStrikeSelection`, and three `MeleeNonMutualGate` sites,
while Original ends the frame at draw 6949. Original's recorded callsite
offsets are 1116921 once, 1804390 three times, and 1807002 five times.

Start from `f31e42a77` or a later integrated baseline and reproduce the exact
current batch before source work; Task500 postdates the screened runner and
must be present even though its owner is unrelated. Capture the complete
preceding-frame actor/combat state and both RNG batches, and symbolize offsets
only against the compatible Original binary. Prove the earliest divergent
gate rather than treating Rust's first displayed logical site as the owner.
Task378 commit `bbb8daf52` already cleared this trace's frame-585 retained
sweep/PushAside boundary and is immutable: do not revive retained sweep
execution, consume dummy draws, or suppress the later common tail.

This lane is distinct from Lane38's Pc316 helping-climb handoff and Lane39's
Pc126 post-seek `HitCmd`-versus-`MoveOk` boundary.
Exchange exact actor and source owners if analysis nevertheless converges, and
stop the later lane rather than implementing duplicate RNG gates.

## Lane 33 — nicouzouf Save067 sound frontier

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_nicouzouf/Profile_001/Savegame_067/replay-003-session-0001.jsonl.zst`

Earlier state/RNG work now reaches an independent sound boundary after frame
1502. Record the complete Original/Rust sound request queues, source/owner
identity, sample, delay, priority, and the surrounding simulation RNG batch.
Locate the exact gameplay producer and sound-manager insertion/completion
order. Do not add cosmetic sound work to simulation merely because it appears
in the trace, and do not perturb gameplay RNG to reproduce an audio delay.

## Lane 34 — SuN Save013 hidden-primary target list (read-only/blocked)

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_013/replay-009-session-0001.jsonl.zst`

**This lane is read-only and blocked from production edits.** After frame
1667, Soldier87 has Original `list_them=[PC173, PC47]` and Rust
`list_them=[PC173]`. The timer arm for
`SUBSTATE_ATTACKING_BOW_RUNNING_BEHIND_SHIELD_BEARER` calls
`ReinitializeThemList` and then `BattleDecisions`. Original re-adds PC47 from
a nearby attacking friend's live `GetPrimaryTarget`; Rust sees all admitted
friends targeting PC173. The trace does not serialize those Original primary
target pointers, so it cannot identify the contributing friend or the earlier
selection split.

Reproduce and document the boundary, inspect any newly available decisive
evidence, and otherwise return an explicit no-fix/no-commit report. Do not
change `ReinitializeThemList`, friend-target injection, visibility, or list
retention from the final serialized mismatch. Replay 002 is a completed
Task544 exact-EOF exclusion and must not be used as a control.

## Lane 35 — Cyrdach Save015 Pc108 direction/row transition

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_Cyrdach/Profile_156/Savegame_015/replay-005-session-0001.jsonl.zst`

On the current `a8153cfdf` integrated production baseline this retained
canonical trace first diverges after frame 1192 only on Pc108: Original has
direction 7 and sprite row 151, while Rust has direction 6 and sprite row 150.
The RNG stream remains exactly aligned at cursor 793, with no draw and no path
event on either side at the first boundary. Same-save replay-004 matches exact
EOF across all 1,500 recorded frames on the same optimized runner.

Capture Pc108's complete actor, selected order, sequence element, animation
row/frame, direction/current goal, motion state, position, input, and callback
chronology across frames 1191–1193. Identify the earliest direction or row
writer and compare Original's action transition, facing update, sprite
`Hourglass`, and order advancement with Rust in exact statement order. Do not
force the displayed direction/row, add a rounding exception, or alter aligned
RNG to select another branch.

This trace is absent from retirement manifests, completed archives, and the
other nine representatives. It is distinct from Lane31's Pc344
parry-versus-strike selection: Lane35 has no command or RNG difference at its
first boundary and owns only Pc108's later facing/row transition. Exchange the
exact actor/action owner if analysis nevertheless converges, and stop the
duplicate implementation. Keep Tasks 502/527 and their exact-EOF rider traces
as immutable exclusions.

## Lane 36 — nicouzouf Save039 Soldier110 facing/row transition

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_nicouzouf/Profile_001/Savegame_039/replay-007-session-0001.jsonl.zst`

On the last screened `f6fbd2674` baseline this trace is blocked earlier after
frame 143 on Soldier110: `direction`, `direction_goal`, and `sprite_row` differ
between Original and Rust. Reproduce it first on the required global baseline;
the screened boundary precedes Soldier91's death and therefore cannot be owned
by Task542's combat-neighbour cleanup.

Start from `3c48d625b` or a later integrated descendant and reproduce the exact
frame-143 values before source work. Capture Soldier110's selected order and
sequence element, animation and sprite frame/row, current and goal directions,
action and motion state, melee relationship and strike context, target geometry,
and every same-frame facing or animation writer on both sides. Identify the
earliest source owner that selects or applies the differing direction/row; do
not force the serialized facing, remap a sprite row, or alter aligned RNG to
select another branch.

Task542 commit `37fea7ac4` is immutable. On isolated baseline `2338dc115` plus
that exact patch, the former post-frame-406 Soldier90 goal/increment boundary
cleared and the trace advanced 429 frames to an independent sprite-index panic
while replaying Original frame 835 to 836; Savegame_045 replay-010 remained
exact EOF across all 1,500 frames. The current integrated lineage cannot claim
that clearance because its frame-143 Soldier110 split prevents it from reaching
the death boundary. Do not reopen reciprocal combat-neighbour teardown or edit
Task542's damage owner unless the earlier frontier is first proven to originate
there. This lane remains distinct from Lane32's frame-911 melee RNG batch and
Lane40's patrol-coordinate Halt handoff; coordinate and stop duplicate work if the exact
facing/animation owner converges with another lane.

## Lane 37 — nicouzouf Save037 projectile sprite cadence

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_nicouzouf/Profile_001/Savegame_037/replay-015-session-0001.jsonl.zst`

On the current integrated baseline this trace first diverges after frame 1057
only in the paired projectile cadence: Projectile84 has sprite frame 5 in
Original and 3 in Rust, while Projectile85 has sprite frame 3 in Original and
5 in Rust. Same-save replay-012 matches exact EOF and is the required control.

Capture both projectiles' creation and registry order, animation and sprite
state, owner/target, flight state, update eligibility, and every sprite-clock
writer across frames 1056--1058. Compare Original's projectile update and
sprite `Hourglass` ordering with Rust, and prove the earliest cadence or
iteration boundary before editing. Do not swap projectile identities, force
the displayed frames, or add a trace-specific clock adjustment. Earlier work
on this replay that cleared prior boundaries remains immutable.

This paired-projectile presentation boundary is distinct from Lane31's Pc344
parry choice, Lane35/Lane36 facing-row transitions, Lane38's helping-climb
order lifecycle, and Lane39's hit-versus-move split. Exchange the exact
projectile update, registry, and sprite-clock owner with any lane that reaches
the same boundary, and stop the duplicate implementation.

## Lane 38 — linux3 Save067 helping-climb lifecycle

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_linux3/Profile_003/Savegame_067/replay-008-session-0001.jsonl.zst`

On the current integrated baseline this trace first diverges after frame 51860
on Pc316: Original remains in `Move`, while Rust exposes
`EnterHelpingClimb`. Same-save replays 007 and 009 match exact EOF and are the
required controls.

Capture Pc316's complete selected order and sequence, movement and climb
state, helper/recipient relationship, geometry, animation, sprite
`Hourglass`, callbacks, and order installation/advancement chronology across
frames 51859--51861. Identify the earliest gate or statement-order difference
that retains `Move` in Original but admits `EnterHelpingClimb` in Rust. Do not
force the serialized command, suppress a valid climb request, or alter
geometry/tolerance without source proof.

This lifecycle boundary is distinct from Lane39's `HitCmd`-versus-`MoveOk`
split and the facing/RNG/projectile lanes. Exchange the exact Actor, Sequence,
movement, climb, and `Hourglass` owner with any lane that converges, and stop
the later duplicate rather than implementing the same transition twice.

## Lane 39 — linux3 Save001 hit-versus-move transition

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_linux3/Profile_001/Savegame_001/replay-012-session-0001.jsonl.zst`

On the current integrated baseline this trace first diverges after frame 3240
on PC126: Original exposes `HitCmd`, while Rust exposes `MoveOk`. Same-save
replays 011 and 001 match exact EOF and are the required controls.

Capture PC126's complete damage/hit state, selected order and sequence,
movement result, animation, sprite `Hourglass`, attacker/target relationship,
callbacks, and order translation/advancement chronology across frames
3239--3241. Prove the earliest owner that preserves or installs `HitCmd` in
Original while Rust advances to `MoveOk`; do not patch the displayed command,
hold a terminal card artificially, or suppress valid movement completion.

This hit-versus-move boundary is distinct from Lane31's parry selection and
Lane38's helping-climb lifecycle. Exchange the exact damage translation,
Actor, Sequence, movement, and `Hourglass` owner if either lane converges, and
stop the later duplicate rather than implementing the same order transition
twice.

## Lane 40 — linux2 ExQuickSave patrol-coordinate Halt handoff

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_linux2/Profile_002/ExQuickSave/replay-011-session-0001.jsonl.zst`

On the optimized `233012233` runner this retained canonical trace first
diverges after frame 1312 in exactly six fields. Soldiers144 and 145 both have
animation 142, command `LeaveAttentiveMode`, and alert status 0 in Original;
Rust gives Soldier144 animation 5 and Soldier145 animation 51, with command
`MoveOk` and alert status 1 for both. The simulation RNG cursor remains aligned
at 3350 with no draw on either side, and neither side records a path event.
Same-save replay-010 matches exact EOF across all 1,144 recorded frames on the
same runner.

Immediately before the boundary both soldiers are attentive, Yellow,
`DefaultOnPost` patrol members when chief Soldier143 sends
`CALL_PATROL_COORDINATE`. Capture the complete chief/member stimulus order,
AI state and substate, attentive and alert state, macro/timer state, actor
outbox boundaries, selected sequence/order, priority, command, animation, and
movement request across frames 1311--1313. Original
`RHArtificialIntelligence::CoordinatePatrol` calls `StopAll()` and deliberately
falls through to its patrol `GoTo`; `StopAll()` synchronously calls the NPC's
`Halt()` before that replacement work (`original-code/RHartificialintelligence.cpp:7136-7157,7772-7827`).
Compare that exact Halt, transition generation, priority/postponement, and
GoTo ordering with Rust's queued `stop_all` and `coordinate_patrol` effects in
`crates/robin_engine/src/ai/controller.rs`. Prove the earliest differing
boundary before editing; do not force `LeaveAttentiveMode`, clear alert status,
or suppress the valid patrol movement merely to match the displayed fields.

The representative and replay-010 control are canonical, absent from the
retirement manifests and completed archives, and distinct from the other nine
lane representatives. Task463 also mentions `CALL_PATROL_COORDINATE`, but its
completed owner was an upstream PassDoor-direction error that caused a
spurious coordinate event; this lane already agrees that the event occurs and
owns only its downstream synchronous Halt-to-GoTo handoff. Keep Task463,
Task544's Save013 replay-002 fix, its exact-EOF Nescafe control, and the older
Save045/Soldier58 work immutable. If another lane reaches this exact patrol
Halt/priority boundary, stop the later duplicate rather than implementing it
twice.

## Required report

For every lane report: branch and baseline; representative and current first
boundary; all differences or RNG sites; Original and Rust file/line owners;
source-faithfulness and distinctness (or precise no-fix rationale); files and
focused tests; representative/control replay results; build, format, and diff
checks; commit hash or explicit no-commit disposition.

The integration owner must review every commit, merge only independently
source-proven changes, run the full engine suite and optimized parity build,
update shared bookkeeping, and produce
`external-parity-31-40-integration`.
