# Audit 3: save-store authority, recovery, allocation and deletion

Implements findings 2, 3 and 4 of `CODE_QUALITY_AUDIT_3.md`. The source lane
starts at `7be45078e` and merges the user's main `246d6cd86`, preserving its
prepared-save identity, in-memory browser Restart and autosave sequencing work.

## Ownership and compatibility

`SaveGameManager` is a runtime owner and intentionally does not implement serde.
The persisted `SaveIndex` contains only slots and allocation state; an old
`save_directory` field is accepted as an unknown legacy field but cannot grant
storage authority. Loading binds the caller's root before recovery. New indexes
omit that old field. This is deliberate forward migration; older executables
requiring that field cannot read newly written indexes without migration.

`SlotName` is a serde-validated portable basename. Paths, separators, extensions,
control-file names, Windows devices and empty names are rejected. Index names
are case-insensitively unique, and published special-slot metadata must agree
with the filename. Index validation precedes recovery I/O. Existing quick-save
digest checks and strict autosave ownership remain intact.

The public metadata list remains mutable for existing in-process game and
autosave callers. Paths revalidate slot identities and index publication rejects
duplicate names. This is not descriptor-pinned filesystem isolation: local
symlink replacement by another process is outside this desktop storage contract.

Session-only Restart remains entirely memory-owned. Its upsert and deletion do
not consult desktop recovery receipts or require a writable directory; opening
an index cannot reconstruct its process-local payload or identity authority.

## Open and allocation policy

Only a genuinely missing index starts an empty manager. JSON, read, metadata,
quick-recovery and deletion-recovery failures return errors; the original files
are not quarantined, overwritten or automatically discarded. `open_for_context`
propagates these errors, including autosave-manifest errors. Existing top-level
non-fallible menu/callback constructors stop with an explicit recovery diagnostic
instead of granting writes against a fabricated empty store. Original-compatible
player-profile first-launch policy is unchanged.

Recovery is conservative: repair/restore the reported index or receipt, preserving
the existing files, then reopen. There is no guessed reconstruction of a corrupt
manual index. Missing-index orphan payloads and thumbnails are reserved during
allocation, as are already indexed slots, even with stale `next_id`. Unexpected
allocation filesystem errors stop allocation rather than imply nonexistence.
An unpublished manual slot uses atomic no-clobber publication, so a payload
appearing between selection and write is not silently replaced. Published slots
retain the explicit overwrite behavior. A failed post-publication directory sync
can leave a new payload present; retry will refuse to clobber it, requiring
inspection/recovery rather than pretending publication never happened.

This does not introduce cross-process coordination of the index itself. Two
simultaneous applications sharing a profile are not supported index writers.

## Deletion protocol

`remove` and `remove_by_filename` are fallible store operations:

1. Validate identity and refuse auto-managed autosaves.
2. Publish and synchronize a deletion-intent receipt.
3. Remove the logical row in memory and durably publish the updated index.
4. Remove payload and optional thumbnail, then synchronize the directory.
5. Retire the receipt and synchronize its retirement.

A missing payload or optional thumbnail is already-cleaned state. Other unlink,
index-publication or synchronization errors are returned and logged by UI callers.
After published intent, the row is logically deleted even if physical cleanup is
pending. The receipt remains, ordinary writes are blocked, and reopening retries
the idempotent protocol. An ambiguous intent-publication failure checks whether
the new receipt is visible and updates the in-memory list accordingly. There is
no claim that two unlinks and an index rename form a single filesystem transaction.

Opening with both quick-save and deletion receipts first installs digest-matched
quick metadata, then completes deletion. Private recovery publication can proceed
while the deletion receipt blocks ordinary writes. Main-menu selections carry a
`SlotName` to the newly opened callback manager, avoiding stale vector indexes
after deletion/reordering. Picker-model structural consolidation is a separate
coordinated lane; this lane only migrates error handling and that reopen boundary.

## Regression coverage and acceptance

Added focused tests for legacy-root relocation with the old root still present;
path/device/control-name and casefold-duplicate rejection; corrupt, obsolete and
unreadable indexes; missing/stale-index orphan allocation; delete/reopen and stable
selection; unlink failure and retry; index failure with payload preservation;
failed intent; simultaneous quick/delete recovery; corrupt quick receipts; and a
payload appearing after new-slot selection.

`cargo fmt --all` and `git diff --check` run in this lane. Heavy Cargo acceptance
is intentionally delegated to the combined client integration lane to avoid
duplicate clean dependency graphs. This document does not claim tests passed
until that lane records their actual results.

TODO: a dedicated user-facing recovery workflow can later replace the explicit
top-level fail-closed diagnostics; it must not restore the former empty writable
fallback. Background special-slot writers retain their existing lifetime model;
this work does not claim cross-process or detached-thread transaction isolation.
