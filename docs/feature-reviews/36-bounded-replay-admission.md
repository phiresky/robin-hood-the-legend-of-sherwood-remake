# Feature 36 — bounded canonical replay admission

## Status

Revised implementation is complete on `codex/feature36-compact-admission` and
has been reconciled through Feature 29 (`main` at `ab5d105fc`). It is not on
`main` yet. The implementation does not allocate a save/replay/network schema;
it always compiles against and admits only the current `ReplayFile` schema and
the exact current source-build identity.

## Decision implemented

Production replay admission accepts exactly one representation:

```text
rhrec-<12 lowercase hex source identity>-<base64url-no-pad(zstd-19(bitcode(ReplayFile)))>
```

There is no content negotiation, format discriminator, legacy replay schema,
JSON/JSONL fallback, or alternate ranked representation. JSONL remains only as
the explicitly named local crash-safe recorder/developer lane
(`*.rhrec.jsonl`). Public browser, native RPC, file, and leaderboard admission
use the compact representation.

## Threat model and containment

The envelope may be supplied by an untrusted local file, browser URL, RPC
caller, or leaderboard submitter. Base64 and zstd expansion are directly
bounded, but a typed decoder can allocate from attacker-controlled collection
lengths before post-decode validation. The game therefore never uses its live
process as the first typed-decoding boundary.

- Native public playback starts the same executable in a hidden, pre-runtime
  one-shot helper mode. On Unix the parent installs address-space, CPU, file,
  and descriptor limits before `exec`, applies a 15-second wall timeout,
  verifies a SHA-256 receipt for the exact input, bounds worker output, and
  kills/reaps helpers on timeout or broken input pipes. Platforms without a
  hard containment implementation fail closed. This includes Windows until a
  Job Object implementation is added; an unconstrained subprocess is never
  treated as isolation.
- Browser playback imports a minimal non-threaded validator wasm in a fresh
  Dedicated Worker. Its private, non-shared linear memory has an explicit
  384 MiB maximum. Publishing inspects the final optimized wasm and fails if
  memory is imported, shared, missing, duplicated, or has a different maximum.
  The worker is terminated after one reply or after 15 seconds. The game wasm
  accepts a one-shot SHA-256 proof for only the exact compact string.
- The leaderboard API performs only bounded envelope preflight and opaque
  quarantine/spooling. Its independently resource-limited verifier worker is
  the sole server-side typed decoder and imports the same
  `robin_replay_format` crate.

## Validation performed inside containment

- Exact ASCII envelope grammar, unpadded base64url, compressed and
  decompressed byte ceilings, and zstd frame-window ceiling.
- Exact current source identity and `REPLAY_SCHEMA_VERSION`.
- Current native `ReplayFile` bitcode only.
- Canonical re-encoding at all three stages: bitcode, zstd, and base64url.
- Dense, ordered frame ordinals; declared/materialized frame agreement; and
  in-range hash/save/load metadata.
- Per-frame, aggregate-entry, typed-collection, typed-string, aggregate-string,
  and aggregate-serializer-work limits with checked arithmetic.
- The embedded full-fidelity `Campaign` is itself bounded, bitcode-decoded,
  canonical-byte checked, traversed under the same aggregate budget, and
  checked with `Campaign::validate_history_schema`. It is not treated as an
  opaque blob that can defer hostile work until mission startup.

The limits are independent so a caller cannot trade unused allowance in one
dimension for unbounded work in another. Defaults are deliberately above the
observed local corpus while the external 384 MiB/CPU/wall boundaries remain the
last line of defence.

## Feature 34 composition

Feature 34's mission-generation spool remains the sole export source. It
publishes only complete JSONL record boundaries, caps the local spool at
64 MiB, rejects stale writers, poisons on ambiguous durable-write failure, and
uses a single-flight background export. The export result is then encoded into
the same compact production representation defined by
`robin_replay_format`; Feature 36 adds no second export or upload format.

## Research and parity notes

- Zstd frame-window parsing follows RFC 8878 section 3.1.1 and also configures
  the library decoder's `window_log_max`; the hand preflight is not the sole
  enforcement mechanism.
- Native containment follows `setrlimit(2)` semantics. Windows is intentionally
  fail-closed because Microsoft Job Objects are the appropriate hard process
  memory boundary and have not yet been implemented here.
- Browser containment relies on the WebAssembly memory declaration plus Worker
  process isolation, and CI inspects the post-`wasm-bindgen`/post-`wasm-opt`
  artifact rather than trusting source flags alone.
- `original-code` is absent from this worktree. The original game has no known
  compact Rust replay envelope to preserve; this feature is a safety boundary
  around the Rust port's deterministic replay system, not an original-game
  parity behavior.

Primary references:

- <https://www.rfc-editor.org/rfc/rfc8878.html>
- <https://man7.org/linux/man-pages/man2/getrlimit.2.html>
- <https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects>
- <https://webassembly.github.io/spec/core/syntax/modules.html#memories>

## Verification

- `cargo test -p robin_replay_format`: 16/16 passed on Feature 29 main.
- `cargo test -p robin_rs replay_format`: 2/2 focused unit tests passed.
- `cargo test -p robin_rs --test replay_admission_process`: cold native
  helper accepted the exact current compact artifact and SHA-256 and rejected
  plausible JSON without fallback.
- `pnpm test`: 11/11 passed, including cold public-load ordering and rejection
  before proof installation/game RPC.
- `pnpm typecheck`: passed.
- `cargo fmt --all -- --check`: passed after the containment hardening patch.
- `cargo build -p robin_rs --bin robin`: passed.
- The release-profile validator wasm built successfully, instantiated in Node,
  and rejected malformed input. Inspection of the final
  `wasm-bindgen`/`wasm-opt`/`wasm-strip` artifact reports exactly one private
  memory with `initial=65 max=6144` pages (384 MiB).

## Review focus

The material policy choices are the 16 MiB compact/decompressed ceilings,
384 MiB worker memory ceiling, 500,000-frame ceiling, 15-second timeout, and
the deliberate Windows fail-closed behavior. The codec shape itself reflects
the owner's confirmed compact-bitcode-only decision.
