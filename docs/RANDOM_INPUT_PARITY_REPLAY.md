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
| R04 | Audited; partial current validation | `position_goal_map` | The fresh 700-trace frozen baseline contains nine strict first-field members. The apparent three-way `MoveOk` group has separate producers. `fff5ecdcc` fixes Original's literal 40-unit `SwordstrikeDown` seek and its exact Savegame 007 frame-3056 boundary now clears, advancing to an independent Soldier 94 `ai.substate` mismatch at frame 3313. The six-trace current release cohort now has one exact EOF and five advances to independent later boundaries; the former Cyrdach stop and nicouzouf attentive-transition holdouts clear with `ec57536a3`. Details and logs are recorded below. |
| R05 | Unassigned | `actor.command` with posture/direction | Inspect wrapper versus concrete command lifetime and the action-change marker that commits posture. |
| R06 | Audited; shield and shadow pairs cleared | `ai.substate` | The fresh 700-trace frozen baseline has ten strict first-field traces. `34e4810d7` restores the live ordered `seen_last_frame` Enemy detectable projection for periodic `RefreshArrowProtection`; both shield-entry boundaries clear on the current release runner and advance to independent command mismatches. `aaebc38c9` removes two non-Original per-tick detectable reconciliation loops; both Nescafe shadow boundaries clear from frame 282 to independent frame-507 RNG cardinality failures. Special-strike entry/exit and heard-steps delivery ordering remain open. |
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
   candidate for both traces. Current validation clears both frozen boundaries:
   Savegame 004 advances to an independent later position mismatch and Savegame
   009 reaches exact EOF. The validated Whistle replacement family remains
   supporting evidence for the same general selection ordering.

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
   current element. The remaining delayed-card fault was that Rust captured
   `was_selected` at `SetState` but then reinterpreted it when the queued card
   was dispatched after a new Wait had become current. Commit `ec57536a3`
   makes that terminal-time identity authoritative; replacement arbitration
   continues to record `false` explicitly when Original selects the incoming
   element first. Commit `fd248c2ff` updates the focused fixtures to exercise
   that explicit replacement-selected path. All three frozen stop boundaries
   now clear: the two earlier Bow traces advance as recorded below, while
   Cyrdach advances from frame 536 to the independent frame-1126 command
   mismatch, PC 108 Original `MoveOk` versus Rust `MoveWaiting`.

3. **The three apparent active-`MoveOk` waypoints have three distinct
   producers.** None is a pathfinder waypoint. In linux3 Savegame 001 frame
   2704, classical `EnterSwordfight` refreshes a direct entity-target seek.
   Relative to the same target snapshot, Original installs a point exactly 50
   units away while the frozen Rust runner installs one exactly 30 units away.
   Original gets this distance from the selected PC's constructor-owned
   `mpSword->GetRange(DEFAULT)` (`RHelementactorsoldier.cpp:2011-2018`); current
   Rust resolves the PC character profile's HtH weapon default distance in
   `engine/commands.rs`. Both Robin profiles in the current profile data resolve
   to 50, so the frozen 30 cannot be attributed to current nominal data. The
   creation site now emits an opt-in trace containing PC profile index, raw HtH
   weapon ID, movement animation, and resolved seek distance. A current exact
   rerun must establish whether this landed profile/save work already clears the
   trace. If not, add the same values plus chase-speed classification and
   effective tolerance at each `RefreshSeek`; the parity schema currently has
   no seek-parameter event.

   In Savegame 031 frame 480, a stopped PC begins the generated
   `TransitionWaitingUprightWalkingUpright` start order. Original's destination
   is exactly 4 units from the pre-start position and frozen Rust's is exactly
   6 units away. Original and Rust `InsertTransitionStart` geometry agree; the
   differing input is `RHSprite::GetDistanceForAnimation` versus Rust
   `Sprite::distance_for_animation`. A rerun or diagnostic must capture the
   active sprite profile/cache key, mapped row, row `sum_distance`, and the
   pre/post transition order list to identify why Original maps this actor's
   transition to 4 while frozen Rust maps it to 6.

   In Savegame 007 frame 3056, `SwordstrikeDown` creates another direct entity
   seek. Both goals lie on the same target ray, exactly 40 units away in
   Original and 30 in frozen Rust. Original passes literal 40 to
   `AddInteractionWithSeek` (`RHelementactornpc.cpp:4261` onward), while Rust's
   generic interaction fallback returned 30. Rust now maps
   `Command::SwordstrikeDown` explicitly to 40 with a focused regression test.
   This is an exact general source fix, pending a release-mode exact rerun to
   EOF (or its next independent boundary).

   Current validation confirms the focused
   `swordstrike_down_uses_original_literal_seek_distance` test passes and the
   release comparator builds successfully. The exact Savegame 007 trace clears
   its old frame-3056 `position_goal_map` boundary and advances to frame 3313,
   where Soldier 94 has Original `ai.substate=1` versus Rust `2`. The old
   SwordstrikeDown boundary is therefore closed; the trace remains open at that
   independent frontier. The preserved log is
   `output/parity-audits/r06-shield-swordstrike-current-head/linux3-profile003-save007-replay001.log`.

4. **An attentive transition retains the completed movement goal (one
   trace).** Soldier 63 finishes `MoveOk` at exactly
   `(1183.040283, 743.690674)`, with both its current position and sprite goal
   equal to that point, then begins `EnterAttentiveMode` animation 141. Original
   does create the transition order with a default zero destination
   (`RHelementactorsoldier.cpp:330-353`), but the Soldier Execute arm dispatches
   `RHANIMATION_TRANSITION_WAITING_UPRIGHT_WAITING_ALERTED` through
   `RHSprite::PerformAction` (`RHelementactorsoldier.cpp:762-775`). That entry
   point never writes `PositionGoalMap`, so the prior completed `MoveOk` goal is
   retained. The frozen Rust runner instead exposed zero. This explicitly
   retracts the earlier `PerformMotion` diagnosis: there is no zero-to-current
   fallback in Original. `RHSprite::PerformMotion` installs
   `pOrderCurrent->pointDestination2D` verbatim on a fresh motion order
   (`RHsprite.cpp:1445-1457`) and would therefore install zero if this order were
   routed through it. Commit `b86a53d25` landed the general structural rule that
   selected non-movement orders execute through the generic action owner;
   `184e0bd5d` hardened that routing with an explicit Original Execute-arm
   catalog classifying this exact Soldier transition as `GenericAnimation`.
   Commit `95c376973` is complementary: it permits creation of the attentive
   transition while movement is postponed, but is not itself the goal-retention
   fix. Current generic dispatch calls Rust `Sprite::perform_action`, which also
   leaves the map goal untouched. The remaining zero came from a Rust-only
   `set_soldier_attentive_mode` special case that cleared the goal whenever
   `StopMovement` had rewritten the front order to a waiting transition.
   Original's subsequent `POSTPONE_CURRENT` selects the attentive element and
   calls movement `Postpone`, which clears orders and restores `MoveOk` to
   `Move` without a condolence card; the goal therefore survives. Commit
   `ec57536a3` removes that unconditional clear. The exact trace clears frame
   374 and advances to an independent sound-manager invariant at frame 480 ->
   481: exclamation 55 for actor 51 has no pending request.

The current rerun should preserve these four labels while reporting, for each
trace, either exact EOF or its next independent first boundary. Do not merge
the three waypoint values with stop/condolence fixes merely because the first
compared field is the same.

#### R04 six-trace current release validation

The current release runner was applied sequentially to six source-backed R04
candidates. The original cohort logs are preserved under
`output/parity-audits/r04-current-head/`; focused owner-handoff diagnostics are
under `output/parity-diagnostics/r04-owner-handoff-current/`, and the fresh
post-fix reruns are under
`output/parity-validation-2026-08-01/r04-owner-handoff-after-fix/`. One trace
reaches exact EOF and five clear their frozen R04 boundary before an independent
later frontier. The complete movement-goal regression filter passes 7/7 and the
fresh release parity runner builds successfully.

| Trace | Current result | Disposition |
|---|---|---|
| `Savegame_Cyrdach/Profile_156/Savegame_010/replay-001` | Old frame 536 clears; frame 1126 PC 108 command Original `MoveOk`, Rust `MoveWaiting` | Advanced |
| `Savegame_linux3/Profile_002/QuickSave/replay-001` | Old frame 19932 clears; frame 19968 PC 126 command Original `Wait`, Rust `ShootBow` | Advanced |
| `Savegame_randomguy/Profile_004/Savegame_008/replay-002` | Old frame 137 clears; frame 671 PC 105 direction Original `1`, Rust `2` | Advanced |
| `Savegame_SuN1Sh1nE/Profile_004/Savegame_004/replay-003` | Old frame 15812 clears; frame 16062 Soldier 203 position X Original `8.944593`, Rust `8.944144` | Advanced |
| `Savegame_SuN1Sh1nE/Profile_004/Savegame_009/replay-003` | Every recorded frame matches | **Exact EOF** |
| `Savegame_nicouzouf/Profile_001/Savegame_055/replay-002` | Old frame 374 clears; frame 480 -> 481 sound-manager invariant: exclamation 55 for actor 51 has no pending request | Advanced |

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

1. **Shadow entry: extra Rust detectable membership (two traces, fixed).**
   Targeted predetection and delivery traces first ruled out the former timer,
   patrol, latch-restoration, threshold, and cadence theories. Soldier 210's
   Rust-only shadow edge came from PC 342: sharpness began at frame 275 and the
   pre-scan suspect reached the threshold at frame 282. The authoritative
   Original frame dump gives the decisive membership proof. At frames 270-278,
   Soldier 210 has exactly 111 Enemy detectables, all Soldiers 97-255, and no PC
   detectable; consequently Original never issues a visibility query for
   Soldier 210 to PC 342. Rust had synthesized PC 342 by frame 1.

   Original `RefreshDetection` only iterates the existing serialized list;
   initialization and explicit `AddDetectable` paths own membership changes.
   Rust instead had two global reconciliation loops in `detection.rs`: the
   acoustic pass appended every missing PC and the optical pass appended every
   missing eligible PC/soldier on every tick. Commit `aaebc38c9` removes both
   loops, preserving loaded-save and explicit runtime membership exactly. The
   focused `detection_tick_preserves_authoritative_enemy_membership` regression
   opens both acoustic and optical phases and proves an absent PC stays absent;
   that test and the release parity-runner build pass.

   Both frame-282 shadow boundaries now clear. Profile 001 advances to frame
   507, where Rust consumes draws 3624-3632 while Original ends at 3630; the
   extra-call cluster is two `RuntimeBuildingExitWait` calls followed by
   `VipIdleRemark`/`BoredAnimationChoice` and further idle remarks. Profile 003
   likewise advances to frame 507, where Rust consumes draws 3686-3694 while
   Original ends at 3692, ending with the analogous idle cluster and an
   `AiRandomValueRectangle` call. These are independent R07 frontiers, not a
   reason to restore the non-Original membership reconciliation. Preserved
   rerun logs are under
   `output/parity-audits/r06-after-detectable-membership/`; the large targeted
   diagnostics that established the source proof remain under
   `output/parity-audits/r06-shadow-diagnostics/`.

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
   write. The exact missing input was the seen-last-frame latch: periodic
   `The16thFrame` builds its context through
   `build_npc_tick_data_without_forecasts`, whose generic tick-data builder
   populated nearby fighters but left `seen_last_frame_enemies` empty. Both
   frozen traces contain one ordered, latched, live bow threat beyond the
   150-unit minimum. `34e4810d7` now projects the live Enemy detectable list in
   Original order into every generic/off-detection tick context. The focused
   `seen_last_frame_enemy_projection_preserves_detectable_order` test passes,
   and the current release comparator builds successfully.

   Both exact old boundaries clear on that runner. SuN1Sh1nE Profile 004
   Savegame 001 replay 002 advances from frame 606 to frame 631, where PC 252's
   command is Original `Wait` versus Rust `ShootBow`. Linux3 Profile 003
   Savegame 054 replay 001 advances from frame 534 to frame 574, where Soldier
   205's command is Original `LowerShield` versus Rust `Generic`. These are new
   independent frontiers; neither trace is EOF-clean. Logs for both runs and
   the companion SwordstrikeDown validation are preserved under
   `output/parity-audits/r06-shield-swordstrike-current-head/`.

   The two later frontiers have separate causes. At SuN frame 630, Original's
   `EquipBow` reaches `RHMOTION_DONE`; frames 631 and 632 expose `Wait`, and the
   queued shot starts only at frame 633. `RHElementActorHuman::Instruct` stores
   a rejected shot in `mlpsequenceShootList` before Actor `Instruct` translates
   it, while `ProcessShootList` retries from the Human `Hourglass` prelude only
   after the sprite's last animation becomes `AimingWithBow` or
   `AimingWithBowUp` (`RHelementactorhuman.cpp:297,3030-3045,13005-13015`). Rust
   previously represented that list as an already-instructed cross-postponed
   element and resumed it synchronously when `EquipBow` terminated. Commit
   `0fbeeed54` restores the source lifecycle: both eager live launches and
   manager-dispatched elements enter one pre-Actor-Instruct guard, retained
   elements remain pristine `Todo` references in `HumanData::pending_shoots`,
   and HumanPrelude retries only the FIFO front while the last animation is
   exactly `AimingWithBow` or `AimingWithBowUp`. `ClearShootList` and
   `EnterSwordFight` clear that real pointer FIFO. Commit `5c7b52a8c` corrects
   the focused regression's assertion for the constructor-default action
   state; the regression proves loading completion and the following Wait
   frame leave priority, transition state, orders, and selection untouched,
   and only the aiming frame admits the held shot.

   Linux3 frame 574 is command provenance rather than shield animation timing.
   The exact Original RNG branch directly launches an
   `RHCOMMAND_LOWER_SHIELD` element (`RHartificialmalignity.cpp:4520-4532`).
   Rust's AI helper instead queued a bare `LoweringShield` order, which the
   generic order drain wrapped in `Command::Generic`. Commit `60c827804` makes
   `AiController::lower_shield` set the existing explicit lower-shield outbox
   flag, so the engine's preemption drain launches `Command::LowerShield` and
   uses the established shield dispatcher. Its focused controller regression
   proves the explicit flag is set while the generic order queue remains empty.

   Both focused regressions pass, and the current-HEAD release
   `original_parity_replay` build succeeds. Targeted scan-all validation clears
   both source boundaries: the SuN trace advances through frame 631 before an
   independent frame-637 RNG over-consumption
   (`VipIdleRemark`/`ArrowPiercingProtection`), and the Linux3 trace advances
   through frame 574 before an independent frame-718 sound-manager invariant
   failure (exclamation 5 for actor 86 has no pending request). These scan-all
   results must not be confused with ordinary first-divergence runs. SuN still
   reports the earlier unrelated frame-448 `sprite_frame_count` mismatch for
   Soldier 125 (Original `65535`, Rust `0`); Linux3's ordinary next frontier is
   frame 607, where PC 296 is Original `UnequipBow` versus Rust `MoveWaiting`.
   Logs for all four runs are preserved under
   `output/parity-validation-2026-08-01/`.

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

#### Current release validation of the frozen cohort

The complete 45-member cohort was first rerun serially with the release runner built
from `aaebc38c9` (including `750224e6f`). The immutable input snapshot, one log
and one numeric status per member are preserved under
`output/parity-audits/r05-wait-raise-bow-current-head/`. After the obstacle-mask
fix and its targeted survivor validation, all 45 old `Wait` / `RaiseBow`
boundaries are cleared. The current result is:

- 12 cleared to exact EOF;
- 32 cleared that boundary and reached a later state or RNG frontier;
- 1 cleared the boundary and then hit a runner invariant failure; and
- 0 timeouts.

The twelve exact members are SuN1Sh1nE Profile 004 Savegames 019 and 021;
linux2 Profile 002 QuickSave and Savegame 041; linux3 Profile 001 Savegames
003 and 025; linux3 Profile 003 Savegame 035; nicouzouf Profile 001 Savegame
026 and all three listed Savegame 033 replays; and randomguy Profile 004
Savegame 002. Raw process statuses are twelve `0`, twenty-nine `1`, and four
`101`. These are the raw statuses from the initial complete sweep before the
sole survivor fix. Three `101` exits are authoritative later RNG-order
frontiers and are therefore included in the cleared-to-later count: linux2 Profile 002
Savegame 001 at frame 1411, nicouzouf Profile 001 Savegame 002 at frame 659,
and nicouzouf Profile 001 Savegame 075 at frame 293. The actual runner failure
is linux3 Profile 003 Savegame 052 replay 001 at the frame 4769-to-4770 step:
the sound manager resolved exclamation 14 for actor 136 without a pending
request. Treating all nonzero statuses as equivalent would incorrectly report
four failures rather than one.

The initially sole unchanged member was
`Savegame_linux3/Profile_003/Savegame_038/replay-001-session-0001.jsonl.zst`.
At frame 29146 Original remains `Wait` and Rust launches `RaiseBow`, exactly as
in the frozen baseline. A targeted `robin_engine::engine::input=trace` rerun is
preserved as
`output/parity-audits/r05-wait-raise-bow-current-head/linux3-profile003-save038-replay001-resolved-bow-trace.log`
with status `1` in the sibling status file. Rust PC 183 is in
`AimingWithBow`, its current order is `AimingWithBow` (Original animation 89),
and the resolved result is `bow_status=Valid`, `shoot_mode=Long`,
`command=Some(RaiseBow)`. The target is `(1487, 1560.6747, 648.6747)` and the
Rust hand point is `(1301, 1535.001, 555.00104)`. Thus `dx=186`, `dy=25.6737`,
`dz=93.67366`, the 3D distance is approximately `209.833`, and the
projectile-aspect-scaled XY distance is approximately `189.108`. The hand is
`93.674` below the target, so Rust uses the cylinder arm; `189.108 < 400`
makes its range result valid.

A bounded entity dump and direct asset audit resolve the remaining branch.
Original PC 179 maps to Rust PC 183 and has bit-identical map position
`(1301.8938, 1005.2984)`, elevation `530.00104`, direction 10, upright
posture, action state 4, and animation 89. `RobinTown.rhs` row 1674 is the
direction-10 row for action 89 and stores hotspot `(150, 150)`, equal to the
profile center. Both engines therefore floor `map - center` to sprite
top-left `(1151, 855)` and compute the same range hand given above; this is
not a hotspot or world-transform divergence. Character profile 1 stores
one-based shooting-weapon id 1, which selects bow profile 0 with normal range
250, long/max range 400, and long shots enabled. The `209.833` 3D distance is
therefore within the normal threshold, so Original `RHBow::GetShootType` and
Rust `get_shoot_mode_for_distance` both initially select `Normal`.

For the ensuing LOS query, raw `RobinTown.rhs` row 1706 is action 93
(`ShootingWithBow`) at direction 10 and stores hotspot `(122, 164)`. Both
`ComputeBowPoint` implementations consequently use ray origin
`(1273, 1549.00104, 570.00104)` and destination
`(1487, 1560.6747, 649.6747)` after the target's `+1` Z adjustment. The PC is
upright, so the leaning-out override cannot produce `Long`. Rust's final
`Long` can only be its subsequent blocked-LOS upgrade. Original staying in
`Wait` proves that Original's FastFindGrid judged this same ray clear while
Rust `is_reachable_3d` judged it blocked.

This is a real classifier/geometry frontier. Current Original
`RHElementActorHuman::CanShootWithBowAt` computes its hand point, applies the
same cylinder/cone range test, chooses shoot type from the 3D norm, and may
upgrade it for blocked LOS (`original-code/RHelementactorhuman.cpp:7047-7144`;
`original-code/RHBow.cpp:335-344`). Its `AimWithBowAt` then necessarily queues
`RaiseBow` for a valid long result while action state and animation are both
the ordinary bow-aim values (`original-code/RHelementactorhuman.cpp:6924-6963`).
Rust follows those boundaries in `engine/input.rs:1635-1736` and
`engine/input.rs:2710-2786`. Because the recorded Original command stays
`Wait`, Original did not observe the same final valid-long classification. A
second targeted run enabled both input and sight-obstacle traces and identified
the Rust blocker as static obstacle index/id 245; its exact artifact is
`output/parity-audits/r05-wait-raise-bow-current-head/linux3-profile003-save038-replay001-bow-and-sight-trace.log`.
The engine's 466-entry active vector has obstacle 245 enabled. It is an
on-ground quadrilateral spanning approximately X `1468.48..1591.68`, Y
`1509.56..1562.70`, and Z `0..731`, and its type value is 41:
`SOLID | MOUSE | SHOW_SHADOW_POLYGON`, notably without `OPAQUE`. The preserved
level asset is
`output/parity-audits/r05-wait-raise-bow-current-head/linux3-profile003-save038-f29146-level-assets.json`.

This exposed the exact general bug. Original
`RHSightObstacle::IsOfType(type)` implements `(mType & type) == type`
(`original-code/RHsightobstacle.h:104`), and every FastFindGrid obstacle
candidate helper calls it. Therefore the bow's combined `SOLID | OPAQUE`
request excludes solid-only obstacle 245. Rust instead skipped only when the
intersection was zero, treating the mask as any-of and admitting obstacle 245.
Commit `ece711a29` adds the shared all-requested-bits predicate and uses it in
boolean 3D reachability, nearest 3D impact, vertical fall, and vertical rise.
Explicit single-property `is_solid` and `is_opaque` queries retain their
any-bit semantics. The focused
`combined_type_filter_requires_every_requested_bit` regression reproduces
obstacle 245's solid-only shadow flags and verifies that it blocks a SOLID
query but not a combined SOLID-and-OPAQUE query. This is not a replay-specific
exception and does not weaken the valid-target guard.

The focused regression passes, and the current-HEAD release comparator builds
successfully. Its targeted Savegame 038 validation clears frame 29146 and
advances normally to a new independent state frontier after frame 29352:
PC 180 `actor.wait_time` is 25 in Original and `4294967243` in Rust. The exact
status-1 validation is
`output/parity-audits/r05-wait-raise-bow-current-head/linux3-profile003-save038-replay001-after-all-bits-fix.log`
with its sibling status file. This moves the last old-boundary member into the
32 cleared-to-later group and closes the frozen 45-member bow cohort at 45/45.

When another fix lands, add the commit, Original source boundary, focused test,
all affected traces, and their new exact result or next independent frontier
here. Do not silently delete old rows: mark them superseded by the fix so the
historical coverage remains visible.

### `bfe56ad36` + `88420a6e1` + `b60af1491` + `3e3d28eeb` — deferred parry and panic redetection

`Savegame_linux3/Profile_001/Savegame_008/replay-001-session-0001` first
failed at frame 4149 with Rust consuming four transient WaitingSword combat
draws after the five legitimate draws in Original. PC 173 in Original maps to
PC 172 in Rust. A ParrySword registered during the actor pass was still waiting
for SequenceManager, but Rust eagerly interrupted the PC's current smalltalk
strike, installed a synthetic Wait, and executed that Wait before manager
dispatch. `bfe56ad36` and `88420a6e1` preserve owner instruction FIFO across
the synchronous and deferred manager queues; the focused
`lazy_wait_does_not_leapfrog_preexisting_owner_instruction` regression checks
that deferred ParrySword dispatches before a later synthetic Wait.

The remaining fifth Original draw at callsite `0x1ebc58` was
`DoActionAndEventuallyPlayRemark` for the already-selected smalltalk strike.
Original `RHElementActorPC::ConsiderSwordAttack` calls
`LaunchSequenceElement`, so ParrySword priority arbitration waits for
`RHSequenceManager::Hourglass` after the actor loop. `b60af1491` routes this
actor-Execute launch through `register_owned_element_deferred`; the current
strike now reaches Motion START and performs its authoritative remark draw
before ParrySword replaces it. No replay-specific draw or state override was
added.

That advanced the trace to frame 4603. Original consumed twelve panic draws
(`0x11cf3c`/`0x11cf69` five times, then `0x11ce17` and `0x11c3a2`) for civilian
38 on every frame beginning at 4602, while Rust performed the batch only once.
Original's successful RunToHide/RunToDoor arrival and exhausted FleeingPanic
arms both call `RHElementActorNPC::BlinkEnemy(NULL)` unconditionally when
entering `FleeingHiding`. This clears `seen_now` and `seen_last_frame`, so an
enemy that remains visible rises as a new `EVENT_VIEW` on the following
detection pass and can immediately restart panic. Rust incorrectly assumed an
alert-status refresh supplied that side effect. `3e3d28eeb` queues the existing
`blink_all_enemies` effect in both source arms; the focused
`entering_fleeing_hiding_blinks_visible_enemies_for_redetection` regression
covers both transitions.

The current release runner now matches every recorded frame through EOF. The
exact current-HEAD runner output is preserved at
`output/parity-diagnostics/random-combat-owner/linux3-save008-r001-current-head-eof.log`.
It exited successfully and ends with
`parity trace matched every recorded frame`. The
preserved frame-4603 investigation dump is
`output/parity-diagnostics/random-combat-owner/linux3-save008-r001-frame4604-panic.jsonl`;
the compact Original AI transition extract is
`output/parity-diagnostics/random-combat-owner/frame4600-4604-original-ai.jsonl`.

### Battle-decision entry snapshot — proud-soldier speech

`Savegame_nicouzouf/Profile_001/Savegame_055/replay-002-session-0001`
reached frame 480, then Original resolved ordinary-soldier exclamation 55
(`REMARK_PROUD_DONT_FIGHT`) for soldier 51 at frame 481 while Rust had no
pending speech request. Original `RHArtificialMalignity::BattleDecisions`
captures the entry-time `mCurrentSubstate` in a stack-local `oldSubstate` and
uses that value to decide whether this is the first battle decision. Rust had
instead tested its serialized `previous_substate`, which corresponds to
Original's unrelated `mPreviousSubstate` used by the Charly-reunion flow.

Rust now snapshots `current_substate` on entry and threads that local value to
the decision executor. Focused regressions poison the serialized previous
field in both directions: an entry from `AttackingReactiontime` still queues
`ProudDontFight`, while a later proud-observer entry does not queue it merely
because the serialized field contains `AttackingReactiontime`. Full replay
validation remains pending until the active production sweep releases the
runner.

### Group-move formation authorization — actor-exact move boxes

`Savegame_linux3/Profile_003/Savegame_052/replay-001-session-0001`
reached frame 4769, where a selected PC 136 group move targeted sector 429 on
layer 6. Original rejected the formation slot and resolved PC expression 14
(`HERO_UNABLE_TO_DO_SOMETHING`) on frame 4770; Rust had no pending request.
Original `RHEngine::PerformGroupMove` authorizes the translated live
`GetMoveBoxMap()` for ordinary sectors, or `GetMoveBox(RHPOSTURE_UPRIGHT)` for
an actual lift. Rust instead constructed every box from pathfinder
half-diagonal table entry zero, which can accept a slot that the selected
actor's box rejects.

Group moves now rebuild the candidate box from each selected actor's live
position interface before `FindAuthorizedPosition`. Replay goal overrides
also resolve their authoritative sector back to its lift, door, or jump kind
instead of assuming every recorded goal is an ordinary motion sector. Focused
regressions distinguish live-map boxes from generic boxes, retain off-centre
saved box state, cover the lift-only upright source, and preserve all three
recorded sector kinds. Full replay validation remains pending until the active
production sweep releases the runner.

### Lift-target approach — geometry selects the high and low endpoints

Three frozen-sweep traces (`linux2/Profile_002/Savegame_023/replay-001` and
`linux3/Profile_002/ExQuickSave/replay-002`/`replay-003`) stopped while an
enemy reconsidered a target standing in lift sector 77. Both shipped endpoint
records for that lift are tagged `DOOR_LIFT_LOW`, so Rust's invented lookup by
door-type tag could not find a high endpoint.

Original `RHSectorLift::InitializeFromProtoStream` instead selects
`mpHighestDoor` and `mpLowestDoor` by minimum/maximum `PointOut.Y`; its
high-door type assertion is explicitly commented out. Enemy approach now
uses that same geometric selection and retains the chosen door's point-out,
outside sector, and outside layer. A regression with two deliberately
low-tagged doors covers the shipped malformed metadata. Full replay
validation remains pending until the active production sweep releases the
runner.

### Group-instruction continuation — close caller-local state work inline

`linux3/Profile_001/Savegame_016/replay-002` reached frame 7667, where officer
79's final accepted `CALL_INSTRUCTION` continuation changed
`SeekingOfficerInstructGroupPointing` to
`SeekingOfficerWaitForInstructedGroup`. Rust applied the pure AI fields but
left the corresponding owner-local `SetState` callback queued past the
officer's Hourglass slot.

Original resumes the officer's stack immediately after the instructed
soldier's `Think` returns. The result continuation now closes its caller-local
state/speech/timer work before processing the next result-bearing call. A
focused accepted-instruction regression asserts both the resulting wait state
and an empty owner-work queue at that direct-call boundary. Full replay
validation remains pending until the production runner is available.

### Fresh 739-trace current-HEAD baseline and R07 inventory

The serial release sweep frozen at `fd248c2ff` completed all 739 inputs in its
manifest: 225 exact EOF, 423 ordinary state divergences, 71 RNG-order exits,
8 speech/sound-manager exits, 12 other invariant panics, and zero timeouts.
The manifest includes 717 complete recordings and 22 explicitly incomplete
recordings; three of the latter account for truncated-JSON invariant exits.
Raw statuses, logs, the mutually exclusive per-trace classification, and the
release-runner fingerprint are preserved under
`output/parity-audits/random-short-current-head-20260801/`.

#### Frozen speech and invariant inventory

The eight frozen speech exits are not one sound-manager problem. Source and
trace inspection divides them as follows; "landed" means a general source fix
exists after the frozen `fd248c2ff` runner, not that this old result was
rewritten or skipped:

| Frozen trace boundary | Original speech | Classification after source audit |
|---|---|---|
| `nicouzouf/Profile_001/Savegame_055/replay-002`, 480 -> 481 | Soldier 51, `PROUD_DONT_FIGHT` (55) | Landed in `96cd51790`: use the entry-time battle-decision substate. |
| `nicouzouf/Profile_001/Savegame_047/replay-002`, 639 -> 640 | Soldier 63, `PROUD_DONT_FIGHT` (55) | Same proud-entry mechanism: `AttackingReactiontime` (raw 153) enters the proud observer state (raw 195) while `LeaveAttentiveMode` starts. Covered by `96cd51790`. |
| `linux3/Profile_001/Savegame_010/replay-002`, 32531 -> 32532 | Soldier 119, `PROUD_DONT_FIGHT` (55) | Same expression family; retain in the proud-entry validation cohort until the post-fix sweep confirms it. |
| `linux3/Profile_003/Savegame_052/replay-001`, 4769 -> 4770 | PC 136, `HERO_UNABLE_TO_DO_SOMETHING` (14) | Landed in `d392354a5`: authorize a group slot with that actor's live move box. |
| `SuN1Sh1nE/Profile_004/Savegame_026/replay-001`, 799 -> 800 | PC 282, `HERO_UNABLE_TO_DO_SOMETHING` (14) | Landed in `61d7b570c`: PC 282 is an anonymous archer, and Original accepts `EquipBow`; the unable bark comes from the older Move that reaches `Actor::Instruct` at the manager boundary, where anonymous archers reject movement. Registering the action-bar `EquipBow` through `LaunchSequenceElement` preserves that older callback. Eagerly instructing EquipBow during input had arbitrated it away. |
| `nicouzouf/Profile_001/Savegame_065/replay-001`, 368 -> 369 | Soldier 45, `WOUNDED` (29) | Landed in `b3d930220`: this is a live projectile/damage boundary, not a speech-filter mismatch. Arrow creation order 133 flies on frames 364--366; Original registers `ReceiveArrowDamage` during the projectile slot, then the manager applies 100 damage and `SayOuch`. Rust had applied projectile damage eagerly. Deferring the damage element to the same manager boundary restores the wounded request. |
| `linux3/Profile_001/Savegame_009/replay-003`, 13169 -> 13170 | Civilian 37, remark 4 | Landed in `d0220fb7c`: translated Rust PC 172 is Original PC 173, whose one-ration Eat at frame 13048 must use Original's silent `SetAmmoAmount` path rather than generic ability-decrement speech. |
| `SuN1Sh1nE/Profile_004/Savegame_002/replay-003`, 35715 -> 35716 | Soldier 217, `REMARK_WARCRY` (9) | Landed in `e196cf36a`: this is a special-strike boundary, not a PC-expression boundary. Original's `EstimateDamageOfSwordStrike` reads live health for nearby civilian 63 as well as PC 344, so both count as victims and round thrust H is viable. Rust's four proposal collectors assigned civilians zero health, lost their damage/victim contribution, rejected H, and therefore omitted its warcry. All collectors now use the canonical human life-point accessor. |

The twelve frozen invariant panics likewise contain only nine engine
boundaries. Three files are physically truncated JSONL tails and must remain
input-integrity failures: Nescafe Profile 001 restart-attempt 4716 replay 002
(line 356), Profile 002 continue-attempt 4966 replay 002 (line 330), and
Profile 003 restart-attempt 5236 replay 002 (line 350). Three lift panics are
one malformed-metadata cohort for sector 77 and are fixed generally by
`db4d3a18f`. The linux3 Profile 001 Save 016 replay 002 owner-work leak is
fixed generally by `b78d182f3`.

The remaining five invariant exits are live bow-release mappings, not
load-time projectile restoration: SuN1Sh1nE Profile 004 Save 011 replay 001
(Original projectile order 231), linux3 Profile 002 QuickSave replay 001
(171), linux3 Profile 003 Save 024 replay 001 (213), linux3 Profile 003 Save
052 replay 003 (325), and nicouzouf Profile 001 Save 045 replay 001 (143).
In every case the Original projectile appears when a PC's `ShootBow` order
reports done after trace start. The mapping assertion alone cannot distinguish
"Rust did not spawn" from "Rust spawned and retired a different entity in the
same frame"; capture the unmatched Rust creation orders and active-shot state
before changing creation-order mapping.

#### Projectile impact ordering

Source audit of the live arrow boundary found a separate general ordering
error after collision. Original `RHElementArrow::HitHuman` first registers a
`ReceiveArrowDamage` element, then calls the victim NPC's `EVENT_GET_ARROW`
`Think` inline, and only later does `RHSequenceManager::Hourglass` instruct the
damage element. Rust had applied arrow and stone damage immediately from the
projectile pass and merely queued `EVENT_GET_ARROW`, reversing both boundaries
and making the AI result depend on entity creation order.

Projectile damage is now registered for the sequence-manager phase. Arrow
impacts run the one `EVENT_GET_ARROW` Think synchronously while preserving any
older deferred detection stimuli, and the damage element retains the arrow
identity until dispatch so the victim turns from the arrow's flight direction
at Original's post-translation boundary. The deferred handler no longer
rechecks arrow-hurtable posture: Original performs that test only in
`HitHuman`, before the intervening Think is allowed to change AI/posture. Bow
kill XP is likewise tested after damage translation using Original's
post-damage `IsDead()` condition, rather than only on a live-to-dead edge.
Validation is pending the next sole-slot release run.

#### Last-ration speech boundary

The stale PC 172 `HERO_OUT_OF_AMMO` request in linux3 Profile 001 Save 009
replay 003 is the translated identity of Original PC 173's one-ration Eat at
frame 13048. Original completes that action with
`SetAmmoAmount(RHACTION_EAT/GUZZLE, remaining)`, not
`DecreaseAmmoAmount`: the empty action slot is disabled, but no
`HERO_OUT_OF_AMMO` request is produced. Rust had routed Eat through the generic
ability-decrement helper and left its extra request at the FIFO head until the
civilian remark resolved 121 frames later.

Rust now has a source-specific ration-consumption path which disables the Eat
or Guzzle slot without speech, including Original's preference for the Guzzle
slot when the PC has that action. A unit regression covers both profiles.
Validation is pending the next sole-slot release run.

The 71 R07 exits group by the first Rust draw site as follows:

| First Rust site | Count |
|---|---:|
| `VipIdleRemark` | 12 |
| `AiRandomValueRectangle` | 11 |
| `BoredAnimationChoice` | 10 |
| `RuntimeBuildingExitWait` | 7 |
| `SeekPointSelection` | 6 |
| `MacroRand` | 6 |
| `DrunkCombatFreeze` | 6 |
| No Rust draw in the Original frame | 6 |
| `ScriptRand` | 3 |
| `DefaultPostLook` | 3 |
| `AiPanic` | 1 |

Symbolizing the Original return-address stream resolves 46 boundaries without
rerunning them. The largest exact pairs are Original
`RHArtificialMalignity::The16thFrame` versus Rust `VipIdleRemark` (8),
`RHArtificialMalignity::SeekArea` versus `SeekPointSelection` (5),
`RHArtificialIntelligence::StopAll` versus `AiRandomValueRectangle` (5),
`RHScript::AttachScrollToNPC` versus `ScriptRand` (3), and
`RHArtificialMalignity::ReconsiderSwordfight` versus `DrunkCombatFreeze` (2).
The first, second, fourth, and fifth pairs name the same logical source draw on
both sides. Their cardinality mismatch is therefore evidence of a differing
owner/state gate or owner ordering, not a reason to add or discard a random
draw. Expanding the complete Original/Rust sequences proves that the apparent
eight-member periodic bored-remark family is not one cause: `VipIdleRemark` is
only a common matching prefix. The next differing calls split across missing
battle-predecision work, seek-point cardinality, Rust-only arrow protection, a
later periodic owner, waypoint/actor work versus a macro continuation,
principal-opponent assignment, and strike-damage estimation. Do not change
the matching periodic callsite or assign those eight recordings as a cohort.

Future RNG cursor assertions now include the Original frame's simulation-only
callsite offsets alongside Rust's `RngSite` sequence. This makes the exact
Original/Rust boundary available in the ordinary log without decompressing a
multi-gigabyte single-frame JSONL recording. Owner-specific replay diagnostics
remain necessary before changing any matching source callsite.

One independent macro-lifecycle discrepancy found during this audit is now
fixed. Both out-of-bytes branches of Original `ExecuteNextMacroCommand` call
`KillTimer(true)`, whose boolean selects the **macro** timer
(`RHartificialintelligence.cpp:844-887`, `RHelementactornpc.cpp:3931-3940`).
Rust had instead cleared `timer_is_running`, killing an unrelated normal AI
timer and potentially leaving a macro deadline armed. Macro completion now
clears only `macro_timer_is_running`; the focused regression preserves the
normal timer and its deadline. This is source-backed lifecycle parity, not a
claim that it resolves one of the still-unrerun RNG frontiers.

The same source audit removed another invented timer semantic. Original
`RHElementActorNPC::LaunchTimer` stores `universal_frame + frames` verbatim for
both timer kinds; zero is not clamped (`RHelementactornpc.cpp:3906-3921`).
Rust had changed zero-frame deadlines to one frame. Both timer launch helpers
now preserve zero and use `u32` wrapping addition to match Original `ULONG`.
Whether a zero timer fires immediately still follows call position: a timer
armed before its later Hourglass polling phase can fire in that frame, while a
timer rearmed from inside the polling phase is not polled again until the next
frame.

`BreakMacro` now also preserves the dormant macro byte stream, cursor, and
remaining-byte count. Original only clears `mbMacroInProgress`, kills the
macro timer, and clears Charly (`RHartificialintelligence.cpp:938-943`); its
serializer still writes the cursor and remaining count afterward. Rust had
eagerly erased all three fields. That cleanup changed save/reload state and
would make the newly recorded macro diagnostics diverge even when behavior was
otherwise identical.

The seven `RuntimeBuildingExitWait`-leading traces are one H12 owner cohort:
civilian 85 enters the building on replay frame 187, finishes the door pass
and enters `DefaultInMacro` on frame 208, and Original remains inactive in its
WAIT/`DefaultInMacro` state beyond frame 528. Rust resumes the macro at frame
508, builds the route out, and consumes the two building-exit wait draws.
Original does call inactive elements' `Hourglass`, so skipping inactive AI is
not a valid fix. The old recordings expose only AI state/substate, leaving an
AI/script-lock deadline extension indistinguishable from a macro-timer value
difference. Do not patch this cohort until a recording with the optional raw
lock, `was_busy`, macro timer, cursor, and remaining-byte diagnostics reaches
the same boundary.

### Frozen 739-trace ordinary-state regroup (`fd248c2ff`)

The completed current-head audit in
`output/parity-audits/random-short-current-head-20260801/` contains 739
results: 225 exact EOF, 423 ordinary state divergences, 71 RNG boundaries, 8
speech boundaries, and 12 panics. The 423 ordinary failures group by first
logical field as follows:

| First logical field | Count |
|---|---:|
| `actor.command` | 186 |
| `actor.action_state` | 73 |
| `sprite_frame_count` | 36 |
| `direction` | 20 |
| `direction_goal` | 17 |
| `actor.wait_time` | 17 |
| `position_map.x` | 16 |
| `position_goal_map.x` | 16 |
| `posture` | 9 |
| `layer` | 9 |
| `ai.substate` | 6 |
| `ai.state` | 6 |
| `life_points` | 4 |
| `blipped` | 4 |
| `elevation` | 3 |
| `active` | 1 |
| **Total** | **423** |

The four `blipped` first boundaries are one Listen-distance cohort. Each occurs
exactly when a PC's 25-frame Listen countdown reaches zero: two traces leave
one or two elevated NPCs blipped in Rust after Original reveals them, while
two reveal elevated Civilian 74 only in Rust. Original `ListenTo` subtracts
full `GetPosition()` world points and then stretches relative Y by
`INVERSE_ASPECT_RATIO`; Rust subtracted projected map Y while retaining the Z
difference. At Derby frame 988 the correct world calculation puts Soldier 71
inside the strict 750-unit sphere while the projected calculation puts it
outside. At Leicester frame 402 the same error reverses the classification for
Civilian 74. Listen now uses a shared world-position distance primitive, with
both threshold directions covered by exact captured geometry.

The four `life_points` first boundaries are likewise one command family. They
are not projectile impacts: at each boundary Original processes
`ReceiveSwordDamage` and subtracts health while Rust retains the prior health.
The exact members are nicouzouf Save 047 replay 001 frame 264 (15 damage),
nicouzouf Save 008 replay 003 frame 594 (5), nicouzouf Save 020 replay 003
frame 464 (5), and linux3 Save 005 replay 003 frame 54,804 (5). All four
missions use a forest proto (`FoB`, `FoC`, or `FoA`). Rust incorrectly used
that proto's broad `forest_level` flag as `RHGame::IsSherwood`, so its shared
life-point setter granted Sherwood-HQ immunity to PCs in ordinary forest
missions. Damage and concussion contexts now use the current campaign mission
profile's `Sherwood` location instead. The separate `GetPositionGround`
correction remains source-exact, but the level-zero Save 047 combat geometry
proves it was not the cause of this cohort. The frozen members remain
candidates for the next follow-up validation rather than being declared fixed
without a replay run.

The largest repeated exact command signatures are 16 PC
`EnterHelpingClimb -> Wait`, 15 Soldier `MoveWaiting -> MoveOk`, 13 Soldier
`EnterAttentiveMode -> Wait`, 13 PC `Wait -> EnterHelpingClimb`, 10 Soldier
`Wait -> Turn`, 10 PC `ParrySword -> SwordstrikeThrustD`, 9 Soldier
`ShootBow -> Turn`, and 7 PC `ParrySword -> SwordstrikeThrustA`. These are
first-visible signatures rather than assertions that each pair has one cause.
In particular, the two helping-climb directions occur on different launch and
completion boundaries after the earlier 49-member lifetime fix.

This audit is frozen at `fd248c2ff`. Later commits must be credited before
using these numbers as live work totals: `96cd51790` fixes proud-soldier speech,
`d392354a5` fixes group-move authorization and its missing hero speech,
`3acf2093f` adds NPC lock/macro diagnostics, `db4d3a18f` fixes lift endpoint
selection, and `b78d182f3` closes caller-local group-instruction work inline.
Only a new release sweep can provide authoritative post-fix counts.

The 15 `MoveWaiting -> MoveOk` members reduce to five save situations repeated
three times. In the representative nicouzouf Profile 001 Save 024 frame-107
boundary, Original synchronously queues paths for soldiers 80, 82, and 84,
while Rust exposes `MoveOk`; the trace records all three exact sources, final
goals, and half diagonals. Six members from Saves 053 and 059 expose coupled
`direction_goal`, `position_goal_map`, and `position_map` differences when the
Original path request completes invalid while Rust's premature direct order has
already displaced the soldier; those are consequences of this same first
command boundary, not separate geometry bugs. Source comparison found a general
dispatch mismatch:
Original `RHElementActor::InstructOwner(RHCOMMAND_MOVE)` bypasses A* only for
`RHMOVE_MAP`, `RHMOVE_STRAIGHT`, or a successful `IsReachableThick` test.
Rust additionally treated `RHMOVE_LINE` and pass-door state as unconditional
direct movement. Line-goal metadata controls `PostProcessPathToLine`; it does
not authorize skipping `AddPathRequest`. The Rust predicate now mirrors the
Original condition, with a focused regression proving that a blocked
`MoveFlags::LINE` movement still enters pathfinding. Validation is pending the
next sole-slot release run.

The frozen direction groups also exposed a source-level discrepancy shared by
all four shield commands.  Original `RAISE_SHIELD`,
`RAISE_SHIELD_INSTANTLY`, `LOWER_SHIELD`, and `PARRY_SHIELD` construct their
orders with `bComputeDirection = false`
(`original-code/RHelementactorhuman.cpp:2018-2054`).  Rust retained the generic
order default, allowing selection of a shield animation to derive a new facing
goal from its zero-valued destination after `Focus` had already chosen the
authoritative shield direction.  Shield orders now preserve that facing.  A
focused regression covers all four translated order types; attribution of
individual frozen traces awaits the next sole-slot release sweep.

All nine frozen `layer` first-boundary traces are one repeated projectile
signature: newly created Projectile 283 is detached in Original
(`layer=sector=0xFFFF`) but Rust assigns prospective landing layer 2, sector
89.  This is not an isomorphism issue.  Original `ComputeTrajectory` retains
the exact `RHSightObstacle*` returned by its 3D impact query; after assigning
that obstacle it immediately leaves layer/sector detached when the obstacle is
not a projection area.  Rust retained only the final waypoint and later
rescanned its projected footprint, allowing an overlapping projection area to
replace the actual solid impact obstacle.  Arrow trajectory construction now
carries the terminal obstacle identity into landing-membership resolution,
and exact ordinary-solid impacts cannot fall back to overlapping projection
geometry.  Focused regressions cover both identity transport and the
solid-over-projection membership rule; the nine replay boundaries await the
next sole-slot release sweep.

The frozen `active` group initially had one trace around Arrow 122 retirement.
Recording occurs after `PerformHourglass` but before display refresh in
Original.  The terminal Hourglass frame must therefore remain active, while
`RHElementArrow::Refresh` sees the empty trajectory plus stationary sprite and
retires it before the following recorded frame.  A first attempt removed the
Rust scheduling latch, but a broad follow-up sweep exposed twelve regressions:
the stopped-flight branch is already reached in the terminal owner slot, so
immediate retirement incorrectly changed that frame's snapshot.  The latch is
required to bridge the unmodeled post-record Refresh phase: its first visit
preserves the terminal active frame and its second retires the retained entity
slot.  The focused lifecycle regression now encodes both boundaries.

The remaining frozen `elevation` boundary at Linux3 Profile 001 Savegame 040
frame 27918 is the direct half of `RHElementActor::PassDoor`. Original changes
PC 320 from layer/sector 0/0 to 1/72 while retaining the door-rail elevation
`93.3318`; only the non-direct branch calls `ComputePositionAll`. Rust's common
door-completion helper instead snapped map XY through the installed plane and
replaced Z with `90.00101`. Direct completion now rebuilds coherent world XY
from the exact endpoint map coordinate and the already-computed rail Z without
probing the plane. Non-direct completion retains its projection recomputation,
including the outside plane installed when leaving a building. A focused
sloped-plane regression distinguishes the two source branches.

The post-fix 171-trace follow-up grouped 133 ordinary exits into ten repeated
loaded-save boundaries whose only differences were tied soldiers'
`sprite_frame_count` values. Every affected actor had posture `Tied`, the
serialized `unconscious` flag set, and an in-progress `Wait` element whose
selected order was `BeingTied`. Rust's generic Execute path treated any
unconscious actor outside its corpse/KO hold whitelist as settled and returned
without advancing the sprite; `BeingTied` was missing from both copies of that
whitelist. Original `RHElementActorHuman::Execute` unconditionally calls
`PerformAction` for `RHANIMATION_BEING_TIED` and then returns `IN_PROGRESS`, so
the loaded one-frame hold continues toggling its sub-frame counter. Rust now
retains that live hold despite the unconscious flag. A focused lifecycle
regression reconstructs the exact loaded `Wait`/`BeingTied` state and requires
the counter to advance.

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
