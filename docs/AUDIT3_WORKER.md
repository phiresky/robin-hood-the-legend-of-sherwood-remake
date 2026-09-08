# Audit3 worker completion ownership

Implements finding 1 of [CODE_QUALITY_AUDIT_3.md](CODE_QUALITY_AUDIT_3.md), based
on `7be45078e`. No performance branches or admin backup implementation changed.

## Ownership contract

The existing detached `run_owned_fenced_operation` remains the cancellation
owner. Its caller may disappear, but the inner operation retains the shared
database fence. The worker heartbeat now drains its operation after refresh
failure instead of dropping it, and retains the original heartbeat error even
if the operation subsequently fails. Lease release occurs only after draining.

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
  release and SQL pool close (`backup_scheduled` and
  `release_backup_gate_and_close_pool_under_exclusive_fence`). Its heartbeat
  still drops its own future on error, and filesystem completion there needs
  a distinct audit of backup destination jobs, publication and failure cleanup;
  SQL draining alone is not a filesystem drain. The legacy `backup` path also
  uses that helper with a different fence lifetime. No blanket claim that either
  path is fixed or safe follows from this worker change, and they were not
  consolidated into the worker owner.

## Regression coverage and validation

Added actual database-fence and lease assertions around a latch-controlled
blocking filesystem mutation for success, heartbeat rejection, heartbeat error,
operation error, caller cancellation and recoverable operation panic. Heartbeat
failure is injected at the refresh boundary, with a notification proving the
failure branch has run before checking the still-held fence. Panic coverage is
explicitly ignored in the ordinary Cranelift lane.

A one-blocking-thread regression also proves registration covers queued jobs
whose join handle is dropped before the job starts. A source guard prevents
ordinary raw `tokio::task::spawn_blocking` from being reintroduced in the four
worker store/launcher modules (not a substitute for reviewing new async paths).

Validation results will be appended after the frozen committed source runs.
