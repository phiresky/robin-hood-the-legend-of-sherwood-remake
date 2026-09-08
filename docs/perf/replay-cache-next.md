# Repeat browser replay cache audit (2026-09-08)

The existing HTTP cache already removes every required WASM, boot, and mission
asset transfer on repeat playback. No additional application cache, persistence,
or Service Worker is justified by this measurement.

## Method and scope

`scripts/wasm_production_startup_chrome.mjs` now accepts repeatable
`--repeat-replay FILE`. Each sequence keeps one origin and one Chrome profile,
navigates away to dispose the old game, and starts another real admitted replay.
Each run writes its own JSON and screenshot; `.repeat.json` summarizes the
sequence. Different build envelopes are rejected because the supplied package
must match every replay. Trace mode and repeat mode are deliberately exclusive.

The fixture server previously marked even HTML immutable. It now follows the
production distinction: runtime build paths, datadir paths, and hashed site
assets are immutable; the shell revalidates. The server counts body bytes on
actual writes, including worker requests. Resource Timing entries provide a
second view of cache reuse. No CDP cache-disable override or application cache is
installed. The local shell has no ETag and sends a full 2,008-byte response;
a deployed conditional 304 could be smaller.

Three fresh-profile sequences at a shared 16 Mbit/s server bandwidth each run:

1. Cold Leicester replay, 306 records.
2. The same replay in the warm browser.
3. A different six-record Restart replay in that warm browser, sharing Leicester
   assets and the same build identity.

This tests a different recording, **not a different mission**. Other missions
would fetch their own uncached parts while reusing identical shared URLs. Cache
eviction, browser restart/disk persistence, and real CDN behavior are not
measured here. This is a local production-layout/cache-policy check, not a claim
that a deployed edge was inspected.

The immutable package/site are the prior validated `c7244b5ddca8` artifacts,
including their matching replay envelopes and captured HTTP Brotli bodies.
The ordinary corpus is used; the removed sprite-deferral flag is absent. No
engine or replay changes are included in this cache track. Parallel background
work may contend for CPU, so use these timings to locate warm-load costs, not as
a before/after implementation speedup or replacement for controlled cold-load
benchmarks.

## Results

| Load | Median bootstrap | Median first present (range) | Body bytes through bootstrap |
|---|---:|---:|---:|
| Cold Leicester | 17.635 s | 17.813 s (17.584–18.272) | 31,095,658 |
| Same replay, warm | 4.856 s | 5.044 s (4.682–5.127) | 2,008 |
| Different Restart recording, warm | 4.617 s | 4.796 s (4.134–5.906) | 2,008 |

All nine runs reached decoded replay playback and first mission presentation
without browser exceptions. The required asset body transfer falls from
31,095,658 to 2,008 bytes (the HTML shell), with no runtime, boot, terrain, or
mission-part body transfer before bootstrap on either warm navigation. Some
post-presentation audio requests vary with the recorded actions and capture
interval; those are retained in the raw results and are not counted as startup
asset misses.

This is an existing cache benefit, not a new 31 MB reduction delivered by this
change. The next warm-load opportunities are CPU work—mission materialization,
renderer initialization, and replay setup—not another download cache. The
transport track must preserve immutable canonical runtime URLs and cache headers
when changing compressed responses.

## Reproduction and evidence

```sh
node scripts/wasm_production_startup_chrome.mjs \
  --pkg /tmp/robin-startup-more/final-replay-pkg \
  --site /tmp/robin-startup-more/final-site \
  --core /tmp/robin-startup-more/baseline-core \
  --datadir /tmp/robin-startup-more/corpus-trimmed \
  --replay /tmp/robin-startup-more/replay-fixture/leicester-300-c7244b5ddca8.rhrec \
  --repeat-replay /tmp/robin-startup-more/replay-fixture/leicester-300-c7244b5ddca8.rhrec \
  --repeat-replay /tmp/robin-startup-more/replay-fixture/restart-c7244b5ddca8.rhrec \
  --http-wasm-br /tmp/robin-startup-more/wasm/final-game-http.br \
  --http-admission-br /tmp/robin-startup-more/wasm/final-admission-http.br \
  --mbit 16 --output /tmp/replay-cache-sequence
```

Evidence: `/tmp/robin-replay-cache-next/sequence{1,2,3}-{0,1,2}.json`, PNGs,
per-sequence `.repeat.json`, and `summary.json`. Raw JSON contains input hashes,
browser version, exact per-request byte/chunk timing, console logs, resource
timings, replay identity/state, and startup endpoints. Payload counts exclude
HTTP headers and TCP overhead. First presentation is the present-return marker,
not physical screen scanout. Bootstrap and first presentation are distinct.

Validation: nine actual browser startup runs, JavaScript syntax check,
`cargo fmt --all`, and `git diff --check`. No Rust source changed.
