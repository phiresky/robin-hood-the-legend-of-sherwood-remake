# Shared multiplayer client protocol core

Base: `cc36f8d75`. This is a bounded continuation of the existing shared frame
policy and ranked admission helpers, not a new wire protocol.

## Implemented

- Native and browser transports use one encoder and decoder for the existing
  five-byte class/length header and `NetMsg` body. Direction-specific limits are
  checked before allocating the body; decoded message classes must match.
- Both clients carry a shared `ClientHandshake` in their stream session. Its
  explicit phases are prelude, content pending, awaiting Welcome, complete and
  failed. Only the existing platform content flow can advance content readiness;
  repeated, reordered or late admission messages fail closed.
- The core validates offers against the authenticated endpoint and retains the
  browser's signed-invitation session binding through post-content Welcome.
  Native admission still has no browser invitation requirement.
- Both reconnect adapters use exactly the same mounted-offer comparison and
  authoritative seat, mission, seed, simulation config, speech locale and
  multiplayer-session comparison. A hash match alone cannot replace an offer.

Stream reads/writes, timeout durations, retries, cancellation, ranked admission,
simulation events and deterministic seat connection ordering remain in their
existing adapters. In particular, the dedicated receive task still owns whole
frame reads; it was not moved into a cancellation-prone per-frame select.
Graceful EOF remains only an empty header EOF, never a partial header/body.
Wire bytes, protocol version and frame limits are unchanged. A few diagnostic
messages now use shared wording instead of platform-specific prefixes.

## Validation

Seven new shared-core tests cover exact and oversized limits for every
direction/class, invalid classes/bodies, exact wire encoding, native/browser
event traces, terminal/reordered/late admission failures, authenticated endpoint
and invitation checks, exact reconnect offers and all six metadata fields.

`CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test --locked -p robin_rs --lib --features multiplayer multiplayer::client_protocol`
is running on the isolated worktree's initially cold target directory. Results
and integrated native/browser acceptance are to be recorded after completion.

## Deliberately staged follow-ups

- TODO: Migrate gameplay/ranked transitions incrementally, with an explicit
  adapter policy for the existing difference where native unresolved-ranked
  BeginSim downgrades to browse-only but browser rejects it. Do not silently
  unify these behaviors.
- TODO: Consolidate server seat metadata after the independent campaign-owner
  change settles. Server indexes and generation/authority transitions were not
  restructured in this client-admission slice.
