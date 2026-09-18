#!/usr/bin/env bash
# Build the native game executable. Pass normal cargo build flags, e.g.
# --release --features robin_rs/full --target TRIPLE.
set -euo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$repository"
cargo build --locked -p robin_rs --bin robin "$@"
