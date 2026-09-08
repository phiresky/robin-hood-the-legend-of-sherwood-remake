# Command-family boundaries

## Implemented

The command dispatcher still owns the exhaustive `PlayerCommand` match and the
ordering contract. Family execution now has focused private modules:

- `commands/quick_actions.rs`: capture, recording storage choice, automatic queue
  advancement, native and legacy replay, and replay validity. Recording-store
  selection and icon-phase policy live with their consumers.
- `commands/interaction.rs`: target and ground-target execution after shared
  recording. A recorded interaction creates its authored titbit and stops
  recording without also launching a live sequence.
- `commands/selection.rs`: PC/portrait selection with explicit nested-message
  interpretation, and box-selection host followups.
- `commands/seat_lifecycle.rs`: connection and disconnection for the payload's
  target seat, preserving selection during reconnect.

There is no second command router, boolean "handled" fallback, new command
representation, or independent recording hook. Helpers receive the concrete
fields already matched by the exhaustive dispatcher. Existing quick-action
replay still re-enters the same command boundary.

## Preserved contract

1. Host authority rejection precedes issuing-seat allocation. Reusable-cloak
   authorization and same-seat/same-PC batch adjacency remain outside dispatch.
2. Object-Take reachability, recorded interaction identities, recorded DropAle
   route authorization, and sword-gesture validation precede shared recording.
3. Shared recording runs once, then family execution. Recording-only returns
   do not execute live work.
4. Group-move execution, per-actor acceptance speech, callback closure, and
   command-to-command effects retain their prior ordering.

No RNG operations, command/wire/save layouts, historical replay interpretation,
or gameplay behavior were intentionally changed. No new serialized state or
dependencies were introduced.

## Validation

- `cargo fmt --all` and `git diff --check` passed locally.
- A mechanical comparison confirmed that the entire quick-action implementation
  body is unchanged apart from visibility, module paths and formatting; the
  preflight-through-recording source block is byte-for-byte unchanged.
- Three new regression tests assert full deterministic state-hash preservation:
  unauthorized seat lifecycle cannot allocate seats; unreachable object Take
  cannot capture a step or launch work; missing recording identities panic
  before capture or dispatch.
- Native compilation and test execution are intentionally assigned to the
  combined integration lane, avoiding another cold Cargo target. Required:
  `cargo test --locked -p robin_engine` and affected replay/parity suites.
  Particularly retain the existing nested/independent-selection, sound-boundary,
  recorded interaction/ground-throw, DropAle-route and invalid-quick-action tests.

## Deliberate limits

The outer dispatcher is not replaced with an ECS or generic handler registry.
Small one-step arms remain inline, and specialized seek/combat route construction
remains alongside dispatch for now. TODO: only extract additional interaction
families once their route and callback contracts have equally strong regression
coverage; do not move preflight into those handlers.
