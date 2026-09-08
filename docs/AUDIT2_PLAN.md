# Second audit implementation pass

Base: `53e36bdc46542b51a5358674e663961de9f25d1b`, including the CI and
pnpm/Node TypeScript fixes merged after the read-only audit.

The user requested background implementation of all nine audit findings, using
parallel subagents. Eight implementation lanes combine the overlapping
persistence and HTTP upload findings; a coordinator reviews and integrates them.
Main remains unchanged by this pass during implementation and validation.

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

TODO: record final corrected native suites, pure-assets boundary, client feature
lanes, GPU, browser and replay/runtime acceptance. Retained runtime evidence root:
`/tmp/robin-audit2-final.rhlV3O` (temporary local storage, not a durable publication).
