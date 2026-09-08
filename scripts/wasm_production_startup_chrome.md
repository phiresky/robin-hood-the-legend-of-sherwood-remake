# Production startup benchmark

Build the production frontend, then point the harness at an optimized wasm-bindgen
package and a matching converted data corpus. Inputs are read-only; each invocation
uses a fresh Chrome profile and records `.json`, `.png` and console output:

```sh
pnpm --dir wasm-www install --frozen-lockfile
pnpm --dir wasm-www build:site
node scripts/wasm_production_startup_chrome.mjs \
  --pkg /tmp/robin-perf-next/rebased \
  --datadir /tmp/robin-perf-next/corpus-1m \
  --output /tmp/production-startup --mbit 16
```

Run sequentially, alternating baseline and candidate packages, after builds stop.
The fixture automatically serves the actual built frontend and its published
`/wasm/<hash>` and `/datadirs/demo-leicester/` URL layout. The dummy hash is only a
fixture directory name. Record source commits separately; the harness records input
paths, runtime options and Chrome version. It stages no files into either input.

The frontend exercises the production compressed WASM fetch/decompression/streaming
compile path. The fixture compresses WASM reproducibly with `gzip -9 -n`; text uses Node
zlib level 9 HTTP Content-Encoding. The recorded payload byte lengths are authoritative.
This is a local HTTP/1.1 model with zero added RTT, no packet loss or TCP/header overhead,
not a simulation of Cloudflare's HTTP/2 or HTTP/3 scheduler. All response bodies,
including those requested by workers, share **one** 2,000,000 B/s queue at 16 Mbit/s.
16 KiB chunks are round-robin paced against cumulative deadlines; clients cannot
each obtain a separate bandwidth allowance. Fresh profiles preserve normal caching
within a navigation (for example, workers importing already-fetched glue).
Use `--mbit unlimited` for an unshaped loopback control through the same loader;
this bypasses pacing entirely instead of approximating it with very short timers.

`--mission auto` is the default and omits the mission query, exercising normal demo
startup with its configured demo team. Explicit `--mission Dem_Lei_MP`
uses the engine's forced-mission campaign path; it is **not equivalent** and the
historical fixture immediately loses with that team's missing required characters.
The existing direct harness also accepts `--mission auto`, retaining its old default.
`--query KEY=VALUE` supports repeated experimental switches without changing the shell.
Useful same-package controls are `mission-downloads=ordered` (default prioritized
all-at-once admission), `mission-downloads=unbounded` (alphabetical all-at-once),
`mission-downloads=prioritized` (eight-request admission), `audio-downloads=eager`
(disable the speculative-audio pause), and `renderer-preparation=late` (disable
renderer preparation/reuse during loading). Download admission is separate from
the bounded decode-worker scheduler; these switches do not omit required content.

The 1024×768 viewport uses production CSS unchanged; its canvas currently measures
1008×752 because of page margins. Hardware concurrency is fixed to four, with
`wasm-threads=4`. Headless Chrome uses software SwiftShader. JSON records actual
canvas dimensions, console phases, resource entries, each served request and each
payload chunk's timestamp, and payload bytes by category at bootstrap. Server and
page timestamps are aligned through their absolute performance time origins.
The local fixture has no leaderboard API, so its 404 follows the real browse-only
failure path. Favicon 404 is also retained. Neither endpoint is fabricated.

Bootstrap is a precise engine log endpoint. `--require-present` additionally requires
`startup timing: first mission present returned` from an instrumented candidate.
That mark indicates return from the normal mission render submission path; it is
**not** GPU completion or compositor presentation. Screenshots are captured after
bootstrap, two animation callbacks and 500 ms settle. Their request/completion times
are reported separately and the images must be inspected. They demonstrate rendered
content but do not identify the first physical display frame. The harness makes no
claim that bootstrap or rAF equals visible game presentation.

The benchmark fails if the required bootstrap/present marker is missing. On failure
it saves `.failure.json` with logs, served requests and Chrome diagnostics.

```sh
node --test scripts/startup_throttle.test.mjs
```

Use `--trace` to retain a Chrome trace (`.trace.json`) with timeline, GPU and V8
CPU sampling events, and `--cpu-profile` for the page-thread `.cpuprofile`.
Both cover navigation through the settled screenshot. Profiling adds overhead;
these are attribution diagnostics, not the uninstrumented timing comparisons.

Use `--http-wasm-br PATH` to serve ordinary `.wasm` requests with a captured
HTTP Brotli response. The harness verifies the supplied bytes decode exactly to
the selected package before starting Chrome and records their size and hash.
Without this option ordinary WASM is served with identity encoding, exercising
the loader's static-host fallback. Explicit `.wasm.br` remains a raw sidecar.
Retain compression-capture provenance: local Wrangler and deployed Cloudflare
can produce different compressed sizes for the same content. Do not substitute
offline Brotli quality 11 for the platform's actual HTTP response in Chrome
measurements.

`--replay FILE` launches the actual production URL replay path with `paused=0`.
It preserves the compact envelope's build identity, requires the package's real
isolated `replay_admission` module, and lets the replay header choose the mission.
The harness requires decoded playback, the first mission-present marker, and a
non-null replay RPC state, then records the replay hash and observed playhead.
Do not use a forced mission or a live-game bootstrap as a replay timing proxy.
For cross-build controlled experiments, prepare explicitly identified compatible
fixtures separately; the harness never rewrites the replay build identity.

Use `--http-admission-br PATH` alongside `--replay` to serve the isolated
`replay_admission_bg.wasm` with its separately captured HTTP Brotli response.
The harness verifies it against the admission module and records both hashes.
Capture it with `scripts/capture_wasm_http_brotli.mjs --worktree REPO
--wasm PACKAGE/replay_admission_bg.wasm --output /tmp/admission.br`; retain its
JSON provenance. Main-engine and admission modules require different captures.

Use `--http-wasm-gzip PATH` and `--http-admission-gzip PATH` to replay captured
HTTP gzip bodies at their canonical `.wasm` URLs. Each fixture must decompress
to its package module exactly; Brotli and gzip options for the same module are
mutually exclusive. Output records the encoded bytes and hashes. These options
model known responses explicitly; the local server does not reproduce CDN
encoding negotiation. Retain the capture request headers and provenance.
