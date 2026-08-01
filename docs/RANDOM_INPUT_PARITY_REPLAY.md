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
| R02 | In progress | `direction_goal`, frequently at frame 516 | Silent Linux2 Save002 proved that Original patrol snapshots expose an active PassDoor member at its committed gate side, while Rust observed its interpolated sprite position and queued a stale patrol target. Door-snapped shared-AI views and exact endpoint completion are under validation; random members still require their own failure-only reruns before this whole group is attributed to that cause. |
| R03 | Listen fix landed; timer rerun required | `actor.wait_time`, alone or beside action state | Original uses the single serialized `mulWaitTime` for Whistle and Listen. Rust now synchronizes its phase-local mirrors and, while Listening, ignores sprite completion until that counter reaches zero. The prior sweep's three unchanged timer pairs (`2 -> 14`, stale `25 -> 4294967269`, and `24 -> 25`) still require current-fix reruns; do not normalize any timer in the comparator. |
| R04 | Unassigned | `position_goal_map` | Audit whether the outgoing selected command is detached, postponed, or retained. Existing Halt and raising-sword fixes are relevant but not assumed sufficient. |
| R05 | Unassigned | `actor.command` with posture/direction | Inspect wrapper versus concrete command lifetime and the action-change marker that commits posture. |
| R06 | Unassigned | `ai.substate` at frame 282 | Compare synchronous command side effects and owner-local AI callback ordering. |
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

## Fix ledger

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
