# WASM transport investigation (2026-09-08)

No production transport change is enabled: the proposed Worker did not reduce
bytes with normal Chrome encoding negotiation in local Wrangler. A br-only
request looked promising, but was not representative of the browser request.

All captures use the same existing final package from
`/tmp/robin-startup-more/final-replay-pkg/robin_bg.wasm` (21,511,574 B), decoded SHA256
`60d96a212aed25d59ec746c3b93314e86dd08f5eb86ba0c92eb7e6a05c8ce6f5`.
This is the prior c7244b5ddca8 benchmark artifact, not a new build of main.

| Local Wrangler 4.127.1 request | Existing static route | Experimental manual sidecar Worker |
|---|---:|---:|
| `Accept-Encoding: br` | 6,639,157 B Brotli (prior capture) | 5,435,816 B Brotli |
| `Accept-Encoding: gzip, deflate, br, zstd` | 7,822,465 B gzip | 7,822,465 B gzip |

The apparent br-only saving is 1,203,341 B (18.1%), or 0.602 seconds of ideal
16 Mbit/s transfer. The measured mixed-header saving is **zero bytes**. The mixed
responses even have the same encoded SHA256:
`87b2cd01e568a7730d4bc382b55a4d756c3857087b61366cee8441d1bbadafb4`.

Chrome 152 over local HTTPS successfully ran `WebAssembly.compileStreaming`
against the candidate canonical URL; its response was gzip. A single compile
sample was about 594 ms, not a statistically measured improvement. Explicit
identity, gzip, and `br;q=0, gzip` requests all decoded to the exact raw module.
`no-transform` did not prevent Wrangler's mixed-header transcoding. Immutable
cache policy, security headers and MIME were preserved.

Cloudflare documents [manual response encoding](https://developers.cloudflare.com/workers/runtime-apis/response/)
for precompressed bodies, and [original client encoding](https://developers.cloudflare.com/workers/runtime-apis/request/)
when request headers are normalized. The experimental Worker follows both, but
local Wrangler is not evidence of the production edge's exact negotiation.
No production deployment or Cloudflare mutation was performed. The next
transport decision needs a representative edge response or a proven equivalent
local path; neither an offline compressor size nor a forced br-only request is
sufficient. The prior 6,639,157 B capture remains a valid Brotli representation,
not proof that default Chrome requests negotiate it.

## Reproduction

From the task checkout, with installed pinned Wrangler:

```sh
node scripts/capture_wasm_http_brotli.mjs --worktree . \
  --pkg /tmp/robin-startup-more/final-replay-pkg \
  --output /tmp/runtime-candidate-mixed.http --precompressed-worker \
  --accept-encoding 'gzip, deflate, br, zstd'
```

Run without `--precompressed-worker` for the static baseline. Run with
`--accept-encoding br` to isolate the Brotli representation. Outputs use exclusive
creation; choose fresh names for each capture. The temporary probe Worker is
never referenced by checked-in deployment configuration.

Retained artifacts: `/tmp/replay-wasm-transport-next.br{,.json}`,
`/tmp/replay-wasm-transport-{baseline,chrome}-mixed.http{,.json}`,
`/tmp/replay-wasm-transport-http-check.json`, and the local Chrome HTTP harness
`/tmp/replay-transport-http-check.py`. The harness was run before moving the
experimental Worker from `wasm-www/deploy` to `scripts`.

## Validation and implementation boundary

The committed changes add the optional local probe, encoding-selection support
in the capture script, and 16 tests covering negotiation, original client
headers, fallback, HEAD/304, range requests, validators, MIME and security/cache
headers. Frontend typecheck and site build passed under Node 24.19.0.

An attempted deployment integration exposed an additional existing constraint:
operator deployment snapshots seal their exact file closure read-only, while a
script-bearing Wrangler dry run tries to write `deploy/.wrangler/tmp`. A future
integration must include the Worker source in the audited deployment authority
and put Wrangler output in writable scratch, without weakening that closure.
The experimental deployment edits were removed because the browser transfer
benefit was unproven. Current deployment topology and snapshot behavior remain
unchanged.
