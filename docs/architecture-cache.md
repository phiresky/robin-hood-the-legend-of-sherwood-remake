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
them without joining. Completion after cancellation is discarded. Explicit
panic capture releases waiters before resuming unwinding, without poisoning the
owner lock, so the next caller can retry. This does not depend on destructor
unwinding. Abort-on-panic profiles still terminate on panic.

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
prepared-snapshot identity. On source commit `f108c7c2b`, the focused suite passed:
14 passed, 0 failed, 1 ignored (the explicitly selected LLVM case below). The cold
isolated build took 20m49s; tests completed in 0.02s.

The real panic/retry test is explicitly ignored in the default Cranelift suite:
`cfg(panic = "unwind")` alone does not guarantee destructor unwinding with this
repository's native backend. The default suite directly exercises the same
failure completion path; run the real panic case with LLVM:

```sh
CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo --config 'profile.test.package.robin_rs.codegen-backend="llvm"' test --locked -p robin_rs --lib process_asset_cache::lifecycle_tests::panicking_worker_does_not_poison_owner_and_next_caller_retries -- --ignored --exact
```

The exact real panic/retry command also passed on `f108c7c2b`: 1 passed, 0 failed,
0 ignored; LLVM package rebuild took 1m14s. No lower-level read cancellation or
performance benchmark is claimed by these lifecycle regressions.
