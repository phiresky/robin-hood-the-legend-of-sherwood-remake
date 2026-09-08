# Cleanup 5: shared picker event/action controller

## Scope

`ingame_menu/save_picker/controller.rs` now owns the common event policy and
persistent button/capture state used by both production picker adapters. It
interprets keyboard navigation, list selection and double-click, scrolling,
button activation, acceptance, deletion requests and cancellation. It emits
typed `PickerAction` intentions carrying `SlotName` targets, not storage writes.

The cooperative load picker still advances one frame at a time. The standalone
loop still owns Save-only committed text, caret editing, IME start/stop and its
async confirmations. Rendering and error-notice scheduling remain separate;
neither adapter polls input again during a notice frame. Existing thumbnail
retirement, draft visibility and partial-deletion error handling are unchanged.

Both adapters now use the same persistent button frame instead of reconstructing
widget state each frame and separately accepting any mouse-up within a button.
Hover/press/release capture survives frame boundaries and stationary repeated
clicks. Dragging a pressed button onto the list neither activates the button nor
selects the underlying row. If a selected save disappears and its button becomes
disabled, the controller releases that disabled button's capture.

Acceptance checks the current model after navigation, so Down followed by Enter
in the same event batch works without the old frame-start enable-state lag.
Once accepted, its target remains the selected stable identity; later events in
that batch cannot retarget it. Cancel/quit takes precedence over acceptance.
Storage confirmation and writes remain explicit adapter operations.

## Regression coverage

- Paired cooperative-refresh and standalone traces for navigation, acceptance,
  pointer deletion, Escape and Quit; they exercise the actual shared controller.
- Persistent pointer capture, stationary repeated clicks and drag-off behavior.
- Accepted identity retained across presentation reordering.
- Disabled-button capture cleared when the selected row disappears.
- The standalone text adapter receives non-ASCII committed input and caret edits
  without duplicate navigation/activation or loss of editable state.
- Existing model, real-store deletion, draft-filter and thumbnail-policy tests
  remain in place; the prior navigation bridge test now exercises the controller.

These are controller/model/text-widget tests, not a claim of driving an OS IME or
both rendered modal loops. `cargo fmt --all` and `git diff --check` pass locally;
explicit client package suites, binary build and browser/native acceptance are
consolidated in the root integration lane. No Cargo test pass is claimed here.
