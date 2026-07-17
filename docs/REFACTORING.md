# Gameplay Runtime Refactoring Roadmap

This roadmap turns [PARITY_AUDIT.md](PARITY_AUDIT.md) into mergeable slices. It
is ordered around invariants and test seams, not file size. No slice may claim
Original parity without a source citation and an observable test.

## Goals and non-goals

Goals:

- make tick phase order explicit and testable;
- give campaign, script, RNG, and snapshot state one authoritative owner;
- preserve Original same-frame AI/sequence behavior where gameplay observes it;
- put replay, rewind, rollback checking, and multiplayer on one deterministic
  snapshot/replay contract;
- fail loudly when required state or scripts are missing.

Non-goals:

- reproducing the platform-specific C `rand()` bit stream;
- retaining the legacy binary save format instead of serde;
- putting renderer/audio resources in deterministic snapshots;
- claiming Lua is original-game behavior;
- mixing behavior changes with large mechanical file moves.

## Required invariants

1. A normal mission has exactly one campaign. Script access is a scoped
   borrow/lease, not an optional second owner.
2. Every value read by a deterministic tick is in the snapshot, immutable level
   assets, or derived solely from those inputs.
3. Every gameplay RNG draw advances the Engine stream. Missing scope is an
   error; replay decode failure never chooses a seed.
4. Sequence command-level advancement, WAIT dispatch, immediate effects, and
   recursive completion have named barriers matching Original.
5. AI snapshots may solve aliasing but cannot move an observable event across
   its Original entity-order/same-frame boundary without a documented adaptation.
6. Required missing references, campaign state, scripts, and snapshot fields
   error or panic with context. They never become false, zero, empty, or a new
   default merely so execution can continue.
7. Host side effects are tick outputs. Host state feeds later simulation only
   through explicit commands or snapshot fields.

## Target ownership

```text
Application / session
  Campaign between missions
  immutable LevelAssets cache
  presentation Host

MissionRuntime (only while a mission is active)
  required Campaign
  Engine / deterministic frame / RNG
  deterministic display-control state read by Engine
  script policy
    SCB VM: snapshotted Engine state
    Lua: rejected in deterministic modes, or future serializable adapter
  Timeline
    snapshot schema + command log
    replay / rewind / rollback / multiplayer retention views
```

`MissionRuntime::finish(self)` returns the same campaign. SCB and Lua natives
receive a scoped `MissionAccess<'_>` rather than moving campaign/entities among
optional holders. If the current VM still requires swapping, wrap it in an RAII
guard that always swaps back and asserts pre/post ownership.

## Explicit tick phases

Introduce phase names and trace points before moving logic:

1. `PreExitAndScript` — exit flags, one-second script, victory check.
2. `AdvanceClockAndLockGate` — increment clock, then zoom/engine-lock return.
3. `PreElements` — loss, reinforcement, cleanup, paths, collision.
4. `ElementsInOriginalOrder` — local human/AI/movement/animation effects with
   explicit mutation barriers.
5. `SequenceManager` — FIFO dispatch and same-frame immediate cascades.
6. `PostElements` — titbits, selection validity, anonymous timers.
7. `AdaptedDeterministicTail` — Rust-only deterministic presentation state and
   side-effect packaging, each item labelled as adaptation.

The coarse `tick.rs` phase split and ordered `HourglassPhase` trace have landed.
Further extraction must preserve that trace and wait for PA-013's per-entity
interleaving evidence; phase names are a seam, not proof that batched internals
match Original.

## Dependencies, risk, tests, merge order, conflicts

This status reflects the current merge from `main`: campaign teardown and
fast-grid validation, the shared timeline primitive, frame-paced paths, exact
NPC subphase order, synchronous condolence/message re-entry, and the
post-refresh PostInitialize boundary have landed. The core tick phase split,
script `SendMessage` sequence path, door authorization, `Sees`, shoulder-action
gate, and collinear impact ordering have also landed. They remain prerequisites
and regressions, not work to reimplement.

| Order | Mergeable slice | Status | Depends on | Risk | Focused tests | Main conflicts |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | **Finish required-state errors.** Make replay-header preload fatal and missing sequence references loud. Preserve the landed campaign and fast-grid rejection paths; no phase moves. | Partial | None | Medium: exposes latent corruption. | bad replay header; missing campaign; corrupt grid lengths; stale sequence ref. | `setup.rs`, `game_session/mod.rs`, `rollback_safe.rs`, `sequence.rs` |
| 2 | **Snapshot input closure.** Inventory every `apply_commands`/`perform_hourglass` read; move sim-relevant display control into snapshot/Engine. | Open | 1 | High: save, camera, rollback. | zoom replay; save/load during zoom; one-frame live/replay hash. | `peripherals.rs`, `camera.rs`, `tick.rs`, `sim_timeline.rs`, `save_file.rs` |
| 3 | **Harden the shared timeline.** Consolidation has landed; verify rewind, rollback checker, multiplayer history/join, and EngineManager all use the same snapshot/replay boundaries. | Landed; audit callers | 2 for snapshot closure | High: correction and stepping. | equivalent reconstruction by every caller; missing-command error; correction truncation. | `sim_timeline.rs`, `rewind.rs`, `rollback_checker.rs`, multiplayer/tick session files |
| 4 | **Finish MissionRuntime ownership.** Mission frame/runtime extraction has landed; make Campaign required beside Engine and add a panic-safe checked script access guard. | Partial | 1 | High: all exits/save/load/natives. | identity-equivalent campaign on every exit; panic-safe guard; save around script call. | `engine/mod.rs`, `engine/types.rs`, natives, `game.rs`, session/mouse files |
| 5 | **RNG context.** Encapsulate/replace TLS with tick/script context tied to one Engine; retain serialized fastrand state/call order. | Open | 2, 4 | Medium-high: broad signatures. | RNG round trips; nested/missing scope; replay next roll; Lua Initialize random. | `sim_rng.rs`, `tick.rs`, AI/combat callers, `robin_lua/state.rs` |
| 6 | **Finish sequence barriers.** Synchronous condolence and immediate traces have landed; make WAIT launch-time and invalid refs loud. | Partial | 1 | High: action timing. | WAIT launch; command levels; recursive immediate; FIFO; stop/preemption. | `sequence.rs`, `tick.rs` |
| 7 | **Lua contract gate.** Reject Spellforge in deterministic modes; required startup/runtime failures abort; correct public event claims. | Open | 1, 4, 5 | Medium vanilla/high mods. | required failure; deterministic rejection; optional callback; return coercion. | `lua_session.rs`, `robin_lua`, `host.rs`, session startup |
| 8 | **Lua event completeness.** Add Timer/victory/Finalize/per-entity routing from exact versioned Spellforge provenance; decide snapshot strategy first. | Open | 7 | High: new script semantics. | cadence; FilterAIEvent; ProcessMessage; target/zone/waypoint; save policy. | AI, sequence, script manager, Lua session |
| 9 | **Finish AI transaction barriers.** Exact NPC subphase order and recursive self-stimuli have landed; retain read-only views only where they preserve Original entity-order visibility. | Partial | 6 + trace seam | Very high: central gameplay. | storage order; list mutation; recursive DONE/REACH/IMPOSSIBLE; locked FIFO; phase wrap. | `engine/ai/*`, `tick.rs`, `sequence.rs` |
| 10 | **Audit the landed tick phase extraction.** The coarse split, phase trace, simulation gate, runtime seams, and PostInitialize boundary have landed. Keep batched internals stable until PA-013 proves the remaining per-entity interleavings. | Landed core; PA-013 open | 2, 6, 9 for deeper moves | Medium for seam cleanup; very high for interleaving | full phase trace; first tick; lock return; mission exit; replay corpus. | `tick.rs`, `game.rs`, `sim_timeline.rs` |

Required-state errors come first so later work can distinguish invariant failures
from accepted fallbacks. Snapshot closure precedes consolidation because sharing
a deficient snapshot multiplies the bug. Mission ownership precedes Lua because
Lua natives need stable access. Sequence precedes AI because Original `Think`
can launch and complete sequences re-entrantly. Tick extraction is last because
moving unknown order creates conflicts without proving behavior.

Slices 2 and 4 may develop in parallel only if they agree that MissionRuntime
owns the final snapshot schema. Slices 5 and 6 may develop in parallel after
slice 1, but both touch `tick.rs`: merge sequence barriers first, then rebase RNG
context. Serialize AI and tick-phase work.

## Risk controls

### Ordered traces, not only final hashes

Final hashes miss transient ordering when effects cancel or host outputs are
excluded. Add a test-only trace enum with phase, stable entity/sequence IDs,
stimulus, and transition. Compare exact vectors for small worlds. Do not make log
strings the contract.

### Provenance comments

Behavior-sensitive barriers should cite:

- `RHEngine::PerformHourglass`, `original-code/RHengine.cpp:3446-3777`;
- `RHElementActorNPC::Hourglass` / `RefreshDetection`,
  `original-code/RHelementactornpc.cpp:1371-1675,3495-3659`;
- `RHArtificialIntelligence::StartThink` / `EndThink`,
  `original-code/RHartificialintelligence.cpp:914-1519`;
- `RHSequence::NextSequenceElementsGo`,
  `original-code/RHsequence.cpp:235-290`;
- `RHSequenceManager::Hourglass` / `RegisterSequenceElementToGo`,
  `original-code/RHsequencemanager.cpp:931-970`;
- `RHEngine::Serialize`, `original-code/RHengine.cpp:2408-2875`.

Where exact evidence is unavailable, add `PARITY TODO` naming what is missing.
Spellforge/Lua references must name their upstream version/source, not
`original-code/`.

### Validation ladder

1. Run new focused unit/trace tests.
2. Run affected crate tests (`robin_engine`, `robin_lua`, or `robin_rs`).
3. Build `robin`.
4. Replay a fixed mission corpus; compare per-frame hashes and selected ordered
   traces from the same initial snapshot.
5. For network/timeline changes, test two peers and late-input rollback across
   the boundary.

Never update a golden trace merely because a refactor changed it. Resolve the
change against Original provenance or record an intentional adaptation.

## Conflict map

| Area | Likely concurrent work | Resolution rule |
| --- | --- | --- |
| `engine/tick.rs` | sequence, RNG, AI, display, extraction | Merge behavior barriers before extraction; preserve provenance and rerun traces. |
| `engine/mod.rs`, `rollback_safe.rs` | serialization, campaign, snapshot schema | Ownership/schema define truth; never restore default shims to fix compilation. |
| `sequence.rs` | typed commands, immediate dispatch, AI condolations | WAIT/immediate timing tests win over convenient queue shape. |
| `engine/ai/*` | module extraction and parity work | Rebase extraction onto transaction tests; do not resolve by delaying drains. |
| `game_session/mod.rs` | modals, multiplayer, Lua, ownership | Use MissionRuntime; do not add another take/install pair or campaign default. |
| timeline/rewind/rollback files | correction and HTTP stepping | One replay primitive/schema; retention differences are wrappers. |
| `robin_lua/*` | native coverage and sandboxing | Keep deterministic gate until a tested snapshot policy exists. |

## Definition of done

A refactoring slice:

- changes one invariant or phase boundary;
- cites exact Original sources or labels an extension/TODO;
- adds focused tests that fail on the pre-change mismatch;
- rejects missing required data instead of synthesizing defaults;
- runs formatting, affected tests, and the `robin` build;
- updates `NEW_FEATURES.md` only for genuinely post-port behavior;
- documents save/replay compatibility for schema changes;
- contains no unrelated cleanup or clippy churn.
