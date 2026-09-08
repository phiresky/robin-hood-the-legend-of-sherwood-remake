#!/bin/sh
# Local-dev linker driver: clang driving the Wild linker (fast links).
# Referenced by [target.x86_64-unknown-linux-gnu] linker in .cargo/config.toml;
# CI replaces it wholesale with CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc.
# Living in a script (not rustflags) keeps target rustflags free for the shared
# cfg()-based target-cpu baseline, which CARGO_TARGET_*_RUSTFLAGS merges with
# instead of replacing.
if command -v clang >/dev/null 2>&1 && command -v wild >/dev/null 2>&1; then
    exec clang --ld-path=wild "$@"
fi
# Keep the measured fast path when installed, but allow ordinary Linux
# toolchains to build a fresh checkout without a machine-specific prerequisite.
echo "robin: clang/Wild unavailable; linking with cc (install both for faster links)" >&2
exec cc "$@"
