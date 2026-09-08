# Second audit after structural cleanup

Reviewed source: `010e62d5dcd1c87a7077aa10428d5cd8b83ff390`.
Implementation base: `4bd6bf8d89d81c38ed4db07b88f0b42b287fdb81`.
The intervening changes fix module imports/accessors and test fixtures, not the
audited policies. Three independent read-only reviews ran while initial tests
completed. The first-wave full release client suite subsequently passed (1,773
library cases, eight ignored, plus integration/doc tests); service tests passed
232 cases with seven explicit ignores. These are not final second-wave gates.

This bounded audit is now closed: implement all seven findings below, review
their implementations, and run combined acceptance. It does not promise that
no further improvements exist. Performance/deployment and the user's concurrent
replay-tooling work remain outside scope.

## Findings and implementation contracts

### S1. One publication transaction for synchronous and background saves

`savegame.rs::write_save_from_engine` publishes a payload and updates memory;
executor callers separately publish the index. The background path already owns
a recovery receipt. A synchronous index failure can therefore leave payload and
metadata disagreeing without the equivalent recovery evidence. This is a
source-derived failure window, not a reported user incident.

Use the manager-owned payload/metadata/index transaction for synchronous manual
and diagnostic saves too. Return committed evidence and remove caller-managed
publication sequences. Preserve nonfatal thumbnail behavior, special-slot policy,
on-disk formats and browser persistence. Test new drafts, overwrites, diagnostics,
unrelated slots, and failure before/after payload replacement and index publication.

### S2. Keep exact prepared-load ownership through mission handoff

`callbacks.rs::preflight_load_with_origin` accepts independent slot/payload/origin
values; `PendingLevelLoad` also exposes redundant mission identity. A valid slot
handle can be paired with a different decoded payload by a caller.

Represent unresolved local selection, prepared local load and committed remote
load explicitly. Private prepared ownership must keep the exact handle/payload
together; derive mission identity from the payload and preserve generation checks
without another disk read. Test stale/decoded handles, cross-mission payload
retention and slotless remote admission. Do not invent remote authority from data.

### S3. Make modal publication a typed authority transition

`ingame_menu/modal_net.rs::publish` conflates successfully queued client proposals
with send failures. Batch/mission/dialogue drivers sometimes ignore its boolean
and decide completion from role alone, allowing local completion or an authority
wait after a failed send.

Distinguish host decision queued, client proposal queued and error. Share the
transition policy across callers; retain pending outcomes on failure and never
record/complete a failed publication. Test disconnected channels for both roles,
successful client waiting, successful host completion and authoritative close once.

### S4. Route touch planning in event order

`game_session/event_hud.rs` pre-scans an entire batch for any HUD press, then
`frontend_input.rs` filters the entire batch. This consumes unrelated preceding
and following world clicks and collapses multiple HUD taps into one toggle.

Use one chronological capture/router pass with explicit toggle/cancellation
outputs. Preserve unrelated prefix/suffix input and matching release across
batches. Test multiple taps, keyboard/right-button preservation, suppression and
interaction reset. Keep the existing command dispatcher and planning policy.

### S5. Application ownership for cache-maintenance completion

`ingame_menu/spellforge_content.rs` owns the only cache-clear completion receiver
in a disposable settings panel. Closing the panel loses the result while the
native/browser deletion continues; reopening permits another request.

Move single-flight pending/completed ownership into application services. Panels
observe retained completion notices. Preserve mount/pin protections and make
worker disconnect explicit; do not claim receiver drop cancels physical work.
Test close/reopen before success/failure, duplicate admission, disconnected worker
and independent application owners. This is an ownership/reporting gap, not a
demonstrated cache-corruption incident.

### S6. Authenticated submission owner and one storage projection

`robin_highscores::submission::authenticate_reserved_offer` returns unit, while
`complete_upload` takes raw signed data. Reservation and finalization independently
derive controller/genesis/participant projections. Current HTTP callers authenticate
correctly; no authentication bypass is claimed.

Return an immutable private authenticated owner retaining the exact document and
derived projection. Require it for upload completion; use the same projection for
reservation/finalization. Preserve live SQL lease/challenge checks, exact reserved
offer equality and bounded-preflight ordering. Test signature/offer rejection,
projection consistency, committed/busy retries and interrupted exact retries.
Correct the stale database comment about reading replay bytes before reservation.

### S7. Shared preflight-grant binding policy

Four request/offer routines in `robin_run_protocol::envelope` repeat host/session/
nonce/ranked-digest comparisons and continuation membership/chain checks.

Use narrow private shared binding routines with explicit request/offer adapters.
Keep request campaign facts from ranked claims and offer facts from authoritative
starting-state expectations; preserve offer-only predecessor/hash/length checks,
validation ordering, error fields and canonical/wire bytes. Test an independent
field-mutation matrix, controller membership and request/offer-specific authority.

## Execution

Reuse clean first-wave source worktrees for S1/S2/S4/S5. Add isolated
`cleanup5-modal`, `cleanup5-submission` and `cleanup5-protocol` worktrees for
S3/S6/S7. Native client tests remain consolidated in integration; service tests
remain in the warmed admin lane; the protocol lane runs its explicit pure suite.
Do not change source or HEAD during builds or provenance-bound runtime gates.
Final outcomes and exact acceptance evidence are recorded separately.

## Disposition

S1–S7 are implemented and independently reviewed. The implementation reports
and combined validation results are linked from
[final acceptance](CLEANUP5_ACCEPTANCE.md), including the two intermittent UI
attempts that passed unchanged reruns and remain documented rather than being
claimed as fixed.
