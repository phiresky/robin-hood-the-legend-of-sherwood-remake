# Backup physical completion ownership

Follow-up to the separate admin finding in [AUDIT3_WORKER.md](AUDIT3_WORKER.md),
based on `a6884ab70`. The change is confined to the admin implementation; it does
not alter the worker/API maintenance owners or authenticated backup formats.

## Contract

Scheduled backup now clones its invocation inputs into one detached owner,
including admission, operation.lock, the exact backup token, both exclusive
kernel fences, physical copying and final pool/token cleanup. Cancelling and
awaiting cancellation of the caller does not cancel that owner. It retains the
existing exact-token reconciliation and SQL pool-close ordering before EX ends.

The heartbeat operation uses the existing non-nested physical-work scope. On a
refresh error or recoverable refresh panic it continues draining the operation
and its registered physical work, preserving the original refresh failure if
the operation also fails or panics. Exact-token checks immediately before
authenticated installation and status publication remain in place: draining is
not permission to publish after token replacement.

All admin destination mutation helpers now perform their writes in registered
blocking closures: create-directory, permission updates, bounded document writes,
streaming file copy and file/directory synchronization. File copy retains its
pinned source descriptor and uses bounded-buffer `std::io::copy`, not a whole-file
allocation. Tokio File conversion happens before destination mutation. Closure
completion precedes a returned I/O error, so ordinary error cleanup cannot race
an implicit Tokio write. Existing exact-owned cleanup and authenticated final
installation/status publication remain synchronous.

The private destination SQLite scrub has its own worker, separate from the live
database pool. It now catches update/transaction unwind, lets any transaction
rollback queue, and explicitly awaits connection close on success, error and
panic before returning. A live-pool drain is not substituted for that close.

The legacy `backup()` path is **test-only**, not another shipping command. Its
whole invocation is also detached, including its former pre-gate destination
creation. It now shares the exact production gate/EX admission helper, acquires
operation.lock, and performs directory/store initialization inside the tracked
heartbeat operation. It uses the same exclusive-fence token/pool cleanup.

## Tests and limits

New controlled tests use real database fences and operation.lock around a
latch-blocked physical copy. They cover success, refresh failure, operation
error, acknowledged caller cancellation and replaced token. A single-blocking-
thread case proves a queued-not-started copy is still owned. Pending assertions
hold the latch long enough to detect early release, then verify EX and
operation.lock admit another owner only after copying finishes. The replaced-
token continuation must not reach its publication marker.

Explicit LLVM-only regressions cover operation panic, refresh panic, operation
panic after refresh error, and destination SQLite transaction unwind. Ordinary
destination SQL failure is also tested: the rollback journal disappears and an
immediate exclusive connection is admitted after the helper returns.

These checks establish mutation completion ownership, not production backup
corruption or a remotely reachable exploit. Panic can leave an owned partial for
the existing next-run recovery rather than guessing at destructive cleanup from
an interrupted publication state. A post-publication error remains an error;
authenticated publication/status semantics are unchanged.

Process kill/abort, runtime destruction and permanently stuck kernel I/O are not
recoverable graceful-drain cases. No bounded timeout is allowed to falsely
declare physical quiescence. The existing process-death recovery and stale-partial
policy remain necessary. No detached async descendants are introduced, and no
nested physical-work scope is used.

Validation results will be recorded after frozen committed tests complete.
