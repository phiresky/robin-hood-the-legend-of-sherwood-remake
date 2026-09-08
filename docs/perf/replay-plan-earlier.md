# Earlier authoritative replay downloads — 2026-09-08

Browser URL replays prepare the admitted launch once and start the first eight
prioritized mission dependencies before the normal mission-load boundary.
Requests are joined by that loader, with no second dependency index or extra
mission payload. The normal decode, terrain, renderer and audio preparation paths
remain in place. `?replay-preparation=late` retains a same-package comparison.

This is a small scheduling improvement at 16 Mbit/s, not a download-size reduction.
The earlier proposal to fetch during the main WASM transfer remains rejected.

## Matched browser measurements

Twenty starts, five alternating early/late pairs at each rate, used frozen source
`13ff8839fabb`, four decode workers, fresh Chrome 152 profiles and SwiftShader.
No task builds or profilers ran during the comparison. Both arms used the same
production site, replay, corpus, modules and encoded HTTP responses. Every input
hash remained unchanged and every run reached actual replay playback without a
browser error or duplicate mission request. The complete mission request sets
matched across arms.

| Rate | Late first-present median | Early first-present median | Median paired saving |
| --- | ---: | ---: | ---: |
| 16 Mbit/s | 17.941 s | 17.840 s | **172 ms** |
| Unlimited loopback | 4.426 s | 4.481 s | **−39 ms** |

All five 16 Mbit/s pairs improved: 172, 182, 111, 69 and 181 ms. Loopback pairs
were mixed: 90, −39, −98, −55 and 87 ms; there is no demonstrated loopback gain.
The difference of medians is not the median of paired differences. At 16 Mbit/s,
the median first mission request moved from 6.750 s to 6.594 s, consistent with
hiding some preparation latency. In the initial unlimited diagnostic, the early
batch started after the window was ready but before the worker pool joined;
“early” therefore means before normal mission loading, not necessarily before
GPU readiness on every machine.

Both modes transferred **32,344,294 bytes before bootstrap**. Captured local
Wrangler responses using Chrome's mixed `Accept-Encoding` header were gzip:
7,820,809 bytes for the game and 382,718 for admission. The harness replays those
verified responses explicitly; this is not a production-edge latency estimate.
The shared rate limiter supplies 2,000,000 payload bytes/s, without added RTT,
packet loss or TCP overhead. First present is a submission-side endpoint, not
physical display completion.

These totals and absolute times supersede neither the older forced-Brotli
fixture nor its source comparison: the transport representation and source
revision differ. Compare the two arms above to assess this scheduling change.

## Ownership and behavior

Only admitted nonempty browser replay URLs take the early path. The replay's
resolved mission assets and restored campaign/profile selection remain the
source of truth. Custom archives use the existing restoration path. Interactive
and multiplayer launch ordering is unchanged.

The bounded compressed-file handoff is bound to the exact datadir allocation and
an owner token. Whole-batch path/conflict validation precedes requests. Replaced
or abandoned launches drop unused handoffs and abort their requests; dropping
an old owner cannot remove a newer owner's entry. Local authenticated preloaded
files take precedence. Decode errors still propagate through normal loading.
The early batch's loading-byte count becomes available when each body completes;
subsequent requests keep their existing chunk progress reporting.

Source `4a350e912` enables the already-measured early branch by default; the
comparison build required `replay-preparation=early`. The final optimized
package was rebuilt from `4a350e9126e5` and checked without that query flag.

## Validation and retained evidence

- Implementation native client suite: 1,639 passed, seven existing ignored.
- Native game build, threaded optimized WASM build, formatting and diff checks passed.
- Final default-policy focused suite: 15 passed, one existing ignored.
- Explicit early-mode browser correctness: normal 306/306 and Restart 6/6 records
  reached EOF without browser errors or replay hash failures.
- Admission retains its isolated, nonshared 384 MiB memory contract.
- Harness gzip decoding/option-conflict guards and actual gzip browser startup
  passed. The new EOF gate also passed the retained Restart fixture.

Artifacts: `/tmp/robin-replay-plan-earlier/paired/{summary,runs,inputs-before}.json`,
all 20 JSON/log/PNG runs, `compare.py`, the two HTTP capture metadata files,
`early-eof.*` and `early-restart-eof.*`. Fixture provenance records benchmark-only
build-envelope repinning; recorded payloads and simulation verification are
unchanged. The reusable harness now supports verified HTTP gzip fixtures and
`--replay-eof` correctness runs.

Final default-mode browser checks passed normal 306/306 and Restart 6/6 EOF,
with the early-batch marker present and zero duplicate mission requests. The
matching admission module accepted both fixtures and rejected the prior build
identity. Final game WASM is 21,506,125 raw bytes and 7,820,803 captured HTTP gzip
bytes (the six-byte change from the comparison capture includes build identity
and default selection). Final results, exact hashes and capture headers are in
`final-validation.json`, `final-package-provenance.json`,
`final-admission-validation.json`, `final-game.http.gz.json`,
`final-admission.http.gz.json` and `final-*-eof.*` under the same artifact directory.
These final correctness runs are not another timing series.
