# Cleanup 5: staged save/load execution

The public `perform_pending_save_load` boundary remains compatible. It owns
exactly two actions before dispatch: consuming the pending request once, and
draining the save publication owner. A failed drain still consumes the request,
queues the save-failure notice and returns a processed failure without any
engine/replay effects. An empty queue still does not drain or emit a banner.

## Boundaries

- The dispatcher receives the save manager and notice mailbox, not the entire
  callback owner. It cannot replace the next request or use application-level
  leaderboard/autosave ownership accidentally.
- Immutable load routing consumes the exact preflighted payload and returns
  either an owned cross-mission payload or a private `CurrentMissionLoad`.
  It needs only the current engine/game/profiles, with no storage, networking,
  mutable campaign, or banner dependencies. Local restart, ordinary load and
  quick-load now share this validation stage.
- Engine application accepts only `CurrentMissionLoad`, preserving the
  validation-to-application payload. It returns a private application receipt
  only on success. Replay identity is captured before application and subsequent
  Continue mirroring/fixups. Runtime permission and receipts cannot be restored
  by serde deserialization.
- The completion reducer takes a closed request policy (`Restart`, `Selected`,
  `Quick`) rather than arbitrary success banners or unrelated optional effects.
  It preserves Restart's distinct input-reset behavior and the selected special
  slots' banner/Continue semantics.
- Diagnostic persistence distinguishes allocation from publication failure and
  shares one implementation between manual and quick diagnostics. It has no
  engine application or replay-event authority.

Request-specific persistence and Continue mirrors remain explicit in the
dispatcher. Their ordering and failure notices are unchanged. Multiplayer
proposal occurs before local routing, while already-committed transitions
retain their process-local provenance. Existing exact decoded-payload/handle
preflight and remote authority checks remain at the same boundary; no wire or
save-payload format changed.

## Regression coverage

New tests exercise routing without engine mutation followed by actual payload
application, malformed-route rejection, distinct completion policies, and
rejected receipt deserialization. Existing callback diagnostic, stale-handle,
committed-origin and failed-restart tests continue through the compatible facade.

A real two-owner retirement regression queues an autosave and an owned Continue
write whose destination causes publication failure. Retirement must report the
manual error while still draining the autosave to its durable manifest/payload.
It then verifies that a pending load rejected by the sticky publication barrier
is consumed once without restore, replay event, input reset or restart effects.
This is normal completion/failure coverage, not a panic-unwind claim.

`cargo fmt` and `git diff --check` pass. Cargo compilation and the explicit
`robin_rs` suites are deferred to the coordinator's consolidated build; no
separate cold build was started in this lane.
