# Second audit implementation pass

Base: `53e36bdc46542b51a5358674e663961de9f25d1b`, including the CI and
pnpm/Node TypeScript fixes merged after the read-only audit.

The user requested background implementation of all nine audit findings, using
parallel subagents. Eight implementation lanes combine the overlapping
persistence and HTTP upload findings; a coordinator reviews and integrates them.
The coordinator did not modify main. At the user's explicit request, the root
agent merged reviewed checkpoints before all acceptance had finished; final
follow-up integration and cleanup remain root-owned.

| Branch | Scope | Essential acceptance |
| --- | --- | --- |
| `audit2-menu-cache` | Structured source/resource/subpicture cache identity; missing versus broken assets | No cross-source or arithmetic key aliasing; preserve optional sparse frames and default-first fallbacks |
| `audit2-ranked-runtime` | Admission, signing, submission and presentation boundaries; phase-owned tasks | Sign before frame zero; preserve authority and cancellation/duplicate-response behavior |
| `audit2-persistence` | Cohesive database operations and reserved upload workflow outside HTTP parsing | Preserve fencing, atomic lease/challenge checks, retries, crash recovery and reservation before durable artifact ingestion/campaign streaming |
| `audit2-editor` | Pure document commands, reactive session adapter, viewport owner and validated load candidates | Preserve undo/redo, asynchronous load generations, immutable save snapshots and resource disposal |
| `audit2-release` | Policy/topology, pinned filesystem operations, publication outcomes and activation boundaries | Preserve exact canonical artifacts, descriptor authority and uncertain/published-but-unsynced failure distinctions |
| `audit2-commands` | Cohesive command families inside the existing ordering skeleton | Preserve preflight, recording, callbacks, live/replay interpretation and RNG ordering |
| `audit2-diagnostics` | Validated observational diagnostic configuration | Parse gates consistently; no global-environment test races or new simulation/hash state |
| `audit2-parity-schema` | Frozen historical trace layouts and explicit compatibility modules | Preserve field order/types, historical binary bytes and supported version behavior |

## Coordination

- Worktrees live under `.worktrees/` and have exactly their branch names.
- Each implementation agent owns its scoped source files and a track report.
- Heavy Rust builds are coordinated: avoid eight independent cold client builds.
  Cargo targets remain local to each worktree; no target-directory redirection.
- Use explicit affected package tests, not bare workspace smoke tests. Do not
  run clippy, alter shared compiler-cache services, or filter Cargo output.
- Keep source/HEAD frozen while its build or provenance-sensitive gate runs.
- The coordinator owns `audit2-integration`; only reviewed commits enter it.
- Preserve active performance/startup work and untracked `original-code/`.
  Ranked runtime, mission preparation and publication have known overlapping
  performance branches; record integration seams instead of importing them.
- No deployment, push, main merge or worktree deletion by background agents.

## Completion evidence

### Audit correction: upload ordering

At the base commit, `crates/robin_highscores/src/web.rs:2138` intentionally
buffers and lexically preflights the bounded opaque replay before reservation.
Reservation occurs at `web.rs:2260`; durable ingestion begins at `web.rs:2292`.
The audit's shorthand "reserve before bytes" was too broad. This pass preserves
authentication → bounded replay transport preflight → admission/reservation →
durable replay ingestion/campaign streaming, rather than changing transport
admission policy to match that shorthand.

Track reports must distinguish implemented behavior, executed tests, compile-only
coverage and remaining limitations. Final integration needs affected native,
editor, release-tool and parity suites, plus relevant GPU/browser/runtime gates.
Previous-pass green results are a baseline, not evidence for changed sources.

## Integrated implementation

All eight reviewed branches merged without conflicts at `dad3bea0e`:

| Track | Source checkpoint | Report |
| --- | --- | --- |
| Menu cache | `dba8a1c00` | [Menu identity and lookup](AUDIT2_MENU_CACHE.md) |
| Ranked runtime | `c779dd717` | [Ranked workflow ownership](AUDIT2_RANKED_RUNTIME.md) |
| Persistence/upload | `3836962b8` | [Persistence and upload](AUDIT2_PERSISTENCE.md) |
| Editor | `20ff4c59d` | [Editor ownership](AUDIT2_EDITOR.md) |
| Release tools | `4087de29b`, cleanup `0fb2912a1` | [Release authority](AUDIT2_RELEASE.md) |
| Commands | `7f0cea099` | [Command families](AUDIT2_COMMANDS.md) |
| Diagnostics | `823cbd5c0` | [Observational diagnostics](AUDIT2_DIAGNOSTICS.md) |
| Historical schemas | `c493068a4` | [Historical trace compatibility](AUDIT2_PARITY_SCHEMA.md) |

Two independent read-only reviewers checked command/ranked/parity behavior and
persistence/release security boundaries. No source blockers remained after adding
five actual host-task polling regressions. Historical extraction additionally
matched all 42 declarations/conversion implementations against baseline, including
attributes, after removing only visibility additions, comments and formatting.
The coordinator separately reviewed menu mutation/identity/error boundaries and
editor publication/disposal ordering.

Menu identity invalidates on binding, attachment (including partial failure),
shipping merge, dismissal (including reference-driven dismissal), and encoding.
Clones/duplicates and serde/bitcode decode receive fresh process-local identity.
Lazy recovery and eager decode change residency, not logical content identity.
There is no public mutable `ResourceData` access. Old-generation GPU owners remain
alive until menu retirement to protect already queued draws.

## Validation progress

- Baseline `c67186407`: explicit engine/assets/parity/highscores/manifest package
  suites and doctests passed; cold compilation took 13m04s.
- Editor source `20ff4c59d`: full pnpm verification passed (26 shared/app/runner
  tests, 11 pipeline tests, both typechecks, production build); actual Chrome
  lifecycle passed four mounts/32 loads with no retained tracked resources.
  Exact bundle provenance and limits are in its track report.
- First integrated native checkpoint `dad3bea0e`: all selected packages compiled;
  assets passed, including unchanged binary/JSON wire contracts. Engine reported
  4471 passed, one failed, four ignored. The only failure was a new test relying
  on `catch_unwind` under the repository's Cranelift test backend; the expected
  preflight panic occurred. Correcting this to the supported panic-test contract
  does not alter production behavior. Later suites were not executed by that
  failed command and are not claimed as passed.
- New atomic API deprecation and obsolete release wrapper are cleaned separately;
  unrelated baseline warnings are intentionally untouched.
- Corrected native checkpoint `9f0b52c2a`: six explicit package suites and all
  their integration/doctests passed. Engine: 4472 passed/four ignored; asset
  library: 142 passed/four ignored; highscores: 164 library, 31 admin, five server,
  nine worker and 12 real-router tests passed; manifest: 150 passed; parity: 155
  passed plus dependency closure; verifier: 36 library and 13 CLI tests passed
  plus dependency closure. Expected ignored provisioned/helper tests remain
  distinct from execution coverage.
- Initial browser gate at `dad3bea0e` failed compilation before any Chrome run:
  presentation needed a narrow sibling API for receipt-controller ownership.
  `018c78214` adds that API without exposing admission internals. Failure evidence
  remains in `browser-audio/`; a fresh evidence directory is used for reruns.

## Final acceptance

Production client checkpoint: `f5f531c735e928396e7b9cb2d71a09dca9da5f40`.
Final source checkpoint: `d224ecdaec3a2ad35b941c2a943314c67b290eca`.
The difference is a menu GPU test-fixture repair and one optional parity-client
viewport borrow migration; neither changes the tested game binary's behavior.
All Cargo commands used `--locked`, explicit packages/features, one build job,
two test threads and an empty `RUSTC_WRAPPER`. No target redirection or clippy.

| Acceptance | Checkpoint | Result |
| --- | --- | --- |
| Six affected core packages and integration/doctests | `9f0b52c2a` | Passed; exact counts above; tested core configurations unchanged, optional client adapter separately checked below |
| Default `robin_rs` package | `f5f531c7` | 1578 library tests, all integration suites and seven doctests passed; six library/four corpus tests ignored |
| Release-feature client library | `f5f531c7` | 1682 passed, six ignored |
| Default and release-feature native binaries | `f5f531c7` | Both built separately and retained |
| Pure assets, no default features | `07520a632` | 76 library and five fixture tests passed, three ignored |
| Resolved pure asset/content dependency boundary | `aaae0445f` | Passed; no engine in pure asset graph |
| Named Vulkan GPU gate | `07520a632` | Passed required execution/readback and menu ownership assertions |
| Browser audio/multiplayer | `f5f531c7` | Both target checks, module link and real Chrome 24/24 passed |
| Editor verification and real browser lifecycle | `20ff4c59d` | 26 shared/app/runner + 11 pipeline tests, typechecks/build; four mounts/32 loads passed; integrated editor tree identical |
| Native lifecycle | retained `f5f531c7` release binary | All four ordinary/save-load × headless/graphical EOF phases passed |
| Native multiplayer | same retained binary | Headless and graphical scenarios each passed all ten checks, two observed rollbacks and zero desync |
| Optional parity client | `d224ecdae` | `cargo check -p robin_parity --features client` passed |
| Release audio example | `d224ecdae` | `cargo check -p robin_rs --example audio_decode_bench --no-default-features --features release` passed |
| Named formatting gate and whitespace checks | final source/docs | Passed |

Validation exposed and corrected three fixture/API issues without weakening
production validators: the Cranelift panic harness noted above; five host-task
tests whose offer expiry did not match their signed grant (`7b61d5130`); and the
GPU helper's unsupported flattened integer-map `serde_json::Value` construction
(`3ddb65de6`). The latter now loads actual SRES/PIC bytes and produces corrupt
encoded data through the public shipping encoder. All cache assertions remain.
These are concrete reasons to prefer valid domain fixture builders over ad-hoc
serialized object construction.

The optional parity-client check also found a pre-existing missed consumer:
`draw_background` already required `ViewportState` at the base, but the adapter
still passed `Host`. `5ad9693fd` now borrows the exact viewport populated directly
above the call. No historical layout or rendering API changed.

### Retained evidence and limits

Evidence root: `/tmp/robin-audit2-final.rhlV3O`, outside all worktrees. It is
temporary local storage, not a durable publication. Preserve it before host
cleanup if longer retention is needed.

- `robin-default`: SHA256 `434e904c26dbd52ecaa3bc29e506319ba8d64db85d7852e02b04b02deec6c5ff`.
- `robin` (release features): SHA256 `75358d5cbe8bdb1ba1d52cf722d6f339968bca70d59d6cb50ae7453b1303941f`.
- `browser-audio-final2/summary.json` and retained WASM SHA256
  `321b1f3b78889678abbf8116e06e1398880b17f21c0d4ce2b23c1d83612093db`:
  Chrome/driver 152.0.7977.64, wasm-bindgen runner 0.2.127. Earlier failed
  `browser-audio/` evidence remains separate from corrected runs.
- `native-lifecycle/summary.json`: exact binary/source and immutable replay
  digests, real quicksave/load restoration and post-restoration replay hashes.
- `multiplayer-headless/summary.json`, `multiplayer-graphical/summary.json`:
  isolated loopback namespaces, both-seat commands, rollback, in-process snapshot
  reconnect, seat retention and periodic hash agreement. No process-restart
  durability or remote service coverage is claimed.

An additional named GL gate failed before test execution because this host lacks
amd64 EGL/GLES libraries (loader inventory exposes only i386 EGL/GLES). Vulkan
passed; GL is not reported as passed and no system packages were installed.
Browser tests are synthetic ownership/protocol/identity tests, not browser game
GPU or audible-output acceptance. Native runtime disabled sound. No original
licensed parity corpus, Android/Windows execution, deployment or publication was
performed. Existing unrelated warnings and explicitly ignored tests remain.

The fresh code-quality audit is separate from this implementation pass. In
particular, independent review found a pre-existing worker filesystem-quiescence
gap: heartbeat failure can release a lease/fence before detached blocking storage
work finishes. This is a source-based scheduling counterexample, not a reproduced
backup-corruption incident or an audit2 regression. Save-index authority/error
handling and further design debt also remain in `CODE_QUALITY_AUDIT_3.md`; green
acceptance here is not a claim that those unrelated defects were fixed.
