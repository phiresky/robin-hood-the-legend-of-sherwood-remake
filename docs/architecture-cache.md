# Native asset cache loading lifecycle

The application cache now selects a keyed single-flight loading job under a short
owner lock. Synchronous parsing and waiting for background work happen outside
that lock. Same-key callers share the completed product. Locale invalidation
increments the epoch, detaches and cancels the active job, and wakes its waiters;
it does not join a potentially blocked asset reader.

Publication requires both the original job identity and a freshly captured cache
key, including the current localized epoch and external mission/mount/content
identity. Existing stable-bank reuse checks are unchanged. A completed warmup's
stable banks remain reusable after a locale-only invalidation. Cancelled partial
products are never published or substituted for required asset data.

Background jobs do not retain the application owner. Owner destruction cancels
them without joining. Completion after cancellation is discarded. An unwind
guard releases waiters if a worker panics, without poisoning the owner lock, so
the next caller can retry. Abort-on-panic profiles still terminate on panic.

Cooperative cancellation checks run between sprite, FX, menu, and speech stages.
TODO: Individual filesystem reads and sprite decoders are not interruptible;
cancelled work may retain its inputs until the current stage finishes. Cancellation
does not claim an immediate memory release or a measured speed improvement.

Validation command:

```sh
CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test --locked -p robin_rs --lib process_asset_cache::lifecycle_tests
```

Tests cover concurrent callers, unlocked builders, invalidation during a blocked
worker, wakeup and stale completion rejection, owner destruction, failed worker
cleanup/retry, generation changes, stable-bank reuse, reader confinement and
prepared-snapshot identity. Validation is pending the isolated worktree build.
