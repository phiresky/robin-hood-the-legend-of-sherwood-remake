# Admin authority decomposition

Task base: `b70925eab`. The 12,701-line admin binary is being decomposed without
changing command syntax, authenticated formats, publication policy or runtime
ownership. The binary entry point only starts the runtime and calls dispatch.

## Boundaries

| Module | Responsibility |
| --- | --- |
| `cli` | Typed command parsing, config bootstrap and dispatch |
| `key_activation` | Transaction-bound key intent, pinned activation lock and recovery |
| `verification` | Pinned offline/transaction verification and exact object inventories |
| `execution` | Detached backup completion owner, heartbeat, EX/gate cleanup and publication |
| `cleanup` | Authenticated journal recovery, exact-owned deletion and retention |
| `filesystem` | Pinned descriptors, tree observations and tracked physical I/O primitives |
| `sources` | Pinned restore sources and preserved immutable release authority |
| `capacity` | Conservative capacity estimates; no write admission |
| `policy` | Fixed deployment paths and format constants |

Production imports name their dependencies; there is no root wildcard prelude.
Key activation exposes only initialization/completion. Verification and source
capabilities retain private fields and construction, with read-only accessors
for sibling consumers. Backup gate, exclusive guards and physical-work scope
remain local to execution; splitting files does not split their lifetime.

Tests live under their authority modules. Only explicit shared data constructors
live in the test-only fixtures module. The legacy backup remains test-only and
uses the same completion owner and gate/EX admission as scheduled backup.

The initial relocation was a bounded mechanical top-level item move from the
fixed base. Follow-up edits adapt cross-module reads to accessors. No SQL,
heartbeat, tracked-I/O, exact-token publication or cleanup sequence is intended
to change. Existing uncertain-publication error types remain owned by execution.

## Validation

Pending committed full `robin_highscores` package tests and the two explicit
LLVM admin unwind regressions. Default Cranelift execution is not unwind proof.
