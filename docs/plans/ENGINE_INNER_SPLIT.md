# Completed architecture: `EngineInner` ownership split

Status: completed on 2026-07-19. This file records the resulting architecture
and the invariants future changes must preserve. It is no longer an
implementation plan.

## Result

`EngineInner` remains the atomic deterministic mission-state root required by
save games, replay hashes, rollback, rewind, and multiplayer. Its former flat
field bag is now grouped into nine cohesive owners:

```rust
pub struct EngineInner {
    mission_domain: MissionDomain,
    control: SimulationControl,
    ai: AiRuntime,
    world: WorldState,
    script_domains: ScriptDomains,
    orders: OrderRuntime,
    scripts: ScriptRuntime,
    players: PlayerRuntime,
    feedback: FeedbackRuntime,
}
```

The important boundary is ownership, not source-file size. The owners are
serialized and hashed as one world; they are not independently ticking
services and this is not an ECS conversion.

## Ownership map

| Owner | Authoritative responsibility |
| --- | --- |
| `MissionDomain` | required campaign, mission state/statistics, victory/loss state, briefing state |
| `SimulationControl` | frame clock, deterministic RNG, `SimConfig`, gates, speed, fast-forward |
| `AiRuntime` | global AI state and AI-wide configuration |
| `WorldState` | entities, stable PC order, spatial grid/path state, weather, shield, sight and zone state |
| `ScriptDomains` | canonical doors, patches, buildings, scrolls, production sectors, and deterministic script-facing UI state |
| `OrderRuntime` | sequence manager, IDs, messenger, path/movement work and deferred deterministic queues |
| `ScriptRuntime` | SCB VM state, script globals and ordered script effects |
| `PlayerRuntime` | seats, selection, quick actions, user locks and recording state |
| `FeedbackRuntime` | deterministic camera/marker/titbit/sound-simulation state and pending host effects |

Runtime/static mission attachments are decoded once into `LevelAssets` and
enter through checked prepare/adopt/restore paths. Missing required state is an
error or contextual panic; it is never replaced with a default.

## Execution boundaries after the follow-up cleanup

- `engine/tick.rs` owns the top-level `HourglassPhase` order and cross-domain
  orchestration.
- `engine/sequence_runtime/` owns sequence-phase dispatch, immediate commands,
  and synchronous script sequence driving.
- `engine/level_loading/` owns the ordered entity, environment, PC, and finish
  stages used by `initialize_from_mission`.
- AI value types, contexts, effects, controller behavior and tests live under
  `src/ai/` behind the existing `crate::ai` facade.

These moves are structural. They must not reorder `perform_hourglass`, RNG
draws, entity handles, sequence actions, progress callbacks, or post-load
attachment.

## Snapshot and lifecycle contract

- A live mission owns exactly one concrete `Campaign` in `MissionDomain`.
- Mission teardown consumes the Engine and returns the campaign, next RNG seed,
  and `SimConfig` by value.
- Save, replay, rollback and network state use the explicit current snapshot
  schema. Historical save/replay compatibility is intentionally unsupported.
- Static assets are reattached and parallel structures are validated after
  decode. Invalid lengths, IDs, or missing attachments fail loudly.
- Deterministic presentation state remains in the Engine when rollback or later
  gameplay observes it. GPU, device and external audio resources remain host
  owned.

## Original ordering anchors

- construction and lifecycle: `original-code/RHengine.cpp:323-606`;
- serialized subset and post-load repair: `RHengine.cpp:2408-3007`;
- authoritative tick order: `RHengine.cpp:3446-3777`;
- refresh, sound and `PostInitialize`: `RHgame.cpp:1798-1842`.

## Remaining policy

There is no further physical `EngineInner` mega-split scheduled. When behavior
work touches a command family, narrow its borrows to the relevant owners and
add ordering tests. Do not introduce a context that merely recreates unrestricted
`&mut EngineInner`, temporary ownership parking, or mirrored state.
