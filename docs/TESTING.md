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

`bash scripts/check-quality.sh format` runs `cargo fmt --all -- --check` over
Cargo-discovered sources. Client modules are explicit declarations, so rustfmt
traverses their real module tree; no file needs a separate direct pass.

The legacy mlua comparison tests in `robin_rs` are compiled only with
`cargo test -p robin_rs --features lua`; ordinary client suites do not build
Luau's C++ sources.

## Crate-to-gate matrix

The Rust quality workflow runs the fixture-free Rust rows as separate Linux
jobs; `native-lifecycle` is local/provisioned only. All Cargo
commands use `--locked`. `scripts/test_quality_suites.py` checks that every
workspace member has an explicit gate, so adding a crate requires assigning it.

| Suite argument | Scope | What it checks |
| --- | --- | --- |
| `core` | `robin_util`, `robin_display_text`, `robin_state_hash_derive`, `robin_spellforge`, `robin_lua` | Unit, integration and doc tests |
| `scripting-llvm` | `robin_spellforge`, `robin_lua` | Explicit poison recovery and native-session unwind tests with LLVM package overrides |
| `engine` | `robin_engine` | Deterministic simulation tests |
| `assets` | `robin_assets`, `robin_data_io` | Assets with/without engine adapters; fixture resolver tests; engine and `robin_run_types` graphs exclude `robin_run_protocol`/Ed25519 |
| `protocols` | `robin_run_types`, `robin_run_protocol`, `robin_replay_format`, `robin_identity_signer` | Wire, bounded replay admission and isolated signer tests |
| `services` | `robin_highscores`, `robin_replay_verifier` | Server and verifier tests |
| `parity` | `robin_parity` | Runner unit/contract tests; does not replay licensed corpora |
| `client` | `robin_rs`, default features | Client tests, then a separate `robin` binary build |
| `client-release` | `robin_rs`, `release` features | Client library tests and binary build with desktop/audio/multiplayer/updates; audio example check |
| `tools` | `robin_modding_tools`; `robin_rs` with `tools` | Modding CLI and encoder tests; explicit converter/dump tests; check tool binaries and examples |
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
pure `robin_assets` must not pull in `robin_engine`. The checker prints
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

Small inspection tools are built on demand:

```sh
cargo build -p robin_rs --example count_quads --example list_mods --example verify_rollback
target/debug/examples/count_quads datadirs/demo_leicester_ecoste/Data/Levels/Dem_Lei_MP.scb
target/debug/examples/list_mods datadirs/mods
target/debug/examples/verify_rollback --data-dir datadirs/demo_leicester_ecoste
```

`count_quads` inspects the supplied SCB; `list_mods` reports launchable and broken
mod entries. `verify_rollback` runs the Original-loader determinism probe; its
`--data-dir` overrides `ROBINHOOD_DATA_DIR`. The completed sprite research probes
and one-time schema-10 replay migration are retained in Git history, not built
as maintained tools. The operational sprite benchmarks remain opt-in examples.

The Ubuntu 26.04 CI job installs the native library development packages it needs.
The services suite also needs bubblewrap 0.11.1 or newer and a system POSIX shell. Its runner
allows unprivileged user namespaces for the bwrap-sandboxed verifier.
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
the dev-only `robin_test_support` crate. Fixture gates first enumerate the
compiled tests using the same target, features and selector as execution, and
fail if the selection is empty; renaming a fixture cannot silently remove coverage.

```sh
ROBINHOOD_DATA_DIR=/absolute/path/to/leicester-demo bash scripts/check-quality.sh fixtures-demo
ROBINHOOD_DATA_DIR=/absolute/path/to/full-game bash scripts/check-quality.sh fixtures-fullgame
```

Each root must contain `Data/` (original distribution casing is supported).
Both profile fixture gates read the shipped `Data/Configuration/profile.cpf`
directly and validate its decoded profiles. They also export its canonical JSON
document into an isolated in-memory filesystem and exercise the engine JSON
loader, comparing every decoded field. No generated `profile.json` or
`profile.cpf.json` is required, and neither gate writes into the game-data root.
The demo additionally checks the full profile serde round trip, font, sprite
banks, and script decoding/manager fixtures.
It also checks the Ecoste demo's five dialogue portraits in
`DATA/Interface/DEFAULT.RES` (resource 267), including archive loading and RGBA
conversion; this fixture is explicitly selected with `engine-adapters`. The
full-game suite checks its profile and script collection. Both suites explicitly
run the converter's typed-edition tests; the demo also checks the English
`1033/Data/Interface/Start.sxt` picture (using case-aware resolution). Its root
must therefore include that locale alongside `Data/`. These converter fixtures
are ignored in ordinary runs and fail setup if explicitly selected without data.
Run each distribution
separately: blindly selecting every ignored test would combine incompatible
fixture requirements, GPU execution, and other opt-in integration scenarios.
No licensed data is fetched or assumed present on public CI.

The regular font tests read the checked-in Arial fixture relative to
`CARGO_MANIFEST_DIR`, so their behavior does not depend on the shell directory.

### Legacy Linux save fixtures

The five Linux i386 v48 save-parser cases are ignored in the ordinary engine
suite and explicitly selected by their own gate:

```sh
ROBINHOOD_DATA_DIR=/absolute/path/to/fullgame_linux bash scripts/check-quality.sh fixtures-legacy-linux
```

This requires `Data/Savegame/Profile_000/Restart` (the Lincoln golden save),
`Data/Savegame/Profile_001/Continue`, and `Data/Configuration/profile.cpf`.
Both campaign cases require the profile and run their bootstrap/history
assertions; missing inputs fail rather than silently reducing coverage.
Restart retains exact golden offsets/values; Continue is mutable profile state
and retains its structural checks. An arbitrary full-game distribution or save
is not a substitute for these fixtures, so this gate is separate from
`fixtures-fullgame`.

The two retail Windows v48 cases remain ordinary tests using the required,
tracked `reference-saves/Savegame_SuN1Sh1nE/Profile_004/Savegame_005`. They do not
use `ROBINHOOD_DATA_DIR` and fail if the checkout fixture is absent.

## Editor-generated map routes

The regular `engine::movement::tests::compiled_navigation` engine unit tests
run queued path requests, native order postprocessing and actor movement ticks.
They check routes around wall ends and across curved, rising and rotated
walkways in both directions, including collision clearance and receiving height
at every tick. These fixtures use a synthetic walking animation. The level-data
`compiled_elevation` tests additionally cover fractional seams, tiny overlaps,
gaps and legacy integer endpoint validation.

The ignored `exported_receiving_seams_support_actor_crossings` test reads the same
`ROBIN_ASSET_MAP_DIAGNOSTICS` batch used below. It selects one collision-clear,
24-unit perpendicular crossing per eligible receiving-plane pair and walks both
directions using actor ticks. Endpoints must lie inside different receivers;
intermediate exact-boundary positions accept either adjacent receiver. Other
positions must match the queried receiver, and every tick must match its height.
Its `actor-receiver-crossing-report.json` records eligible pairs and directed
crossings per map, including zero-coverage maps. This samples initial-state seams
with at least 16 units of edge length; it does not certify short seams, ground-only
boundaries, every point along an edge, or changing traversal surfaces.
The companion `exported_ground_boundaries_support_actor_crossings` checks one
crossing per eligible plane-to-ground pair in both directions and writes
`actor-ground-crossing-report.json`. A regular synthetic fixture also walks up
onto a raised receiving plane and back to uncovered ground, checking receiver
removal and restored ground height.

For editor-generated navigation, the ignored integration test
`library_exports_route_between_collision_connected_samples` reads a complete
`diagnostics.json` batch from `ROBIN_ASSET_MAP_DIAGNOSTICS`. It samples numbered
ordinary motion sectors, flood-fills actor-sized collision-clear connections,
and asks for forward/reverse routes across each sampled component. Every returned
segment must clear the same 6×3 half-diagonal footprint. It reports routing and
sampling times separately, and writes `route-sampling-report.json` after each map.
The report remains `complete: false` until every map passes. Run it with fresh
asset-only exports:

```sh
ROBIN_ASSET_MAP_DIAGNOSTICS=/absolute/path/to/exports RUSTC_WRAPPER= \
  cargo test -p robin_engine --test asset_map_compilation -j1 \
  library_exports_route_between_collision_connected_samples -- --ignored --nocapture
```

This is a routing/collision consistency check, not an oracle for missing authored
geometry. Coarse samples omit narrow passages, inter-sector traversal and dynamic
state changes; those require their separate gameplay fixtures.

To sample the ordinary movement areas changed by each exported transition, run:

```sh
ROBIN_ASSET_MAP_DIAGNOSTICS=/absolute/path/to/exports RUSTC_WRAPPER= \
  cargo test -p robin_engine --lib -j1 \
  exported_state_routes_match_live_collision_components -- --ignored --nocapture
```

This reuses the live pathfinder through initial, applied and reset states. Each
state recomputes collision-connected samples and checks returned route clearance.
`state-route-sampling-report.json` records counts per transition/state, including
zero-coverage entries. The companion ignored test
`exported_combined_state_routes_match_live_collision_components` applies all
transitions together, then resets them in reverse order, writing
`combined-state-route-sampling-report.json`. These checks cover independent and
all-applied states; arbitrary combinations, door permissions, inter-sector
traversal and narrow unsampled passages still need separate coverage.

Complete actor walks through every directed pair of stair entrances can be
audited separately:

```sh
ROBIN_ASSET_MAP_DIAGNOSTICS=/absolute/path/to/exports RUSTC_WRAPPER= \
  cargo test -p robin_engine --lib -j1 \
  exported_stairs_support_complete_actor_routes -- --ignored --nocapture
```

`actor-stair-route-report.json` records successful routes, forbidden routes and
failures per map. It uses synthetic walking frames with the stock human 6×3
half-diagonal and checks arrival, exact sector/layer, and receiving surfaces and
height outside passage animation intervals. Arrival always checks the receiver.
During a passage, sector membership changes before the two approach movements
finish, so its receiving plane can temporarily belong to the adjoining sector.
This is an initial-state stair test, not coverage of ladders, walls, all character
profiles, live sprite resources or mission behavior.

Climbing routes use a complete character animation profile instead of synthetic
walking frames. Only that explicitly supplied RHS file is mounted; source-level
files and mission scripts are unavailable. Navigation comes from the compiled
editor descriptors:

```sh
ROBIN_ASSET_MAP_DIAGNOSTICS=/absolute/path/to/exports \
ROBIN_CLIMB_RHS=/absolute/path/to/Data/Characters/RobinTown.rhs RUSTC_WRAPPER= \
  cargo test -p robin_engine --lib -j1 \
  exported_climbs_support_complete_actor_routes -- --ignored --nocapture
```

`actor-climb-route-report.json` checks every directed entrance pair of ladder and
wall sectors, including crenellated walls. It requires arrival in the exact
destination sector/layer with its receiving plane and height. During climbing,
animation movement may retain the preceding plane; the ordinary receiving
lookup assertion applies after landing. This exercises animation timing and
movement, not rendered frame alignment, other profiles or actor contention.
`ROBIN_LIFT_AUDIT_MAP=derby.level.json` restricts either audit to one map and records
that filter in its report. `ROBIN_LIFT_TRACE=1` adds climb and receiver diagnostics.
The ignored `placed_climbs_support_complete_actor_routes` test needs only
`ROBIN_CLIMB_RHS`: it checks a constructed ladder, ordinary wall and crenellated
wall at every whole-degree rotation plus 22.5°, traversed both ways (2,166 routes).
The regular `arbitrarily_rotated_stairs_support_complete_actor_routes` test checks
722 stair routes over the same angles. These catch clearance and receiving-plane
failures independently of imported layouts, including wall-top direction repair
while preserving the animation's fixed radius.
`compiled_lift_endpoints_and_ai_follow_height_after_rotation` also checks the
cached fall destination and actual AI forecast at ordinary, horizontal and
reversed placements. Shipping codec tests preserve endpoint identities through
both datadir v23 and mission v14 round trips and reject the preceding formats.

## GPU execution

Scenery profile authoring tests run with
`pnpm --filter pipeline exec node --test src/extract-scenery-profile.test.ts`
from `level-editor`. They verify exact atlas crop pixels, lossless copying of
complete PNG sequences, timing/offset preservation and rejection of malformed
resources. The existing standalone authoring suites check placement and export.
A real six-frame candle profile was extracted to
`work/map-compile/scenery-candle-profile-20261004.rhs.d` and authored into
`work/map-compile/scenery-candle-asset-20261004`. The native sprite family encoder
verifies all six frames in `scenery-candle-native-20261004.sprites.vq.zst`.
These artifacts test resource authoring and decoding, not rendered animation.

The candle asset also passes offline library staging and a fresh export from the
staged runtime catalog (`scenery-candle-staged-export-20261004`): all 15 archive
entries match the pre-publication export exactly. Three independent placements
exercise rotation and elevation. Run `full_editor_archive_constructs_native_map_without_base_datadir`
in the `robin_rs` integration test `editor_mod_export`, with `--ignored --nocapture`,
`RUST_MIN_STACK=16777216` and `ROBIN_EDITOR_MAP_ZIP` pointing at that directory's
`editor-scenery-library.zip`. It mounts only the ZIP, decodes all pinned frames,
constructs the engine, verifies each effect's anchor/elevation, and advances cloned
native sprites through all six frames. Its gradient ground is diagnostic artwork;
this does not certify native rendering, occlusion or full-engine frame scheduling.
The ordinary level-data regression covers animation-only, mixed control/animation,
and empty construction groups.

The normal mission preload has separate coverage in
`game_session::setup::custom_sprites::tests`: a directory/ZIP fixture loads scenery
with no characters and skips an unrelated malformed bank. Run
`published_scenery_archive_uses_normal_mission_preload` with `--ignored` and
`SCENERY_LIBRARY_ZIP` pointing at the candle archive to verify all six frames
through the production preload, without manually injecting profiles.

`app/src/scenery-frames.test.ts` checks the initial sentinel tick, inclusive frame
delays, looping and the maximum unsigned delay. The browser fixture
`tests/scenery-live.html` checks a two-frame placed asset: GPU-projected pixels,
legacy transparency, frame offsets, native-coordinate placement, a live asset
drag, copies sharing textures, hidden assets, bake isolation and resource disposal.
Run it with `TEST_PAGE=scenery-live.html` through `app/tests/run-lifecycle.mjs`
against the editor development server. `tests/scenery-preview.html` separately
checks palette artwork. These checks do not certify native scenery compositing,
shadow previews, other action rows or orientation-specific artwork.

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

To include an editor-exported depth PNG in the pixel contract, first generate a
fresh fixture directory by setting `SCENERY_DEPTH_EXPORT_DIR` while running
`level-editor/app/src/scenery-resources.test.ts` through the editor's Node test
runner. Pass the same absolute directory to the GPU gate. It decodes the packaged
16-bit PNG with the normal map loader and checks masked sprite pixels at three
ground depths. The fixture uses authored depth steps; it does not test the editor's
3D baking shader, complete entity-mask selection, or scenery-overlay ordering.

For a real 3D-baked input, run the editor browser fixture `tests/map-bake.html`
with `TEST_BAKE_ZIP` set, then extract that ZIP into a fresh directory. Set
`BAKED_DEPTH_EXPORT_DIR` to its absolute path when running the GPU gate. This
checks ground and raised surfaces, a transparent cutout, ownership-filled
geometry and both sides of a bake tile seam at three character depths. The
browser fixture also validates nonzero crop rebasing before exporting the ZIP.

To exercise changing assets, run the same browser fixture with
`TEST_QUERY='?export=state'` and extract its `TEST_BAKE_ZIP` into a separate fresh
directory. Pass that absolute directory as `BAKED_STATE_EXPORT_DIR` to the GPU
gate. The native background loader reads its appearance manifest and state images;
pixel checks verify initial, applied, transitioning and reset color/depth on both
sides of a bake tile seam. This assigns patch flags directly to isolate exported
image binding and rendering; gameplay patch callbacks are tested separately.

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

The editor browser harness also accepts `TEST_PAGE=scenery-preview.html` when
pointed at its lifecycle Vite server. It renders real `AssetPreview` cards from
pinned fixture banks and checks canvas pixels for legacy transparency and RGBA
color preservation. This covers static palette thumbnails, not in-map animation.

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

## Wire format freezes

Native parity traces and shipping assets have independent codec contracts.
`robin_parity` pins crates.io `bitcode` 0.6.9 for authoritative v68 traces;
shipping assets pin the workspace `bitcode` git revision for datadir v18 and
mission v9. Two sources are intentional: dependency deduplication must not
silently change either format. Updating one codec requires checking its frozen
contract independently, not assuming a matching package version is compatible.

Shipping's independent frozen-layout checks live in
`robin_assets/src/shipping_v18_contract.rs` and `shipping_v9_contract.rs` and run
with `cargo test -p robin_assets`. Native trace checks run with
`cargo test -p robin_parity`; validate real frozen replay evidence before
changing its codec or layout. Runtime native readers accept v68 only; older
artifacts require offline migration, never a fallback decode guess.

Layout changes require an explicit version/magic bump and regeneration or an
offline migration tool. Do not update frozen descriptors merely to make a
changed live layout pass. Authored sprite envelopes use separately versioned,
budget-preflighted JSON and do not relax the shipping or replay wire contracts.

Gameplay-preserving asset publication is covered by
`python3 -m unittest test_promote_staged_publication` from
`level-editor/refinement` (21 tests). Ambient recovery after draft publication is
covered by `node --test pipeline/src/author-ambient-sound-asset.test.ts` from
`level-editor`. These checks do not certify complete map gameplay coverage.

`pipeline/src/recover-light-region.test.ts` also checks that sloped light segments
exclude upper and lower floors and compile after translation, elevation changes
and four rotations. The ignored native test
`recovered_light_exports_preserve_contours_layers_and_ambience` checks actual
loaded shadow sectors against exported contours/layers under ambience masks 1, 2
and 4; set `ROBIN_ASSET_MAP_DIAGNOSTICS` to a diagnostic export directory.

`node --test pipeline/src/author-linked-appearance.test.ts` from `level-editor`
checks nonmutating shared-control authoring, joined/detached compilation,
coordinate conversion of trigger contours and receiving segments, and rejection
of conflicting ownership. It does not verify rendered appearance pixels.

The recovered drawbridge state fixtures in
`level-editor/work/map-compile/lincoln-bridge-state-hWz7IQ` cover baseline and
jointly moved bridge/gatehouse placements. With that directory in
`ROBIN_ASSET_MAP_DIAGNOSTICS`, the ignored native test
`recovered_asset_transitions_apply_and_reset_native_geometry` verifies ten controls
per placement, including the independent obstacle-state and six-door-rights controls.

`work/map-compile/lincoln-plateau-state-gaTztD` adds the plateau/terrace reveal:
native apply/reset checks all eleven controls in both baseline and moved fixtures.
Reopening the published scene and compiling its pinned assets exactly reproduces
the tested baseline geometry (eleven controls, ten lifts, 33 light sectors).

The `map-bake.html` browser fixture checks cropped appearance rendering against
full-frame color/depth pixels under three sun directions and across multiple tile
boundaries. Keeping the same tile grid is required for exact edge rasterization.
The 13 appearance unit tests also check independent regional state rendering and
restoration after cancellation. The client archive smoke test passes; its ignored
full-ZIP test additionally validates appearance state dimensions and references
against the loaded engine's controls. This does not certify a full Lincoln ZIP
until that bake and ignored test complete.

`work/map-compile/nottingham-gate-control-OiiIqc` verifies transfer of the gate's
three door lanes from its receiving floor to the arch. Baseline exported door
data is unchanged; independent one-unit moves of the arch and floor prove the
control follows only the arch. Native apply/reset passes all nine controls in
each of the three fixtures. This is placement/permission coverage, not animated
portcullis rendering or arbitrary-placement traversal certification.

The full-library browser fixture retains its generated ZIP as a `Uint8Array`.
The CDP runner converts only individual download chunks to JSON arrays; converting
the entire archive with `Array.from` failed after Lincoln's 45 appearance renders
and packaging completed. That failed harness run produced no saved ZIP and does
not count as a successful native archive round trip.

`work/map-compile/nottingham-tower-state-KurPT1` covers the independent east-tower
reveal. Both the baseline and one-unit tower move compile without missing
appearance bindings, retain the receiving sector/layer, and pass native apply/reset
for all ten controls. A fresh compile of the published scene matches the tested
baseline exactly. This does not verify animated sprites or unrecovered masks.

The full Lincoln browser archive now passes
`full_editor_archive_constructs_native_map_without_base_datadir` with
`ROBIN_EDITOR_MAP_ZIP=level-editor/work/map-compile/lincoln-plateau-state-gaTztD/editor-lincoln-current.zip`
(use an absolute path when invoking Cargo). The 152,115,399-byte archive decodes
color, depth, minimap and its embedded editor scene, and constructs 546 sight
obstacles, 86 door projections and 69,920 grid blocks without base game files.
Three appearance regions contain 48 states, referencing eleven native controls;
the descriptor retains ten lifts and 33 light sectors. It contains no explicit
masks. This is archive/loading validation, not full-map rendered-state parity.

`work/map-compile/published-controls-audit-20261004` exports all ten saved scenes
from current pinned assets. Native apply/reset passes all 60 compiled controls
in that snapshot. Best-effort omissions remain: Wychford reports 25 doors, 15 masks,
five jumps, one lift and one control omitted; Derby and Leicester report 98/203 unbound
appearance-part warnings. These are part warnings, not distinct asset counts.
`work/map-compile/derby-appearance-controls-lwHfaC` subsequently adds two Derby
reveals: all four controls pass baseline and independent hall/gatehouse moves,
leaving the east hall's sight-state appearance unfinished.

`work/map-compile/derby-hall-sight-state-fWXBwB` closes that east-hall binding:
one initial and eight applied sight volumes switch, with unchanged baseline
navigation/doors/lifts. Native apply/reset passes all five controls in baseline
and one-unit hall-move exports.

`work/map-compile/leicester-appearance-controls-MJmVl8` passes native apply/reset
for twelve baseline/jointly moved controls and thirteen with the church-side
tower detached. All modeled appearance bindings resolve. Baseline navigation,
doors and lifts are unchanged. The joint move leaves three independent light
regions without receivers; this is a warning-bearing placement test, not a
complete lighting-placement pass. Published Derby and Leicester scenes compile
exactly to their tested baselines. Animated effects and mask coverage remain open.

The compiler/terrain suites pass 120 tests, including best-effort retry input
preservation and terrain edits between exports. Wychford's profiled baseline
(`work/map-compile/wychford-control-audit-JQmg56`) and terrain-reuse result
(`work/map-compile/wychford-control-audit-SHbyM9`) have byte-identical descriptors
and warning lists. Observed compile time fell from 128.75 to 77.84 seconds; the
baseline used CPU profiling, so these are diagnostic timings, not a controlled
benchmark. Precision and clipping rules are unchanged.
Native construction and apply/reset pass both controls in the updated Wychford
descriptor; that diagnostic took 176.82 seconds and does not certify all routes.

Receiving-boundary indexing is checked against exhaustive construction in the
level-data seam regressions and ten native geometry fixtures. The comparison
includes precise endpoints, receiver identities and output ordering. To compare
a larger exported descriptor, set `ROBIN_COMPILED_GEOMETRY` to its absolute path
and run `cargo test -p robin_level_data exported_spatial_index_matches_exhaustive_boundaries
-- --ignored --nocapture`. This prints both timings; the exhaustive scan can take
several minutes on densely triangulated terrain.

For `wychford-control-audit-SHbyM9/wychford.level.json`, all 28,749 boundaries
match exactly: indexed construction took 5.17 seconds, exhaustive construction
296.18 seconds. Native construction plus applying/resetting both controls takes
6.86 seconds with indexing, compared with the previous 176.82-second run. These
are debug diagnostic timings, not controlled benchmarks. The exhaustive test's
candidate enumeration also allocates lists that the former implementation did
not, so its timing must not be treated as the previous loader's timing.
The native map-compilation suite passes 61 tests (five data-dependent tests
ignored).
Fourteen ordinary actor-navigation tests also pass (five data-dependent tests
ignored), including the stair rotation sweep. Re-running the historical ten-map
`published-controls-audit-20261004` snapshot passes all 60 control apply/reset
checks in 7.47 seconds. That snapshot predates the latest Derby/Leicester/Wychford
asset updates and must not be described as a fresh library export.
