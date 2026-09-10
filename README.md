# Robin Hood: The Legend of Sherwood

A from-scratch Rust reimplementation of the 2002 stealth-tactics game
[Robin Hood: The Legend of Sherwood](https://en.wikipedia.org/wiki/Robin_Hood:_The_Legend_of_Sherwood)
by Spellbound. Loads the original game's data files (demo or full release -
see [DATADIRS.md](docs/DATADIRS.md) for known versions and where to get them)
and plays them through a pure-Rust engine.

![Ferris in Robin Hood](docs/ferris-in-robin-hood.avif)

## Creation

I record the original game's random outcomes and gameplay events in very large JSONL files. Then I replay these files in my engine and have an AI keep iterating until the replay matches exactly. In between I review and refactor this project to improve its quality.

I took a set of 500 savegames of the original game (from the internet and myself), then created a dataset of 60s replays with random mouse input, 10 per save. These now replay almost 100% correctly. Then in addition I'm going to play through a bunch of missions in the instrumented original so I have more complete recordings and see that those match as well.

## Status

The engine mostly works. Most gameplay works exactly like the original. I have some perf problems and some bugs especially with the UI and save handling.

Some new features are already added, some incomplete, some TODO or "maybe later. Multiplayer for example - the basics work but it's not extensively tested. See [NEW_FEATURES.md](docs/NEW_FEATURES.md).

## Building

Currently only tested on a Linux host. Bare builds use no optional client
features. Use `--features desktop` for the normal native game (audio, OS
data-directory lookup/dialogs, gamepads, and hardware reporting). Large
integrations are opt-in: `multiplayer` (iroh/DHT matchmaking), `video`
(intro/outro via ffmpeg-next), `retroarch-shaders` (librashader), and `lua`
(Spellforge custom missions). Packaged desktop builds additionally enable
`auto-update` (Velopack). Enable every runtime integration with:

    cargo build -p robin_rs --bin robin --features full

The native packaging workflow uses `--features release`, which includes the
desktop, Lua, multiplayer, and auto-update features. It will switch to `full`
once FFmpeg libraries can be bundled consistently on every release target.

At the workspace root, bare `cargo build` and `cargo test` intentionally cover
only the small utility/proc-macro smoke set. Build the minimal client with
`cargo build -p robin_rs --bin robin`, select any suite with `-p <crate>`, or
use `--workspace` for everything. Conversion/inspection bins require
`--features tools` and therefore stay out of bare builds.

Theoretically, all the following platforms should be supported:

- Linux (wayland or X11)
- Windows
- MacOS
- Android (with touch support)
- Browser (WASM)

The toolchain (nightly Rust + cranelift
codegen backend) is pinned via [rust-toolchain.toml](rust-toolchain.toml)
and will be installed automatically by rustup.

    cargo build -p robin_rs --bin robin          # debug
    cargo build -p robin_rs --bin robin --release

### Native release packages

GitHub Releases provides x86-64 Windows and Linux builds, with stable releases
for version tags and rolling nightly prereleases. Windows downloads include
`robinhood-remake-windows-Setup.exe` and `robinhood-remake-windows-Portable.zip`;
Linux uses `robinhood-remake-linux.AppImage`.
The two `.nupkg` assets are automatic-update payloads: `-windows-full.nupkg` is
Windows, and `-linux-full.nupkg` is Linux. Neither needs to be downloaded manually.
Installed packages should update automatically within their release channel.

Release automation stages a draft candidate, verifies uploaded artifact hashes,
and only then promotes it. Retries reuse the original run's
`nightly-YYYY-MM-DD-RUN_ID` candidate and require identical bytes; they do not
delete or overwrite previously published releases.

The packages do not include the original game data. Set `ROBINHOOD_DATA_DIR` to
an extracted game data root as described in [Game data](#game-data). Intro and
outro video playback is not currently included.

Debug builds are tuned for fast iteration with the Wild linker when available
(otherwise the system C linker), Cranelift,
and dependencies built at `opt-level=3`. Machine-local settings such as an
optional `sccache` wrapper belong in the user's Cargo configuration.
See [AGENTS.md](AGENTS.md) for the full notes.

Tests and formatting:

    cargo test -p robin_engine -p robin_assets -p robin_rs
    cargo fmt

Bare `cargo test` covers only the utility smoke set. Use the explicit suite
matrix in [TESTING.md](docs/TESTING.md) for the affected subsystem, original-data
tests, browser checks, and platform features. Clippy cleanup is handled in
dedicated sessions rather than interleaved with gameplay refactors.

### Testing with original game data

Some ignored integration tests require original game data. Point
`ROBINHOOD_DATA_DIR` at an absolute path containing `Data/`, then run the tests
for that distribution:

    ROBINHOOD_DATA_DIR=/absolute/path/to/leicester-demo \
        cargo test -p robin_assets demo_script -- --ignored

    ROBINHOOD_DATA_DIR=/absolute/path/to/full-game \
        cargo test -p robin_assets fullgame_scripts -- --ignored

### WebAssembly (browser)

The game builds for `wasm32-unknown-unknown` and uses `wasm-bindgen`
browser glue.  Audio is enabled for wasm builds; `ffmpeg-next` and
OS-data-dir support stay disabled.

    cargo build -Zbuild-std=std,panic_abort \
        --target wasm32-unknown-unknown \
        --profile wasm-dev            \
        --no-default-features         \
        --features audio              \
        -p robin_rs --bin robin

Swap `--profile wasm-dev` for `--profile wasm-release` for the smallest
optimized build.  The release wasm profile uses `opt-level = "z"`, fat
LTO, one codegen unit, no debuginfo, and aborting panics, with
`robin_assets` (the asset-decode hot path) overridden to `opt-level = 3`.
The `-Oz` LTO pipeline contains no vectorizer passes, so the wasm
rustflags inject them via `-C passes=…` — per-function size attributes
keep the injected passes conservative everywhere except the O3 codec
crate (see the profile comments in the workspace `Cargo.toml`).  Wasm
builds enable the `simd128` target feature (`.cargo/config.toml`).  The
two custom profiles force the LLVM codegen backend — cranelift doesn't
target wasm.

The browser build can use the threaded sprite decoder when the production
origin supplies cross-origin-isolation headers. Every decode path retains its
serial fallback. The deployment does not install a service worker to rewrite
response headers.

Run `wasm-bindgen --target web` on the produced `.wasm` into
`wasm-www/pkg/`, then build the web package from `wasm-www/`:

    pnpm build

That command builds both the game shell and `/leaderboards/`. To apply the wasm
optimization step to raw wasm-bindgen output, run:

    pnpm strip:wasm-pkg

To split/strip a single Cargo-produced wasm before a wasm-bindgen pass,
call the helper with that file path:

    node wasm-www/scripts/optimize-wasm.mjs target/wasm32-unknown-unknown/wasm-release/robin.wasm

### WebAssembly deployment

Production uses only Cloudflare Static Assets and the VPS:

- `https://robinhood.phiresky.xyz/` serves the game and leaderboard site.
- The more-specific `/wasm/*` route serves engine artifacts from the
  runtime-assets Worker. `/datadirs/*` is a separately authorized and manually
  deployed immutable Demo corpus on `robinhood-datadir-assets`; its bytes never
  enter normal runtime, site, publication, or VPS bundles.
- The `/api` prefix, including query strings, is a no-script Cloudflare route
  which continues to the Rust service on the VPS.
- `https://identity.robinhood.phiresky.xyz/` is a separate static signer
  origin. Private key material never enters the public site origin.

There is no GitHub-hosted production site, engine/data origin, or fallback.
The runtime Worker stores only wasm builds under `/wasm/`, indexed by the same
12-character git hash that Rust embeds in `ROBIN_GIT_HASH`. Its external
datadir deployment receipt is metadata, not game bytes:

    /wasm/<short-hash>/robin.js
    /wasm/<short-hash>/robin.js.gz
    /wasm/<short-hash>/snippets/.../browser_identity_client.js
    /wasm/<short-hash>/robin_bg.wasm
    /wasm/<short-hash>/robin_bg.wasm.gz
    /wasm/<short-hash>/manifest.json
    /wasm/latest.json
    /wasm/datadir-deployment.json

The dedicated datadir Worker stores only the separately authorized Demo
closure:

    /datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst
    /datadirs/demo-leicester/robinhood-web-content.json
    /datadirs/demo-leicester/missions/*.rhmission.zst
    /datadirs/demo-leicester/rhs/*.rhmission.zst
    /datadirs/demo-leicester/terrain/*.rhmission.zst
    /datadirs/demo-leicester/audio/*.rhmission.zst
    /datadirs/demo-leicester/audio/assets/*.opus
    /datadirs/demo-leicester/audio/bundles/*.bin

The shell fetches `/wasm/latest.json` when no query parameter is present. It
loads the exact static JavaScript import closure declared by that manifest.
The build stages the engine role before hashing: the public identity client is
retained, the private identity vault is removed, and every remaining imported
module is bound by canonical relative path, byte length, and SHA-256. Runtime
verification rejects a missing, substituted, tampered, orphaned, or vault
module. The shell can expand deterministic `.gz` wasm objects with the browser's
`DecompressionStream` when the static response has no `Content-Encoding`. It
falls back to the ordinary files for old browsers and local development. With
`?replay=rhrec-<hash>-...`, it extracts `<hash>` and loads that exact
artifact directory. The game data is not rebuilt by CI because the source game
data cannot be stored in this repository. Build the production web artifact
with the canonical wrapper (which always selects JXL q80 maps, Opus audio, and
the wasm-safe zstd window):

    scripts/build_web_shipping_datadir.sh \
        datadirs/demo_leicester_ecoste /tmp/robin-web-shipping

Publish the generated `Data/datadir.bin` as
`/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst`, preserving its generated
`Data/robinhood-web-content.json`, `Data/missions/`, `Data/rhs/`,
`Data/terrain/`, and `Data/audio/` closure beside it. The
browser initially fetches only the manifest, then fetches the selected
mission's bounded core, terrain, and exact RHS dependency closure concurrently.
Web audio is a deterministic, content-addressed Opus catalog under `audio/`.
Menu audio warms alongside engine startup; mission loading fetches the active
logical bundles and decodes only its dialogue, voices, music, and long
ambience. Other effects remain lazy, with cold playback requests held on real
mixer channels until their shared decode completes. A cold transient
non-looping effect reservation expires if playback has not started within 10
seconds; voice, loop, jingle, and music requests wait until explicit halt,
load failure, backend drop, or mission transition.
Encoded bytes and decoded PCM never enter wasm memory. Only `arial.ttf`
and the required Rust UI PNG overlay assets remain beside the wasm artifact and
are listed in
`/wasm/<short-hash>/preload-assets.json`; the shell preloads those files before
`wasm_boot` starts the game loop.
Replay delivery itself remains handled by the existing browser/RPC path.
Wasm logging defaults to `info`; add `?wasm-log=debug` (or `trace`,
`warn`, `error`) to the URL to override it for browser sessions.

The checked-in deployment topology and security headers are validated by
`pnpm test:deployment`. Runtime publication must retain the complete prior
corpus; a missing wasm build or Demo object is a deployment failure rather
than permission to replace production with a partial directory.

### Android

Android builds use winit's `android-activity` NativeActivity glue. The
Android entry point is exported from the `robin_rs` cdylib, the
packaging manifest lives at `android/AndroidManifest.xml`, and the
Leicester demo shipping datadir is bundled under `android/assets/Data/`:
`datadir.bin` must be generated separately with the converter's default
`--audio-format source`; do not use the web-only Opus artifact. Its generated
`missions/`, `rhs/`, and `audio/` directories must be copied alongside it.
Android reads selected payloads directly through `AAssetManager`.
The retail-content-free `assets/core-datadir/` is packaged separately into
every APK. Its canonical manifest pins the 13 font/config files and 19 engine
UI PNGs (size plus SHA-256) to the shipping schema declared in that manifest; Gradle checks the
source inventory, and Android validates and mounts it ahead of shipping and
mission bundles before UI initialization. Packaged desktop startup validates
the same manifest and exact loose-file inventory before registering the
overlay. Missing, extra, symlinked, or corrupt core entries fail startup rather
than falling back to game data.

Prerequisites:

    rustup target add aarch64-linux-android
    # Install Android SDK/NDK, then make the NDK clang visible to cc-rs:
    export ANDROID_NDK_HOME=/home/phire/tmp/android-sdk/ndk/29.0.14206865
    export PATH="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
    export CC_aarch64_linux_android=aarch64-linux-android35-clang
    export CXX_aarch64_linux_android=aarch64-linux-android35-clang++
    export AR_aarch64_linux_android=llvm-ar
    export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=aarch64-linux-android35-clang

Build the Rust shared library (the crate defaults to `rlib` so ordinary native
builds and tests do not also link an unused shared library):

    RUSTC_WRAPPER= cargo rustc -p robin_rs --lib --crate-type cdylib \
        --target aarch64-linux-android \
        --profile android-dev \
        --no-default-features --features android

`RUSTC_WRAPPER=` disables the workspace `sccache` wrapper for Android
cross builds; this environment currently rejects the wrapper for that
target with `Operation not permitted`.

The workspace's normal dev profile uses cranelift for Linux iteration;
the `android-dev` and `android-release` profiles force LLVM for Android
cross-compilation.

The APK must load `librobin_rs.so` and use the included NativeActivity
manifest metadata:

    <meta-data android:name="android.app.lib_name" android:value="robin_rs" />

Runtime shipping data is loaded from the bundled APK asset. The validated core
overlay has higher VFS priority. Loose
filesystem data is still supported as a developer override via
`ROBINHOOD_DATA_DIR` or a `Data/` folder under the app files directory.
Saves go to the app internal data directory under `saves/`. Video is
disabled in the Android feature set for now; ffmpeg packaging is a
follow-up once the native APK is booting on device.

## Running

The engine expects a `Data/` folder (and a locale subfolder like `1033/`)
in the current working directory, or pointed at via `ROBINHOOD_DATA_DIR`:

    ROBINHOOD_DATA_DIR=datadirs/demo_leicester_ecoste cargo run -p robin_rs --bin robin

Logging verbosity is controlled by `RUST_LOG` (`info`, `debug`,
`robin_rs=debug`, `trace`, etc.).

### CLI flags

- `--no-sound` — disable audio
- `--no-script` — disable mission script execution
- `--highlander2` — spawn enemy NPCs as invulnerable
- `--no-fog` — bypass fog sprite loading on converted data
- `--whatsup` — show the AI debug overlay
- `--goldeneye` — NPCs cannot see the player (debug cheat)
- `--no-default-loose` — ignore the default mission-lost condition
- `--record-default-key-config` — record the current shortcut config as default
- `--check-sound-data` — validate cached sound data during startup
- `--record <file.rhrec.jsonl>` — record a replay of this session
- `--replay <file.rhrec.jsonl>` — replay a previously recorded session
- `--mission <name> [--proto <map>]` — launch a mission directly
- `--mission <name>` also launches hackable JSON levels
  (`Data/Levels/<name>.level.json` in a `mods/<mod>/` overlay), e.g. the
  bundled `--mission Dover`
- `--custom-mission <zip>` — mount a vanilla custom-mission archive for a
  direct `--mission` launch
- `--view-cones` — render every NPC's view cone continuously
- `--rollback-check` / `--no-rollback-check` — per-frame rewind + replay
  desync detector (on by default in debug builds)

### Developer tools

The native release packages include four command-line [modding tools](docs/MODDING_TOOLS.md)
alongside the game executable: `cpf_to_json`, `encode_mod_sprites`, `disasm_scb`,
and `dump_res`. Build them with `cargo build -p robin_modding_tools --bins`.

Other examples, built on demand with `cargo run --example <name>`:

    run_script        — run a mission script headlessly
    count_quads       — render diagnostics
    render_mission_map — render a mission's full map at a chosen frame to PNG
    batch_run         — run many missions back-to-back (CI/regression)
    verify_rollback   — deterministic replay + state-hash verifier

Render all retail missions (revealed NPCs, frame 10) into `mission-maps/`:

    scripts/render_all_mission_maps.sh mission-maps 10 datadirs/fullgame_gog

## Game data

The repo ships without assets and requires either data from either the Demo (available online) or the actual purchased game.
Point `ROBINHOOD_DATA_DIR` at an extracted
installer - any of these are known to work:

- Leicester demo (2002, ECoste or Pariso build) - the default target
- Lincoln demo ("Free Lincoln" / DEMO II)
- Full retail release (original 2003 CD, GOG, Runesoft Linux port, Steam version, …)

See [DATADIRS.md](docs/DATADIRS.md) for the exhaustive list of installers,
hashes, and download sources for every known version and language.

On my machine, several pre-laid-out datadirs live under `datadirs/` for development:
`demo_leicester_ecoste` (default), `demo_leicester_linux`, `demo_lincoln`,
`fullgame_linux`, `fullgame_gog`.

## Workspace layout

    crates/robin_engine/       pure-sim tick, entities, AI, combat, pathfinding
    crates/robin_rs/           host: winit window/input, wgpu renderer, audio, UI, save I/O
    crates/robin_lua/          legacy direct-call Lua adapter for tools/tests
    crates/robin_spellforge/   production deterministic Lua runtime
    crates/robin_assets/       asset decoders (sprites, sounds, scripts, levels)
    crates/robin_replay_format/ bounded replay transport and admission formats
    crates/robin_replay_admission_wasm/ isolated browser replay decoder
    crates/robin_run_protocol/ signed run, query, content and build contracts
    crates/robin_ranked_verification/ approved-content and campaign validation
    crates/robin_replay_verifier/ authenticated replay worker
    crates/robin_highscores/   leaderboard API, worker, storage and administration
    crates/robin_official_content/ sealed official content projection
    crates/robin_manifest_tool/ release/content manifest tooling
    crates/robin_parity/       original-game trace conversion and comparison
    crates/robin_util/         shared helpers
    crates/robin_state_hash_derive/ — derive macro for rollback state hashing
    level-editor/              editor app, shared geometry, reconstruction pipeline
    wasm-www/                  browser shell, identity signer and leaderboards
    assets/                    icons, fonts
    datadirs/                  game data (gitignored)

## Intentional divergences from the original

- **Save format is serde JSON**. Saves live
  under the OS-appropriate user data dir (`dirs::data_dir()`), not next
  to the binary. Save loading is current-version-only and rejects corrupt or
  incompatible formats rather than migrating older saves.
- **Deterministic lockstep sim**, with a per-frame state hash, replay
  files, and a rollback checker - prerequisites for multiplayer (see
  [MULTIPLAYER.md](docs/MULTIPLAYER.md)).
- **GPU-accelerated rendering** on top of the original 16-bit RGB565
  software pipeline.

Further Rust-side additions and planned features are tracked in
[NEW_FEATURES.md](docs/NEW_FEATURES.md).

The [documentation index](docs/README.md) links testing, platform and operator
guides.


## AI Use disclaimer

I used AI to help create most of this code. I've been a [professional software engineer](https://github.com/phiresky) for more than 10 years, but by now AI is better at slogging through hundreds of thousands of lines of code while I can spend time planning, architecting, and playing this game :)
