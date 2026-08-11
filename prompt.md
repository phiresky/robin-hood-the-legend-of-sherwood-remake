# External parity lanes 21–30

Work from baseline `7890a5983` (main after the second
`external-parity-11-20-integration` merge). The objective is to advance the
authoritative corpus toward 100% parity with source-faithful fixes.

Run each lane in its own branch/worktree, then merge reviewed commits into
`external-parity-21-30-integration`. A source-proven no-fix or already-cleared
result is valid and should not create a commit.

## Global rules

- Read `AGENTS.md`; never use `git stash`.
- Always consult `./original-code` before changing behavior.
- Every representative below is an authoritative member of the current
  4,715-trace snapshot and was reproduced with frozen current runner
  `7890a5983`. Reproduce it again on the lane branch.
- Build and run separately. Do not pipe/redirect Cargo and do not run clippy.
- Record the exact first boundary, all state differences or RNG sites, and the
  responsible entity/AI/sequence owner.
- Do not infer a fix from serialized output alone. If decisive Original
  transient state is unavailable, report no-fix instead of guessing.
- Never return fake defaults for missing required data; fail with identity and
  context.
- Prefer one narrow production boundary plus a focused regression. Re-run the
  representative and a listed control where present.
- Run focused tests, `cargo fmt --all -- --check`, and `git diff --check`.
- Remove diagnostics before committing. Commit only lane-owned files; do not
  edit shared campaign/archive/retirement files from lane branches.
- Search completed archives and lanes 11–20 before editing. Stop if the root
  cause is already owned or integrated.

## Internal exclusions

Fifteen coordinator sessions are concurrently working on these families;
external lanes must not touch them:

- nicouzouf P001 Save065 replay015 post-1370;
- schema-14 linux3 P003;
- nicouzouf P001 Save039 replay008 post-596;
- SuN P004 Save024 replay004;
- schema-12 linux3 P001 and linux2 P002 Save035;
- old/short nicouzouf Save037;
- `c45e0feed` failed-path barrier, campaign/progress tooling, and candidate
  inventory.

Completed lanes 11–20 are also excluded: orphan-sword Turn ordering, Save024
replay011 RNG, the second-damage triplet, civilian panic/bored-time residuals,
AlertOfficer door forecast, schema14 Nescafe Save001 replay001 seek/path,
Nescafe Save000 replay013 Smalltalk, Save037 replay004 detectable/state, and
retired schema12 view-query captures.

## Lane 21 — Nescafe restart/continue initial Turn cohort

Primary:
`parity-random-save-replays-60s-15x/traces/Savegame_Nescafe/Profile_002/Restart/replay-001-session-0001.jsonl.zst`

Control: same profile, `Continue/replay-001-session-0001.jsonl.zst`.

All 15 Restart seeds share the frame-7 signature and all 15 Continue seeds
share it at frame 16. Original Soldier164/173 is Wait, animation3, substate11;
Rust is Turn, animation50, substate8, with a one-sector direction-goal shift.
Treat all 30 as one family. Find the startup/loaded sequence owner that authors
the Rust Turn; do not force the serialized direction.

## Lane 22 — SuN Save016 LowerBow completion

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_016/replay-009-session-0001.jsonl.zst`

At frame318 PC134 is Original LowerBow/animation91 versus Rust Wait/animation92.
Replay010 is separately retired and must not be used as a control. Trace the
LowerBow sequence/order completion and action-state restoration on both sides;
do not force the animation or resurrect retired projectile behavior.

## Lane 23 — cross-corpus PC direction/row projection

Primary:
`parity-random-save-replays-60s-15x/traces/Savegame_Nescafe/Profile_002/Savegame_016/replay-007-session-0001.jsonl.zst`

Second-corpus control:
`parity-random-save-replays-60s-15x/traces/Savegame_Cyrdach/Profile_156/Savegame_015/replay-007-session-0001.jsonl.zst`; replay011 in that save is an additional control.

At frame685 Nescafe PC104 is direction Original3/Rust2 and sprite row147/146.
At frame1017 Cyrdach PC108 is Original7/Rust6 and sprite151/150; replay011
repeats at frame1442. Exclude retired Cyrdach replay009 and
source-inconsistent replay005. Treat these as one family unless source proof
identifies different writers. Prove whether row is purely downstream, locate
the exact restored-facing or direction-vector/aspect owner, and preserve
Task36 controls. Do not add a row-only correction.

## Lane 24 — SuN Save036 elevation/movement ownership

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_036/replay-006-session-0001.jsonl.zst`

At frame300 PC318 has Original elevation93.3318 and moving=false versus Rust
elevation90.00101 and moving=true. Same-save replays001 and002 are exact-EOF
controls on the frozen baseline. Identify the exact flight, vertical movement,
or movement-completion writer; retain literal binary32 operation order and do
not snap elevation or clear movement from serialized output alone. Do not use
the later failing replays007,009,011 as controls; they belong to other active or
completed families.

## Lane 25 — Nescafe P001 Rust-only seek burst

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_Nescafe/Profile_001/Restart/replay-005-session-0001.jsonl.zst`

At frame 892 Rust consumes 185 SeekPointSelection draws plus acceptance before
the common VIP tail. Task249 cleared an earlier frame792 boundary and recorded
this as independent. Identify the AI owner and why Rust enters SeekArea while
Original does not; do not suppress/cap the draw loop.

## Lane 26 — SuN Save038 soldier action restoration

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_038/replay-015-session-0001.jsonl.zst`

At frame935 Soldier131 is Original action Walking/animation6 versus Rust
NoAction/animation5; the concurrent path event for Soldier134 matches on both
sides. Same-save replays001 and002 are exact-EOF controls on the frozen
baseline. Trace Soldier131's own action/sequence restoration and do not
attribute the boundary to the unrelated matching Soldier134 path event. Do not
use replay006 as a control; its later melee reaction boundary overlaps internal
Task345.

## Lane 27 — Cyrdach low-bit movement/view arithmetic

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_Cyrdach/Profile_156/Restart/replay-015-session-0001.jsonl.zst`

Control: `Savegame_000/replay-015-session-0001.jsonl.zst`. At frame1294
Soldier70 movement_map.y differs by `0x400` in binary32 representation, while
four visibility-destination Y values each differ by one ULP; the control
repeats at frame1462. Establish whether both arise from a shared Original
operation order or only the query coordinates share an owner. Do not add an
epsilon comparator.

## Lane 28 — Nescafe P003 single-sector direction

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_Nescafe/Profile_003/Savegame_000/replay-014-session-0001.jsonl.zst`

At frame1419 Soldier73 direction is Original6/Rust5. Identify the live
FaceTo/Turn/movement writer and its aspect conversion. Do not use P002 Save000
as a control; that save belongs to completed lane18 and has different melee
state. Stop if lane23 proves this Soldier boundary uses the same conversion
owner; do not implement the same fix twice.

## Lane 29 — identical path events, different movement state

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_Nescafe/Profile_001/Savegame_000/replay-002-session-0001.jsonl.zst`

At frame1421 Soldier76 is Original MoveOk/animation51 versus Rust
MoveWaiting/Freezing292 although both record the same two queued path events.
Audit ProcessPathRequests timing, request identity, and the owner callback
after the integrated `c45e0feed` failed-path barrier. Treat that commit as an
immutable control: prove this divergence is outside or downstream of it, and
do not modify its synchronous failed-path boundary. Do not force MoveOk or
ignore a request.

## Lane 30 — SuN Save008 movement completion

Representative:
`parity-random-save-replays-60s-15x/traces/Savegame_SuN1Sh1nE/Profile_004/Savegame_008/replay-012-session-0001.jsonl.zst`

At frame1431 Soldier158 is Original MoveOk/animation303 versus Rust
Wait/animation54. Prove which movement/sequence element Original retains or
promotes and whether the difference is completion, condolence, or replacement
ordering. Preserve path-request and owner-boundary ordering.

## Required report

For every lane report: branch/baseline; trace count and current result;
representative and first boundary; Original and Rust file/line boundaries;
source-faithfulness/distinctness or no-fix rationale; files/tests; replay,
build, format, and diff results; commit hash or explicit no-commit disposition.

The integration owner must review every commit, merge only proven changes, run
the full engine suite and optimized parity build, update shared bookkeeping,
and produce `external-parity-21-30-integration`.
