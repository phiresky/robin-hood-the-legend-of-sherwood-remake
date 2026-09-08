# Ranked runtime ownership refactor

## Scope

The leaderboard runtime now has three private workflow modules:

- `admission.rs`: immutable authority discovery, host/controller preflight and signing before frame zero.
- `signing.rs`: post-mission host authorization and peer co-signing, including native/browser signer adapters.
- `presentation.rs`: cooperative preparation/modal ownership and controller detachment.

The parent retains mission-lifetime capture, terminal evidence materialization and board construction. Its existing game-session API is unchanged. This is a workflow boundary, not a change to wire documents, identity storage, replay admission or cryptographic policy.

Host authorization phases now own their local signer, submission envelope, pending request instances and signatures. A completed phase owns its result until one consumption; a finished phase retains only progress information. Errors terminalize the task, drop pending result receivers and prevent later polls from consuming transport events.

Peer duplicate requests are rejected before invoking another durable signer. Terminal peer failures discard the armed request and pending receiver. Dropping a browser receiver does **not** abort an already detached signing future: durable identity work may finish, but the retired owner cannot publish its result.

## Preserved ordering

Exact context validation and host membership precede authorization. Envelope validation precedes local signing. Local signing precedes ordered remote publication. Responses must match key, authenticated seat and exact purpose/session/offer instance before consuming a pending request. Final success still passes the existing complete authorized-submission validator.

The host retains its existing bounded 64-event drain, including rejection of an extra response queued after completion in the same drain. No new retry, background transport owner or signing lane was introduced.

## Regression coverage and validation

Added focused tests for delayed signer ownership and retirement, duplicate peer requests rejected before starting/replacing a signer, and wrong seat/key/session/offer responses leaving pending evidence untouched, followed by valid acceptance and duplicate rejection.

The actual host `try_take` driver additionally has deterministic tests for delayed signer error/wrong operation, wrong local identity, wrong response instance, terminal re-poll without inbox consumption, valid final response with and without a queued duplicate, and failure after one successful remote publication without retry. The success test reaches the existing complete cryptographic validator. A closed transport adapter has a fixture variant only under `cfg(test)`; normal builds retain only the authenticated transport.

Formatting and whitespace checks passed. Coordinated final acceptance at `f5f531c7` passed: default client 1578 library tests plus all integrations/doctests, release library 1682 tests, and the focused signing suite 8/8. The first full default run exposed an invalid test offer expiry; `7b61d5130` derives it from the retained signed grant without changing any validator.

Additional executed integration checks:

- `cargo test --locked -p robin_rs` (includes existing admission/terminal materialization tests and new signing tests).
- Release-feature library suite, especially `game_session::leaderboard_runtime`, `leaderboard_mission_end`, ranked lifecycle and multiplayer authorization suites.
- Browser audio/multiplayer target checks, linked module and real Chrome 24/24 passed. `018c78214` supplied a narrow sibling receipt-controller API after the initial browser compile exposed private-field access.

Both native binaries built; isolated headless and graphical multiplayer scenarios passed all ten checks each, and all four native replay/save-load lifecycle phases passed. [Final acceptance](AUDIT2_PLAN.md#final-acceptance) records exact artifacts and limits.

The partial-publication regression injects an itinerary after constructing validated single-player evidence, to isolate the task's non-retry boundary; it does not assert multiplayer roster/admission validity. These focused task tests do not themselves run real network I/O or durable browser signing; the separately executed transport/browser gates have the narrower coverage documented above.

## Concurrent work seam

Active performance work also modifies leaderboard runtime startup. It was not imported. When integrating that work, apply pre-frame authority/performance changes to `admission.rs`; mission capture remains in the parent. Preserve the exact retained prepared-input authority and signing-before-frame-zero boundary rather than restoring the former monolithic file.
