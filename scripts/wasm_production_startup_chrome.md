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
compile path. WASM uses `gzip -9 -n`, exactly as deployment staging; text uses Node
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
