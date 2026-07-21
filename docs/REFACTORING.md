# Gameplay runtime refactoring roadmap

Updated 2026-07-19. This document describes the current architecture and the
remaining behavior-sensitive work. Completed migration plans are summarized in
[`plans/`](plans/); their old future-tense PR sequences have been removed.

## Landed foundations

- `EngineInner` is the atomic deterministic root over nine cohesive owners.
- A live mission owns one required campaign and returns it by value at finish.
- SCB natives borrow canonical Engine domains; no `GameHost`, mirrored query
  state, swaps, campaign leases or compatibility shell remain.
- `SimulationContext` carries one serialized `SimulationRng` and the existing
  `SimConfig` through mission selection, save, replay, restart and multiplayer.
- `run_mission` and the headless driver use explicit bootstrap, runtime,
  frontend, frame and consuming-finish owners.
- `HourglassPhase` records the coarse tick order. Sequence dispatch, immediate
  commands and synchronous script driving live in `engine/sequence_runtime/`.
- Ordinary actor movement and evidenced rider movement arms run inside the
  live legacy-slot Actor owner coordinator. Mutable order, target, mobile
  geometry, crossing, completion, and callback inputs are sampled at the live
  owner boundary and close before ActionChange/tails; mobile geometry is not a
  frame-global prepared snapshot.
- Mission ingestion is split into ordered entity, environment, PC and finish
  stages under `engine/level_loading/`.
- AI model/context/effect/controller code and the giant Engine tests are split
  into focused modules.
- Rendering debug, HUD and minimap code is separated from normal entity/frame
  rendering.
- Save version 49, replay version 6 and network version 12 are the only current
  schemas. Historical save/replay compatibility is intentionally unsupported.

## Required invariants

1. A deterministic value has one authoritative owner. Do not add mirrors,
   refresh caches, swap protocols, or optional parking values.
2. Every simulation read comes from the snapshot, immutable `LevelAssets`, or
   data derived solely from those inputs.
3. Every gameplay RNG draw uses the Engine-owned `SimulationContext` and a
   reviewed `RngSite`. Missing scope or invalid bounds fail loudly.
4. Sequence command-level advancement, WAIT launch, immediate effects,
   condolations and recursive script calls retain their tested synchronous
   barriers.
5. Aliasing snapshots and two-pass collection may not move observable work
   across Original creation/FIFO order without a documented adaptation.
6. Missing required IDs, assets, scripts and snapshot fields produce a
   contextual error or panic. They never become false, zero, empty or default.
7. GPU, device and external audio resources remain host owned. Deterministic
   presentation state remains in the Engine when rollback or gameplay observes
   it.

## Current module boundaries

```text
robin_engine::engine
  tick.rs                  top-level HourglassPhase orchestration
  sequence_runtime/        sequence phase, immediate and script-sync drivers
  level_loading/           ordered mission construction stages
  ai/                      Engine-facing AI detection and hourglass behavior
  movement.rs              Engine movement/path integration

robin_engine::ai
  types / model / contexts / effects / macro_patrol / controller

robin_rs::game_session
  bootstrap / runtime / interactive / headless
  frame_prepare / frame_simulate / flow
  event_hud / live_gameplay / debriefing / terminal_debriefing

robin_rs::game_render
  debug / hud / minimap plus the normal rendering facade
```

The boundaries above are ownership and behavior seams, not a target line count.
Do not split a coherent state machine merely to make a file smaller.

## Active roadmap

| Priority | Work | Status and constraint |
| --- | --- | --- |
| 1 | Complete PA-013 per-entity Hourglass parity | High risk. Ordinary movement and Bonus `RefreshDiscovered` are owner-local. Preserve the landed phase trace and creation-order regressions; move one evidenced entity family at a time. PC Listen/object reveal, Target Heard, active combat/abilities, unsupported rider arms, and other entity owners remain. |
| 2 | Close remaining snapshot inputs | Audit every `apply_commands` and `perform_hourglass` read. Simulation-relevant host/display controls must be snapshotted or command-derived. |
| 3 | Finish AI transaction boundaries | Live Enemy-list reconstruction, FIFO edge ordering, civilian/Royalist optical detection, lift approach geometry, and contextual stale-ID failures are landed. Remaining specialized AI states and coordinate-space policy need exact Original evidence. |
| 4 | Decide Spellforge Lua persistence | Deterministic/network modes correctly reject Lua today. A versioned event surface and serializable VM/state policy are prerequisites to relaxing that gate. |
| 5 | Complete true-headless multiplayer admission | **Complete.** `TimelineRuntime` owns the snapshot → ready → begin → wall-clock release state machine, while the shared mission-network drain serves both drivers. Headless bootstrap publishes the real host snapshot and advances without renderer/UI/audio stand-ins. |
| 6 | Continue local owner/API cleanup | Make `MissionWorld` fields private as frame operations move to focused owners; narrow command-family borrows when behavior work touches them. Avoid new mega-contexts. |

The detailed gameplay ledger and Original evidence live in
[`PARITY_AUDIT.md`](PARITY_AUDIT.md). RNG call-order ownership lives in
[`RNG_AUDIT.md`](RNG_AUDIT.md). The focused command/tick ownership ledger lives
in [`SNAPSHOT_INPUT_AUDIT.md`](SNAPSHOT_INPUT_AUDIT.md).

## Tick provenance

The top-level phase order remains in `engine/tick.rs`; extracted sequence
internals live in `engine/sequence_runtime/`. The principal Original anchors
are:

- `RHEngine::PerformHourglass`, `original-code/RHengine.cpp:3446-3777`;
- NPC `Hourglass` and `RefreshDetection`,
  `RHelementactornpc.cpp:1371-1675,3495-3659`;
- AI `StartThink`/`EndThink`,
  `RHartificialintelligence.cpp:914-1519`;
- sequence advancement and manager drain,
  `RHsequence.cpp:199-313` and `RHsequencemanager.cpp:931-970`;
- serialization/post-load repair, `RHengine.cpp:2408-3007`;
- host refresh/sound/`PostInitialize`, `RHgame.cpp:1399-1842`.

Where evidence is incomplete, leave a precise parity TODO that names the
missing source or unresolved coordinate/ordering boundary.

`tick_zone_occupants` remains a documented Rust reconciliation boundary after
the owner walk. The cited Original Actor/Human/PC/Soldier Execute arms do not
establish it as actor-owned work; moving it per owner requires separate source
evidence. Unsupported action arms and active strike/bow/ability ownership also
remain PA-013 debt.

`WorldState::mobile_elements` remains the only mobile-master representation.
Because the Original master and its first `RHElementFXMasked` child are
adjacent, the live entity walk hosts the master Hourglass at that first child
slot without adding an `Entity` variant or mirror. `FxData::mobile_index` and
`MobileElement::sprite_ids` are validated as an exact, non-empty, adjacent,
ordered relationship; the first child runs the master once, and every child
then owns exactly one masked-animation call at its stored slot.

## Validation ladder

1. Add the smallest focused regression or exact ordered trace.
2. Run the affected crate tests.
3. Run full `cargo test` for cross-module and schema changes.
4. Run `cargo build --bin robin` separately.
5. Replay a fixed mission corpus and compare per-frame hashes for timing,
   snapshot or RNG changes.
6. Exercise two peers and late-input rollback for network/timeline changes.

Do not update a golden trace merely because a refactor changed it. Resolve the
change against Original provenance or document an intentional Rust extension.

## Definition of done

A refactoring slice:

- preserves or deliberately corrects one named invariant;
- cites Original sources or labels post-Original behavior;
- adds focused tests for behavior-sensitive changes;
- rejects missing required data instead of synthesizing defaults;
- keeps schema incompatibility explicit rather than adding legacy shims;
- passes formatting, relevant tests, full tests when cross-cutting, and the
  Robin binary build;
- contains no unrelated clippy churn or drive-by cleanup.
