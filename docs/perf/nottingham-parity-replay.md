# Nottingham parity-profile performance — 2026-09-13

Follow-up to [the development-build investigation](nottingham-debug-replay.md).
The baseline includes the owner-only ambush fix (`3577147e8`). Both binaries
use the workspace's unmodified `parity` profile: optimization level 3, no LTO,
16 codegen units, incremental compilation, and no debug information.

## Workload and measurements

Build separately with `cargo build -p robin_rs --profile parity --bin robin`.
The baseline executable was preserved as `target/robin-parity-before` after
its original engine and utility code compiled; subsequent source edits were
not part of that executable.

Use the same Nottingham recording and `fullgame_gog` data as the earlier
investigation. Graphical captures run for 45 seconds with:

```sh
perf record -D 5000 -e cycles:u -F 199 -o target/nottingham-parity-before.data -- \
  env ROBIN_GAMEPLAY_PROFILE=1 \
  RUST_LOG=warn,robin_rs::game_session::frame_perf=info,robin_engine::engine::tick::perf=info,robin_engine::engine::tick::phase_perf=info,robin_engine::engine::tick::entity_system_perf=info,robin_engine::replay::perf=debug,robin_rs::game_session::runtime=debug \
  ROBINHOOD_DATA_DIR=/absolute/path/to/datadirs/fullgame_gog \
  XDG_DATA_HOME=/absolute/path/to/worktree/target/run-data \
  timeout 45s target/robin-parity-before --no-sound --fast-forward \
  --rollback-check=false --replay /absolute/path/to/recording.mission
```

The recorded-hash comparison remains enabled when the extra rollback-window
checker is disabled. No build runs concurrently with a measurement. The
graphical replay stops advancing at EOF, so compare the first 12 host timing
windows (1,440 frames) and 14 engine windows (1,400 ticks), excluding later
idle presentation. `perf` sample shares include the entire collection window
and are hotspot indicators, not fixed-workload speedup estimates.

The baseline averaged 16.82 ms/host frame: preparation 1.01 ms, simulation
8.59 ms, rendering/presentation 2.59 ms, and PostInitialize 4.20 ms. Entity-view
construction averaged 0.913 ms/tick and 17.32 calls/tick. State hashing averaged
3.820 ms/call at sample 3,000. `xxh3_stateful_update` accounted for 23.81% of
sampled cycles; no samples were lost. All 61 recorded hash checkpoints matched.
The separate headless baseline exited successfully and its first 120-frame
window averaged 12.473 ms/frame.

## Changes

- Reuse one released entity-view map allocation per worker thread. Only the
  final snapshot reader can release its map; clear every entry before reuse.
  Concurrent/nested snapshots retain independent storage and every new view
  reads current engine state.
- Full view builds no longer allocate and populate a discarded stamp map.
  Prepared detection retains its invalidation stamps and reuses its live-slot
  vector capacity.
- Civilian ambient speech reads its owner's type and admitted animation.
  Civilian periodic calls outside the every-64-frame suffix and expired macro
  timers outside their execution substate avoid unused full contexts. Their
  synchronous owner drains remain in place.
- Batch the canonical scalar byte stream into 1 KiB writes to XXH3. The hash
  algorithm, byte encoding, schema, and all required hash calls are unchanged.

PostInitialize's timing includes the second authoritative no-hourglass frame
boundary and its full-state hash. Skipping that transaction or its empty effect
batches would alter the host contract. This change optimizes the hash itself;
it does not substitute a cached result for potentially mutated state.

## Results

The modified executable was built separately with the same parity command.
`target/nottingham-parity-after.data` captured 3,476 samples with none lost.
Compare the original baseline and modified captures over the fixed windows
specified above:

| Measurement | Before | After |
| --- | ---: | ---: |
| Entity-view builds/tick | 17.32 | 15.72 |
| Entity-view construction, ms/tick | 0.913 | 0.616 |
| State hash, ms/call at sample 3,000 | 3.820 | 1.330 |
| Host preparation, ms/frame | 1.007 | 0.933 |
| Host simulation, ms/frame | 8.591 | 4.946 |
| PostInitialize phase, ms/frame | 4.198 | 1.893 |
| Rendering/presentation, ms/frame | 2.593 | 8.392 |
| Total graphical frame, ms/frame | 16.819 | 16.679 |

Measured view-construction time fell 33%, hashing 65%, and the PostInitialize
phase 55%. Graphical playback remains near the display's 60 Hz cadence;
rendering/presentation absorbs the freed time. These are overlapping timing
buckets, not independent costs to sum. XXH3's streaming update fell below the
1% reporting threshold; remaining samples are distributed across state-field
visits, map lookups, allocation/free, entity views, and snapshot copies.

Host load varied substantially during validation. An additional graphical
baseline capture (`target/nottingham-parity-before-repeat.data`) encountered
a sharp load increase and did not complete the fixed replay window in its
45-second limit. It is excluded from this table. The first modified headless
timing also crossed that load spike and is excluded from the paired comparison.

Afterward, run the saved baseline and modified executable sequentially with
the same profiling environment and `--headless`, using `/usr/bin/time` outside
`timeout 120s` to capture whole-process wall and CPU time. Both completed the
entire 1,503-frame recording with exit code 0. Artifacts are
`target/nottingham-parity-{before,after}-headless-repeat.{log,time}`.

| Headless measurement | Before | After |
| --- | ---: | ---: |
| First 1,440 frames, mean ms/frame | 15.697 | 10.370 |
| Whole process, elapsed seconds (includes loading) | 25.79 | 20.17 |
| Whole process, user + system CPU seconds | 25.91 | 19.05 |
| Peak resident memory, KiB | 332,436 | 317,920 |

This pair measured 34% lower frame time, 22% lower elapsed time, and 26% lower
CPU time. The machine is shared, so treat exact wall-time percentages as
workload measurements rather than isolated benchmark guarantees. The view-build
count reduction and byte-identical hashes are stronger invariants.

TODO: If further CPU savings are needed, profile the remaining state-field
visits and snapshot copies. Their smaller individual sample shares do not
justify weakening snapshot freshness or the authoritative frame/hash contract.

## Validation

`cargo test -p robin_util` passed 36 tests (one existing ignored test).
`cargo test -p robin_engine` passed 4,749 unit tests (14 existing ignored),
15 integration checks, and 38 doctests (one existing ignored).

New regression coverage compares buffered writes against independent XXH3
digests across buffer boundaries and repeated intermediate `finish()` calls;
checks that live snapshot readers prevent map recycling; compares complete
engine hashes for the old full-context and new owner-only civilian speech
paths; and preserves the required missing-layer panic.

The graphical baseline and modified run, and both final headless runs, each
matched all 61 recorded checkpoints (frames 0 through 1,500 at a cadence of 25).
Their checkpoint/hash pairs were also compared directly and are identical.
No desync was reported. `cargo fmt --all`, `git diff --check`, and the separate
`cargo build -p robin_rs --profile parity --bin robin` completed successfully.
