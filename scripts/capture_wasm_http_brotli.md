# Capture the runtime server's HTTP Brotli representation

This tool runs the supplied worktree's installed, exactly pinned Wrangler locally,
serves one raw WebAssembly module with that worktree's runtime `_headers`, and
captures the response to `Accept-Encoding: br`. It does not deploy or modify the
package/worktree. Wrangler's configuration and served files live in a temporary
directory and are removed on success, failure, timeout, or SIGINT/SIGTERM.

```sh
node scripts/capture_wasm_http_brotli.mjs \
  --worktree /absolute/path/to/repository \
  --pkg /absolute/path/to/wasm-bindgen-package \
  --output /tmp/runtime-http.br
```

Use `--wasm /absolute/path/robin_bg.wasm` instead of `--pkg` for an individual raw
module. The output parent directory must exist. Both output files must be new:
`runtime-http.br` contains the encoded HTTP body, and `runtime-http.br.json` records
raw/encoded/decoded SHA-256, byte lengths, response headers, runtime-header SHA-256,
Node version, installed Wrangler version and repository pin, compatibility date,
and input paths. Decompressed bytes must match the input exactly before publication.

Install the supplied worktree's frontend dependencies first. The helper refuses a
Wrangler version differing from its exact `wasm-www/package.json` pin. It uses
`wasm-www/deploy/runtime-headers.txt` and the compatibility date from
`wasm-www/deploy/wrangler-runtime.json`. Metrics are disabled and the server binds
only to loopback with `--local`. It never invokes a deployment command.

The default overall capture deadline is 60 seconds; change it with
`--timeout-ms 120000`. Individual local HTTP requests also have a 10-second timeout.
Subprocess errors, unexpected HTTP responses, wrong content encoding, invalid WASM,
and decoded-byte mismatch fail visibly, retaining Wrangler diagnostics in the error.
The child receives SIGINT during cleanup and SIGKILL after three seconds if needed.

This reproduces the **pinned local Wrangler HTTP encoder**, not a guarantee that
an arbitrary live CDN deployment will produce identical bytes. Preserve the JSON
provenance alongside any startup benchmark using the captured representation.
