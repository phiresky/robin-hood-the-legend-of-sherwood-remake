# Random-input parity replay ledger

This is the evergreen human ledger for parity traces produced with
`-PARITYRANDOMINPUT`. It complements `ORIGINAL_PARITY_REPLAY.md`, which owns
the recorder format, deterministic simulation contract, save adoption, and
general Original-versus-Rust behavior notes.

Random-input traces are valuable because they repeatedly issue resolved player
commands after loading many different saves. They expose command interruption,
replacement, movement, posture, combat, and ability boundaries that a silent
250-frame save replay cannot reach. A failure here must still be fixed from the
general Original behavior; trace-specific entity IDs, coordinates, frames, RNG
values, or command substitutions are not acceptable fixes.

## Canonical artifacts

- Traces: `parity-random-save-replays/traces/`
- Generator logs: `parity-random-save-replays/logs/`
- Initial release audit:
  `.codex-tmp/schema12-random-release-audit-1c02368c8/`
- Frozen runner:
  `.codex-tmp/schema12-release-1c02368c8/original_parity_replay`
- Frozen runner commit: `1c02368c8`
- Frozen runner SHA-256:
  `62049fb5b3a58f13b0788fa714c46cafea29dc0af5595e00b3bd90b9612dc174`

The audit directory contains one log and one numeric status file per completed
run. Those files are the machine-readable authority. This document records the
meaning and disposition of failures. Generation is intentionally allowed to
continue during an audit, so every reported total must name the input snapshot
or say that it is provisional.

## Audit procedure

1. Build the runner in release mode and freeze the binary under a commit-named
   directory. Never rebuild a binary in place during one count.
2. Snapshot the sorted set of complete `*.jsonl.zst` traces. Ignore raw JSONL
   files still being written.
3. Run the first filter with `--no-auto-dump`. Record exit status and preserve
   stdout/stderr separately for every trace.
4. Retire exact passes. Group failures by their first authoritative boundary,
   not merely by the first compared field.
5. Rerun failures with automatic surrounding-frame dumps enabled and consult
   `original-code` before changing Rust behavior.
6. After a general fix, rerun every member of its group. A trace moves to Done
   only after reaching EOF exactly; advancing to another divergence remains In
   progress.
7. Periodically build a new frozen release runner and perform a failure-only
   sweep. Run a complete fresh sweep before publishing final totals.

## Provisional initial sweep

The first release filter started when 309 complete schema-12 random traces
existed. Generation continued past 323 traces while it ran, so later files
require an incremental pass. The early sample is failure-heavy because random
commands exercise incomplete PC command lifecycles quickly; this is expected
and is useful coverage, not evidence that the recorder is invalid.

At the first checkpoint, 46 traces had completed: 9 exact and 37 divergent.
The sweep continued after that checkpoint. Use the audit status directory for
the live count until a frozen input snapshot has completed.

## Shared-cause ledger

| ID | Status | First visible family | Working interpretation / next proof |
|---|---|---|---|
| R01 | In progress | PC `actor.action_state`, commonly Original idle versus Rust `WalkingUpright` while both retain `MoveOk` | Largest cluster. Determine the exact Original selected-movement/animation completion boundary under repeated resolved moves. Assigned as one general lifecycle issue, not one fix per trace. |
| R02 | In progress | `direction_goal`, frequently at frame 516 | Likely shares the synchronous patrol/door/turn boundary already exposed by silent Linux2 Save002. Prove whether PC random-command cases share that cause before merging groups. |
| R03 | Unassigned | `actor.wait_time`, alone or beside action state | Compare the owning command and timer launch/termination frame. Do not normalize the timer in the comparator. |
| R04 | Unassigned | `position_goal_map` | Audit whether the outgoing selected command is detached, postponed, or retained. Existing Halt and raising-sword fixes are relevant but not assumed sufficient. |
| R05 | Unassigned | `actor.command` with posture/direction | Inspect wrapper versus concrete command lifetime and the action-change marker that commits posture. |
| R06 | Unassigned | `ai.substate` at frame 282 | Compare synchronous command side effects and owner-local AI callback ordering. |
| R07 | Unassigned | RNG cardinality/order | Treat the first missing or excess call as a downstream symptom until the responsible Original callsite and state gate are identified. Never consume a trace value merely to realign the stream. |
| R08 | Unassigned | `position_map` | Requires exact movement increment, collision, transition, and command ownership comparison; no coordinate tolerance or replay-specific snap. |

## First-divergence ledger

This table is the first human extraction made while generation and the release
sweep were still running. New failures discovered after this extraction must be
appended or folded into a proven shared cause. Names use `__` in place of path
separators, matching audit log filenames.

| Trace | Frame | First compared boundary |
|---|---:|---|
| `Cyrdach__Profile_156__Restart__replay-001` | 349 | `actor.action_state` |
| `Cyrdach__Profile_156__Restart__replay-002` | 83 | `actor.action_state` |
| `Cyrdach__Profile_156__Savegame_000__replay-001` | 496 | `actor.action_state` |
| `Cyrdach__Profile_156__Savegame_000__replay-002` | 230 | `actor.action_state` |
| `Cyrdach__Profile_156__Savegame_001__replay-001` | 429 | `actor.action_state` |
| `Cyrdach__Profile_156__Savegame_001__replay-002` | 516 | `direction_goal` |
| `Cyrdach__Profile_156__Savegame_001__replay-003` | 358 | `direction_goal` |
| `Cyrdach__Profile_156__Savegame_010__replay-001` | 536 | `position_goal_map` |
| `Cyrdach__Profile_156__Savegame_010__replay-002` | 751 | `actor.action_state`, `actor.wait_time` |
| `Cyrdach__Profile_156__Savegame_010__replay-003` | 365 | `actor.action_state`, direction |
| `Cyrdach__Profile_156__Savegame_015__replay-002` | 808 | `actor.action_state` |
| `Cyrdach__Profile_156__Savegame_015__replay-003` | 760 | `actor.action_state`, `actor.wait_time` |
| `Cyrdach__Profile_156__Savegame_023__replay-002` | 1120 | Original has one additional RNG draw |
| `Nescafe__Profile_001__Restart__replay-003` | 282 | `ai.substate` |
| `Nescafe__Profile_001__Savegame_000__replay-001` | 496 | `actor.action_state` |
| `Nescafe__Profile_001__Savegame_000__replay-002` | 230 | `actor.action_state` |
| `Nescafe__Profile_001__Savegame_001__replay-001` | 385 | `actor.action_state` |
| `Nescafe__Profile_001__Savegame_001__replay-002` | 516 | `direction_goal` |
| `Nescafe__Profile_001__Savegame_002__replay-001` | 1416 | `actor.action_state` |
| `Nescafe__Profile_001__Savegame_002__replay-002` | 1319 | `actor.action_state`, `actor.wait_time` |
| `Nescafe__Profile_001__Savegame_002__replay-003` | 1162 | `actor.action_state`, `direction_goal` |
| `Nescafe__Profile_001__Savegame_007__replay-002` | 172 | `actor.action_state` |
| `Nescafe__Profile_001__Savegame_007__replay-003` | 426 | `actor.wait_time` |
| `Nescafe__Profile_001__Savegame_015__replay-002` | 318 | command and posture |
| `Nescafe__Profile_001__Savegame_015__replay-003` | 270 | `actor.action_state`, `actor.wait_time` |
| `Nescafe__Profile_002__Continue__replay-003` | 501 | command and `direction_goal` |
| `Nescafe__Profile_002__Restart__replay-001` | 192 | five `direction_goal` mismatches |
| `Nescafe__Profile_002__Restart__replay-003` | 507 | Rust has two excess RNG draws; building-exit/idle cluster |
| `Nescafe__Profile_002__Savegame_000__replay-001` | 429 | `actor.action_state` |
| `Nescafe__Profile_002__Savegame_000__replay-002` | 163 | `actor.action_state` |
| `Nescafe__Profile_002__Savegame_001__replay-001` | 397 | `actor.action_state` |
| `Nescafe__Profile_002__Savegame_001__replay-002` | 516 | `direction_goal` |
| `Nescafe__Profile_002__Savegame_001__replay-003` | 326 | `direction_goal` |
| `Nescafe__Profile_002__Savegame_005__replay-001` | 474 | `position_map` |
| `Nescafe__Profile_002__Savegame_005__replay-003` | 253 | `actor.action_state` |
| `Nescafe__Profile_002__Savegame_016__replay-001` | 278 | `actor.wait_time` |
| `Nescafe__Profile_002__Savegame_016__replay-002` | 584 | `actor.action_state`, `actor.wait_time` |
| `Nescafe__Profile_003__Restart__replay-003` | 282 | `ai.substate` |
| `Nescafe__Profile_003__Savegame_000__replay-001` | 369 | `actor.action_state` |
| `Nescafe__Profile_003__Savegame_001__replay-001` | 375 | `actor.action_state` |
| `Nescafe__Profile_003__Savegame_004__replay-003` | 225 | `actor.wait_time` |

## Fix ledger

No random-input failure has been marked Done yet. The first active cluster is
R01. When a fix lands, add the commit, Original source boundary, focused test,
all affected traces, and their new exact result or next independent frontier
here. Do not silently delete old rows: mark them superseded by the fix so the
historical coverage remains visible.

## Maintenance checklist

- Update the available/completed/audited totals only from complete compressed
  traces and per-trace status files.
- Add newly observed first divergences before beginning speculative debugging.
- Merge rows into a shared cause only after source or state-dump evidence.
- Record every committed fix and every full-trace EOF validation.
- Keep generated caches, logs, dumps, and screenshots out of Git.
- Preserve old frozen audit directories; create a new commit-named directory
  for each production runner.
- When generation stops, run one incremental pass for traces absent from the
  initial snapshot, then a complete final release sweep.
