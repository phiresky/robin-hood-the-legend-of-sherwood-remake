# Replay startup checkpoint — 2026-09-08

Implementation and validated measurements are saved on main at `246d6cd86`.
See [the full research log](COMPRESSION.md#replay-startup-and-boot-payload-reduction-2026-09-08).

## Current results

- Leicester browser replay first-present median: 20.010 → 17.137 seconds at
  16 Mbit/s; 4.059 → 3.930 seconds on unlimited loopback, five pairs per rate.
- Normal and Restart replays pass to EOF, including the terminal-update fix.
- The removed sprite-deferral experiment showed the first image about 2.2 seconds
  earlier but added roughly seven seconds of playback stalls and 9,966,216 bytes.
- Completed performance worktrees and 34 stale task branches were removed.
  Active audit/release worktrees and unrelated branches were left alone.

## Current direction

The user superseded the proposed demand-streaming follow-up with removal of the
separate simulation masks and experimental deferred-pixel path. The partitioner,
mask format, experiment query flag and presentation waits have been removed.
Keep the normal corpus and the validated startup improvements above.

The separate masks did not save usable replay startup time: the first pose appeared
about 2.2 seconds earlier, but playback then stalled for roughly seven seconds.
Standalone compressed masks measured about 8.9 MB; total partition overhead was
9,966,216 bytes. Masks were embedded in initial parts, not fetched by a separate
HTTP request. The ordinary corpus never included this overhead, so removal is
not an additional 10 MB saving over the measured default startup.

## Retained evidence and entry points

- `/tmp/robin-startup-more/replay-matched/summary.json`: 20 matched startup runs.
- `/tmp/robin-startup-more/experimental-final-progress/`: six progress comparisons.
- `/tmp/robin-startup-more/final-replay-pkg`: tested optimized build `c7244b5ddca8`.
- `/tmp/robin-startup-more/replay-fixture/`: actual recorded replay and EOF evidence.
- `/tmp/robin-startup-more/corpus-first-frame/`: partitioned corpus and provenance.
- `/tmp/robin-startup-more/mission/first1-opacity/`: mask/partition research data.
- Commit `c7244b5ddca8`: historical mask format, partitioner and readiness barriers.
- `crates/robin_rs/src/shipping_mission.rs`: retained ordinary streaming loader.

Temporary scripts may still name the removed decoder-perf worktree. Use the
current repository path when resuming; retain original evidence/provenance.

## Removal validation

- `cargo test --locked -j2 -p robin_assets -p robin_rs --lib` passed; the client
  suite reports 1,602 passed and seven ignored.
- `cargo build --locked -j2 -p robin_rs --bin robin` passed.
- Threaded `wasm32-unknown-unknown` / `wasm-release` check with
  `audio,wasm-threads` and `scripts/wasm-threads.cargo-config.toml` passed.
- `cargo fmt --all` and `git diff --check` passed.

These validate the removal; the browser timings above are the retained earlier
measurements, not a fresh benchmark of this cleanup.

## Four-track follow-up completed

All four investigations are integrated as reports and reusable probes; no new
production speedup was verified. [Payload audit](perf/replay-payload-next.md)
found no safe large closure reduction. [Early-fetch diagnostics](perf/replay-early-fetch-next.md)
rejected admission-time bulk fetch because it delayed WASM; historical traces
leave roughly 350 ms for post-WASM transfer overlap.

[Cache measurements](perf/replay-cache-next.md) confirm existing required-asset
reuse: about 31.10 MB cold versus 2,008 shell bytes warm. Warm first-present
medians were 5.04 s for the same replay and 4.80 s for a different recording of
the same mission; concurrent CPU activity limits timing comparisons.

[Transport investigation](perf/replay-wasm-transport-next.md) corrects the prior
Brotli assumption: 6.64 MB was a forced-Brotli capture; normal Chrome encoding
negotiation in local Wrangler produced 7.82 MB gzip for both candidates. The
historical startup harness used the captured representation explicitly.
Production deployment is unchanged. Integrated probe tests and syntax checks pass.
