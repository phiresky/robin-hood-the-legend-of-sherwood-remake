# Robin Hood: The Legend of Sherwood

A from-scratch Rust reimplementation of the 2002 stealth-tactics game
[Robin Hood: The Legend of Sherwood](https://en.wikipedia.org/wiki/Robin_Hood:_The_Legend_of_Sherwood)
by Spellbound. Loads the original game's data files (demo or full release -
see [DATADIRS.md](docs/DATADIRS.md) for known versions and where to get them)
and plays them through a pure-Rust engine.

![Ferris in Robin Hood](docs/ferris-in-robin-hood.avif)

## Creation

I instrumented the original game to hook `rand()`, made pathfinding synchronous, and had it log every single thing that happens. This creates very large JSONL files. Then I replay these files in my engine and have an AI keep iterating on the code until the replay matches exactly. In between I look through code and refactor / refocus to try and ensure quality.

I took a set of 500 savegames of the original game (from the internet and myself), then created a dataset of 60s replays with random mouse input, 10 per save. These now replay almost 100% correctly. Then in addition I'm going to play through a bunch of missions in the instrumented original so I have more complete recordings and see that those match as well.

## Status

The engine mostly works. Most gameplay works exactly like the original. I have some perf problems and some bugs especially with the UI and save handling.

Some new features are already added, some incomplete, some TODO or "maybe later. Multiplayer for example - the basics work but it's not extensively tested. See [NEW_FEATURES.md](docs/NEW_FEATURES.md).

## Building

Currently only tested on a Linux host. Optional features: `video` (intro/outro via ffmpeg-next, on by default), `native-fs`
(OS data-dir lookup, on by default).

Theoretically, all the following platforms should be supported:

- Linux (wayland or X11)
- Windows
- MacOS
- Android (with touch support)
- Browser (WASM)

The toolchain (nightly Rust + cranelift
codegen backend) is pinned via [rust-toolchain.toml](rust-toolchain.toml)
and will be installed automatically by rustup.

    cargo build --bin robin          # debug
    cargo build --bin robin --release

### Native release packages

GitHub Releases provides x86-64 Windows and Linux builds, with stable releases
for version tags and rolling nightly prereleases. Windows downloads include ZIP
and `Setup.exe`; Linux downloads include a tarball and AppImage.
Installed packages should update automatically within their release channel.

The packages do not include the original game data. Set `ROBINHOOD_DATA_DIR` to
an extracted game data root as described in [Game data](#game-data). Intro and
outro video playback is not currently included.

Debug builds are tuned for fast iteration: `wild` linker,
`sccache` rustc wrapper, cranelift backend, dependencies built at
`opt-level=2`. See [AGENTS.md](AGENTS.md) for the full notes.

Tests and lints:

    cargo test
    cargo clippy --all-targets -- -D warnings
    cargo fmt

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

The deployed browser build is THREADED: `scripts/build-wasm-threads.sh`
wraps the same pipeline with `-C target-feature=+atomics,+bulk-memory,
+mutable-globals` and a shared, 4 GiB-max imported memory (extra rustflags
merged from `scripts/wasm-threads.cargo-config.toml`), plus the
`wasm-threads` cargo feature, which enables a `wasm-bindgen-rayon` worker
pool for parallel VQ sprite decode at mission install. The pool requires a
cross-origin-isolated page — `wasm-www/public/coi-serviceworker.js`
injects the COOP/COEP headers on header-less hosts like GitHub Pages (one
automatic reload on first visit) — and every decode path keeps a serial
fallback when isolation or the pool is unavailable, so the same artifact
still boots anywhere. The plain single-threaded pipeline above remains
supported and byte-identical.

Run `wasm-bindgen --target web` on the produced `.wasm` into
`wasm-www/pkg/`, then build the web package from `wasm-www/`:

    pnpm build

For the GitHub Pages shell, run:

    pnpm build:shell

That type-checks and bundles the TypeScript loader, then inlines the
compiled module into `dist-inline/index.html` so Pages can still deploy a
single HTML file. To apply the wasm optimization step to raw wasm-bindgen
output, run:

    pnpm strip:wasm-pkg

To split/strip a single Cargo-produced wasm before a wasm-bindgen pass,
call the helper with that file path:

    node wasm-www/scripts/optimize-wasm.mjs target/wasm32-unknown-unknown/wasm-release/robin.wasm

### WebAssembly deployment

GitHub Pages is split across two repositories:

- This repo deploys only `wasm-www/index.html` via
  `.github/workflows/deploy-wasm-shell.yml`.
- `.github/workflows/publish-wasm-binaries.yml` builds the
  `wasm-release` binary, runs `wasm-bindgen`, optimizes the served wasm
  with `wasm-opt -Oz` + `wasm-strip`, and pushes the versioned artifact to
  `phiresky/robin-hood-the-legend-of-sherwood-remake-binaries` on its
  `gh-pages` branch.

The binaries Pages repo stores wasm builds under `/wasm/`, indexed by the
same 12-character git hash that the Rust build embeds in `ROBIN_GIT_HASH`:

    /wasm/<short-hash>/robin.js
    /wasm/<short-hash>/robin.js.gz
    /wasm/<short-hash>/robin_bg.wasm
    /wasm/<short-hash>/robin_bg.wasm.gz
    /wasm/<short-hash>/manifest.json
    /wasm/latest.json
    /datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst
    /datadirs/demo-leicester/missions/*.rhmission.zst
    /datadirs/demo-leicester/rhs/*.rhmission.zst
    /datadirs/demo-leicester/audio/assets/*.opus

The shell fetches `/wasm/latest.json` when no query parameter is present. It
prefers the deterministic `.gz` JS/wasm siblings and expands them with the
browser's `DecompressionStream`, because GitHub Pages does not attach
`Content-Encoding` to static `.gz` files. It falls back to the ordinary files
for old builds, old browsers, and local development. With
`?replay=rhrec-<hash>-...`, it extracts `<hash>` and loads that exact
artifact directory. The game data is not rebuilt by CI because the source game
data cannot be stored in this repository. Build the production web artifact
with the canonical wrapper (which always selects JXL q80 maps, Opus audio, and
the wasm-safe zstd window):

    scripts/build_web_shipping_datadir.sh \
        datadirs/demo_leicester_ecoste /tmp/robin-web-shipping

Publish the generated `Data/datadir.bin` as
`/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst`, preserving its generated
`Data/missions/`, `Data/rhs/`, and `Data/audio/` directories beside it. The
browser initially fetches only the manifest, then fetches the selected
mission's bounded core, terrain, and exact RHS dependency closure concurrently.
Web audio is deterministic, content-addressed Opus under `audio/assets/`; each
file is fetched and decoded by Web Audio only when it is first played, so its
encoded bytes and decoded PCM never enter wasm memory. Only `arial.ttf`
and the required Rust UI PNG overlay assets remain beside the wasm artifact and
are listed in
`/wasm/<short-hash>/preload-assets.json`; the shell preloads those files before
`wasm_boot` starts the game loop.
Replay delivery itself remains handled by the existing browser/RPC path.
Wasm logging defaults to `info`; add `?wasm-log=debug` (or `trace`,
`warn`, `error`) to the URL to override it for browser sessions.

The publishing workflow needs:

- `BINARIES_REPO_TOKEN`: a token that can push to the binaries repo.
- A manually maintained `/datadirs/demo-leicester/` shipping tree in the
  binaries repo (manifest plus `missions/`, `rhs/`, `terrain/`, and `audio/`).

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
every APK. Its canonical manifest pins the 13 font/config files and 13 allied
control PNGs (size plus SHA-256) to shipping-datadir schema v9; Gradle checks
the source inventory, and Android validates and mounts it ahead of shipping
and mission bundles before UI initialization. Missing or corrupt core entries
fail startup rather than falling back to bundled game data.

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

    ROBINHOOD_DATA_DIR=datadirs/demo_leicester_ecoste cargo run --bin robin

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

Notable examples, built on demand with `cargo run --example <name>`:

    cpf_to_json       — dump a character-profile .cpf file as JSON
    dump_res          — inspect a .res resource archive
    disasm_scb        — disassemble a compiled .scb mission script
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
    crates/robin_lua/          Spellforge Lua runtime integration
    crates/robin_assets/       asset decoders (sprites, sounds, scripts, levels)
    crates/robin_util/         shared helpers
    crates/robin_state_hash_derive/ — derive macro for rollback state hashing
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


## AI Use disclaimer

I used AI to help create most of this code. I've been a [professional software engineer](https://github.com/phiresky) for more than 10 years, but by now AI is better at slogging through hundreds of thousands of lines of code while I can spend time planning, architecting, and playing this game :)
