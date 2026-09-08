# Audit 3 implementation acceptance

All eight findings in [CODE_QUALITY_AUDIT_3.md](CODE_QUALITY_AUDIT_3.md) are
implemented. That file remains the historical, read-only audit; the completion
map and per-track reports are linked from [AUDIT3_PLAN.md](AUDIT3_PLAN.md).

## Source and review

Implementation began at main `7be45078e`. Five implementation worktrees and one
integration worktree isolated the worker, save store, picker, user stores and
protocol fixture changes. Independent reviewers inspected worker mutation paths,
admin's distinct backup lifetime, and save-store recovery schedules.

The integrated refactor checkpoint was
`c44fc9bd8fc27744b62f5d06572f4cb5db734280`. It includes the user's concurrent
main through `246d6cd86`, preserving prepared-save identity, browser Restart and
autosave sequencing changes. Main subsequently removed its sprite-streaming
experiment in `2c504019a`. That was merged without conflicts into final tested
source `98081fc4090726392e6958e6d1d5a69f6dec693f`, tree
`2c17c40011e0c247b52c34314df71596e6df445e`. The sole overlapping source file,
`game_session/mod.rs`, had independent module-declaration and stable-load-identity
changes. A separate source review found no dependency on the removed sprite APIs.
Final assets/client, browser, Vulkan and live-session checks were refreshed.

Notable reviewed implementation commits:

- Worker: `6b51dfb5f`, `0f3781732`; evidence update `7365736ee`.
- Save store: `6e08fc606`, `092f70596`, `9d0cf5708`, `ff766b7af`, with main
  integration and compatibility corrections in the intervening commits.
- Picker: `f5fd22106`, `9391c1c64`.
- User stores/global API: `1687b3aa0`.
- Ranked fixtures: `2ca8b6530`.

Review corrections were substantive, not just formatting: preserve the legacy
directory field's output shape without trusting it; reserve the autosave manifest
name; omit unpublished drafts from the index and never unlink a foreign payload
when discarding one; reconcile pending quick recovery before live deletion and
new payload publication; route refresh panics through physical drain; consolidate
the remaining duplicate picker row mapping; make deletion fixtures lawful under
the new published-index validation. Regression tests cover these schedules.

## Executed native/service checks

Cargo commands used `RUSTC_WRAPPER= CARGO_BUILD_JOBS=1`; test commands also used
`RUST_TEST_THREADS=2`. Each worktree retained its normal `target/`. No clippy,
target redirection, dependency installation or shared compiler-cache changes.
Builds were unbounded; game execution was separate and bounded. Sources stayed
unchanged throughout each build and provenance-sensitive gate.

At `c44fc9bd8`, all commands below exited zero:

```sh
cargo test --locked -p robin_engine -p robin_assets -p robin_rs
cargo build --locked -p robin_rs --bin robin
cargo test --locked -p robin_rs --lib --no-default-features --features release
cargo build --locked -p robin_rs --bin robin --no-default-features --features release
cargo test --locked -p robin_highscores
cargo test --locked -p robin_highscores --bin robin-highscores-worker \
  --config 'profile.test.package.robin_highscores.codegen-backend="llvm"' \
  physical_work_is_drained -- --ignored
bash scripts/check-quality.sh format
bash scripts/check-quality.sh tooling
git diff --check
```

The engine library passed 4,477 tests (four ignored). The default assets/client
package suites, integration targets and doctests passed. The release-feature
client library passed 1,740 tests (seven ignored). Service results were 166
library, 31 admin, five server, 14 worker and 12 router tests; the separately
selected LLVM lane passed all three actual-unwind regressions. Ignored corpus,
GPU, host and unwind-specific cases were not counted as ordinary suite passes.

Worker test sensitivity was also established before integration: temporarily
removing the error-path physical drain caused the exact heartbeat-loss regression
to fail with `worker owner returned while physical mutation was still blocked`.
The substitution was restored, source cleanliness checked, and the positive
tests rerun. No negative-policy code remains. See [AUDIT3_WORKER.md](AUDIT3_WORKER.md).

After merging the user's sprite cleanup, these passed at final `98081fc409`:

```sh
cargo build --locked -p robin_rs --bin robin --no-default-features --features release
cargo test --locked -p robin_assets -p robin_rs --no-default-features \
  --features robin_rs/release
cargo check --locked -p robin_parity -p robin_rs \
  --features robin_parity/client,robin_rs/release,robin_rs/tools,robin_rs/projection-export \
  --bins --examples
bash scripts/check-quality.sh format
git diff --check
```

The final full package test includes integration targets and all seven client
doctests. The consumer command checks the optional parity client and applicable
developer binaries/examples, including `audio_decode_bench`; this is a deliberate
compatible-feature compile check, not a claim to have run every named tools suite
or every possible feature combination. Service source and Cargo.lock are unchanged
between `c44fc9bd8` and the final snapshot, so its service evidence carries forward.

The exact ignored Vulkan regression
`gpu_upscale::tests::headless_downlevel_device_executes_every_multipass_profile`
was executed from each freshly built client test executable with
`WGPU_BACKEND=vulkan`, `--ignored --exact` and a 180-second outer timeout. Both
passed, including the offscreen renderer ownership contract. The final run used
the test executable rebuilt after the main merge (1.96 seconds). This is the same
test selected by the named `gpu` suite, run directly to avoid waiting on Cargo's
unrelated build lock. No new GL acceptance is claimed.

## Final browser evidence

The named `browser-audio` gate passed at both snapshots. At final `98081fc409`:

- Audio and audio/multiplayer WASM bin/test target checks passed.
- The actual WASM test module linked and ran in real Chrome: **29 passed,
  zero failed, zero ignored**.
- The explicit new
  `autosave::tests::browser_save_store_open_uses_memory_and_propagates_autosave_errors`
  pass is present in the log, as are profile/key persistence cases.
- Chrome and ChromeDriver: `152.0.7977.64`; explicitly selected matching
  wasm-bindgen runner `0.2.127`. The differently versioned PATH runner was not used.
- Initial/final harness source and dirty-state checks matched the frozen snapshot.

Final evidence directory: `/tmp/robin-audit3-browser-final.gLUCEf`, containing
`summary.json`, `browser-tests.log` and retained `browser-tests.wasm`.
WASM SHA256: `d92f2b4f14018dcbc79d6f8253ad739b027f710edfd42f278887cc4af211fe78`.
Earlier `c44fc9bd8` evidence remains in `/tmp/robin-audit3-browser.zbfcne`.

## Final native runtime evidence

The release-feature development executable was copied outside worktrees with
adjacent assets/mods before execution. Source: `98081fc4090726392e6958e6d1d5a69f6dec693f`.
Binary: `/tmp/robin-audit3-runtime.IKUbj1/final/robin`.
SHA256: `29fe5aabb84e18e1e6fc545619da6d33104d2384f2c6d4cd4888a9b620ef7fd3`.
Its hash was checked before and after all scenarios; later compilation did not
overwrite the retained executable.

Three gates ran concurrently using private profiles, isolated loopback networking
and `/home/phire/robinhood/datadirs/demo_leicester_ecoste`. No real profile or
deployed service was accessed. Sound was disabled; this does not claim native
audio-device playback validation.

| Gate | Final result | Evidence under `/tmp/robin-audit3-runtime.IKUbj1/final/` |
| --- | --- | --- |
| `native-lifecycle` | Four phases passed: ordinary and save/load replay, each headless and graphical; actual quicksave restored the engine, recording continued, post-load hashes matched through EOF | `native-lifecycle/summary.json` |
| Headless multiplayer | Ten checks passed, two rollback observations, reconnect plus post-input agreement; zero desyncs/missed comparisons | `multiplayer-headless/summary.json` |
| Graphical multiplayer | Same ten checks passed, two rollbacks, reconnect and post-input agreement; zero desyncs/missed comparisons | `multiplayer-graphical/summary.json` |

Both multiplayer runs observed agreed hash frames 0/25/50/75. They used
`--observe-hashes-before-reconnect`, not process-restart mode. Native lifecycle
used a 1,500-second outer bound with its per-phase limits; multiplayer used
600-second outer bounds and the driver's 540-second limit. All processes stopped.
Initial/final harness source stayed clean and unchanged. Earlier `c44fc9bd8`
runtime results are retained separately in the parent evidence directory.

These `/tmp` artifacts survive worktree removal but are local, non-permanent
evidence, not a remotely archived CI release. The committed report preserves
their identities and results.

## Deliberate limits and follow-ups

- Corrupt manual indexes/receipts fail closed with an explicit diagnostic. A
  user-facing repair workflow is still TODO; no guessed reconstruction or empty
  writable fallback is provided.
- Local save stores do not coordinate competing processes or promise protection
  against adversarial symlink replacement. Directory-sync failures can mean
  published-but-not-confirmed-durable state and are reported as such.
- Profile/key publication is atomic per file, not a multi-file identity transaction.
  Browser localStorage and the existing first-launch identity policy stay separate.
- Picker bridge/model tests do not automate every GPU modal or IME interaction.
  Save-only editing and the two scheduling loops intentionally remain separate.
- The worker drain covers canonical mutating work and verifier lifetime, not
  process abort, runtime destruction, unbounded kernel I/O or arbitrary returned
  temporary-value destructors. Future detached async descendants need explicit
  ownership; task-local tracking does not automatically propagate to spawned tasks.
- Separately reviewed admin backup cancellation may leave unpublished `.partial`
  work/cleanup running after its exclusive fence's SQL drain. No late authenticated
  publication or published-backup corruption was established. That narrower
  pre-existing admin follow-up is documented, not claimed fixed by this worker pass.
- No licensed-corpus parity sweep, optional GL success, process-restart reconnect,
  deployment, push or new performance measurement is claimed.
