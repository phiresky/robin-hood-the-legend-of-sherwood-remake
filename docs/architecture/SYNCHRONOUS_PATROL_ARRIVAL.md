# Synchronous patrol arrival

The first AI architecture slice moves `DefaultGotoRoute / EventReachPoint`
into `EngineInner::think_patrol_arrival`. The engine releases each actor borrow
before invoking scripts, then continues the same Rust call after the callback.
Enemy and friendly admission/completion still use their existing role gates.

The call performs the state-change notification, commits the state, initializes
the patrol, and reads the resulting path. An authored turn is registered with
the sequence manager without halting the selected movement. If no turn is
needed, the call dispatches filtered `EventDone` recursively. Queued caller
siblings and deferred ancestor completions are isolated while that recursion
runs, then restored for the outer completion.

This removes the route-specific suspension helper, continuation producer,
continuation dispatcher, resume helper, copied position-vector payload, and
artificial suspended-handler completion. The old enum slot remains reserved to
preserve codec discriminants and state hashes; a snapshot containing an actual
obsolete pending continuation is rejected explicitly. Removing that wire slot
requires a separately versioned migration.

This slice establishes the engine-owned call boundary; it is not yet a large
net line-count reduction. Shared admission scaffolding and regression coverage
are added. Broad observation rebuilding and geometry overlays remain pending
the separate [scheduling audit](AI_SCHEDULING_AUDIT.md). Further conversions
should reuse the boundary and delete their corresponding queues and overlays.

## Validation

Baseline: `479f524b0e23cde9c31ad67b4d61e88ba2eb74db`.
The unchanged seven-fixture gate reached exact EOF on the baseline. Baseline
runner SHA-256:
`de0acc19579eb23f66f340ce3e9a7d44ee0c9da35416b0fab6fcdd1d8c4d0ab0`.

Focused regressions exercise live path endpoint direction, explicit same-facing
turn registration, selected-move preservation, and a state callback that locks
the actor before recursive Done admission. The callback test also keeps an
enclosing queued sibling pending until arrival returns.

Final runner SHA-256:
`3328edad2ffcabf7943069be0a6ece6a3885c8d64c99f7357ed85c705ed3ed60`.
The unchanged seven-fixture gate also reached exact EOF on this runner.

- `cargo test --locked -p robin_engine -p robin_parity -- --quiet`: passed,
  including integration tests and doctests. Engine unit tests: 4,745 passed,
  14 ignored; parity unit tests: 161 passed. One engine doctest remains ignored.
- `cargo build --locked -p robin_rs --bin robin`: passed.
- `cargo fmt --all -- --check`: passed.
- Supplementary sweep: 866 recordings / 482,350 recorded frames reached exact
  EOF (all 444 idle recordings and 422 randomized 30-second recordings).
  The sweep was stopped at the user's request to prioritize a larger
  restructuring; unfinished recordings are not counted. The seven-fixture
  gate separately covers longer and chained interactive recordings.

The parity comparator and goldens are unchanged. Do not interpret a bounded
corpus pass as proof for recordings outside that corpus.
