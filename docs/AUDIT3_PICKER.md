# Audit 3: shared save/load picker behavior

Implements finding 6 in `CODE_QUALITY_AUDIT_3.md`, on top of the save-store API
migration in `6e08fc606` and subsequent save-store hardening. The lane includes
current main through that prerequisite; picker changes do not modify performance
code.

## Changes

- `ingame_menu/save_picker.rs` owns stable `SlotName` selection, presentation
  mapping, load/save and multiplayer-diagnostic filtering, viewport navigation,
  confirmation identity, and deletion outcomes. Reordering retains the selected
  save; disappearance clears selection rather than selecting the new occupant
  of an old vector index. Keyboard navigation keeps selection in the viewport.
- Both production adapters in `save_load.rs` use that model and the same input
  navigation bridge and row-mapping implementation. The cooperative one-frame driver still returns control to
  the mission host; the standalone async loop retains save-name editing,
  modal input handling, caret behavior and IME start/stop.
- Both deletion paths use one confirmation/storage bridge. It deletes the
  confirmed basename and rebuilds from the manager after every outcome,
  including a logical deletion followed by failed cleanup. Errors remain in
  model state and are logged, not silently treated as successful deletion.
- Thumbnail ownership is keyed by `SlotName`, not a manager/list offset. Both
  adapters use shared retirement and widget-reset helpers; clear/close and
  changing identity retire the owned surface, while reordering alone does not.
  Manual deletion of autosaves remains prohibited by both model and store.

The remaining `ListRow` values and manager indices are frame-local presentation
coordinates used by existing label/metadata rendering. They do not own selection
or a pending confirmation. `SaveLoadOutcome::Slot` is still an immediate outcome
against the caller's current manager; the save-store lane separately changed
main-menu selections carried across manager reopenings to stable names.

## Regression coverage and validation

Seven new tests cover:

1. Stable selection/confirmation across reordering, and explicit stale-slot
   confirmation failure.
2. Cancellation, pre-publication error, and a partial-success cleanup error.
3. Navigation and scrolling clamped after row deletion.
4. Shared autosave, special-save and multiplayer diagnostic filtering.
5. Thumbnail retirement decisions on identity change, clear and stable identity.
6. The actual shared input bridge receiving equivalent traces with cooperative
   per-event refresh boundaries versus one standalone event batch.
7. The actual shared deletion bridge with a real temporary store: cancellation,
   successful deletion, and a deliberately non-unlinkable payload after durable
   logical deletion. The fixture validates published metadata and reopens its
   index before use, after successful deletion, and after cleanup recovery.

These are model/adapter-bridge tests, not a claim that GPU modal loops or IME were
driven by the tests. Existing lifecycle/browser acceptance remains relevant to
renderer and scheduling behavior.

`cargo fmt --all` and `git diff --check` pass in this lane. Per coordination,
heavy Cargo work is consolidated into the root integration lane: run the explicit
`robin_rs` package tests (including `ingame_menu::save_picker` and
`ingame_menu::save_load`), build the `robin` binary, and retain native/browser
acceptance results there. No Cargo test pass is claimed in this source handoff.
