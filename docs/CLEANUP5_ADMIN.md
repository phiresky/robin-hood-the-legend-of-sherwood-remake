# Admin authority decomposition

Task base: `b70925eab`. The 12,701-line admin binary is decomposed without
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

Frozen code `640b63066` passed both commands below with
`RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2`, using the worktree's
ordinary isolated target directory:

```sh
cargo test --locked -p robin_highscores
cargo test --locked -p robin_highscores --bin robin-highscores-admin \
  --config 'profile.test.package.robin_highscores.codegen-backend="llvm"' \
  backup_owner_unwind -- --ignored
```

The full suite passed 166 library, 35 admin, 5 server, 14 worker and 12 router
tests: 232 total, with the same seven expected ignored cases. Both explicit
LLVM regressions passed, covering operation panic, heartbeat panic, operation
panic after heartbeat failure, and destination SQLite transaction unwind.
Default Cranelift execution is not presented as unwind/destructor proof.

The initial split required local import and accessor call-site corrections;
the compiler also caught four cleanup identity literals crossing the new
private-field boundary. Those comparisons now use `matches_parts`, which
compares exactly device/inode/owner and preserves the existing journal
authentication order without exposing identity construction. No validation
failures required changing backup behavior. Final admin compilation introduces
no new warnings. Formatting and whitespace checks passed.

## Remaining limits

The entry point is seven lines; production owner modules range from 29 to 1,803
lines. The long execution workflow and its publication integration fixture
remain intentionally intact so review can follow the complete lock/EX/drain
sequence. Shared filesystem primitives still warrant focused review: their
module does not turn arbitrary paths or observed tree identities into backup
authority. Destructive cleanup continues to authenticate and revalidate at
the same boundaries as before.

This is structural cleanup, not new crash recovery, a wire-format migration,
or evidence of a production vulnerability. Process abort, runtime destruction
and indefinitely stuck I/O retain the previously documented limits.
