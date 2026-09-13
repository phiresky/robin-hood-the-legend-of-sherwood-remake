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

## Uncapped fast-forward presentation

The earlier graphical runs retained FIFO presentation despite `--fast-forward`.
The flag now disables VSync during gameplay as well as the pacing sleep, including
after accepting in-game options, without changing the saved graphics preference.

Rebuilt with the same parity profile and repeated the graphical Linux `perf`
capture with a 35-second timeout and `robin_rs::window=info` added to logging.
`target/nottingham-parity-uncapped.log` confirms `present_mode=AutoNoVsync`.
Over the same first 1,440 frames, total frame time averaged 10.231 ms (97.7 FPS),
versus the previous optimized FIFO run's 16.679 ms (60.0 FPS).
Rendering/presentation averaged 1.495 ms instead of 8.392 ms. Shared-machine
load still varies; these measurements demonstrate removal of the display cap.

All 61 checkpoint/hash pairs matched the previous optimized run exactly.
`target/nottingham-parity-uncapped.data` contains approximately 5,000 samples
with none lost; its whole-capture sample shares also include idle frames after
replay EOF. The timeout ended the graphical process after playback completed.
The client library suite passed 2,196 tests, with 17 existing ignored tests.

## Detection preparation follow-up

Starting from `30e29cc5e`, replace per-owner target HashSet construction and
the subsequent full human scan with sorted, deduplicated target IDs and direct
live entity lookup. Slot order and stale-target omission remain unchanged.
Owner view-radius cache import rejects mismatched viewer/frame and zero entries
before handle conversion and instrumented lookup. It still copies the same
matching values and preserves surface ownership and zero-as-miss behavior.

The separate parity build and full `robin_engine` suite passed (4,749 unit
tests, 15 integration checks, 38 doctests; existing ignores unchanged).
Both graphical captures matched all 61 replay checkpoints, but unrelated phases
slowed substantially in the modified capture, making that wall-time comparison
inconclusive. Artifacts: `target/nottingham-detection-{before,after}.{log,data}`.

Complete headless replays, measured with Linux `perf stat -e
instructions:u,cycles:u,task-clock`, both exited successfully and matched all
61 checkpoint/hash pairs. These runs include loading and execute the same
1,503-frame recording. Artifacts:
`target/nottingham-detection-{before,after}-headless.{log,stat}`.

| Measurement | Before | After |
| --- | ---: | ---: |
| User-space instructions, billions | 94.94 | 85.47 |
| CPU task time, seconds | 20.54 | 18.77 |
| Whole-process elapsed seconds | 20.68 | 19.11 |
| Detection, ms/tick over first 1,400 ticks | 3.858 | 3.050 |
| Simulation, ms/frame over first 1,440 frames | 8.761 | 7.863 |
| Total headless frame, ms/frame over first 1,440 frames | 11.483 | 10.564 |

This measured 10% fewer instructions, 8.6% less CPU time, and 21% lower
detection time. Exact wall-time gains remain sensitive to shared-machine load.

TODO: Profile remaining detection-context collection and diplomacy work before
changing its lifetime. Cache import still scans the obstacle array; a sparse
index would need correct invalidation across writes, restores, and owner changes.

## Four parallel optimization experiments

Starting from `c0d9462d7`, four independent investigations produced:

- Lazy Enemy detection context construction: build only for a nonempty
  VIEW/OUTOFVIEW block, after final latch updates and before queued Think calls.
  Retain the same owner snapshot for combat metadata. This also avoids an
  unused primary-target multiplicity map clone and duplicate latch collection.
- Bulk byte hashing: a slice hook keeps the general element encoding but lets
  `u8` write its payload in one call. Arrays still omit the length prefix;
  slices/vectors retain their canonical 64-bit length prefix. No state fields,
  hash calls, or schema versions are removed or changed.
- PC optical snapshots resolve the selected sequence element once for both
  current order and PassDoor, preserving PassDoor with an empty order queue.
- Fighter registry sorting caches creation-order keys for one sort, retaining
  stable tie order and avoiding repeated tree lookups. No persistent cache is
  introduced; unrelated actor registry sorting remains unchanged.

`cargo test -p robin_util -p robin_engine` passed 39 utility tests, 4,751 engine
unit tests, 15 integration checks, and 38 doctests; existing ignores unchanged.
New tests compare byte encodings with independently constructed XXH3 inputs
across buffer boundaries and verify fighter ordering across ties, insertion,
deletion, and missing creation identities. Existing detection FIFO and live
snapshot tests passed. Idle detection no longer validates unused combat context;
eventful scans retain the required validation.

The combined parity build passed. Linux `perf stat` measured complete headless
replays before and after; the initial pre-build baseline had substantially
higher CPU/wall time, so a saved-binary baseline was repeated after the modified
run. Both baseline instruction counts agreed within 0.001%. Use the later pair
for wall-time comparison, retaining the shared-machine caveat:

| Measurement | Before (repeat) | Combined changes |
| --- | ---: | ---: |
| User-space instructions, billions | 85.47 | 70.74 |
| CPU task time, seconds | 14.74 | 10.93 |
| Whole-process elapsed seconds | 14.90 | 11.19 |
| Detection, ms/tick over first 1,400 ticks | 2.352 | 1.292 |
| Simulation, ms/frame over first 1,440 frames | 6.110 | 4.236 |
| Total headless frame, ms/frame over first 1,440 frames | 8.325 | 5.961 |

The combined changes measured 17.2% fewer instructions, 25.9% less CPU time,
and 28.4% lower headless frame time. This experiment does not isolate the
contribution of each individual change. Artifacts:
`target/nottingham-four-before-headless-repeat.{log,stat}` and
`target/nottingham-four-after-headless.{log,stat}`; the earlier baseline is
`target/nottingham-four-before-headless.{log,stat}`.

The graphical modified capture (`target/nottingham-four-after.{log,data}`)
also matched all 61 checkpoints, byte-for-byte with the complete headless runs.
It averaged 12.874 ms/frame in its first 1,440 frames. This was measured under
different host load from the earlier graphical runs and is not a controlled
graphical speedup comparison. Linux perf recorded approximately 4,000 samples
with none lost; sample shares include idle rendering after replay EOF.
The graphical timeout ended the process after the full replay completed.
