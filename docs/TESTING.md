# Testing and refactor gates

Bare `cargo test` intentionally selects only `robin_util` and
`robin_state_hash_derive`. It is a local smoke check, not the application test
suite. CI and local verification use the same explicit entry point:

```sh
bash scripts/check-quality.sh engine
```

Run the suite owning your change, then its affected consumers. Cargo retains
the checkout's normal `target/` directory. Build and run the game separately;
do not hide Cargo output behind filters. These gates do not run Clippy.

`bash scripts/check-quality.sh format` checks Cargo-discovered sources. Client
modules are explicit declarations, so rustfmt traverses their real module tree.
Only the shared fixture helper included by test macros needs a direct pass.

## Crate-to-gate matrix

The Rust quality workflow runs the fixture-free Rust rows as separate Linux
jobs; `native-lifecycle` is local/provisioned only. All Cargo
commands use `--locked`. `scripts/test_quality_suites.py` checks that every
workspace member has an explicit gate, so adding a crate requires assigning it.

| Suite argument | Scope | What it checks |
| --- | --- | --- |
| `core` | `robin_util`, `robin_state_hash_derive`, `robin_spellforge`, `robin_lua` | Unit, integration and doc tests |
| `scripting-llvm` | `robin_spellforge`, `robin_lua` | Explicit poison recovery and native-session unwind tests with LLVM package overrides |
| `engine` | `robin_engine` | Deterministic simulation tests |
| `assets` | `robin_content`, `robin_assets`, `robin_data_io` | Content with/without simulation codecs; assets with/without engine adapters; fixture resolver tests; resolved pure-content dependency boundary |
| `protocols` | `robin_run_protocol`, `robin_replay_format`, `robin_official_content`, `robin_ranked_verification`, `robin_identity_signer` | Wire, content, native helper discovery/containment/protocol, admission and isolated signer tests |
| `services` | `robin_highscores`, `robin_manifest_tool`, `robin_replay_verifier` | Server, manifest and verifier tests |
| `parity` | `robin_parity` | Runner unit/contract tests; does not replay licensed corpora |
| `client` | `robin_rs`, default features | Build native admission helper, client tests, then a separate `robin` binary build |
| `client-release` | `robin_rs`, `release` features | Client library tests and binary build with desktop/audio/Lua/multiplayer/updates; audio example check |
| `tools` | `robin_modding_tools`; `robin_rs`, `tools` and `projection-export` | Modding CLI and encoder tests; explicit converter/dump tests; minimal export example tests; check tool binaries and examples |
| `wasm` | `robin_replay_admission_wasm`, `robin_rs`, `robin_identity_signer` | Target checks for `wasm32-unknown-unknown` using `wasm-dev` |
| `browser-audio` | `robin_rs`, WASM `audio,multiplayer` | Audio and multiplayer target checks, linked module, real Chrome audio ownership/residency, shared protocol and identity tests |
| `native-lifecycle` | Provisioned prebuilt native `robin` and Leicester demo | Ordinary live/export plus save/load-back, each replayed headlessly and graphically to EOF |
| `gpu` | `robin_rs` Vulkan execution | Explicit ignored GPU test; missing adapter is an error |
| `gpu-gl` | `robin_rs` GL execution under Xvfb | Same required execution contract using the browser runtime's GL backend family |
| `host` | `robin_rs`, `hardware-info` | Explicit ignored real-memory query; requires an accessible host backend |

Feature choices are deliberate. Do not substitute `--all-features`; video,
Android, browser threads, and shader tooling have distinct dependencies.

CPU parity replay needs engine-owned timing separately from licensed game data:
`original_parity_replay --core-datadir /path/to/assets/core-datadir TRACE`.
The development default is `assets/core-datadir` relative to the invocation
directory, resolved before entering `ROBINHOOD_DATA_DIR`. Copied/installed runners
must supply their core data root explicitly; they never read a build-checkout
path or substitute licensed-data timing for missing/corrupt core timing.
The `client` feature rejects this CPU-only option and uses client asset startup.
Fixture/sweep Python drivers accept `--core-datadir` (default: their checkout's
core assets); the shell release sweep accepts `PARITY_SWEEP_CORE_DATADIR`.
`--allow-legacy-result` in the fixture gate omits the new runner flag for older
baseline executables, which retain their original asset lookup behavior.
The assets gate checks resolved normal/build dependencies across all targets:
offline `robin_modding_tools` must not pull in the `robin_rs` client;
pure `robin_assets` must not pull in `robin_engine`; pure `robin_content` also
excludes `robin_util`, `robin_state_hash_derive` and `bitcode`. The checker prints
the complete Cargo tree and rejects an empty or unexpected graph.
The wasm gate is a compile check, not a browser execution or memory-cap test.
The separate browser-audio gate runs synthetic browser tests, not full-game
browser rendering or audible-output acceptance. Native-lifecycle is a local
provisioned gate, not public CI: it needs licensed data and a compatible host.
Runtime publication retains its existing emitted-artifact checks. Consult the
platform runbooks for native packaging and Android checks; this matrix does
not claim Android or Windows execution coverage.

The scripting LLVM gate selects only its two fixture-free unwind cases and
overrides the relevant package code generation backend explicitly. It verifies
destructor/unwind behavior: the default Cranelift Lua panic harness can pass
without proving that cleanup ran, and poison recovery cannot continue through
`catch_unwind` there. The gate does not activate every ignored script corpus
test or change normal build profiles.

Client worker/window unwind regressions likewise require an explicit LLVM
override. Select one ignored test at a time and verify that it actually runs:

```sh
CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=2 cargo test --locked -p robin_rs --lib \
  --config 'profile.test.package.robin_rs.codegen-backend="llvm"' \
  panicking_worker_does_not_poison_owner_and_next_caller_retries -- --ignored
```

Use the same command with `unwound_worker_releases_scheduler_reservation`,
`unwinding_panic_disconnects_before_waking_the_loop`, or
`llvm_owned_worker_panic_is_joined_and_reported_once` as the filter to check
terrain scheduling, window exit publication, or save-worker retirement. Do not
select every ignored test: other cases require different fixtures/backends.

The client CI job also compares the browser runtime contract to compiled Rust
constants. Run the same contract check locally after changing protocol/schema
versions:

```sh
cargo build --locked -p robin_rs --example export_runtime_contract
target/debug/examples/export_runtime_contract --check wasm-www/runtime-contract.json
```

The Ubuntu 26.04 CI job installs the native library development packages it needs.
The services suite also needs bubblewrap 0.11.1 or newer and a system POSIX shell. Its runner
allows unprivileged user namespaces for the verifier launched through a pinned
file descriptor.
The services gate first checks the high-score library and all production binaries
without features, including compile-fail documentation tests for the database
boundary. It then explicitly enables `robin_highscores/test-support` for the
corruption/concurrency fixtures. For a complete package-only suite, run
`cargo test --locked -p robin_highscores --features test-support`; a plain package
test omits the router integration target and admin execution fixture module.
Never enable this raw database fixture access feature in deployment builds.
For a machine without the local Wild linker, use
`CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc`; preserve the repository CPU
flags. The pinned Rust toolchain includes the configured code generation
backend. Install the wasm standard library with
`rustup target add wasm32-unknown-unknown` before the wasm suite. Set
`CARGO_BUILD_JOBS=1` when sharing a machine with parallel worktree builds.

## Original-data fixtures

Ordinary tests use checked-in or synthetic fixtures. Original-data tests are
explicitly ignored with a reason; when selected, missing/invalid fixtures fail
setup instead of appearing as passing tests. The common resolver lives in
`test-support/original_data.rs` and is included only by test modules.

```sh
ROBINHOOD_DATA_DIR=/absolute/path/to/leicester-demo bash scripts/check-quality.sh fixtures-demo
ROBINHOOD_DATA_DIR=/absolute/path/to/full-game bash scripts/check-quality.sh fixtures-fullgame
```

Each root must contain `Data/`. The demo suite checks the original profile,
profile JSON round trip, font, sprite banks, and script decoding/manager fixtures. The
full-game suite checks its profile and script collection. Run each distribution
separately: blindly selecting every ignored test would combine incompatible
fixture requirements, GPU execution, and other opt-in integration scenarios.
No licensed data is fetched or assumed present on public CI.

The regular font tests read the checked-in Arial fixture relative to
`CARGO_MANIFEST_DIR`, so their behavior does not depend on the shell directory.

## GPU execution

Shader translation tests remain in the normal client suite. GPU execution is
an explicit separate gate:

```sh
bash scripts/check-quality.sh gpu
bash scripts/check-quality.sh gpu-gl
```

The Vulkan gate explicitly sets `WGPU_BACKEND=vulkan`; the GL gate sets
`WGPU_BACKEND=gl` and runs under `xvfb-run -a`. CI installs Mesa Vulkan drivers
and, for GL, native amd64 `libegl1`, `libegl-mesa0`, `libgles2`, `xvfb` and
`xauth`. Both configure an owned `XDG_RUNTIME_DIR`. Native GL execution checks
the backend family used by the wasm runtime, but is not browser GPU execution.
The test first requests a fallback adapter, then a
hardware adapter. Selecting this test promises the prerequisite is available:
failure to create an adapter/device fails the gate. This checks multipass
execution and the renderer's synthetic exact-pixel/readback contract. Original
game image comparisons remain separate provisioned scenarios.

## Host hardware queries

```sh
bash scripts/check-quality.sh host
```

This named CI job enables `hardware-info` and explicitly selects the ignored
real-memory query test. Linux requires access to `/proc`; restricted sandboxes
may not expose it. An unavailable query remains a test failure when this gate
is selected. Ordinary minimal builds have no hardware-query backend; selecting
this test without `hardware-info` fails with an explicit feature requirement.
Ordinary native suites report it ignored instead of assuming every runner
exposes host data or silently running zero tests for a missing feature.

## Tooling and web checks

```sh
python3 scripts/test_quality_suites.py
bash scripts/check-quality.sh tooling
bash scripts/check-quality.sh web
bash scripts/check-quality.sh editor
```

Tooling uses an explicit allowlist of shell/Python regression suites; it never
discovers arbitrary capture, publishing or deployment commands. Python 3.11+
(for `tomllib`), Bash, GNU coreutils (`timeout`), `flock`, `setsid`, `zstd` and
`rsync` are the Linux
prerequisites. Negative-path tests intentionally print error diagnostics.
The suite exits nonzero on the first failed check.

The web suite delegates to `pnpm --dir wasm-www verify:web` and requires the
pinned Node/pnpm versions and a frozen-lockfile install in `wasm-www`. It runs
TypeScript checking, top-level browser tests, leaderboard/signer tests,
deployment script tests, shared JavaScript identity tests, and site/signer-shell
builds. See platform runbooks for the additional publication checks.

JavaScript workspaces pin pnpm 12.3.4. For local CI parity, select Node with
`pnpm runtime set node 24.19.0 -g` and ensure pnpm’s bin directory is on PATH.
Editor and pipeline TypeScript scripts run directly with Node; relative source
imports include `.ts`, and TypeScript checks enforce erasable syntax.

The editor suite runs `pnpm --dir level-editor verify`: shared/app/pipeline
tests, app and pipeline typechecks, and the app build. Install its independent
workspace first with `pnpm --dir level-editor install --frozen-lockfile`.
It has a dedicated PR job; it does not call remote reconstruction providers.

The same job then exercises the production editor lifecycle in real Chromium:

```sh
CHROME=/path/to/google-chrome bash scripts/check-quality.sh editor-browser
```

Run this after normal editor verification: `build:browser` stages the separate
test entry after the ordinary app build clears its output. The gate owns a
preview process group on port 5181, waits for its lifecycle page, runs the
browser harness with a 120-second outer timeout and always terminates the
preview group (including escalation for an unresponsive process). Logs are
printed on cleanup. The harness uses in-memory GLB/doc fixtures and same-origin
built resources, and owns Chromium plus its temporary profile. CI uses the
hosted `google-chrome` executable when available; otherwise it explicitly
installs Google's stable Debian package and prints the browser version. A
missing browser is an error, not a skipped test.

## Provisioned lifecycle acceptance

See [the lifecycle gate runbook](validation/lifecycle-gates.md) for exact inputs,
isolation, retained evidence and limitations. Both explicit gates fail when
prerequisites are missing; neither silently skips, installs system tools, edits
goldens, publishes artifacts remotely, or uses the real player profile.

```sh
CHROME=/absolute/path/to/chrome \
CHROMEDRIVER=/absolute/path/to/matching/chromedriver \
WASM_BINDGEN_TEST_RUNNER=/absolute/path/to/matching/wasm-bindgen-test-runner \
bash scripts/check-quality.sh browser-audio
```

The browser CI matrix provisions a matching browser/driver and installs the
test runner version selected by Cargo.lock. It retains machine-readable
results, the tested WASM module, and runner output even on failure.

## Interpreting a refactor baseline

Record the exact commit, suite, feature/target selection and outcome. A green
compile check does not mean gameplay was exercised, and ignored original-data
tests do not establish parity. For simulation/scheduling/snapshot changes,
also record the representative replay's provenance and validated full EOF
result; compare exact ordering/RNG/hash contracts before updating goldens.
Keep existing failures separate from new regressions. CI branch-protection
configuration must select these named jobs in repository settings; committing
the workflow itself does not change required checks.

### Complete save/reload playback in Chromium

With a matching built native game, wasm package (including replay admission),
production site, and converted Leicester demo datadir, create a short idle
recording containing ordinals 0–75. Build the playback regression fixture and
run it through the production shell:

```sh
python3 scripts/validation/replay_history_fixture.py <root-chunk.rhrec.jsonl> target/history.rhrec.jsonl
target/debug/examples/replay_to_compact target/history.rhrec.jsonl target/history.rhrec
node scripts/wasm_production_startup_chrome.mjs --chrome /usr/bin/chromium \
  --pkg <wasm-package> --datadir <browser-datadir> --site wasm-www/dist \
  --replay target/history.rhrec --replay-eof \
  --seek-replay 76 --seek-replay 105 --seek-replay 0 --seek-replay 133 \
  --query wasm-threads=0 \
  --output target/history-browser --mbit unlimited
```

The fixture includes two hash-checked save markers, abandoned gameplay, a loss,
a win, and three restores (including a return to the older save). The harness
requires decoded playback, a rendered frame, complete EOF without browser
exceptions or replay desyncs, and exact ordinal positions after each seek.
Inspect its screenshot for lingering terminal UI and its logs for all three
restored boundaries. Their logged restored-state hashes must match
the native run at the same ordinals. Play the same compact fixture natively
with `--headless --no-sound --http-server 0 --replay target/history.rhrec`.

The console-generated outcomes deliberately make this fixture unranked. Normal
leaderboard eligibility and recorded post-restore hash validation are covered by the engine
ranked-resimulation and client archive tests. The fixture retains original
prefix hashes only: persisted-load reconciliation can change the state, so a
pre-save checkpoint must not be reused as a post-load expected hash.
