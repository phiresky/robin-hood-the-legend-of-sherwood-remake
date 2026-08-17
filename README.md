# Robin Hood: The Legend of Sherwood

A from-scratch Rust reimplementation of the 2002 stealth-tactics game
[Robin Hood: The Legend of Sherwood](https://en.wikipedia.org/wiki/Robin_Hood:_The_Legend_of_Sherwood)
by Spellbound. Loads the original game's data files (demo or full release -
see [DATADIRS.md](docs/DATADIRS.md) for known versions and where to get them)
and plays them through a pure-Rust engine.

![Ferris in Robin Hood](docs/ferris-in-robin-hood.avif)

## Status

Playable mostly on the Leicester demo and the full campaign: main
menu, missions, save/load, replays should all _mostly_ work. Most testing has been done on the demo so the campaign logic is likely not fully working yet.
A fair amount of things are still broken, like bow / special items and some triggers.
The six-crate workspace contains 300K+ lines of Rust and 3,000+ tests. See [NEW_FEATURES.md](docs/NEW_FEATURES.md)
for new and future additions.

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

Debug builds are tuned for fast iteration: `mold` linker,
`sccache` rustc wrapper, cranelift backend, dependencies built at
`opt-level=2`. See [AGENTS.md](AGENTS.md) for the full notes.

Tests and lints:

    cargo test
    cargo clippy --all-targets -- -D warnings
    cargo fmt

### Testing with original game data

Tests that inspect shipped mission scripts are explicitly ignored because the
repository does not contain proprietary game data. They never silently pass
when data is unavailable. Point `ROBINHOOD_DATA_DIR` at an extracted data root
containing `Data/`, then select the tests for that distribution:

    ROBINHOOD_DATA_DIR=datadirs/demo_leicester_ecoste \
        cargo test -p robin_assets demo_script -- --ignored

    ROBINHOOD_DATA_DIR=datadirs/fullgame_gog \
        cargo test -p robin_assets fullgame_scripts -- --ignored

Running an ignored data-backed test without the variable, or against a data
root that lacks its required fixture, fails with an explicit diagnostic.

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
optimized build.  The release wasm profile uses `opt-level = "z"`, full
LTO, one codegen unit, no debuginfo, and aborting panics.  The two custom
profiles (defined in the workspace `Cargo.toml`) force the LLVM codegen
backend — cranelift doesn't target wasm.

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
    /wasm/<short-hash>/robin_bg.wasm
    /wasm/<short-hash>/manifest.json
    /wasm/latest.json
    /datadirs/demo-leicester/v4-q80.rhdata.zst

The shell fetches `/wasm/latest.json` when no query parameter is present.  With
`?replay=rhrec-<hash>-...`, it extracts `<hash>` and loads that exact
artifact directory.  The game-data blob is not rebuilt by CI; build it
locally and push it manually to
`/datadirs/demo-leicester/v4-q80.rhdata.zst` in the binaries repo.
Audio assets for the demo are published beside the wasm artifact and listed in
`/wasm/<short-hash>/preload-assets.json`; the shell preloads those files into
Rust before `wasm_boot` starts the synchronous game loop.
Replay delivery itself remains handled by the existing browser/RPC path.
Wasm logging defaults to `info`; add `?wasm-log=debug` (or `trace`,
`warn`, `error`) to the URL to override it for browser sessions.

The publishing workflow needs:

- `BINARIES_REPO_TOKEN`: a token that can push to the binaries repo.
- A manually maintained `/datadirs/demo-leicester/v4-q80.rhdata.zst` in the binaries
  repo.

### Android

Android builds use winit's `android-activity` NativeActivity glue. The
Android entry point is exported from the `robin_rs` cdylib, the
packaging manifest lives at `android/AndroidManifest.xml`, and the
Leicester demo shipping datadir is bundled as
`android/assets/Data/datadir.bin` from
`../../../binaries/datadirs/demo-leicester/v4-q80.rhdata.zst`.

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

Runtime data is loaded from the bundled APK asset first. Loose
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
