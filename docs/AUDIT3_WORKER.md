# Audit3 worker completion ownership

The full service suite and three LLVM unwind cases also passed at the combined
checkpoint; see [AUDIT3_ACCEPTANCE.md](AUDIT3_ACCEPTANCE.md) for final integration evidence.

Implements finding 1 of [CODE_QUALITY_AUDIT_3.md](CODE_QUALITY_AUDIT_3.md), based
on `7be45078e`. No performance branches or admin backup implementation changed.

## Ownership contract

The existing detached `run_owned_fenced_operation` remains the cancellation
owner. Its caller may disappear, but the inner operation retains the shared
database fence. The worker heartbeat now drains its operation after refresh
failure instead of dropping it, and retains the original heartbeat error even
if the operation subsequently fails or panics. Refresh construction/polling also
has a narrow unwind boundary that routes panic through this same drain path.
Lease release occurs only after draining.

The new runtime-only `physical_work` scope registers blocking jobs **before
enqueueing** them. A guard in each blocking closure retires its registration
after completion or unwind. The scope catches an operation panic and waits for
all registrations before returning. Thus an error/panic that drops a join handle
does not drop the physical completion barrier. SQL pool draining remains owned
by the existing database fence implementation afterward.

Worker-reachable store readiness probes, hard-link publication, quarantine
rename/unlink, stale-temporary cleanup, root/shard creation and the bounded
verifier launcher use this wrapper. `CampaignStore::import_bytes` performs its
bounded create/write/chmod/fsync in one tracked closure: wrapping only explicit
blocking calls would have missed Tokio File's internal detached writes. Root
creation is also tracked; permission changes are synchronous and cannot outlive
their polling call. This adds one bounded owned copy of campaign bytes, required
to hand borrowed input to a `'static` blocking closure.

## Boundaries and separately reviewed paths

- Runtime ownership is not serialized. The helper is not a general task system.
  It tracks closure execution, not arbitrary destructors of returned values;
  detached verifier output cleanup can retire its private temporary directory
  later. That directory is not a canonical campaign/replay store publication.
- No worker operation spawns detached async descendants. Task-local tracking is
  not inherited by `tokio::spawn`; future such work must explicitly retain the
  completion owner. Nested drain scopes are rejected, not silently untracked.
- The wrapper preserves ordinary Tokio behavior outside a worker scope. The API
  maintenance owner and its policy are unchanged. API-only streaming upload
  writes still use Tokio File; they are not part of this worker guarantee.
- Remaining worker Tokio file operations are read-only. This contract is that
  mutating filesystem jobs and the verifier process cannot outlive worker
  mutation ownership, not that every read-only OS handle has been closed.
- Recoverable Rust unwind is covered only by an explicit LLVM test lane. Abort,
  process kill, runtime destruction and permanently blocked kernel I/O cannot
  be converted into successful graceful drain. The owner intentionally waits
  rather than falsely declaring quiescence; existing process cleanup/lease TTL
  mechanisms remain necessary for process death.
- Admin `run_with_backup_lock_heartbeat` was reviewed separately. Its scheduled
  backup path retains an **exclusive** admission/quiescence pair across gate
  release and SQL pool close (`backup_and_publish_status_with_limit_and_publisher_and_hooks` and
  `release_backup_gate_and_close_pool_under_exclusive_fence`). Its heartbeat
  still drops its own future on error. Partial-backup async filesystem jobs can
  therefore outlive EX after that SQL drain; authenticated final installation
  and status publication are synchronous and cannot themselves publish late
  through this cancellation schedule. No published-backup corruption has been
  demonstrated. TODO: give partial-backup work/cleanup its own physical drain;
  SQL draining alone is not a filesystem drain. The legacy `backup` path also
  uses that helper with a different fence lifetime. These are narrower admin
  follow-ups, not fixed or consolidated into the worker owner in this pass.

## Regression coverage and validation

Added actual database-fence and lease assertions around a latch-controlled
blocking filesystem mutation for success, heartbeat rejection, heartbeat error,
operation error, caller cancellation and recoverable operation panic. Heartbeat
failure is injected at the refresh boundary, with a notification proving the
failure branch has run before checking the still-held fence. A bounded pending
assertion prevents a premature snapshot of the owner from masking early release;
caller cancellation is awaited before checking ownership. Panic cases cover the
operation, refresh, and an operation panic after a heartbeat error, and are
explicitly ignored in the ordinary Cranelift lane.

A one-blocking-thread regression also proves registration covers queued jobs
whose join handle is dropped before the job starts. A source guard prevents
ordinary raw `tokio::task::spawn_blocking` from being reintroduced in the four
worker store/launcher modules (not a substitute for reviewing new async paths).

First frozen full package suite at `6b51dfb5f` passed: 166 library, 31 admin,
5 server, 14 worker and 12 router tests, plus doctests. Worker had two ignored
cases (process helper and explicit LLVM panic). This baseline run preceded the
narrow refresh-panic catch and stronger pending assertions above; final validation
at `0f3781732` passed the same full package suite again (166 library, 31 admin,
5 server, 14 worker, 12 router, doctests; worker now has four ignored tests).
Both runs include existing real bubblewrap/process-group cleanup, SQL lease-loss,
API cancellation, canonical object-publication and admin exclusive-fence tests.

Commands used `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2`, with the
worktree's unchanged local target directory:

```sh
cargo test --locked -p robin_highscores
cargo test --locked -p robin_highscores --bin robin-highscores-worker \
  --config 'profile.test.package.robin_highscores.codegen-backend="llvm"' \
  physical_work_is_drained -- --ignored
```

The LLVM command executed all three panic regressions successfully, including
their post-unwind assertions (3 passed, 0 failed, 0 ignored; 0.51 seconds).

Regression sensitivity was checked with a temporary, uncommitted substitution
of `let drained: anyhow::Result<()> = Ok(());` for the heartbeat error path's
`let drained = (&mut operation).await;`. This reproduces the old release-before-
join policy. The exact heartbeat-loss test failed with exit 101 and
`worker owner returned while physical mutation was still blocked` (0.14 seconds).
The substitution was restored with `apply_patch`; `git diff --exit-code` proved
the source exactly matched `0f3781732` before the LLVM run. A final ordinary
focused worker matrix passed 5 tests, with the 3 LLVM-only tests ignored. No
negative-policy code remains in the branch.

Formatting and whitespace checks passed. This report's final evidence update
is documentation-only; no source changed after the verified snapshot above.
