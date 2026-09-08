# Encoded browser audio retention

Application-owned browser audio now retains encoded bundles in a byte-bounded
least-recently-used cache. Hits refresh recency. The default encoded budget is
32 MiB, separate from the existing 96 MiB decoded PCM budget. This is a
conservative engineering ceiling (one third of the PCM allowance), **not** a
value derived from representative play-session traces. The policy accepts an
explicit budget for tests and future platform tuning; no new user setting was
introduced.

Oversized and empty bundles bypass retention without failing the requesting
fetch/decode and without displacing unrelated retained bundles. Eviction drops
only the cache's handle: shared fetch results and already-running decodes retain
their own references. Therefore this is a cache-residency bound, not a bound on
total browser memory, pending network responses, or JS garbage collection.

Any consumer joining a shared fetch can request bundle retention, including when
the initial consumer did not request retention. Success and failure both remove
this intent when completing the shared operation. Existing session retirement,
request cancellation, generation checks, and catalog isolation remain unchanged.

Debug events expose operations, hits, misses, evictions, bypasses, resident bytes,
entry count, and configured budget. TODO: collect representative mission/locale
traces before tuning the default or claiming reduced total memory/reload latency.

Validation:

- Four platform-independent policy tests cover LRU refresh, surviving borrowed
  values, exact accounting on replacement, oversized/empty/disabled retention,
  repeated synthetic locale/catalog workloads, and deserialization without
  restoring cached authority.
- A browser test uses actual preloaded WAV data to verify shared-request retention
  promotion, eviction while a shared result remains usable, and successful audio
  decoding when the encoded bundle exceeds the configured budget.
- `CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test --locked -p robin_rs --lib
  audio_bundle_cache::tests` passed all four policy tests on source `d45ef8864`
  (cold build: 20m 38s). Browser execution and final combined verification are
  recorded by the browser-audio gate and integration report.
