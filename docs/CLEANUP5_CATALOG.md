# Save catalog responsibility cleanup

The runtime owner no longer maintains separate metadata, state, and generation
collections. A private `SlotCatalog` stores one `SlotEntry` per slot, with an
immutable validated basename, metadata, explicit lifecycle state, and generation.
The basename is intentionally separate from the legacy metadata filename:
replacement must prove identity unchanged, never silently rename a live handle.

Catalog insertion/removal changes the complete entry. Publishing metadata,
renaming, and sorting preserve its generation; deletion/recreation grants a new
generation. Autosave refresh preserves manual handles and intentionally retires
old autosave handles, including same-name replacements. Refresh validates all
incoming rows, retained-row collisions, and generation capacity before changing
anything. Failed replacement leaves the entire catalog intact.

The public manager remains the ordered operation facade. Its `saves()` accessor
now returns an exact-size, double-ended iterator of borrowed metadata; consumers
cannot mutate catalog identity or lifecycle. There is no compatibility snapshot
or slice emulation. Test-only malformed-fixture mutation remains isolated behind
`cfg(test)`.

The new boundaries have deliberately narrow authority:

- `catalog` owns runtime identities, lifecycle transitions, selection resolution,
  presentation ordering, and the validated Published-only persistence projection.
- `recovery` only interprets receipts and reads digest-matching payload candidates.
  It cannot mutate a catalog, publish an index, or retire recovery evidence.
- `persistence` publishes a data-only index under the caller-bound root and retires
  quick-save evidence only after durable index publication.
- `SaveGameManager` coordinates owned completion and quick → owned → deletion
  recovery, applying candidates through the same catalog transitions as live saves.

The serialized index and receipt formats are unchanged, including the legacy
`save_directory` compatibility field. That field never grants runtime authority.
Drafts and memory-only Session checkpoints remain excluded from persisted indexes.
Runtime catalog/entries deliberately do not implement serde: owner identities and
generations must be newly granted, not reconstructed from untrusted persisted data.

Regression coverage includes promotion, sorting, rename, refresh, stale generation
rejection, invalid identity replacement, cross-collection case-folded collisions,
generation exhaustion, and Published-only projection. Existing recovery,
publication-failure, browser Restart, and owned-worker schedules remain in the
manager suite. Formatting and diff checks run in this lane; Cargo acceptance is
coordinated in the combined integration lane to avoid duplicate native builds.

Remaining scope: deletion orchestration and payload capture stay on the facade
because they coordinate publication ordering; this change does not introduce a
generic storage framework or alter their recovery protocol.
