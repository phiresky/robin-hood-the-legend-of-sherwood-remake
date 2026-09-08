# Repeatable lifecycle acceptance

These named gates complement ordinary package tests. Select them only on a
provisioned host; a missing dependency is a failure, not an ignored test.
`scripts/validation/lifecycle_gate_test.py` tests their orchestration using
synthetic fixtures and a bounded dummy process. It does not run Cargo, game
data, Chrome, or deployments and is included in the tooling allowlist.

## Browser audio and shared multiplayer protocol

The suite name remains `browser-audio` for compatibility with existing callers.

Requires Linux, Python 3.11+, the pinned Rust toolchain, the
`wasm32-unknown-unknown` target, Chrome, matching ChromeDriver, and a
`wasm-bindgen-test-runner` matching the **wasm-bindgen package in Cargo.lock**.
Do not assume the first runner on PATH matches the crate. The wrapper checks
runner version and browser/driver major versions before compiling; incompatible
patch versions still fail browser startup rather than passing untested.

Browser builds require a clean tracked/untracked checkout. Both gates record
HEAD and separate staged/unstaged diff hashes and reject source movement during
acceptance. Native prebuilt runs may start with stable tracked changes (recorded
explicitly); their binary build source remains the caller's separate assertion.
Do not edit scripts or sources while a gate runs.

```sh
CHROME=/absolute/path/to/chrome \
CHROMEDRIVER=/absolute/path/to/chromedriver \
WASM_BINDGEN_TEST_RUNNER=/absolute/path/to/wasm-bindgen-test-runner \
CARGO_BUILD_JOBS=1 bash scripts/check-quality.sh browser-audio
```

The gate first checks client bin/tests with `audio`, then `audio,multiplayer`
(so browser transport code is not silently excluded), then separately links
the `audio,multiplayer` library test module with `wasm-dev`. It obtains the artifact
from Cargo's compiler-artifact event, printing every Cargo output line instead
of guessing from stale files in target. Compilation has no runtime timeout and
uses the checkout's ordinary target directory. It then executes all browser
tests with a 240-second outer bound and 120-second runner bound; zero tests or
an output missing passed audio, shared client-protocol, or identity cases is an
error. The reported count must match distinct passing case lines, and both
identity address round-tripping and refusal of a second durable browser identity
must pass. Ignored cases and console mentions do not satisfy these checks.

The shared protocol cases exercise frame limits/codec, content admission,
authentication metadata and reconnect decisions using the same tests as native.
They execute in Chrome alongside audio ownership/residency and browser identity
checks. They do not establish remote multiplayer or network end-to-end coverage.

Chrome receives a fresh temporary profile, background services disabled, and
DNS restricted to loopback. The runner owns a process group which is terminated
on success, failure, interruption or timeout; the profile is then removed.
These synthetic tests need no licensed data or live external service. This is
not a general browser network security sandbox or full-game/browser GPU gate.
The CI lane provisions matched Chrome and driver through the pinned
[setup-chrome action](https://github.com/browser-actions/setup-chrome), then
installs the lockfile-selected CLI. Local execution installs nothing.

Evidence includes `summary.json` (source commit, module hash, tool versions and
test count, selected features and passed cases by required group),
`browser-tests.wasm`, `browser-tests.log`, and the non-secret
WebDriver options. The recorded temporary profile path no longer exists after
completion. It does not retain browser identity keys or assert audible sound.

## Native lifecycle

Requires Linux user/network namespaces, `unshare`, `ip`, `Xvfb`, `libX11`,
Vulkan, and a Leicester demo root containing `Data/`. The development binary
must include the release feature set (desktop, replay HTTP hooks and audio
dependencies); the runtime gate disables sound explicitly. Build separately:

```sh
CARGO_BUILD_JOBS=1 cargo build --locked -p robin_rs --bin robin --no-default-features --features release
git rev-parse HEAD
sha256sum target/debug/robin
```

Record that exact source and binary digest; do not rebuild or replace the
binary during acceptance. Supply those recorded values:

```sh
ROBIN_LIFECYCLE_BINARY=/absolute/path/to/checkout/target/debug/robin \
ROBIN_LIFECYCLE_BINARY_SHA256=RECORDED_SHA256 \
ROBIN_LIFECYCLE_SNAPSHOT=RECORDED_SOURCE_COMMIT \
ROBINHOOD_DATA_DIR=/absolute/path/to/leicester-demo \
bash scripts/check-quality.sh native-lifecycle
```

The supplied binary source is a caller's build-provenance assertion, not
inferred from the current checkout. The summary distinguishes it from the
harness checkout commit. Preserve the adjacent built-in `mods` installation if
copying a binary out of target. Preserve the exact tested binary separately if
the checkout will later be deleted; the wrapper hashes it but does not copy it.

Each of four runs receives a fresh loopback-only network namespace and isolated
save/config/cache/data/runtime roots via the existing `frame_steps_live.py`:

1. Ordinary graphical input, pause and manual steps; export; headless EOF.
2. Graphical EOF on that same immutable export.
3. Actual quicksave, advance, load-back, compare restored engine state, continue;
   export; headless EOF including post-load replay hashes.
4. Graphical EOF on the save/load export, verifying post-load hashes again.

Each driver has a 300-second internal alarm and a 330-second outer process-group
bound. A changed binary, missing summary or incomplete driver is an error. No
licensed assets are downloaded and no existing saves/replays/goldens are edited.
The gate does not exercise native multiplayer; that remains its own scenario.

The top-level summary includes the exact binary digest, supplied build source,
harness source, replay export digests and all four nested driver results.
Individual directories retain actual logs, engine dumps, screenshots, exports
and isolated runtime state. These are local test artifacts, not publication.

## Evidence location and failure handling

By default the gate prints a new `robin-lifecycle-gate-*` temporary directory.
Set `ROBIN_LIFECYCLE_EVIDENCE=/absolute/empty/path` to choose a durable location.
Existing nonempty directories are rejected rather than overwritten. A failure
after initialization writes `completed: false` and its error to `summary.json`.
CI uploads browser evidence even when execution fails. Local `/tmp` evidence
is not durable storage; move it before system cleanup if needed.
