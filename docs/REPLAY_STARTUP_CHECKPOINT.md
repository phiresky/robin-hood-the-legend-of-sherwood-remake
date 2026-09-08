# Replay startup checkpoint — 2026-09-08

Implementation and validated measurements are saved on main at `246d6cd86`.
See [the full research log](COMPRESSION.md#replay-startup-and-boot-payload-reduction-2026-09-08).

## Current results

- Leicester browser replay first-present median: 20.010 → 17.137 seconds at
  16 Mbit/s; 4.059 → 3.930 seconds on unlimited loopback, five pairs per rate.
- Normal and Restart replays pass to EOF, including the terminal-update fix.
- Sprite deferral remains opt-in and off by default. It shows the first image
  about 2.2 seconds earlier but adds roughly seven seconds of playback stalls.
- Completed performance worktrees and 34 stale task branches were removed.
  Active audit/release worktrees and unrelated branches were left alone.

## Authorized next work — not implemented yet

The user approved reducing the simulation-opacity payload and making sprite
streaming follow actual replay demand. Investigate compact/shared shape encoding,
release network slots before decoding, prioritize required animation pixels, and
prefetch enough future replay frames to avoid moving startup waits into playback.
Measure uninterrupted playback progress as well as the first image. Preserve exact
simulation opacity, replay verification, failure propagation, and session ownership.
Keep the default unchanged until the complete browser comparison demonstrates a win.
When finished, commit, integrate current main, merge into main, and clean up task
worktrees and stale branches. Never stash or change Cargo's target directory.

## Pending user question

“How much time does separate loading of the masks take?”

This has not been isolated as a browser timing measurement. The experimental
initial parts contain the masks alongside their other data; there is no measured
standalone mask HTTP request to quote. The standalone mask compression probe is
about 8.9 MB, implying roughly 4.4 seconds of payload transfer at 16 Mbit/s *if
transferred independently*. That is a bandwidth estimate, not measured marginal
startup cost; shared compression, concurrent downloads, decoding and allocation
must be accounted for. Inspect the exact format and measure those costs before
claiming a mask-loading duration.

Separate masks are needed only to answer shape queries while experimental sprite
pixels are unavailable. Lossless sprite encoding preserves shape but does not make
it accessible before download/decode. The normal eager path uses sprite data.

## Retained evidence and entry points

- `/tmp/robin-startup-more/replay-matched/summary.json`: 20 matched startup runs.
- `/tmp/robin-startup-more/experimental-final-progress/`: six progress comparisons.
- `/tmp/robin-startup-more/final-replay-pkg`: tested optimized build `c7244b5ddca8`.
- `/tmp/robin-startup-more/replay-fixture/`: actual recorded replay and EOF evidence.
- `/tmp/robin-startup-more/corpus-first-frame/`: partitioned corpus and provenance.
- `/tmp/robin-startup-more/mission/first1-opacity/`: mask/partition research data.
- `crates/robin_assets/src/sprite_residency.rs`: current ordinary/blipped bit masks.
- `crates/robin_assets/examples/partition_sprite_startup.rs`: experimental partitioner.
- `crates/robin_rs/src/shipping_mission.rs`: streaming fetch/decode driver.
- `crates/robin_rs/src/game_session/sprite_readiness.rs`: rendering readiness barriers.

Temporary scripts may still name the removed decoder-perf worktree. Use the
current repository path when resuming; retain original evidence/provenance.
