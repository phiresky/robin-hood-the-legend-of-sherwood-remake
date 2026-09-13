# Nottingham debug replay CPU profile — 2026-09-13

Linux `perf` and the existing frame timers identify NPC detection and repeated
AI observation construction as the main cost in the reported slow replay.
This is a measurement of the development binary, not a release benchmark.

## Setup

- Code: `160e4be13`, built separately with `cargo build -p robin_rs --bin robin`.
- Replay: `2026-09-13T19-01-59+02-00-2341092-692229296.mission`, Nottingham
  `S01_Not_VL`, seed `1497613574919999461`.
- Data: `datadirs/fullgame_gog`; separate test profile under `target/run-data`.
- Rendering enabled on AMD Radeon 780M / Vulkan.
- Arguments: `--no-sound --fast-forward --rollback-check=false --replay <recording>`.
- No concurrent Cargo build. Both runs terminated at their deliberate time limits;
  neither establishes full replay completion.

First capture: `perf record -D 10000 -e cycles:u -F 199 --call-graph dwarf,16384
-o target/nottingham-perf.data -- env ... timeout 55s target/debug/robin ...`.
It captured 8,959 samples, 143.5 MB, with no lost samples reported. Event collection
started after the first ten seconds to exclude initial loading.

The second capture omitted call stacks, used a 45-second run and wrote
`target/nottingham-detail-perf.data` (6,890 samples). Both enabled
`ROBIN_GAMEPLAY_PROFILE=1` and the `frame_perf`, engine `perf`, and `phase_perf`
log targets documented in `docs/GAMEPLAY_PROFILING.md`. The second also enabled
`robin_engine::engine::tick::entity_system_perf=info`.

## Measurements

First 120-frame host aggregate in each capture, milliseconds per frame:

| Phase | First capture | Detailed capture |
| --- | ---: | ---: |
| Preparation | 6.54 | 7.21 |
| Simulation/modal handling | 159.79 | 181.15 |
| Rendering/presentation | 5.55 | 6.29 |
| PostInitialize | 20.55 | 21.83 |
| Pacing | 0.024 | 0.023 |
| Total | 192.88 | 216.91 |

The first capture corresponds to about 5.2 host frames/second. Entity systems
averaged 141.44 ms over the first 100 engine ticks, within a 142.88 ms engine
tick total. The second 100-tick window measured 153.00 ms and 154.34 ms,
respectively.

The detailed capture's first 100-tick aggregate measured:

- Entity systems: 164.97 ms/tick.
- NPC tail: 125.92 ms/tick.
- Detection refresh: 123.66 ms/tick, 12,800 calls (128/tick).
- Entity-view construction: 81.29 ms/tick, 10,529 calls (105.29/tick).
- World-view construction: 1.82 ms/tick, 200 calls (2/tick).

These nested timers overlap; their times must not be added together. Detailed
per-callback timing and run-to-run variation also affect the second capture.

`perf` samples include `tick_enemy_ai_refresh_detection`,
`build_owner_context_scratch_without_forecast`, `build_entity_views_and_stamps`,
entity-ID comparisons/B-tree searches, and xxHash/SSE helpers. DWARF unwinding
and inline source resolution were incomplete for this Cranelift development
binary; inclusive call-tree percentages are not reliable subsystem totals.
The existing wall-clock timers provide the aggregate attribution above.

## Optimization target

`engine/ai/tick_data.rs::build_ai_observation` builds full entity views and
sight obstacles. `engine/ai/mod.rs::build_entity_views_and_stamps` walks the
live entity set, builds each eligible view, and computes its stamp. The
post-detection dispatch path constructs fresh observations at synchronous
stimulus boundaries, explaining why many rebuilds can occur in one tick.

TODO: Reduce repeated full observation construction in NPC detection, then
measure with the same replay. Preserve predecessor-stimulus mutations,
creation-order position projections, entity removal, and deterministic replay
hashes; replacing these observations with a single stale tick-start snapshot
would change gameplay semantics. PostInitialize is a secondary measured cost.
No performance implementation changes were made during this investigation.
