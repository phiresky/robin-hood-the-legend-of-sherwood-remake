#!/usr/bin/env bash
# Build both native runtime executables in the same target/profile. Pass normal
# cargo build flags, e.g. --release --features robin_rs/full --target TRIPLE.
set -euo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$repository"
cargo build --locked -p robin_rs -p robin_replay_format \
    --features robin_replay_format/native-admission \
    --bin robin --bin robin-replay-admission "$@"
