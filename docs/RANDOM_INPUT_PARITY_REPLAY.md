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

## Fresh release sweep (`2026-08-01`)

The current short-corpus snapshot contains 700 traces.  It was run with the
frozen release runner whose SHA-256 is
`edbbb6b39c0c8776d8fffb1d2f809f1ea021e453e1668429dfed25f34a831079`.
The machine-readable results are under
`output/parity-audits/random-short-fresh/`.

- 102 traces reached exact EOF.
- 598 traces failed their first authoritative boundary.
- 521 failures are ordinary state comparisons.  Classified by the first
  logical field, they are: 192 `actor.command`, 132 `actor.action_state`, 75
  `other`, 37 `actor.wait_time`, 29 `direction_goal`, 19 `direction`, 10
  `ai.substate`, 9 `position_goal_map.x`, 9 `layer`, 4 `life_points`, 3
  `position_map.x`, 1 `elevation`, and 1 `ai.state`.
- 55 failures are simulation-RNG cardinality/order boundaries.
- 15 traces contain resolved input commands not understood by this frozen
  runner (principally `drop_ale_at` and `shield_select_protected`).
- 6 failures are resolved-speech FIFO/order boundaries.
- 1 trace hit the 900-second watchdog and requires a dedicated hang run.

The groups are first-visible boundaries, not 598 independent engine bugs.
Many traces are expected to collapse after each shared cause is fixed.  Work
is currently split between the command-lifecycle group, the action/wait
timing group, and the comparator `other` group.  Direction/position, RNG,
speech, unsupported resolved commands, and the watchdog trace remain explicit
follow-up groups rather than being silently folded into those assignments.

### Fresh command-lifecycle subgroup ledger

Of the 598 failures, 192 have `actor.command` as their first reported logical
field. The strict first-field cohort groups by its first Original/Rust command
pair as follows:

| Original command | Rust command | Count |
|---|---|---:|
| `EnterHelpingClimb` | `Wait` | 49 |
| `Wait` | `RaiseBow` | 45 |
| `EnterBeggar` | `Wait` | 37 |
| `MoveWaiting` | `MoveOk` | 17 |
| `Wait` | `Turn` | 6 |
| `Wait` | `ShootBow` | 4 |
| `EquipBow` | `Wait` | 4 |
| `ShootBow` | `Turn` | 3 |
| `ParrySword` | `SwordstrikeThrustA` | 3 |
| `MoveOk` | `HitCmd` | 3 |
| `EnterBeggar` | `MoveOk` | 3 |
| `RaiseShield` | `MoveOk` | 2 |
| `QuitSwordfight` | `MoveOk` | 2 |
| `EnterHelpingClimb` | `MoveOk` | 2 |
| Twelve one-off command pairs | mixed | 12 |
| **Total** |  | **192** |

The largest concrete cause is the posture-command lifetime shared by the
`EnterHelpingClimb -> Wait` and `EnterBeggar -> Wait` groups. Original
`RHElementActor::Instruct` first generates a `Bored -> Waiting` transition;
the PC translator then appends the requested stance animation. The stance
body validates only after that prefix has established Waiting, remains the
selected command through the animation, and applies posture/action/tool-state
effects at `MOTION_DONE`. Rust instead revalidated the body against the
still-visible Bored state and marked it Impossible; its successful path also
snapped posture immediately and terminated the element. The general repair
retains the complete prefix/body lifetime for enter/leave beggar and
helping-climb commands and moves their authoritative state effects to the
animation-DONE boundary. No trace IDs, frames, coordinates, or recorded values
participate in the behavior.

The frozen release runner under
`.codex-tmp/random-command-release-20260801/` reran all 49 strict
`EnterHelpingClimb -> Wait` members serially. Every member cleared its
assigned boundary: 7 reached exact EOF and 42 advanced to a later independent
field or RNG frontier. As a cross-command check, Linux2 Profile 002 Save 034
replay 003 (`EnterBeggar -> Wait`) now reaches exact EOF as well. The remaining
36 strict beggar members still need a failure-only sweep before that adjacent
group is closed as a count.

### Fresh action-state and wait-time subgroup ledger

The 132 traces whose first logical field is `actor.action_state` split by the
exact Original/Rust transition as follows:

| Original | Rust | Count |
|---:|---:|---:|
| 1 (`Bored`) | 0 (`Waiting`) | 70 |
| 2 (`Moving`) | 0 (`Waiting`) | 28 |
| 7 (`WaitingSword`) | 8 (`MovingSword`) | 10 |
| 8 (`MovingSword`) | 9 (`MovingFastSword`) | 8 |
| 0 (`Waiting`) | 2 (`Moving`) | 6 |
| 3 (`MovingFast`) | 0 (`Waiting`) | 5 |
| 8 (`MovingSword`) | 2 (`Moving`) | 4 |
| 0 (`Waiting`) | 8 (`MovingSword`) | 1 |

The dominant `Bored/Moving -> Waiting` family contains 88 Whistle, Eat,
EnterListen, or Heal launches: 59 Whistle, 21 Eat, 7 EnterListen, and 1 Heal.
Original `RHElementActorPC::Translate` only appends the corresponding orders;
it neither calls `Stop()` nor changes the actor's logical action state. Rust's
ability launch helpers did both, erasing the pre-command state and path.
Original also uses the one serialized `RHElementActor::mulWaitTime` for both
Whistle and Listen, while Rust had updated only private phase/render mirrors.

The source-backed launch fix preserves the path/action state and synchronizes
the authoritative wait counter. A one-worker release sweep of all 88 affected
launch traces is preserved under
`output/parity-audits/random-action-wait-fix1/`. All 88 advance beyond their
old first boundary: 21 now reach exact EOF, 60 reach a later state/command
boundary, and 7 reach a later RNG-cardinality boundary. Whistle also clears
its subsequent countdown boundary. Eat advances to an independent
command-lifetime mismatch. EnterListen exposed a later countdown divergence;
the source-backed resolution is recorded below. The remaining later frontiers
stay open in their corresponding command, movement, and RNG groups; they are
not failures of the launch fix.

The same release runner also covered the five `MovingFast -> Waiting` ability
launches and all 38 traces that reported `actor.wait_time` without an
action-state mismatch. All five ability launches advance to later boundaries.
Of the 38 wait-only traces, 35 advance (11 to exact EOF, 23 to a later compared
field, and 1 to a later RNG boundary). Three remain unchanged and are separate
timer families: `2 -> 14`, an unrelated stale `25 -> 4294967269`, and the
known one-frame `24 -> 25` termination lag. Thus the shared ability-counter
repair eliminates 128 old first boundaries across the action/wait cohorts; it
does not claim those three independent timer mismatches.

The fresh current-HEAD rerun then exposed the actual EnterListen countdown
failure in two representatives (`21 -> 22`). Original
`RHANIMATION_LISTENING` decrements `mulWaitTime`, ignores the listening
sprite's `DONE`/`TERMINATED` result, and keeps returning `IN_PROGRESS` until
the timer reaches zero. Rust first ran that countdown arm and then also ran
the generic ability animation arm in the same owner slot. The short looping
sprite consequently exhausted after three countdown ticks and advanced to
the exit transition with 22 frames still remaining. The owner envelope now
treats the countdown/detection arm as the complete Listen Execute arm and
does not run generic ability completion while `ListenPhase::CountingDown`.
A one-frame-sprite regression proves that repeated owner ticks retain the
Listening order while the authoritative timer advances `24, 23, 22, 21`.
Both current release representatives clear their old countdown boundary and
advance 21 frames, from 967 to an independent blip-state boundary at 988 and
from 381 to an independent blip-state boundary at 402.
The preserved representative engine dump is
`output/parity-diagnostics/random-listen-countdown/rust-950-967.jsonl`.

The next action-state family was all 10 `WaitingSword -> MovingSword`
mismatches on the first `ReceiveSwordDamage` / `BeingHitSword` frame. Original
implements `BEING_HIT_SWORD`, `EXTRACTING_ARROW_SWORD`, and
`BEING_STUNNED_SWORD` in `RHElementActorHuman::Execute`; their `MOTION_START`
branch applies Upright plus WaitingSword to PCs and soldiers. Rust had put
those transitions in a soldier-only side-effect dispatcher, so PCs retained
their pre-hit MovingSword state. Commit `358af9a7d` moves the shared human
transitions to the universal active-animation path. All 10 release reruns
advance beyond the old boundary (6 to later compared fields and 4 to later RNG
boundaries); results are under
`output/parity-audits/random-action-wait-fix2/`.

All eight `MovingSword -> MovingFastSword` mismatches occurred when
`make_pc_fast` rewrote an already-running directional sword move. Original
mutates the selected `RHOrder` in place, and `FaceOpponent` maps both logical
walking/running sword tokens to the same concrete directional animation; only
the `PerformMotion` method becomes fast. Rust reseeded the order ID, causing a
false sprite `START` and an immediate MovingFastSword state change. Commit
`20a106adc` preserves the live order identity for this exact sword-speed token
pair. All eight release reruns advance beyond their former action-state,
direction, and position boundary; their later results are under
`output/parity-audits/random-action-wait-fix3/`.

Six residual `Moving -> Waiting` boundaries were missing movement Execute
state results: five crouched walk-entry transitions and one corpse-carrying
walk. Original sets `(Crouched, Moving)` when
`TRANSITION_WAITING_CROUCHED_WALKING_CROUCHED` reaches Done/Terminated, and
sets `(CarryingCorpse, Moving)` / `(CarryingCorpse, Waiting)` at the
`WALKING_WITH_CORPSE` Start/Terminated boundaries. Commit `9adde9617` adds
those shared results. All six release reruns advance to later independent
comparisons; artifacts are under
`output/parity-audits/random-action-wait-fix4/`.

### Fresh `other` subgroup ledger

All 75 failures whose only first logical field is `other` have an exact
`selected_pcs` mismatch; there are no miscellaneous comparator messages hidden
in this bucket.  The mutually exclusive cardinality split is:

| Subgroup | Count | Exact first boundary |
|---|---:|---|
| O01 | 72 | Original selected one PC and Rust retained a different single PC |
| O02 | 3 | Original selected one PC and Rust retained two PCs |

O01 and O02's shared cause was inactive indoor-PC selection.  Original
`RHElementActorPC::IsSelectable` checks the PC position interface's stored
`GetSector()->IsBuilding()` value.  Rust instead performed a fresh fast-grid
point query.  At coordinates covered by overlapping sectors, that query can
return a non-building sector and reject the resolved `SelectPc`, leaving the
old selection intact.  Rust now uses the entity's stored sector, matching the
Original predicate.

The one-worker release rerun is preserved under
`output/parity-audits/random-short-other-after-selection/`.  All 75 traces
cleared their former selected-PC boundary with no recurrence: 17 reached exact
EOF and 58 advanced to later independent divergences.  O01 and O02 are
therefore closed as first-boundary groups; the 58 later frontiers belong to
their new command, movement/direction, timer, RNG, speech, or invariant groups
rather than remaining counted as selection failures.

### Resolved-command schema subgroup

The fresh runner initially rejected 15 traces before simulation comparison: 9
contained `drop_ale_at` and 6 contained `shield_select_protected`.  These were
not unknown gameplay operations; both already had authoritative
`PlayerCommand` implementations.  The parity trace decoder now translates the
recorded actor, target/protectee, and running flag into those existing commands.
The variants were appended to `TraceCommand` so existing native bincode cache
discriminants remain stable.

The serial release rerun is preserved under
`output/parity-audits/random-short-resolved-command-after-schema/`.  All 15
traces decode and advance beyond their former parse boundary: 3 reach exact EOF
and 12 expose later independent state, RNG, or speech boundaries.  No command
is skipped or substituted to obtain alignment.

### Resolved-speech FIFO subgroup

The initial short-corpus sweep stopped six traces at the authoritative sound
FIFO assertion.  Five were the same synchronous ordering case: Original
`RHArtificialMalignity::NearbyCiviliansPanic()` completes each eligible
civilian's `Think(EVENT_PANIC)` (and any resulting `Say`) before the initiating
soldier continues to its following `Say(REMARK_STARTS_COMBAT)`.  Keeping that
callback in the AI owner's ordered work FIFO restores the Original civilian,
then soldier sound order instead of batching the civilian reaction afterward.

The sixth trace exposed a separate omission rather than a FIFO policy issue.
Original `RHElementActorHuman::TranslateHitDamage()` synchronously calls
`SayOuch()` before appending its falling-hit order.  `SPEECH_EMERGENCY` thereby
removes a `StartsCombat` exclamation queued earlier in the same engine frame,
before the sound hourglass resolves the replacement `Wounded` sample.  Rust's
hit-damage path now performs the same call for the default posture arm while
retaining Original's silent already-down/flying posture cases.

The serial release rerun is preserved under
`output/parity-audits/random-short-speech-fifo-after-hit-ouch/`.  All 6 traces
clear their former sound boundary with no skipped or reordered resolution: 1
reaches exact EOF and 5 expose later independent direction, movement, command,
or RNG boundaries.

### Stalled-movement RNG subgroup

Nine repeated RNG-cardinality failures appeared at frame 404 or 409 in the
Linux3 Save022 and nicouzouf Save014/Save057 families.  The visible Original
state change was a soldier returning from `DefaultGotoPost` to
`DefaultOnPost`, followed by the expected `GetBoredTime()` draw; Rust emitted
no draw.  The cause was upstream of RNG.  Original's 64-frame stuck recovery
in both `RHArtificialMalignity::The16thFrame` and
`RHArtificialBonhomie::The16thFrame` calls `GoTo` again with the stored
destination and flags.  Rust instead manufactured a raw move order, bypassing
`GoTo`'s destination-sector validation and synchronous already-on-point
`EVENT_REACHPOINT` / `EVENT_DONE` re-entry.

Commit `216617749` routes hostile and civilian retries through the normal
`AiController::go_to` path and mirrors Original's non-null stored-sector test.
The serial release rerun is preserved under
`output/parity-audits/random-rng-stuck-retry/`.  All nine former RNG boundaries
clear: two traces reach exact EOF and seven expose later independent command or
AI-state boundaries.  No recorded draw is consumed merely to realign the
stream.

### Postponed swordfight-entry RNG subgroup

Seven combat traces share a `ReconsiderSwordfight` signature built from
`DrunkCombatFreeze`, sometimes followed by `CombatReposition` and sword-strike
proposal draws.  Three contain true additional reconsideration blocks; four
instead stop one to three draws before Original and therefore require a
separate downstream lifecycle diagnosis.  Original
`RHArtificialMalignity::ReconsiderSwordfight` returns before those draws when
either an `ENTER_SWORDFIGHT` element is registered to launch or the actor's
current sequence element has an immediate postponed `ENTER_SWORDFIGHT`
successor (`original-code/RHartificialmalignity.cpp:13198-13207`).  Rust's two
live AI-context refresh sites checked only the launch queue, so the postponed
form could enter combat reconsideration too early.

Commit `a64f50a46` adds the exact combined query.  It follows only the current
element's intra-sequence or cross-sequence postponed link; it deliberately does
not treat arbitrary postponed work owned by the actor as equivalent.  Both the
normal Think boundary and timer-driven post-detection boundary now use it.  A
focused sequence-manager test proves that an unrelated postponed command does
not satisfy the predicate and the current element's linked successor does.

The direct excess-entry candidates are:

- `Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-001`
- `Savegame_SuN1Sh1nE/Profile_004/Savegame_024/replay-002`
- `Savegame_linux3/Profile_001/Continue/replay-002`

The adjacent short-draw traces retained in the same rerun manifest, but not
claimed as consequences of this gate, are:

- `Savegame_linux/Profile_005/Restart/replay-001`
- `Savegame_linux3/Profile_003/Savegame_031/replay-001`
- `Savegame_nicouzouf/Profile_001/Savegame_039/replay-001`
- `Savegame_nicouzouf/Profile_001/Savegame_039/replay-002`

A current release rerun is preserved under
`output/parity-audits/random-rng-combat/`.  All 18 assigned traces completed
under the 900-second watchdog: 0 reach exact EOF, 3 advance beyond their old
RNG boundary, and 15 reproduce it unchanged.  The three advances are exactly
the direct excess-entry candidates.  Save024 replay 001 advances from frame 68
to an independent `ShootBow -> Turn` command mismatch at frame 105; replay 002
advances from frame 76 to an independent direction-goal mismatch at frame 78;
and Linux3 Continue replay 002 advances from frame 680 to a later RNG boundary
at frame 769.  This confirms the postponed-entry fix without claiming the
later failures.

All four adjacent Rust-short `ReconsiderSwordfight` traces are unchanged, as
expected for an early-return repair, and the other eleven combat-tagged traces
also reproduce their original strike/opponent-lifecycle RNG boundary.  Those
15 remain open until their first excess or missing authoritative call is
source-mapped.  The status split is two ordinary comparison exits and sixteen
RNG assertion exits; none are interrupted or watchdog results.

The 15 unchanged boundaries split by cardinality into 11 Rust-overdraw and 4
Rust-underdraw traces.  Ten overdraw traces reach the post-proposal
`EvaluateSwordfight` tail and contain `MeleeStepBack`; six of those continue to
`SmalltalkStrikeSide`.  Original performs those operations only after its
swordfight, opponent, animation, initiative, range, and proposal gates
(`original-code/RHelementactorhuman.cpp:8268-8456`), so this family is now
localized to an earlier control-flow/ownership disagreement rather than the
RNG implementation itself.  The ten tail members are Linux2 Save029;
nicouzouf Save069; Linux3 Profile 003 Save012; randomguy Restart; Linux3
Profile 001 Save008 replays 001 and 002 and Save010 replay 002; and SuN
Save032, Save034, and Save036.  Linux2 Save002 is the separate `+1` mixed
idle/smalltalk frame.

The four underdraw traces are Linux Profile 005 Restart (`-1`), Linux3 Profile
003 Save031 (`-1`), and nicouzouf Save039 replays 001 (`-3`) and 002 (`-1`).
They remain a distinct missing-call family: the postponed-entry early return
cannot explain them, and no synthetic draw will be added to align their
streams.

The long `60s-15x` snapshot contains 1,791 traces and is being swept with one
runner process to keep memory bounded.  Its totals are provisional until all
status files exist under `output/parity-audits/random-long-fresh/`.

## Provisional initial sweep

The first release filter started when 309 complete schema-12 random traces
existed. Generation continued past 323 traces while it ran, so later files
require an incremental pass. The early sample is failure-heavy because random
commands exercise incomplete PC command lifecycles quickly; this is expected
and is useful coverage, not evidence that the recorder is invalid.

At the first checkpoint, 46 traces had completed: 9 exact and 37 divergent.
The sweep continued after that checkpoint. Use the audit status directory for
the live count until a frozen input snapshot has completed.

### Checkpoint `2026-07-31-update-001`

The second status snapshot is preserved at
`.codex-tmp/schema12-random-release-audit-1c02368c8/checkpoints/2026-07-31-update-001/`.
It contains 182 completed audit statuses: 32 exact and 150 divergent. There
were 362 complete compressed random traces available when this update began;
generation and the initial audit both continued after the checkpoint.

This remains a baseline for the frozen `1c02368c8` runner. In particular it
predates the random-input fix `516728654`, so the 57 R01 entries below are
candidate failure-only reruns, not claims about current `main`.

Every one of the 150 failures is accounted for by the following first-visible
classification. These are triage groups, not yet assertions that every member
has the same root cause.

| Group | Count | First visible boundary |
|---|---:|---|
| R01 | 57 | `actor.action_state`, sometimes with another field |
| R02 | 23 | `direction_goal`, without an action-state mismatch |
| R03 | 8 | `actor.wait_time`, without an action-state mismatch |
| R04 | 4 | `position_goal_map` |
| R05 | 19 | command/posture/direction combinations |
| R06 | 7 | `ai.substate` |
| R07 | 7 | RNG draw cardinality or ordering |
| R08 | 3 | `position_map` |
| R09 | 7 | resolved sound with no Rust pending request |
| R10 | 7 | resolved-sound FIFO/content mismatch |
| R12 | 8 | four `other` comparator reports, three layer/sector reports, and one unclassified panic |
| **Total** | **150** | |

### Live sweep update `2026-07-31-update-002`

The frozen runner has completed 302 of its original 309-trace input snapshot:
43 exact and 259 divergent. Generation has independently reached 413 complete
compressed traces. These are deliberately separate numbers: the 104 traces
created after the runner's input snapshot require an incremental pass.

The initial sweep is not yet declared complete. One shard is inside
`Savegame_linux3/Profile_001/Savegame_003/replay-002-session-0001.jsonl.zst`
under its 900-second watchdog, with six later snapshot entries queued behind
it. A partial total must not be promoted to a final pass rate, and a watchdog
termination must be recorded as its own failure rather than silently omitted.

The 302 results still use frozen commit `1c02368c8`. They therefore predate
`516728654` and all later work. Their 259 failures are a discovery baseline,
not the current engine's expected failure count.

## Shared-cause ledger

| ID | Status | First visible family | Working interpretation / next proof |
|---|---|---|---|
| R01 | Fix landed; rerun required | PC `actor.action_state`, commonly Original idle versus Rust `WalkingUpright` while both retain `MoveOk` | `516728654` preserves Waiting when an entity-target `PerformSeek` remains visibly in progress. All four assigned boundaries clear; one trace is exact and three reach independent later failures. The remaining 53 baseline entries require failure-only reruns before this group can be split or closed. |
| R02 | Fix landed; cohort rerun required | `direction_goal`, frequently at frame 516 | `7db74f013` makes shared AI and owner-ordered `RefreshPatrol` snapshots expose an active PassDoor member at its committed gate side, and commits the exact endpoint before later owner slots can observe it. Linux2 Profile 002 Savegame 002 is exact through frame 726. The remaining baseline members require current failure-only reruns before this whole group can be attributed to that cause. |
| R03 | Listen fix landed; timer rerun required | `actor.wait_time`, alone or beside action state | Original uses the single serialized `mulWaitTime` for Whistle and Listen. Rust now synchronizes its phase-local mirrors and, while Listening, ignores sprite completion until that counter reaches zero. The prior sweep's three unchanged timer pairs (`2 -> 14`, stale `25 -> 4294967269`, and `24 -> 25`) still require current-fix reruns; do not normalize any timer in the comparator. |
| R04 | Audited; current cohort rerun required | `position_goal_map` | The fresh 700-trace frozen baseline contains nine strict first-field members. They split into two replacement-preserve boundaries, three stop-clear boundaries, three unresolved `MoveOk` waypoint/order-destination differences, and one direct attentive-transition initialization boundary. `7910b1c7d` likely covers replacement preservation, while later identity-aware condolence and direct-transition work may cover the clear/attentive members; none may be called closed until all nine are rerun on one current frozen release runner. The waypoint geometry remains unresolved. |
| R05 | Unassigned | `actor.command` with posture/direction | Inspect wrapper versus concrete command lifetime and the action-change marker that commits posture. |
| R06 | Audited; current cohort rerun required | `ai.substate` | The fresh 700-trace frozen baseline has ten strict first-field traces: two shadow-entry boundaries, four special-strike entry/exit boundaries, two shield-protection entries, and two heard-steps/group-ordering boundaries. Every divergent frame has no resolved command and an aligned simulation-RNG batch, so these are not input or RNG-cardinality failures. Later special-strike commits are likely relevant; shadow predetection/patrol dispatch, shield-entry predicates, and heard-steps delivery ordering remain unresolved. |
| R07 | Unassigned | RNG cardinality/order | Treat the first missing or excess call as a downstream symptom until the responsible Original callsite and state gate are identified. Never consume a trace value merely to realign the stream. |
| R08 | Unassigned | `position_map` | Requires exact movement increment, collision, transition, and command ownership comparison; no coordinate tolerance or replay-specific snap. |
| R09 | Unassigned | Resolved speech has no pending Rust request | Separate genuinely absent gameplay `Say` calls from already-fixed synchronous speech boundaries before changing restoration or FIFO policy. |
| R10 | Unassigned | Resolved speech disagrees with pending FIFO | Compare actor, exclamation, forced/random variant, and the synchronous callback that queued it. Never skip an event to realign the stream. |
| R11 | Fix landed; rerun required | Runtime entity creation/mapping | `516728654` exposed two bow arrows whose Rust identities were one early. Save adoption reused beam-me PCs without consuming the provisional construction orders that Original's dynamic load path consumes. The fix restores those invisible counter increments; both exposed arrows now map at their Original identities. |
| R12 | Selection subgroup fixed | Comparator `other`, layer/sector, or unclassified panic | The fresh sweep's 75 `other`-only failures were all `selected_pcs`: O01=72 one-to-one wrong selection and O02=3 one-to-two. The stored-sector fix eliminated all 75 first boundaries; 17 traces now reach exact EOF and 58 expose later independent groups. Layer/sector identity remains a separate isomorphic comparison concern. |

## First-divergence ledger

This table is the first human extraction made while generation and the release
sweep were still running. It is not the complete checkpoint-001 list; the
complete set is preserved by the checkpoint status files and accounted for by
the group totals above. Newly investigated failures must be appended here or
folded into a proven shared cause. Names use `__` in place of path separators,
matching audit log filenames.

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

### R04 complete frozen-baseline inventory

The fresh 700-trace sweep has exactly nine traces whose first reported logical
field is `position_goal_map.x`. The following values and classifications are
evidence from that frozen runner, not claims about current `main`. Every member
must receive a current release-mode rerun. A trace is closed only if it reaches
exact EOF; clearing this boundary but finding a later mismatch moves it to the
new family instead.

| Family | Trace | Frame | Entity | Original goal | Frozen Rust goal | Resolved command boundary |
|---|---|---:|---|---|---|---|
| Stop clears outgoing goal | `Savegame_Cyrdach/Profile_156/Savegame_010/replay-001-session-0001.jsonl.zst` | 536 | `Pc(PcId(107))`, Original raw PC 57 | `(0, 0)` | `(536.9613, 447.9872)` | Select PC 57; select Shield |
| Replacement preserves outgoing goal | `Savegame_SuN1Sh1nE/Profile_004/Savegame_004/replay-003-session-0001.jsonl.zst` | 15812 | `Pc(PcId(345))` | `(1785, 1047)` | `(0, 0)` | Select PC 345; select Guzzle; launch Eat; cancel action |
| Replacement preserves outgoing goal | `Savegame_SuN1Sh1nE/Profile_004/Savegame_009/replay-003-session-0001.jsonl.zst` | 22383 | `Pc(PcId(345))` | `(1466, 551)` | `(0, 0)` | Select PC 345; select Guzzle; launch Eat; cancel action |
| Active `MoveOk` waypoint | `Savegame_linux3/Profile_001/Savegame_001/replay-002-session-0001.jsonl.zst` | 2704 | `Pc(PcId(126))` | `(1549.6039, 281.21027)` | `(1532.9346, 270.1589)` | None |
| Active `MoveOk` waypoint | `Savegame_linux3/Profile_001/Savegame_031/replay-001-session-0001.jsonl.zst` | 480 | `Pc(PcId(125))`, Original raw PC 75 | `(1373.1996, 661.0308)` | `(1373.169, 659.03107)` | None |
| Stop clears outgoing goal | `Savegame_linux3/Profile_002/QuickSave/replay-001-session-0001.jsonl.zst` | 19932 | `Pc(PcId(126))` | `(0, 0)` | `(2689, 993)` | Select PC 126; select Bow; shoot Soldier 80 |
| Active `MoveOk` waypoint | `Savegame_linux3/Profile_003/Savegame_007/replay-001-session-0001.jsonl.zst` | 3056 | `Pc(PcId(192))` | `(2105.0537, 1620.7255)` | `(2114.4731, 1617.3673)` | None |
| Attentive transition initialization | `Savegame_nicouzouf/Profile_001/Savegame_055/replay-002-session-0001.jsonl.zst` | 374 | `Soldier(SoldierId(63))` | `(1183.0403, 743.6907)` | `(0, 0)` | None |
| Stop clears outgoing goal | `Savegame_randomguy/Profile_004/Savegame_008/replay-002-session-0001.jsonl.zst` | 137 | `Pc(PcId(101))`, Original raw PC 51 | `(0, 0)` | `(1592.3058, 636.51764)` | Select PC 51; select Bow |

These nine traces collapse into four source-backed families:

1. **Replacement preserves the outgoing goal (two traces).** Both SuN1Sh1nE
   traces have the same resolved command batch: select PC 345, select Guzzle,
   launch command 105 `Eat`, then cancel the UI action. Original changes from
   active `MoveOk` to `Eat` on that frame while retaining `(1785, 1047)` and
   `(1466, 551)` respectively; only `Eat`'s selected terminal card clears the
   goal on the following frame. `RHElementActor::Instruct` assigns
   `mpSequenceElement = pNewSequenceElement` before interrupting the outgoing
   element (`RHelementactor.cpp:830-950`), so its synchronous
   `SendCondolationCard` fails the selected-pointer identity check and cannot
   clear the goal (`RHelementactor.cpp:6162-6201`). Rust routes
   `LaunchSelfAbility(Eat)` through `InterruptCurrent`, exposes the incoming
   element during the synchronous callback, and commit `7910b1c7d` explicitly
   marks the outgoing card unselected. The call and helper remain unchanged on
   current HEAD; later Stop traversal and postponed-registration changes do not
   alter this replacement boundary. This is one exact general source-backed
   candidate for both traces, but they still require current exact reruns and
   EOF. The validated Whistle replacement family is supporting evidence, not a
   substitute.

2. **A stop clears the still-selected outgoing goal (three traces).** The
   Cyrdach Shield selection and both Bow selections interrupt a live or
   just-completed movement. The frozen Rust runner retains the prior goal while
   Original exposes zero. Bow selection calls `Stop` in
   `RHengine.cpp:12991-13065` and `RHengine.cpp:14456-14475`;
   `RHElementActor::Stop` and `RHSequenceElementMovement::StopMovement` lead to
   the selected element's synchronous condolence, and
   `RHElementActor::SendCondolationCard` clears the sprite goal only when that
   element is still selected (`RHelementactor.cpp:6102-6201` and
   `RHSequenceElementMovement.cpp:442-495`). The matching Rust path is resolved
   action selection in `engine/selection.rs`, `EngineInner::stop_owner`, exact
   current-element interruption in `engine/sequence.rs`, and identity-aware
   goal cleanup in `engine/soldier_helpers.rs::send_condolation_card` using the
   terminal card's captured `was_selected` state. Commits `2f251e446`,
   `4cafa337a`, and `9f25e53e1` landed and generalized that cleanup, while
   `a627bf0d8` aligned `Actor::Stop` traversal with Original's authoritative
   current element. This is one general source-backed fix candidate for all
   three traces, not trace-specific handling. It was already present when this
   frozen inventory was written, however, so source proof does not close the
   cohort: all three exact traces remain **rerun required** on one current
   release runner and close only at EOF.

3. **The active `MoveOk` order has a different destination (three traces).** In
   linux3 Savegame 001 and Savegame 007, the prior goal equals the prior current
   position and the divergent frame installs a new internal waypoint. In
   Savegame 031, a stopped actor begins an internal `MoveOk`; the targets differ
   by about `0.03` in X and `2.0` in Y. There is no resolved input command at
   any boundary, and Savegame 001 has no path event in frames 2702-2704.
   Original `RHSprite::PerformMotion` copies the current
   `RHOrder::pointDestination2D` into `position_goal_map` when a new order ID is
   observed (`RHsprite.cpp:1393-1480`). The disagreement therefore already
   exists in waypoint construction, path post-processing, target snapping, or
   order replacement; it is not a goal-cache cleanup bug. Commit `0294404c7`
   can reseed a stale Rust goal from the Rust order, but cannot make a differing
   Rust order destination equal Original. This geometry/order-production
   family remains unresolved and must be investigated from the producing
   movement order backward.

4. **Zero-destination attentive transition initialization (one trace).** The
   Soldier 63 trace begins `EnterAttentiveMode` animation 141. Original creates
   a transition order with a zero destination
   (`RHelementactorsoldier.cpp:330-353`) and executes it through
   `PerformMotion(TILL_LAST_FRAME)`. New-order initialization replaces an exact
   zero destination with the actor's current map position before installing the
   sprite goal (`RHsprite.cpp:1445-1457`). Frozen Rust left the goal zero.
   Subsequent direct-transition initialization and stale-goal work may already
   cover this rule, but the exact trace must be rerun before assigning it to a
   landed fix.

The current rerun should preserve these four labels while reporting, for each
trace, either exact EOF or its next independent first boundary. Do not merge
the three waypoint values with stop/condolence fixes merely because the first
compared field is the same.

### R06 complete frozen-baseline inventory

Exactly ten traces in the fresh 700-trace sweep report `ai.substate` as their
first logical field. All ten divergent frame records have `commands=[]`. Their
RNG batches also have equal Original/Rust cardinality and order; every listed
draw belongs to the simulation domain. The values and raw Original callsite
offsets below are diagnostics for the surrounding work, not evidence of an RNG
fault. These remain frozen-runner results until a current release runner either
reaches exact EOF or exposes a later independent boundary.

| Family | Trace | Frame | Entity and exact state pair | Boundary RNG batch: `first_index; values; callsite_offsets` |
|---|---|---:|---|---|
| Shadow entry: predetection or patrol dispatch | `Savegame_Nescafe/Profile_001/Restart/replay-003-session-0001.jsonl.zst` | 282 | Soldier 210: Original `11 DefaultOnPost`; Rust `25 DefaultLookingShadow` | `2057; [880120884, 982608437, 1110424676, 833723432]; [1400943, 1202746, 1400943, 1400943]` |
| Shadow entry: predetection or patrol dispatch | `Savegame_Nescafe/Profile_003/Restart/replay-003-session-0001.jsonl.zst` | 282 | Soldier 210: Original `11 DefaultOnPost`; Rust `25 DefaultLookingShadow` | `2083; [1472100780, 726318879, 1393065843, 1688743570, 988560093, 110908859]; [1657414, 1400943, 1202746, 1400943, 1400943, 1400943]` |
| Special-strike entry | `Savegame_SuN1Sh1nE/Profile_004/ExQuickSave/replay-001-session-0001.jsonl.zst` | 35580 | Soldier 97: Original `161 AttackingSwordfightSpecialStrike`; Rust `160 AttackingSwordfight` | `866; [1672075245, 2015418716, 1725795122, 1975707409, 665708928, 347268203, 1112813156]; [1121462, 1657414, 1400943, 1400943, 1441112, 1441162, 1802982]` |
| Special-strike entry | `Savegame_SuN1Sh1nE/Profile_004/ExQuickSave/replay-002-session-0001.jsonl.zst` | 35580 | Soldier 97: Original `161 AttackingSwordfightSpecialStrike`; Rust `160 AttackingSwordfight` | `865; [1315349388, 1672075245, 2015418716, 1725795122, 1975707409, 665708928, 347268203, 1112813156]; [1121462, 1805845, 1657414, 1400943, 1400943, 1441112, 1441162, 1802982]` |
| Special-strike entry | `Savegame_SuN1Sh1nE/Profile_004/ExQuickSave/replay-003-session-0001.jsonl.zst` | 35580 | Soldier 97: Original `161 AttackingSwordfightSpecialStrike`; Rust `160 AttackingSwordfight` | `866; [1672075245, 2015418716, 1725795122, 1975707409, 665708928, 347268203, 1112813156]; [1121462, 1657414, 1400943, 1400943, 1441112, 1441162, 1802982]` |
| Shield-protection entry | `Savegame_SuN1Sh1nE/Profile_004/Savegame_001/replay-002-session-0001.jsonl.zst` | 606 | Soldier 181: Original `183 AttackingProtectingWithShield`; Rust `155 AttackingRunningToEnemy` | `1903; [612615160]; [1115513]` |
| Special-strike exit | `Savegame_SuN1Sh1nE/Profile_004/Savegame_034/replay-002-session-0001.jsonl.zst` | 585 | Soldier 129: Original `160 AttackingSwordfight`; Rust `161 AttackingSwordfightSpecialStrike` | `1907; [2143148521, 613175802, 970680938]; [1115513, 1805845, 1400943]` |
| Heard-steps/group ordering | `Savegame_linux3/Profile_001/Savegame_018/replay-001-session-0001.jsonl.zst` | 27759 | Soldiers 132 and 133: Original `248 SeekingHeardstepsPreReactiontime`; Rust `71 SeekingHeardstepsReactiontime` | `718; [812411044, 984367401]; [1657350, 1400751]` |
| Heard-steps/group ordering | `Savegame_linux3/Profile_001/Savegame_018/replay-002-session-0001.jsonl.zst` | 27738 | Soldiers 132 and 133: Original `248 SeekingHeardstepsPreReactiontime`; Rust `71 SeekingHeardstepsReactiontime` | `699; [1237659534, 896103280]; [1657350, 1657350]` |
| Shield-protection entry | `Savegame_linux3/Profile_003/Savegame_054/replay-001-session-0001.jsonl.zst` | 534 | Soldier 205: Original `183 AttackingProtectingWithShield`; Rust `155 AttackingRunningToEnemy` | `1683; [248395451, 1861723036, 1448197855]; [1115577, 1401007, 1657926]` |

The callsite offsets resolve, depending on the Original build, to ordinary
`The16thFrame`, `DefaultBoredStandardProcedure`, actor-hourglass,
`WillStopAtNextWaypoint`, `StopAll`, `EvaluateSwordfight`,
`ReconsiderSwordfight`, and `EstimateDamageOfSwordStrike` work. None of these
batches has a missing or excess draw. Preserve the global draw stream as-is
while investigating the following four behavior families:

1. **Shadow entry: predetection or patrol dispatch (two traces).** Decoding the
   frozen comparator cache corrects the earlier timer-expiry diagnosis:
   Original Soldier 210 is already `DefaultOnPost` before frame 278 and remains
   there through frame 282, while frozen Rust newly enters
   `DefaultLookingShadow` at frame 282. Original `HandlePredetection` tests
   `(uwSharpness > 0) && (suspects[type] >= threshold)` against the accumulator
   from before the current scan, updates the per-detectable shadow latch, and
   queues `EVENT_SEES_SHADOW` only on a rising edge
   (`RHelementactornpc.cpp:1531-1550, 2007-2076`). Both engines then offer that
   event to whole-patrol dispatch before running the local shadow standard
   procedure (`RHartificialmalignity.cpp:6249-6254, 20017-20103`). The first
   boundary is therefore either a Rust-only predetection edge or a difference
   in patrol chief/member topology, 360-degree detection, or dispatch result;
   the later shadow timer and `max_visibility` handler cannot explain the first
   state transition.

   The current trace schema cannot distinguish those cases. At frames 279-282,
   record for Soldier 210 every scanned detectable's type and logical target,
   integer sharpness, suspect accumulator before the scan, `seen_now`,
   `seen_last_frame`, `shadow_seen_last_frame` before and after the update,
   `last_visibility`, and whether `EVENT_SEES_SHADOW` was queued. At delivery,
   record receiver identity, `to_whole_patrol`, pre-event state/substate,
   patrol chief and ordered member identities, each relevant 360-degree
   detection result, and the final patrol-dispatch result. Also retain the
   aggregate suspect array, maximal suspect, and maximal visibility so the
   diagnostic remains useful if the first differing predicate moves. This path
   draws no RNG.

2. **Special-strike state entry and exit (four traces).** The three ExQuickSave
   traces show a missed or late entry into `AttackingSwordfightSpecialStrike`;
   Savegame 034 shows the inverse, with Rust retaining SpecialStrike after
   Original has returned to ordinary Swordfight. Original's ordinary
   swordfight handler reconsiders on `EventReachPoint`, `EventDone`, or
   `EventTimer`; a successful proposal changes state before `StopAll` and the
   strike sequence launch. The SpecialStrike handler returns to Swordfight only
   on `EventDone` or `EventTimer` (`RHartificialmalignity.cpp:3964-4009`, with
   proposal sites around lines 13525 and 14942). Commits `13777adf8` and
   `ad1c7d4b8` implement the explicit legacy state and delayed completion
   ordering and are likely fixes. Both the three entry traces and the inverse
   exit trace require current exact reruns; one direction does not prove the
   other.

3. **Shield-protection entry (two traces).** Original
   `RefreshArrowProtection` admits `AttackingRunningToEnemy`, scans visible bow
   threats and friendly archers, then either runs to a phalanx slot or performs
   `StopAll`, raises the shield, and enters `AttackingProtectingWithShield`
   (`RHartificialmalignity.cpp:17057-17186`). Frozen Rust remains in
   `AttackingRunningToEnemy`. The difference must therefore precede the state
   write: eligible-fighter snapshot contents, the seen-last-frame latch,
   distance tests, friendly-archer count, or phalanx placement. Neither frame
   contains the random ShieldAdvance draw. `5341bdb03` and later shield commits
   are relevant but do not prove these exact members.

4. **Heard-steps entry and synchronous group ordering (two traces, four state
   mismatches).** Both Savegame 018 recordings disagree for Soldiers 132 and
   133 simultaneously; Soldiers 134 through 139 also acquire Original direction
   goal 11 versus Rust 10 on that frame. Original
   `EventHearStandardProcedure` enters `SeekingHeardstepsReactiontime` directly
   only when the listener is already Seeking, is not in `SeekingGotStopEvent`,
   and is not an officer. Otherwise it enters
   `SeekingHeardstepsPreReactiontime` (`RHartificialmalignity.cpp:8520-8605`).
   Thus Original Pre versus Rust Reaction is evidence that the same noise was
   handled against a different visible pre-event state or at a different
   callback boundary, not an enum-decoding error. The simultaneous two-state
   and six-facing changes point to synchronous multi-NPC noise/group delivery
   order. Compare sender, listener creation order, and every listener's state
   immediately before its nested `Think`; do not assign a replacement state or
   consume an RNG value to align it.

Current validation must rerun all ten traces with one frozen release binary.
Record exact EOF or the new first boundary independently for every trace; do
not close R06 from focused tests or source similarity alone.

## Fix ledger

### `7db74f013` — use committed door-side positions in AI

Original `RHArtificialIntelligence::Position(actor)` returns the movement
element's complete gate-side `RHposition` while its selected command is
`PASS_DOOR`: `GetDirection()` chooses `PositionIn` or `PositionOut`, including
the associated sector and layer. `RefreshPatrol` uses that helper for every
member-distance comparison, so it must not observe the sprite's interpolating
door-rail coordinate.

Rust's shared AI views now apply the same full door-side override throughout
the active translated PassDoor chain. Owner-slot projection preserves that
override, AI self contexts and patrol snapshots consume it, and both ordinary
and transition-resume completion paths commit the exact final endpoint before
a later actor slot can observe the member. This is consistent with Original
zero-tolerance motion completion, which snaps the sprite to its position goal
before terminating the order.

Linux2 Profile 002 Savegame 002 clears the obsolete patrol-coordinate boundary
and matches every recorded frame through frame 726. A current source audit
found the implementation complete and source-correct; no further behavior
change is justified without a new first boundary. The remaining R02 baseline
members still require failure-only reruns on current `main` to determine which
share this cause and to split any independent later direction-goal families.

### `7910b1c7d` — preserve movement goals across replacement interruption

Original `RHElementActor::Instruct` installs the incoming element as
`mpSequenceElement` before setting the outgoing element to Interrupted.  The
outgoing synchronous `SendCondolationCard` therefore observes that it is no
longer selected and must not clear the sprite movement goal.  Rust formerly
derived `was_selected` from its in-progress index while the replacement was
still `Todo`, incorrectly giving the outgoing card ownership of that cleanup.

Replacement arbitration now marks that exact interruption boundary as
unselected. The focused condolence regression passes. A current release build
validates the complete eight-trace Whistle-during-movement family: all eight
clear their `position_goal_map` mismatch. Six advance three frames to an
independent Whistle transition action-state mismatch, while two retain only an
independent wait-counter mismatch on the old frame:

- Linux2 Profile 002 Savegame 033 replay 001 advances from frame 23127 to an
  action-state mismatch at frame 23130.
- Linux2 Profile 002 Savegame 035 replay 001 retains frame 38243, now with only
  `actor.wait_time` mismatching.
- Linux2 Profile 002 Savegame 036 replay 001 advances from frame 53554 to an
  action-state mismatch at frame 53557.
- Linux2 Profile 002 Savegame 037 replay 001 advances from frame 58716 to an
  action-state mismatch at frame 58719.
- Linux3 Profile 001 Savegame 023 replay 001 advances from frame 54231 to an
  action-state mismatch at frame 54234.
- Linux3 Profile 001 Savegame 034 replay 002 advances from frame 29210 to an
  action-state mismatch at frame 29213.
- Linux3 Profile 001 Savegame 035 replay 002 retains frame 50147, now with only
  `actor.wait_time` mismatching.
- Linux3 Profile 001 Savegame 036 replay 001 advances from frame 33192 to the
  action-state mismatch at frame 33195.

An earlier hypothesis that the goal was erased by final movement-transition
completion was rejected by a release replay and reverted before commit.  The
actual clear occurred at the earlier replacement-interruption condolence.

### `f52e92a53` — face throwable targets in world ground coordinates

Original `RHEngine::PerformOrientation` builds the apple, stone, net,
wasp-nest, and purse facing vector as resolved target world XY minus the
actor's `GetPositionGround()` world XY.  Rust had subtracted the actor's
projected map position instead.  Because map Y is world Y minus elevation,
every elevated thrower was aiming at an artificial vertical offset.

Resolved throwable orientation now uses the actor's world position before the
existing isometric sector classification.  The focused regression places an
elevated actor and target at the same world Y, which must face east.  The
assigned Linux3 Profile 001 Savegame 041 replay 002 boundary clears from frame
37294 and advances to an independent RNG-cardinality boundary at frame 37345.
A one-worker release rerun of the complete 32-trace geometry frontier is
preserved under
`output/parity-audits/random-short-geometry-after-world-ground/`.  All 32
statuses are complete: 17 clear their former boundary (4 exact EOF and 13
later independent divergences), while 15 remain unchanged and therefore form
separate movement, command, timer, elevation, and turning families.

### `8e541e882` — face officer attack points in world coordinates

Original `RHArtificialMalignity::CommandSoldiersToAttack` computes the
officer's `POINT` direction from `PositionToPoint3D(mposSeekPosition)` minus
the officer's world-space `GetPosition()`. Rust had instead classified the
raw projected-map delta, so elevation was interpreted as a north/south target
offset. The attack-point sequence now resolves both sides in world XY while
retaining projected-map coordinates for Original's separate 150-unit MaxNorm
gate.

Both repeated Soldier 139 boundaries clear in a release build:

- Linux3 Profile 003 Continue replay 001 advances from frame 13619 to an
  independent multi-archer `ShootBow` versus `Turn` family at frame 13684.
- Linux3 Profile 003 Savegame 025 replay 001 advances from frame 13587 to the
  same independent family at frame 13647.

The old direction/direction-goal mismatch is absent in both traces. A focused
regression covers an elevated officer and target sharing the same world Y;
the officer must point due east rather than classify their projected-map Y
difference.

### `caeaa0b3b` — turn throughout PC beggar animations

Original `RHElementActorPC::Execute` calls `Turn()` before advancing the
sprite action in all three beggar animation arms: the transition into the
disguise, `SIMULATING_BEGGAR` itself, and the transition back to upright.
Rust omitted that family from its per-animation turn dispatch. Consequently,
an Apple orientation issued while leaving the disguise advanced only once in
Rust, while Original advanced once in `PerformOrientation` and once more when
executing `TRANSITION_SIMULATING_BEGGAR_WAITING_UPRIGHT` in the same tick.

The complete source-backed three-arm family is now included for PCs and has a
focused classification regression. A combined release runner validates both
assigned repetitions through exact EOF:

- Linux3 Profile 001 Savegame 046 replay 001 clears its old frame-59372 PC 344
  direction mismatch and matches every recorded frame.
- Linux3 Profile 001 Savegame 047 replay 001 clears its old frame-83142 PC 344
  direction mismatch and matches every recorded frame.

The nearby Linux3 Profile 001 Savegame 043 replay 002 PC 343 boundary is an
independent `EnterHelpingClimb` transition-turn case already repaired by
`18af339fd`; it is not part of the beggar family.

### `516728654` — preserve Waiting for entity-target PC seeks

Original `WalkingUpright::Execute` observes an entity-target `PerformSeek` as
`IN_PROGRESS` and therefore retains the actor's Waiting state. Rust's authored
transition-successor marker synthesized a delayed Start and forced Moving one
frame later. The fix suppresses that deferred PC movement Start for
entity-target seeks while retaining it for point-target continuations.

The focused regression passes. All four initially assigned R01 boundaries
clear:

- Cyrdach Restart replay 002 now matches every recorded frame.
- Cyrdach Restart replay 001 advances past frame 349 to an independent missing
  projectile with Original creation order 159.
- Cyrdach Savegame 000 replay 001 advances past frame 496 to a sword-walk
  divergence at frame 725.
- Cyrdach Savegame 000 replay 002 advances past frame 230 to an independent
  missing projectile with Original creation order 161.

The checkpoint-001 statuses were produced by the older frozen runner and stay
unchanged as historical evidence. A new release runner must rerun all R01
members before publishing the number eliminated by this fix.

The three later R01 frontiers now form two new work items:

- Original projectile creation orders 159 and 161 are assigned together for a
  source-backed runtime-creation diagnosis.
- The frame-725 sword-walk divergence remains separate until its first
  command/animation ownership boundary is identified.

### Beam-me load construction orders — runtime projectile identity

Original `RHEngine::PopulateBeamMes` records the static creation boundary
before constructing the selected team. During `SerializeElements` load, each
saved team PC is consequently reconstructed, consuming a provisional
`gulCreationCounter` value before its serialized identity is restored. Rust
reuses the already initialized profile-matched PC, so save adoption must
advance the counter explicitly for every reused beam-me PC.

The focused counter regression covers one- and multi-PC teams plus overflow.
The two frontiers exposed after `516728654` now create their arrows with the
correct identities:

- Cyrdach Restart replay 001 maps Original creation order 159 and advances to
  an independent sword-walk divergence at frame 578.
- Cyrdach Savegame 000 replay 002 maps Original creation order 161 and matches
  every recorded frame.

### `f9bd4bfe4` — consume eager registration when postponing

The fresh 700-trace sweep's sole watchdog member was
`Savegame_linux3/Profile_001/Savegame_002/replay-001-session-0001.jsonl.zst`.
Focused stage probes narrowed its apparent frame-11465 `SelectAction(Purse)`
hang to the synchronous `UnequipBow` launch. The complete diagnostics are
preserved under
`output/parity-hang-diagnostics/linux3-save002-r001/`: `command-stage.log`
first isolated command application, `stop-recursion.log` proved both Stop
passes returned, and `launch-arbitration.log` captured the recursive edge and
stack overflow.

The invalid postponed self-link was created at runtime, not by Linux save
adoption. At frame 11415, synchronous launch correctly postponed the new
`EquipBow` element `(7896, 0)` behind non-interruptable `PassDoor` `(7883,
3)`, but Rust left the new element's eager manager registration queued. The
Sequences phase instructed the same waiter a second time, saw it already in
the blocker's postponed slot, and recursively postponed `(7896, 0)` behind
itself. A later `UnequipBow` launch followed that cycle until stack overflow.

Original cannot produce that second instruction: `RHSequenceManager::Hourglass`
removes the element from `mlistSequenceElementsToGo` before `Go`/`Instruct`
(`original-code/RHsequencemanager.cpp:938-951`), and
`RHSequenceElement::Postpone` runs inside that boundary
(`original-code/RHsequenceelement.cpp:636-694`). Rust now consumes any eager
deferred or synchronous registration when an element becomes `Postponed`, and
`engine_postpone` asserts the impossible blocker-equals-waiter invariant. The
focused queue regression and the existing injury/postponement regressions
pass.

A normal release-mode 120-second validation no longer hangs or reaches the
watchdog. It advances through frame 11465 and stops normally at the next
independent divergence after frame 11681: PC 126 `actor.wait_time` is 14 in
Original and 0 in Rust. The authoritative result is
`output/parity-hang-diagnostics/linux3-save002-r001/postpone-registration-fix.log`
with status `1` in the sibling `.status` file. The old watchdog group is
therefore closed; frame 11681 is now tracked as an R03 timer frontier.

### `31f787917` + `6cd51b94e` — preserve the complete WaitTimer scalar lifecycle

The frame-11681 timer frontier was the previously unresolved R03 `2 -> 14`
family member. Original PC 126 counted `RHCOMMAND_WAIT_TIMER` from 44 to 14,
then `SelectAction(Purse)` interrupted it into `RHCOMMAND_WAIT` while leaving
`RHElementActor::mulWaitTime` at 14. Original initializes that one scalar in
WaitTimer translation (`original-code/RHelementactor.cpp:3332-3337`) and
decrements it only while the selected command remains WaitTimer
(`original-code/RHelementactor.cpp:610-623`); interruption and ordinary Wait
translation do not clear it.

Rust splits the overloaded Original member into `wait_time` and
`seek_refresh_wait`. WaitTimer translation wrote the timer only to the first
and cleared the second. Once the command changed to Wait with dormant
post-seek ownership still present, the isomorphic legacy view selected the
cleared mirror and reported 0. Commit `31f787917` writes the timer to both
representations and adds a focused interruption regression. Its first replay
validation correctly exposed the other half of the lifecycle at frame 11401:
after an earlier WaitTimer reached zero and handed off to PassDoor, the new
mirror still held 18. Commit `6cd51b94e` therefore synchronizes every positive
WaitTimer decrement and the terminating zero boundary as well. Focused tests
cover initialization, decrement-to-zero, the extra zero frame, and
interruption into Wait; completion/interruption themselves correctly perform
no scalar write.

The combined release validation clears both frame 11401 (`Original=0`, old
Rust=`18`) and frame 11681 (`Original=14`, old Rust=`0`). It advances normally
to a new independent boundary after frame 11860, where PC 126 is `Wait` in
Original and `RaiseBow` in Rust. The authoritative validation is
`output/parity-hang-diagnostics/linux3-save002-r001/waittimer-lifecycle-fix.log`
with status `1` in its sibling `.status` file. R03's `2 -> 14` member is
closed; frame 11860 belongs to the command-lifecycle/RaiseBow family.

### `3c83560e` (Original) + `750224e6f` (Rust) — reject invalid bow-height classifications

The frame-11860 `Wait`/`RaiseBow` boundary was not sequence scheduling or an
LOS-obstacle mismatch. PC 126 was genuinely playing animation 89
(`RHANIMATION_AIMING_WITH_BOW`), but the recorded target lay outside the bow
range cone. Original `CanShootWithBowAt` returns `OUT_OF_RANGE` before assigning
its `ShootType` out-parameter (`original-code/RHelementactorhuman.cpp:7041-7111`).
`AimWithBowAt` ignored that return value and switched on the uninitialized local
(`original-code/RHelementactorhuman.cpp:6924-6957`). In this recording the
undefined value happened not to request a height transition, while Rust treated
its placeholder `Long` mode as authoritative and launched `RaiseBow`.

Original commit `3c83560e` makes the API boundary deterministic by returning
from `AimWithBowAt` unless classification produced `VALID_TARGET`. Rust commit
`750224e6f` applies the same contract before launching `RaiseBow` or `LowerBow`.
The focused Rust regression
`engine::input::tests::out_of_range_bow_aim_does_not_change_bow_height` passes
and retains the valid-long transition case.

The release validation clears frame 11860 and reaches EOF: `parity trace matched
every recorded frame`. The authoritative artifact is
`output/parity-hang-diagnostics/linux3-save002-r001/bow-command-origin-trace-v2.log`.
This completes the entire 751-frame Savegame_linux3/Profile_001/Savegame_002
replay after the watchdog, WaitTimer, and bow-classification fixes.

#### Frozen 45-trace `Wait` / `RaiseBow` candidate cohort

The fresh 700-trace baseline also has 45 strict first-command members with
Original `Wait` and frozen Rust `RaiseBow`. Frame-record inspection ties all 45
to resolved bow orientation: 44 contain `orient_action_at(action=bow)` on the
first-divergent frame. The sole timing variant is
`Savegame_nicouzouf/Profile_001/Savegame_033/replay-003-session-0001.jsonl.zst`:
Select PC/Bow is at frame 747, resolved orientation at 748, and the queued
`RaiseBow` first becomes visible at 749. Fourteen members require normal
Original/Rust PC-ID isomorphism; this does not form a behavior exception.

| Trace root | Exact replay members and first-divergent frames |
|---|---|
| `Savegame_SuN1Sh1nE/Profile_004` | `Savegame_001/replay-001` 212; `Savegame_011/replay-001` 116; `Savegame_016/replay-001` 173; `Savegame_019/replay-001` 667; `Savegame_021/replay-001` 149 |
| `Savegame_linux2/Profile_002` | `QuickSave/replay-001` 2012; `Savegame_001/replay-001` 891; `Savegame_017/replay-001` 142; `Savegame_029/replay-001` 3984; `Savegame_031/replay-001` 11724; `Savegame_041/replay-001` 1996; `Savegame_042/replay-001` 6834; `Savegame_042/replay-002` 6847 |
| `Savegame_linux3/Profile_001` | `Savegame_003/replay-001` 199; `Savegame_025/replay-001` 18509; `Savegame_028/replay-001` 10107; `Savegame_028/replay-002` 10120 |
| `Savegame_linux3/Profile_003` | `ExQuickSave/replay-001` 74565; `Savegame_000/replay-001` 9305; `Savegame_005/replay-001` 54699; `Savegame_014/replay-001` 3807; `Savegame_015/replay-001` 7731; `Savegame_018/replay-001` 17320; `Savegame_035/replay-001` 22203; `Savegame_037/replay-001` 26616; `Savegame_038/replay-001` 29146; `Savegame_043/replay-001` 7643; `Savegame_050/replay-001` 2272; `Savegame_050/replay-002` 2285; `Savegame_052/replay-001` 4143; `Savegame_052/replay-002` 4156; `Savegame_071/replay-001` 4540; `Savegame_072/replay-001` 38869 |
| `Savegame_nicouzouf/Profile_001` | `Savegame_002/replay-001` 181; `Savegame_022/replay-001` 129; `Savegame_022/replay-002` 175; `Savegame_026/replay-001` 632; `Savegame_033/replay-001` 644; `Savegame_033/replay-002` 115; `Savegame_033/replay-003` 749; `Savegame_035/replay-001` 107; `Savegame_037/replay-001` 255; `Savegame_071/replay-001` 163; `Savegame_075/replay-001` 99 |
| `Savegame_randomguy/Profile_004` | `Savegame_002/replay-001` 16587 |

At each causal orientation record the Original PC is in action state 4, plays
animation 89 (`AIMING_WITH_BOW`), and has at least one arrow. Wrong action
state, wrong animation, pending-shot gating, and empty quivers are therefore
not cohort explanations. The resolved records contain target XYZ, but not the
computed `RHBowTarget`, shoot mode, hand hotspot, bow range, or LOS result.
Available geometry is only a diagnostic proxy: target-to-actor-base planar
distance spans 252.843 to 1280.835 (mean 526.390), with 3 at most 300, 11 in
`(300, 400]`, 21 in `(400, 600]`, and 10 above 600. Target Z minus actor base
elevation is below for 10, equal for 19, and above for 16 (range -305 to
+219.925). These values must not be mislabeled as hand-to-target range-cone
classification.

The source signature makes `750224e6f` the shared candidate: before that
commit, Rust launched `RaiseBow` from the placeholder `Long` mode without
requiring `bow_status == Valid`; the deterministic Original contract returns
before interpreting shoot type for every non-`VALID_TARGET` result. Only the
separate frame-11860 exemplar above has a captured `OutOfRange` status and EOF
validation, however. The frozen schema does not prove that status for these 45
members. Consequently none is closed by source similarity or the focused test:
rerun all 45 on one current frozen release runner and require exact EOF. Any
survivor becomes a `Valid + Long` classifier/range/hotspot/LOS investigation,
not command-lifecycle evidence and not justification for bypassing the guard.

When another fix lands, add the commit, Original source boundary, focused test,
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
