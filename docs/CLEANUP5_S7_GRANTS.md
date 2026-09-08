# S7: shared preflight-grant binding

The four submission request/offer adapters now share the host/session/instance/
nonce/ranked-digest identity comparison. Continuations share chain and durable
participant/controller membership comparisons; fresh grants share their complete
submission-binding validator.

Authority stays explicit at the adapters: fresh requests read campaign facts from
the ranked claim, while admitted offers read starting-state expectations.
Continuation requests bind chain/predecessor identities, but only admitted offers
bind predecessor verification, campaign hash and byte length. No wire structures,
canonical implementations, signing domains or serialized authority changed.

Validation still checks the grant itself first, continuation scope second, and
ranked canonicalization before binding. Existing error fields remain unchanged.

Five focused tests exercise all four adapters with independent identity mutations,
fresh campaign authority and scope substitutions, continuation chain/participant/
capacity mutations, offer-only campaign/predecessor facts, controller membership,
and shape/scope error precedence. Existing canonical/signature tests remain in the
package suite.

Validation command (run after freezing the implementation commit):

```sh
RUSTC_WRAPPER= CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test --locked -p robin_run_protocol
```

Result at source commit `7f11493fc`: **106 passed, zero failed/ignored**, plus
zero doc tests. Build completed in 40.13 seconds; unit tests in 0.21 seconds.
Only the existing unused `validation::unique_text` warning was emitted.
