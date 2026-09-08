# Follow-up 4: owned save completion and explicit slot lifecycle

Source starts at `a6884ab70`. This lane implements save-owner follow-ups 1 and 2;
the recovery UI and queued-request handle migrations are coordinated lanes.

## Physical completion belongs to the runtime owner

Native Continue/Restart background saves capture on the game thread, then use a
single bounded owned worker for serialization and physical payload publication.
The API returns `SaveWriteStatus::Queued`, not success-as-if-already-saved. Browser
Restart retains its immediate memory checkpoint and returns `Completed`; browser
Continue reports unsupported persistence rather than launching an unowned task.

Only owner completion installs captured metadata and publishes the index. A next
write, deletion, rename or autosave metadata replacement drains the prior worker
before mutation. Completion uses stable slot identity, never a captured vector
offset. Pending slots are not loadable. Frame polling observes completion without
blocking on running serialization; explicit operations can block to establish
their required ordering boundary. Thus a slow prior save cannot overwrite a later
manual operation or recreate a deleted slot after deletion finishes.

`RustCallbacks` polls background saves at its existing completion boundary and
delivers a save-specific failure notice. Publication failure remains sticky in
the manager; repeated polls do not create an endless notice stream. Session and
direct-mission wrappers explicitly drain both save owners on every normal or
error return, combining retirement errors with the original outcome. Drop joins
and logs as a cancellation/unwind safety net, not the normal success path.

## Durable metadata recovery

The worker serializes the payload, then publishes `owned-save-recovery.json`
containing fully validated Continue/Restart metadata and its SHA-256 digest,
before replacing the payload. The owner publishes the authoritative index before
retiring the receipt. An index failure therefore does not lose the metadata for
an already published payload.

Reopening validates the index, recovers quick-save metadata, reconciles owned
special-save metadata, then completes deletion. Owned recovery accepts only the
two actual background special names and installs metadata only when the payload
matches its digest. A prospective payload that never arrived leaves the old index
untouched. Receipt retirement is fallible and directory-synchronized. Ordinary
index publication cannot discard an owned receipt from a stale manager.

The existing historical index shape, caller-bound root, autosave manifest,
quick-save receipts and deletion receipts are preserved. The new receipt basename
is reserved. This is desktop same-directory atomic publication, not cross-process
locking or protection against hostile filesystem replacement.

## Slot state and identities

`SaveGameManager` is not Clone or deserializable. Its metadata collection is
private and externally read-only. Runtime `Draft`, `Published` and `Session`
states are explicit and keyed by validated slot name; filling a timestamp cannot
promote a draft. Only completed publication installs Published state, and only
Published rows enter the historical index. Browser Restart has Session state.

`create_draft` is fallible and returns a `SlotHandle`. Handles bind an owner ID,
slot name and generation. Deletion/recreation invalidates the previous generation;
opening another manager invalidates every former handle, even at the same root.
Serde deliberately skips handle owner/generation authority, so decoded handles
cannot authorize runtime operations. `SlotName` remains the external selector
used to resolve a main-menu choice *after* a new store opens. Frame-local indexes
remain read adapters; queued requests use the coordinated handle migration.

Test-only fixture insertion names its state explicitly. Production callers cannot
inject arbitrary metadata or recover runtime authority through deserialization.

## Verification

Added owner and real-manager regressions for blocked completion, payload-before-
metadata publication, drain-before-delete ordering, sequential saves, profile
retirement, sticky/deduplicated failures, payload failure preserving old metadata,
index failure followed by receipt recovery, simultaneous quick/owned/delete
receipts, forged receipt names, draft transitions and invalidated/decoded handles.
Latch waits have bounded timeouts and entry handshakes.

`llvm_owned_worker_panic_is_joined_and_reported_once` is deliberately ignored by
the ordinary Cranelift lane. It must run explicitly with LLVM test code generation
to prove panic unwinding reaches terminal completion before join returns.

Formatting and diff checks run in this worktree. Heavy Cargo/package/runtime
acceptance belongs to the combined integration lane; this document makes no
unexecuted test-pass claim. No target-directory changes or clippy runs are used.
