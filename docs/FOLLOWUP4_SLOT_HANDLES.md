# Follow-up 4: queued save-slot handles

Queued `SaveLoadRequest` save/load operations, `PendingLevelLoad`, host-local
snapshot transition bookkeeping and the quick-load confirmation now retain
`SlotHandle`, not a vector index. The owner/generation/name API comes from the
save-owner milestone `1fe11bca5`.

Frame-local picker/debriefing row indices are converted immediately when queuing
an operation. The main-menu/reopen boundary still carries a validated `SlotName`,
which is bound to the newly opened manager before the request is queued. No
engine, replay, network message or save-payload schema was changed.

Every queued local handle is resolved against the current manager before I/O or
application. Cross-mission transitions validate it again before preparing the
next mission's assets. Reordering/deleting a different row does not change the
selected save; deleting and recreating the selected basename does invalidate
the old generation. Invalid selections return errors or are logged explicitly,
without panic fallback to another slot.

The exact preflighted `PreparedGameSave`/`GameSaveFile` flow remains intact:
validating a handle does not reread a payload that has already been decoded.
Consequently replacing the file externally after preflight cannot substitute
different bytes at application time.

## Multiplayer authority

Remote committed snapshots have no local save handle. That is represented by
`None`, replacing the `usize::MAX` sentinel. A missing handle is accepted only
with an already-decoded payload and process-local `CommittedMultiplayer` origin;
ordinary local requests cannot acquire this exemption. In-memory cross-mission
handoff preserves that origin. Serializing and decoding an operation/transition
resets its origin to `Local`; serialized handles likewise cannot restore owner
authority. Host-local handles remain subject to normal generation validation.

## Focused regressions and validation

- A decoded load survives deletion of another row, while a deleted/recreated,
  foreign-manager or serialized handle is rejected.
- A handle-less local decoded load is rejected; a committed remote decoded load
  succeeds, and a committed request without its payload fails.
- A serialized remote transition cannot restore committed authority.
- The existing replaced-file test now uses a real handle and retains the exact
  original decoded payload.

`cargo fmt` and `git diff --check` pass in `followup4-slot-handles`. Compilation
and tests are deferred to the coordinator's combined client build (no separate
cold Cargo build was started here). Required focused tests are under
`main_entry::callbacks::tests` and `main_entry::cli::tests`; full client default
and multiplayer-feature suites remain integration gates. These tests do not
claim panic-unwind recovery or change wire/golden fixtures.
