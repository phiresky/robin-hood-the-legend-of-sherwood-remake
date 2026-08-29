#!/bin/sh
# Local-dev linker driver: clang driving the Wild linker (fast links).
# Referenced by [target.x86_64-unknown-linux-gnu] linker in .cargo/config.toml;
# CI replaces it wholesale with CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc.
# Living in a script (not rustflags) keeps target rustflags free for the shared
# cfg()-based target-cpu baseline, which CARGO_TARGET_*_RUSTFLAGS merges with
# instead of replacing.
exec clang --ld-path=wild "$@"
