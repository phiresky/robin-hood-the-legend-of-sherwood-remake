# Audit 3: native user-store publication and ownership cleanup

Final combined validation passed; see [AUDIT3_ACCEPTANCE.md](AUDIT3_ACCEPTANCE.md).
The lane-local validation notes below describe the original implementation handoff.

Implements findings 5 and 8 of `CODE_QUALITY_AUDIT_3.md` from base
`7be45078e`. No browser persistence, Original-compatible first-launch recovery,
trust revocation, savegame storage or performance code was changed.

## Changes

- Added `desktop_persistence::write_json`, using the existing `tempfile`
  dependency for exclusive same-directory staging and atomic replacement.
  Serialization and partial writes affect only the staging file. The staged file
  is synchronized before replacement; Unix then synchronizes its parent.
- Both native profile and key-configuration saves use this primitive. Existing
  archive schemas remain unchanged. Application-selected directories are still
  rebound on load, and profile saves still reject a mismatched directory.
- Failures preserve `io::ErrorKind` and carry a downcastable
  `PublicationFailure` with stage, detail and `published()` visibility. Errors
  through replacement leave the previous archive intact; directory-sync errors
  mean replacement happened but durability is not confirmed. Callers can retry
  the same desired snapshot. These immutable save APIs neither clear a dirty
  flag nor change their caller's in-memory state (neither store has a dirty
  flag). Existing higher-level error and rollback policies remain in place.
- Key-store load now handles `NotFound` directly, rather than using `exists()`;
  other read failures are errors, not a fabricated first-run empty store.
- Removed the unused global mutex, accessor and stale initializer comment.
  A whole-repository Rust-source search, including optional-feature code and
  examples/tests, found no callers or alias imports of the removed accessor.
  The real `ApplicationContext` owner remains unchanged.

## Regression coverage

New helper tests inject failure at prepare, write, staged-file synchronization,
replacement and post-publication directory synchronization. They assert exact
old/new visibility, structured error classification, staging cleanup and safe
retry. Additional tests cover partial writes, actual JSON serialization failure
and an abandoned staging file after restart. Store-level restart tests verify
key bindings and profile identity survive beside an incomplete staging file;
an unreadable key archive is not treated as a missing store.

`cargo fmt --all` and `git diff --check` passed in this lane. Compilation and
explicit client test suites are intentionally deferred to the coordinated
integration lane to avoid duplicate cold builds. No test execution is claimed
by this lane's source report; final integration evidence belongs in the parent
acceptance report.

## Limits

This primitive is not a multi-file transaction or a concurrent-writer lock.
Non-Unix platforms do not claim parent-directory synchronization. Full
power-loss durability assumes the selected storage directory already exists:
newly created ancestor directories are not individually synchronized (documented
TODO). Abandoned staging files are ignored, not automatically promoted or
deleted; an unrelated staging file cannot replace the live archive on restart.
Tests model injected failure and restart-visible files, not a physical power cut.
