# Code-quality audit after the second refactor pass

Audited source: `f5f531c735e928396e7b9cb2d71a09dca9da5f40`.

This is a fresh, read-only source audit, not another implementation pass. The
user owns the concurrent performance work; no performance branches were edited,
imported or evaluated here. Findings below concern correctness, ownership,
persistence and maintainability. They do not claim exhaustive line-by-line
coverage of the repository or new runtime reproductions of every failure.

The preceding implementation and its acceptance evidence are documented in
[AUDIT2_PLAN.md](AUDIT2_PLAN.md). Its tests are not proof that the newly identified
edge cases below work. Two independent reviewers checked the worker lifecycle
and save-storage findings against the production call paths.

## Conclusion and priorities

The clearest remaining design problem is not file length. It is that some
long-running work and persisted data still carry implicit authority: a worker
future is treated as the lifetime of filesystem work, and a serialized save
index can determine where subsequent writes happen. Other failures are converted
into apparently successful or empty state.

| Priority | Finding | Kind | Suggested scope |
| --- | --- | --- | --- |
| High | Worker filesystem work can outlive its lease/fence after heartbeat failure | Source-confirmed lifecycle gap | Bounded worker operation owner and regression |
| High | Save indexes restore storage authority from serialized paths | Source-confirmed ownership gap | Runtime save-store owner plus validated slot names |
| High | Broken save indexes become empty writable managers | Source-confirmed data-loss path | Explicit open/recovery states and safe allocation |
| Medium-high | Save deletion is neither error-reporting nor index-durable | Source-confirmed consistency gap | One transactional delete operation |
| Medium | Profile and key configuration writes lack atomic replacement | Source-confirmed failure exposure | Shared desktop persistence primitive |
| Medium | Two save/load UI drivers duplicate state transitions | Structural debt | Shared picker model, distinct scheduling adapters |
| Low-medium | Correlated protocol fixtures are too easy to assemble incorrectly | Test-maintenance debt observed during this pass | Small validated fixture builders |
| Low | An obsolete global key-store entry point remains public | Dead/competing ownership API | Remove after whole-workspace reference verification |

All correctness findings predate the audit2 refactors. No production fix for
these new findings is included in this report.

## 1. Make worker ownership include actual filesystem completion

**Evidence:** `crates/robin_highscores/src/bin/worker.rs:55` implements
`run_with_write_lease_heartbeat`. On refresh failure it stops polling the
operation, releases the maintenance-write lease, then leaves the scope containing
the pinned future. The outer `run_owned_fenced_operation` eventually finishes
its database fence. `db_fence.rs:492` waits for the SQL pool to become idle, not
for detached filesystem work.

The operation can reach `process_job`'s final campaign import at
`bin/worker.rs:1089`. `campaign_store.rs:159` calls
`secure_fs::link_immutable_object`, whose implementation at `secure_fs.rs:206`
uses `tokio::task::spawn_blocking`. Dropping the waiting async future is not a
completion barrier for an already queued/running blocking closure.

A reachable ordering is:

1. Final campaign publication queues a blocking hard-link operation.
2. The maintenance lease refresh fails or reports replacement/expiry.
3. The worker releases its lease and its outer fence finishes after SQL drains.
4. The blocking closure publishes the filesystem object afterward.

That violates the expected filesystem-quiescence boundary. It is not evidence
that an unleased result enters the accepted leaderboard: acceptance has its own
lease checks. No backup corruption or production incident was reproduced.

**Refactor:** make the operation owner retain and drain physical work before
releasing the lease and outer fence, while preserving the original refresh
failure as the outcome. The API's `web.rs:568` maintenance owner already waits
for its operation's terminal completion after refresh failure; use that as a
contract reference, not a reason to blindly merge all heartbeat implementations.
If cancellation is needed, it must cancel and join the underlying work rather
than only drop its waiter.

**Required tests:** latch-controlled blocking mutation, forced heartbeat failure,
assert the fence cannot report quiescence before the mutation completes; repeat
for operation error/panic and caller cancellation. Keep verifier process-group
cleanup and SQL lease-loss tests. Audit admin backup heartbeat separately before
assuming its different exclusive-fence lifetime has the same defect.

## 2. Separate the save index from the authority to read/write a directory

**Evidence:** `SaveGameManager` in `crates/robin_rs/src/savegame.rs:253` derives
serialization/deserialization and includes a public `save_directory`.
`load_index` at line 1105 reads the index from the supplied directory but does
not replace its decoded directory with the supplied one. It then calls quick-save
recovery using the decoded root. Save, thumbnail and index paths at lines 809,
814 and 1082 all use that stored root.

Copying an index to a different profile or machine can therefore leave subsequent
reads, writes or deletes pointing at the original location, if it still exists.
In contrast, `PlayerProfileStore` and `KeyConfigStore` explicitly rebind decoded
data to their caller-selected directory.

Manual slot filenames also remain unrestricted strings. Metadata validation at
`savegame.rs:157` checks schema, mission, time and provenance, but not basename
containment. Absolute filenames and parent components can escape the intended
directory through the existing joins. The observed entry point is local persisted
metadata; no remotely reachable exploit path was established.

**Refactor:** deserialize a data-only index, validate it, and bind it to a
runtime-owned save store selected by the application context. Use a validated
slot-name type rather than accepting arbitrary paths. Preserve existing on-disk
compatibility deliberately: an old serialized directory can be read as legacy
metadata without being trusted as runtime authority.

**Required tests:** copied indexes, an old root that still exists, absolute and
parent-relative names, cross-platform separators, duplicate slot names and
profile switching. Verify rejected metadata performs no external reads/writes.

**Existing safeguards retained:** quick-save recovery accepts only the exact
QuickSave/ExQuickSave names and validates payload digests; autosaves have their
own strict generated-name validation. These do not validate ordinary save names
or repair the manager's decoded root.

## 3. Do not turn a broken save index into a fresh writable store

**Evidence:** `load_index` already handles `NotFound` as the genuine first-run
case. Nevertheless, `open_for_context` at `savegame.rs:279` handles every other
error by logging that there is no index and constructing an empty manager.
This includes malformed JSON, permissions, invalid metadata and failed recovery.

The empty manager resets `next_id` to zero. `next_filename` at line 1171 generates
`Savegame_000` without checking existing payloads. A subsequent manual save can
therefore atomically replace an existing payload that disappeared only from the
in-memory index. Atomic replacement does not make that target choice safe.

**Refactor:** explicit `Missing`, `Ready`, and `NeedsRecovery`/error outcomes.
Do not enable ordinary saving against a silently empty view of an unreadable
store. Recovery should preserve the original index, validate discovered payloads,
and derive collision-free allocation state. Even normal allocation should not
reuse an existing payload without the explicit overwrite path.

**Required tests:** corrupt/unreadable index beside a valid `Savegame_000.json`,
obsolete metadata, a corrupt recovery receipt, and stale `next_id`. Assert the
existing save bytes remain unchanged and failure is visible to the caller.

This recommendation is about save-index failure handling, not an instruction to
remove documented Original-compatible first-launch player-profile behavior.

## 4. Give save deletion one durable, fallible owner

**Evidence:** `remove` and `remove_by_filename` at `savegame.rs:757` and 772 call
`remove_files`, which discards both filesystem errors, and then remove the memory
entry. Neither persists `saves.json`. Both UI deletion paths in
`ingame_menu/save_load.rs:150` and 1155 invoke that operation, sort and rebuild
the picker without saving the index.

There is no drop-time index flush. The visible-slot collector does not filter
missing payloads. Reopening the store can therefore bring the deleted row back;
if file deletion failed, the current UI still treats the operation as successful.
Index-based selections also become fragile when deletion shifts the in-memory
ordering while a later mission bootstrap reloads an older disk index.

**Refactor:** one save-store deletion operation that returns a meaningful outcome,
owns durable index publication and handles partial filesystem cleanup. Pick a
recoverable ordering (for example, a tombstone protocol) instead of promising a
multi-file transaction from unrelated `remove_file` calls. Use stable slot identity
when carrying a selection across store reopenings, not a vector index alone.

**Required tests:** delete then cancel/exit/reopen; unlink failure; index-write
failure after payload change; optional missing thumbnail; delete one row then
load another through a newly constructed callback manager. Preserve the existing
rule that manual APIs cannot delete auto-managed autosaves.

## 5. Apply durable publication consistently to small user stores

**Evidence:** native `PlayerProfileStore::save` at
`crates/robin_rs/src/player_profile_store.rs:97` and `KeyConfigStore::save` at
`key_config_store.rs:117` serialize and use `fs::write` on the live JSON file.
Save payloads and the Spellforge trust store already use staged replacement.

A failed or interrupted write can leave the only profile/key archive truncated.
That matters beyond losing a preference: profile recovery deliberately resets
parallel key/trust associations to avoid assigning an old numeric identity to a
new profile.

**Refactor:** a narrow desktop persistence helper for same-directory staging,
file synchronization, replacement and parent-directory synchronization where
supported. Keep browser localStorage behavior separate. Define whether errors
occurred before publication or after publication but before confirmed durability;
retain dirty/retry state appropriately. Do not reuse the server's much stronger
descriptor-pinned deployment machinery indiscriminately.

**Required tests:** failed serialization/write, failure before replacement,
post-replacement sync failure, and restart recovery. The old file must remain
valid whenever publication has not occurred. Preserve explicit startup recovery
policy and ensure errors are observable rather than silently resetting data.

## 6. Share picker behavior without merging its two scheduling loops

**Evidence:** `LoadPickerModalState` at `ingame_menu/save_load.rs:67` is a
cooperative one-frame driver so networking/automation can continue. The standalone
`show_save_load` at line 698 owns an async modal loop and supports save-name
editing. Selection, deletion, scrolling, thumbnail ownership and related
presentation transitions appear in both. The deletion problem above has two
production callers to repair.

**Refactor:** extract a picker model with stable slot identity, selection/scroll
state and explicit actions/effects. Keep the cooperative driver and standalone
driver as separate adapters, and keep save-only text editing explicit. Put
storage errors and confirmation transitions in the shared model instead of
duplicating fixes across the loops.

**Required tests:** run equivalent action traces through both adapters, including
delete failure, scrolling after deletion, stale selections, confirmation cancel,
thumbnail retirement and multiplayer diagnostic-save filtering. Preserve modal
input suppression and IME behavior.

## 7. Make protocol test fixtures lawful by construction

This pass initially had five new ranked driver tests fail before reaching their
intended behavior because their shared offer expiry did not match the signed
preflight grant. The correction is committed in `7b61d5130`; this is not a
remaining production or test failure. It illustrates the maintenance cost of
manually assembling cryptographically correlated fixtures.

**Refactor:** small domain-specific fixture builders that derive linked values
from one authority and validate the result before returning it. Negative tests
should then mutate one intended invariant. Avoid a universal fixture framework
or validation bypasses. Keep byte-level historical golden fixtures independent
of the current encoder.

Also distinguish panic expectation from unwind/recovery tests. The repository's
Cranelift test profile does not establish the latter merely because a panic
occurred; use the existing explicitly selected LLVM lanes when destructor or
recovery behavior is the contract being tested.

## 8. Remove stale competing ownership APIs

`key_config_store.rs:263` still exposes `GLOBAL_STORE` through
`KeyConfigStore::global`, with an initializer comment pointing at the former
global setup. The actual application owns key configurations through
`ApplicationContext`. No call to the old method was found in workspace Rust
sources, and no alias import was found in the checked sources.

This is a small cleanup, not an urgent correctness issue: remove or explicitly
deprecate the obsolete entry point so new code cannot accidentally create a
second key-store owner. Verify all optional tools/features before removal and
update the stale documentation. Do not remove the real application-owned store.

## Suggested next implementation order

1. Worker operation lifetime regression and fix, as a bounded service task.
2. Save-store authority, open/recovery and deletion in one coordinated lane;
   these share the same ownership boundary and should not race each other.
3. Atomic profile/key persistence in a separate lane, coordinating any shared
   storage helper with the save-store work.
4. Shared picker model after the save-store API stabilizes.
5. Fixture builders and obsolete API cleanup as independent smaller tasks.

The existing audit2 package/browser/runtime suites should remain acceptance gates,
but the newly described failure schedules need their own tests. Do not regenerate
goldens, weaken validation, or claim generic happy-path replay coverage proves
crash consistency. No ECS conversion, actor rewrite or new performance project
is recommended by this audit.
