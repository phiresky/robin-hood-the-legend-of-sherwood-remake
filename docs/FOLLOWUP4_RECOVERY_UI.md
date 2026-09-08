# Save-store recovery and visible operation failures

## Implemented boundary

`save_recovery` preserves a typed `SaveStoreOpenError` and returns a runtime-only
`OpenedSaveStore::{Ready, Cancelled, ExitRequested}`. Only `Ready` contains a
manager. A failed open does not create an empty writable replacement, reconstruct
an index, reset a profile, delete anything, or guess another directory.

Main-menu startup, profile switching, direct graphical mission launches and
session callback construction offer Retry, Cancel and Quit on failed admission.
Retry repeats the existing validated store opener. Cancel leaves the attempted
launch: session/menu-launched mission admission returns to the main menu;
initial menu or direct command-line launch cancellation exits safely. An explicit
Quit/window close propagates to the outer application instead of being swallowed
as confirmation dismissal. Headless callback construction returns the contextual
error immediately; it never waits for graphical input. The independent official
projection-export startup bypass must remain before callback creation.

The dialog uses existing menu assets, localized Cancel/Quit labels and a distinct
Retry label. The original menu catalog has no Retry/recovery-guidance entry;
translation TODOs are explicit. Diagnostics preserve the backend error, wrap long
path components at character boundaries and scroll with arrows/wheel. Both
recovery and acknowledgement retain widget state between frames, so pointer
hover/press/release and drag-off cancellation work across frame boundaries.

## Picker and save-operation presentation

Both picker scheduling adapters show a modal acknowledgement for deletion,
draft-allocation or rename failures. The cooperative adapter consumes one frame
and returns control to its host; the standalone adapter yields normally and
retains the save-name/IME owner. Neither polls input twice in a notice frame or
passes acknowledgement input into the underlying picker. A partial deletion's
updated rows remain truthful while its cleanup error stays visible.

Explicit lifecycle state hides unpublished drafts from Load while retaining them
in Save for retry. Creation and rename use fallible owner APIs. Long-lived slot
request handles and background job ownership belong to the adjacent lanes.

Manual/quick/diagnostic/checkpoint failures now produce a visible `SaveFailed`
banner as well as their detailed log. Continue-mirror errors queue a separate
failure notice rather than disappearing behind a successful manual-save banner.
The owner lane's completion polling calls the same `enqueue_save_failed` entry
point. These notifications do not block simulation or networking.

## Verification

Added deterministic tests for real corrupt-index retry before/after external
repair; Cancel versus close versus Retry; persistent recovery and notice buttons
through actual multi-frame widget input, including drag-off; exact backend
diagnostic retention; failure notice ordering; and draft filtering/error
acknowledgement. Existing real-store partial-deletion tests still apply, now using
explicit published fixture states and lawful metadata.

Formatting and diff checks run in this lane. Heavy `robin_rs` tests, binary builds
and native/browser acceptance are consolidated in the root integration lane; no
test execution or graphical acceptance pass is claimed by this source report.
