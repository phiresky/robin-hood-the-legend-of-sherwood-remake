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

`cargo fmt --all` and `git diff --check` pass. No Cargo build or test suite was run in this isolated worktree: the coordinator requested shared integration validation to avoid multiple cold frontend targets.

Required integration checks:

- `cargo test --locked -p robin_rs` (includes existing admission/terminal materialization tests and new signing tests).
- Release-feature library suite, especially `game_session::leaderboard_runtime`, `leaderboard_mission_end`, ranked lifecycle and multiplayer authorization suites.
- Browser audio/multiplayer compile and browser tests to compile the relocated asynchronous signing adapters.

TODO: add a transport-injected end-to-end host authorization harness covering failure after the first remote publication, failed-task re-poll, wrong local result kind, and duplicate queued after final completion. Current new tests exercise production correlation/ownership helpers, not that complete transport/signing sequence.

## Concurrent work seam

Active performance work also modifies leaderboard runtime startup. It was not imported. When integrating that work, apply pre-frame authority/performance changes to `admission.rs`; mission capture remains in the parent. Preserve the exact retained prepared-input authority and signing-before-frame-zero boundary rather than restoring the former monolithic file.
