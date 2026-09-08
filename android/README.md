# Android Build Notes

This directory packages the Rust NativeActivity library and operator-supplied
game assets. Linux and macOS build hosts are supported. Install JDK 17 or later,
configure the Android SDK using `ANDROID_HOME` or `android/local.properties`,
accept its licenses, and install the Rust target:

```sh
rustup target add aarch64-linux-android
android/gradlew :app:assembleDebug
```

The checked-in Gradle wrapper pins Gradle 9.4.1 and verifies the distribution
SHA-256. Its official wrapper JAR digest is recorded in `gradlew`. Gradle pins
NDK 29.0.14206865. Both the manifest minimum SDK and native clang target use
API 26 (Android 8.0+); compile/target SDK remains 35. The locked CPAL 0.18 audio
backend directly links AAudio and has no OpenSL ES fallback. The previously
advertised API 24 minimum was not buildable with that backend; Android 7.x is
not supported by this configuration. Both the manifest minimum and native
compiler use the same API value. To select an optimized native library:

```sh
android/gradlew :app:assembleDebug -PrustProfile=android-release
```

The APK signing variant is independent of the Rust optimization profile.
`buildRustAndroid` always invokes Cargo; Cargo decides which Rust inputs need
rebuilding, including vendored dependencies, shared build scripts, Git refs,
toolchains and target configuration.

To validate the native build and checked-in overlay without operator game data
or a device:

```sh
CARGO_BUILD_JOBS=1 android/gradlew --no-daemon --max-workers=1 :app:buildRustAndroid :app:validateCoreOverlay
```

This produces `target/aarch64-linux-android/android-dev/librobin_rs.so`; it is
not an APK boot test.

Generate native shipping data using the converter's default source audio and
copy its complete `Data/` closure into `android/assets/Data/`, including
`datadir.bin`, missions, RHS, terrain and audio companions. Do not substitute
the browser-only Opus package. This content is not checked in.

The retail-content-free core overlay is separately packaged from
`assets/core-datadir/`. Its exact inventory and hashes are checked before
packaging. Shipping schema comes from `wasm-www/runtime-contract.json`, generated
by compiled Rust constants; CI rejects stale generated metadata:

```sh
cargo build --locked -p robin_rs --example export_runtime_contract
target/debug/examples/export_runtime_contract --check wasm-www/runtime-contract.json
```

Use `--write` in place of `--check` after an intentional schema change. An
overlay/schema mismatch requires an explicit compatible overlay update; the
packager does not silently relabel old assets.

The Android boot path reads the bundled shipping datadir and the core-overlay
manifest from APK assets. It validates every declared core file by size and
SHA-256, mounts the complete bundle at engine-overlay priority, and probes all
required VFS paths before Rust initialization can construct fonts or UI. A
missing/corrupt core asset is a fatal packaging error. Loose filesystem data
remains available for debug installs by
putting a `Data/` directory in the app external files directory, or by
setting `ROBINHOOD_DATA_DIR` before startup in a custom launcher.

Android forces the graphical main menu for the bundled demo and disables
the desktop script-RPC HTTP listener. Hosting the standalone lobby broker
with `--lobby-server` is not supported. Multiplayer clients do not use a
hard-coded server: a custom launcher may provide `ROBINHOOD_LOBBY_WS`, and
the lobby UI reports that multiplayer is unavailable when it is unset.
