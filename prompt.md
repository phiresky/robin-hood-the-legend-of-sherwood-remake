# External parity lanes 31–40

Work from baseline `dc09f516d` (which includes `d0f4c93ab`). The objective is
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
- old nicouzouf Savegame 045 replay 014 has unrepresentable old input
  provenance and is a retirement candidate;
- old SuN Savegame 017 replay 013 contradicts current Original projectile
  membership and is a retirement candidate;
- old SuN Savegame 013 replay 002 is a separate unowned RNG boundary and is
  not a control for lane 34;
- schema-14 linux3 Profile 003 and all families explicitly owned by active
  coordinator sessions remain excluded.

Lane 35 is temporarily reserved by coordinator Task502 and must remain
unstarted on baseline `dc09f516d`. Its integration owner must first supply a
new baseline containing Task502's disposition, or explicitly instruct the
lane to cherry-pick that disposition. Only then may lane 35 validate the
integrated fix or take the still-live downstream frontier. This reservation
prevents an independent implementation of the same source boundary.

Lane 36 is likewise reserved by coordinator Task507's pushed-flight takeoff
work and must remain unstarted on `dc09f516d`. Its integration owner must first
supply a new baseline containing Task507's disposition, or explicitly require
that disposition to be cherry-picked. Only the frontier that remains on that
resulting baseline belongs to lane 36.

Lanes 31 and 40 both begin in projectile trajectory/first-Hourglass territory.
Their owners must exchange the exact Original and Rust source owner as soon as
it is identified. If both converge on the same `ComputeTrajectory`, landing
resolution, material, or first-Hourglass defect, the later lane must stop and
become a control/downstream revalidation for the earlier lane; it must not
implement a second version of the same fix.

## Lane 31 — nicouzouf Save010 Projectile106 topology

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_nicouzouf/Profile_001/Savegame_010/replay-012-session-0001.jsonl.zst`

The current first divergence is after frame 1142 and is owned by
Projectile106's topology. Capture every projectile field at creation and at
the boundary, including active/flying state, layer, sector, obstacle identity,
trajectory endpoint, position/elevation, and old position. Compare the exact
`ComputeTrajectory`/first-`Hourglass` ordering and sector-list traversal in
Original with Rust's launch and landing-resolution owners. Do not copy the
recorded layer or sector, and do not reuse a fix from a different terminal
material family without matching obstacle identity and source ordering.

## Lane 32 — schema-14 linux2 Civilian65 path continuation

Representative:
`parity-random-save-replays-60s-15x-schema14/traces/Savegame_linux2/Profile_002/Savegame_002/replay-009-session-0001.jsonl.zst`

Commit `dc09f516d` clears the former frame-516 PassDoor-direction boundary.
The current independent frontier is after frame 711 on Civilian65's path or
movement continuation. Start from the exact queued/failed path records and
the actor's installed order on frames 710–712. Trace route-source adaptation,
door ownership, callback delivery, and sequence replacement on both sides.
Treat the integrated direct/indirect PassDoor polarity as an immutable
control; do not reverse or special-case it to clear this later failure.

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
retention from the final serialized mismatch. Replay 002 is a separate RNG
family and must not be used as a control.

## Lane 35 — linux2 Save018 rider-charge ordering (Task502 reserved)

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_linux2/Profile_002/Savegame_018/replay-009-session-0001.jsonl.zst`

The current live frontier is after frame 8728 in rider-charge ordering. This
family is reserved by coordinator Task502 (`task502-rider-charge-order`): do
not create its worktree or start from `dc09f516d`. Wait until the integration
owner supplies a new baseline containing Task502's disposition or explicitly
requires that disposition to be cherry-picked. Re-run the representative only
on that resulting baseline. If still live, compare the Original rider-charge
state/event handler, order construction, animation callback, and replacement
sequencing with the Rust owner. Preserve RNG and actor-order statement order;
do not force the charge state or destination.

## Lane 36 — SuN Save034 elevation/visibility (Task507 reserved)

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_034/replay-010-session-0001.jsonl.zst`

The current frontier is after frame 1581 in elevation/visibility state, but
the responsible pushed-flight takeoff family is currently owned by Task507.
Do not create this lane's worktree or start it from `dc09f516d`. Wait until the
integration owner supplies a new baseline containing Task507's disposition or
explicitly requires that disposition to be cherry-picked, then re-run the
representative on that resulting baseline. If a distinct frontier remains,
identify the first differing actor/projectile and record literal world,
projected-map, ground, old-position, layer/sector, posture, and visibility
query endpoints. Prove whether visibility is downstream of an elevation or
position writer before touching detection. Preserve Original binary32
operation order and the distinction among `Position`, `PositionMap`, and
`PositionGround`; do not add a visibility-only coordinate correction.

## Lane 37 — linux2 Save024 out-of-bounds lifecycle

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_linux2/Profile_002/Savegame_024/replay-007-session-0001.jsonl.zst`

The current frontier is after frame 33695 at an out-of-bounds/topology
boundary. Establish the exact actor or runtime entity, authored map bounds,
fast-grid bounds, active/outside-building status, layer/sector, movement
order, and callback sequence. Compare the Original lifecycle predicate and
its callsite with Rust. Do not conflate authored map bounds with loader padding
and do not deactivate or clamp an entity solely from its recorded position.

## Lane 38 — linux2 Continue RNG cadence

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_linux2/Profile_002/Continue/replay-004-session-0001.jsonl.zst`

The current first boundary is an RNG cardinality/order mismatch after frame
8319. Record the complete frame batch: first draw index, values, Rust logical
sites, Original callsite offsets, selected actor, and all state changes on the
preceding frame. Symbolize offsets only against the compatible Original
binary; reject misleading symbols from another build. Prove the exact
Original owner and gate before editing. Never consume a dummy draw or suppress
the later common tail.

## Lane 39 — SuN Save032 RNG cardinality

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_032/replay-012-session-0001.jsonl.zst`

This is a live, independent RNG-cardinality frontier and the screened backup
for the cleared provisional lanes. Reproduce it on the baseline and record its
current frame and complete RNG batch before doing any source work; older
classifications that label the first shared tail site are not an owner. Trace
the first missing or excess call to its actor/AI/script gate and compare that
gate with Original. Stop with no-fix if compatible callsite provenance or
decisive transient state is unavailable.

## Lane 40 — nicouzouf Save045 Projectile121 water/lifecycle

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_nicouzouf/Profile_001/Savegame_045/replay-003-session-0001.jsonl.zst`

The current independent frontier is after frame 1191 on Projectile121's
water/material/lifecycle state. Record the terminal obstacle identity,
trajectory endpoint, water/hole sector candidates, active/flying/disappear
flags, layer/sector, elevation, movement, and refresh countdown. Compare
Original `ComputeTrajectory`, material resolution, first projectile
`Hourglass`, and refresh order with Rust's corresponding owners. Search Tasks
178, 180, 247, 289, 293, 304, and 320 before editing; this lane must prove a
new source boundary rather than reimplement or contradict those projectile
families. Coordinate with lane 31 before editing: if both identify the same
trajectory, landing-resolution, or first-Hourglass owner defect, this lane
must stop and serve as a control/downstream replay for lane 31 (or vice versa,
if lane 40 proved and claimed the owner first).

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
