# Completed architecture: GameHost mirror removal

Status: completed on 2026-07-19. `GameHost`, `game_host`, engine-state swaps,
campaign leases, refresh caches and the compatibility shell have been removed
from production Rust. This file records the replacement architecture.

## Result

Script natives now operate against the canonical Engine domains. There is one
authoritative copy of entities, AI state, campaign values, NPC custom values,
doors, patches, buildings, scrolls, selection, sound-simulation state, zones,
and mission UI state.

```text
EngineInner
  MissionDomain / WorldState / AiRuntime / PlayerRuntime
  ScriptDomains / OrderRuntime / FeedbackRuntime
                         ^
                         | scoped mutable access
MissionScript + SCB VM --+-- Native dispatch
                         |
                         +-- ScriptEffects
                               ordered EngineCommand stream
                               external audio requests
                               deterministic simulation barriers
```

`MissionScript` owns only VM/script state. Immutable authored tables and script
names are borrowed from `LevelAssets`. Transient `This`, current-scroll and
nested-call state is scoped to the active call/driver and cannot appear in a
snapshot.

## Same-callback semantics

Original natives mutate live objects. Rust preserves that property: a mutation
followed by a query in the same callback observes the mutation. Deterministic
changes are applied synchronously or at an explicitly tested simulation
barrier; they are not hidden in a generic end-of-frame queue.

`ScriptEffects` retains global command order across presentation commands,
external sound requests, and simulation barriers. Synchronous sequence
successors are driven depth-first before older hourglass work, matching the
Original sequence/condolation boundaries.

## Snapshot contract

- No call frame, active VM yield, borrowed binding or external resource is
  serialized.
- Snapshot creation rejects an active script call/yield.
- Persistent SCB VM state and unconsumed deterministic script effects are
  serialized and state-hashed.
- Current schemas are save version 50, replay version 7, and network version
  13. Older save/replay compatibility is intentionally unsupported.
- Required targets and bindings fail with contextual errors or panics; missing
  data never becomes false, zero, empty, or a fabricated default.

## Original behavior anchors

- native dispatch and campaign values: `original-code/RHScript.cpp`;
- VM call/yield behavior: `original-code/GVMCoreCustom.cpp` and
  `original-code/Profile/GEngineScript.cpp`;
- synchronous sequence advancement: `original-code/RHsequence.cpp` and
  `original-code/RHsequenceelement.cpp`;
- engine/script lifecycle: `original-code/RHengine.cpp` and
  `original-code/RHgame.cpp`.

## Remaining policy

Do not reintroduce query caches, engine-state swaps, raw-pointer access to the
Engine, or a host-shaped compatibility fixture. New natives should request the
narrow canonical capability they need and emit only genuine host effects.

Spellforge Lua remains rejected in replay, rollback verification and
multiplayer until it has a versioned event surface and serializable state
policy. That containment is separate from the completed SCB/GameHost removal.
