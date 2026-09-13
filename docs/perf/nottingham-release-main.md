# Nottingham release baseline — 2026-09-13

Measured main merge commit `446a1981f`, which incorporates
`fix-dialogue-heads-hud-crash` through `996ead5cb`. Main already had newer
detection and UI architecture; conflict resolution retained its live geometry
and tactical reads, deferred its smaller detection aggregate until needed,
and adapted portrait sub-picture selection to its UI resource access.

Merged validation passed with
`cargo test -p robin_util -p robin_engine -p robin_rs --lib`.
The client passed 2,204 tests and utility passed 36, with existing ignores.
Formatting and `git diff --check` passed.

Built separately with `CARGO_BUILD_JOBS=4 cargo build -p robin_rs --release
--bin robin`. This is the unmodified actual release profile: opt-level 3,
ThinLTO, and the repository's x86-64-v3 CPU baseline. Build completed in
11m 10s. No Cargo process was running during measurements.

## Workload

The previous recording stopped at its frame-zero save-marker hash check:
expected `f3720950a22faae0`, actual `cf024eb48bfdc6af`. Its failed capture
(`target/nottingham-release-headless.{log,stat}`) is excluded. No recorded
hashes or replay validation were modified to admit it.

A fresh 12-second headless recording launched `--mission S01_Not_VL` with
`--record target/nottingham-release-fresh.mission`. The persisted recording
contains 2,320 frames, seed 0, default simulation configuration, and no player
commands. Its committed records replay successfully despite the recording
process ending by timeout. This is a fresh idle Nottingham workload, not the
same input history/configuration as the earlier parity investigation.

Use `fullgame_gog`, `--no-sound --fast-forward --rollback-check=false`,
`ROBIN_GAMEPLAY_PROFILE=1`, and the phase/hash logging targets from
[the parity report](nottingham-parity-replay.md). Local profile data is isolated
with `XDG_DATA_HOME=$PWD/target/perf-release-run-data`.

Headless replay was measured with `perf stat -e
instructions:u,cycles:u,task-clock`. Graphical replay used
`perf record -D 5000 -e cycles:u -F 399` with a 35-second timeout. A separate
headless sample capture used `perf record -D 3000 -e cycles:u -F 399` and exited
normally at replay EOF, avoiding post-EOF idle rendering in the hotspot report.

## Results

Mean host timings cover the first 19 windows (2,280 frames). Engine timings
cover 23 windows (2,300 ticks). These runs establish a new release baseline;
main's other changes and the different recording prevent attributing changes
from prior measurements solely to release optimization.

| Graphical phase | ms/frame |
| --- | ---: |
| Simulation | 2.740 |
| Rendering/presentation | 0.791 |
| PostInitialize | 0.741 |
| Preparation | 0.674 |
| Pacing | 0.012 |
| Other overhead | 0.332 |
| Total | **5.290 (189 FPS)** |

The engine tick averaged 1.956 ms; detection averaged 0.137 ms/tick, entity-view
construction 0.327 ms/tick, and world-view construction 0.007 ms/tick. These
are nested costs, not additional host phases. State hashing averaged 0.713 ms
per call at sample 4,700. Audio was disabled.

Headless replay averaged 4.522 ms/frame, including 2.992 ms in simulation.
The whole 2,320-frame process, including loading, used 77.20 billion user-space
instructions, 12.36 CPU seconds, and 12.17 wall seconds. Shared-machine load
affects wall times; CPU task time includes worker threads.

The simulation-focused Linux perf capture recorded approximately 2,000 samples
with none lost. Largest self sample shares include:

| Symbol/work | Sample share |
| --- | ---: |
| AiActorData state hashing | 8.01% |
| AiController state hashing | 3.53% |
| Entity-system phase | 2.92% |
| Entity-view construction | 2.46% |
| Character-profile key hashing | 2.40% |
| Selected-sequence lookup | 2.27% |
| Sequence index lookup | 1.99% |
| XXH3 streaming update | 1.84% |

ThinLTO inlining and sampling affect attribution; these are hotspot indicators,
not predicted speedups. TODO: inspect NPC hash field traversal and repeated
profile/sequence lookup work before changing semantics or cache lifetimes.

All three successful replay runs matched all 93 checkpoints (frames 0–2,300,
cadence 25); their frame/hash pairs were compared directly and are identical.
Artifacts are `target/nottingham-release-fresh-headless.{log,stat}`,
`target/nottingham-release-fresh.{log,data}`, and
`target/nottingham-release-simulation.{log,data}`.
