# Android Build Notes

This directory contains the NativeActivity manifest and APK assets for
Android packaging. The Rust side exports `android_main` from the
`robin_rs` cdylib and uses winit's `android-native-activity` feature.
`assets/Data/datadir.bin` is copied from
`../../../binaries/datadirs/demo-leicester/v3-lossless.rhdata.zst`.

Build the shared library from the repo root:

```sh
ANDROID_NDK_HOME=/home/phire/tmp/android-sdk/ndk/29.0.14206865 \
PATH=/home/phire/tmp/android-sdk/ndk/29.0.14206865/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH \
CC_aarch64_linux_android=aarch64-linux-android35-clang \
CXX_aarch64_linux_android=aarch64-linux-android35-clang++ \
AR_aarch64_linux_android=llvm-ar \
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=aarch64-linux-android35-clang \
RUSTC_WRAPPER= cargo rustc -p robin_rs --lib --crate-type cdylib \
  --target aarch64-linux-android \
  --profile android-dev \
  --no-default-features --features android
```

External prerequisites are the Android Rust target and an Android NDK
toolchain on `PATH`. This worktree was verified with
`/home/phire/tmp/android-sdk/ndk/29.0.14206865` and
`aarch64-linux-android35-clang`. Set `RUSTC_WRAPPER=` for Android cross
builds if the workspace `sccache` wrapper fails under the sandbox. The
manifest expects the native library name `robin_rs`.

The Android boot path reads the bundled shipping datadir from APK
assets. Loose filesystem data remains available for debug installs by
putting a `Data/` directory in the app external files directory, or by
setting `ROBINHOOD_DATA_DIR` before startup in a custom launcher.

Android forces the graphical main menu for the bundled demo and disables
the desktop script-RPC HTTP listener. Hosting the standalone lobby broker
with `--lobby-server` is not supported. Multiplayer clients do not use a
hard-coded server: a custom launcher may provide `ROBINHOOD_LOBBY_WS`, and
the lobby UI reports that multiplayer is unavailable when it is unset.
