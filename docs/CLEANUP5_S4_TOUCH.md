# S4: chronological touch-planning routing

Replaced the touch HUD's whole-batch press search and whole-batch filtering with
one per-event routing pass. Each event explicitly forwards, belongs to the
captured primary-pointer sequence, or toggles planning with a cancellation
decision. The existing dispatcher emits each requested cancellation in order.
Unrelated world clicks before and after a tap remain in their original order;
two complete taps produce two toggles rather than one.

Capture survives event batches until the matching left release. Suppression or
disabled planning prevents new admission but still drains an existing capture.
Right-button and keyboard input pass through. Pointer cancellation retires the
capture immediately and is forwarded to existing interaction-reset handling;
host resets continue to retire all pointer metadata through `cancel_sequence`.

Tests cover world prefix/suffix preservation, ordered double taps, keyboard and
right-button preservation, matching release across batches under suppression,
disabled admission, explicit pointer cancellation and interaction reset.

Scope remains the touch-routing boundary: downstream frame-level modifier
sampling, world-command dispatch, keyboard planning and host lifecycle are not
redesigned. No storage, network policy or gameplay command format changes.

Validation: `cargo fmt --all` and `git diff --check`. Cargo validation is deferred
to the parent's consolidated client suite; no local build/test claim is made.
