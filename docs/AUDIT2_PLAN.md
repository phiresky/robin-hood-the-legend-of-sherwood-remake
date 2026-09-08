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

TODO: record the implementation commits, independent review, combined validation
and any findings that cannot be completed without broader authority.
