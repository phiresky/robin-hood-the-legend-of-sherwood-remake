# Persistence and upload workflow refactor

## Implemented boundaries

`submission::complete_upload` now owns immutable intent derivation, the immediate
readiness sample, reservation outcomes, invocation of artifact ingestion, and
durable completion. The HTTP adapter owns multipart grammar, bounded replay
preflight, campaign streaming, and response construction. Existing/busy retries
never invoke the ingestion closure. Acquired uploads alone reach storage.

The observed ingress policy is preserved: authenticate the exact signed offer,
buffer and lexically preflight a bounded opaque replay, sample readiness, reserve,
then write artifacts/read the campaign. "Reserve before any artifact bytes" was
too broad in the audit: replay transport preflight deliberately precedes database
challenge consumption. No base64/zstd/bitcode decoding moved into HTTP.

Database responsibilities now have cohesive private implementation modules:

- `db/uploads.rs`: reservation, recovery, artifact registration and finalization.
- `db/worker.rs`: worker leases and verification request/failure transitions.
- `db/public_queries.rs`: public projections and artifact authorization.
- `db/maintenance.rs`: garbage collection, maintenance leases and backup gates.

All remain implementations of the same `Database`, not independent repositories.
The exact pool, pinned descriptors, outer process fence and SQL transactions are
retained. A structural comparison verified 48 moved methods unchanged modulo
formatting and private helper visibility. Acceptance/publication stays in the
existing `db/acceptance.rs`; campaign aggregation stays with its transaction owner.

Admin production code now uses narrow idle/close methods rather than borrowing
the SQL pool. The hidden `pool()` escape hatch remains for corruption/concurrency
fixtures, including binary and integration tests linked to the normal library.
This is deliberately not claimed as compile-time prevention of raw SQL access.

## Validation

- `cargo fmt --all`: passed.
- `git diff --check`: passed.
- Structural comparison of the 48 moved database method bodies: passed.
- Added router regression for interrupted campaign and forbidden fourth field:
  assert abandoned/tokenless reservation, no submission publication, successful
  exact retry, and one submission after a second exact retry.
- Cargo compilation and runtime tests are delegated to the combined integration
  lane; no independent cold build was started here.

Required combined checks: `cargo test -p robin_highscores` (library, admin and
router E2E tests). In particular retain exact compact preflight/retry, red
admission, concurrent reservation, expired lease/recovery, campaign-fork,
worker lease-loss and backup/fence crash-recovery coverage.

## Deliberate limits

No schema, wire, signature, artifact encoding or transaction policy changes.
Ingestion/mark failures still abandon; preparation/finalization failures leave
uploaded state recoverable. Request cancellation still belongs to existing
detached fenced task owners. No nested per-query fences were introduced.

TODO: stronger type-level authenticated/preflight upload admission could replace
the documented internal caller contract in a future pass. The current refactor
does not make deserialized data a new admission capability or duplicate signature
checks in the reserve-to-finalize workflow.
