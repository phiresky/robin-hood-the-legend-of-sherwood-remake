# Audit 3: lawful ranked-driver fixtures

Final combined validation passed; see [AUDIT3_ACCEPTANCE.md](AUDIT3_ACCEPTANCE.md).
The lane-local validation notes below describe the original implementation handoff.

Finding 7 of [CODE_QUALITY_AUDIT_3.md](CODE_QUALITY_AUDIT_3.md) is implemented
entirely in the existing ranked runtime test modules. Production protocol
validation and historical byte goldens are unchanged.

## Changes

- The admission helper accepts the signing key supplied by its caller. The
  existing convenience helper retains its deterministic default for older tests.
- The authorization builder derives offer context, roster counts and grant
  expiry from the signed admission. Before returning it checks the exact request
  context, envelope and a complete signed submission with production validators.
- Response fixtures derive the authenticated seat from that admitted roster.
  A small single-participant pending-response helper centralizes request-instance
  correlation; it asserts its single-participant scope rather than silently
  dropping additional participants.
- The correlation unit test now starts with a real signed response instead of
  arbitrary key/signature bytes. Host-driver regressions mutate exactly one
  response field for wrong seat, key, replay session or offer instance, then
  verify terminal failure preserves the following inbox event.
- A builder regression exercises two host keys and a one-field offer-expiry
  mutation, asserting the intended grant-expiry rejection.

Delayed signer, signer error, wrong local identity, duplicate response,
publication failure and cancellation/drop coverage are retained. The existing
publication-failure test deliberately changes its routing itinerary *after*
validated construction to isolate partial I/O; it is not proof of a lawful
multi-participant session. No universal fixture framework was introduced.

## Validation

`cargo fmt` and `git diff --check` passed in `audit3-fixtures`. Cargo compilation
and execution are intentionally deferred to the coordinator's combined client
lane to avoid redundant cold builds. Required focused command:

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test -p robin_rs --lib game_session::leaderboard_runtime
```

The fixture changes make no panic-unwind recovery claim. The existing explicit
cancellation test drops the phase normally; destructor behavior after a panic
still requires the repository's explicit LLVM lane, not merely a Cranelift
`should_panic` result.
