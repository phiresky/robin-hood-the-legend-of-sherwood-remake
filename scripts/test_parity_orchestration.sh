#!/usr/bin/env bash
# Fixture-free parity gate: no game assets, GPU, remote hosts, or Rust build.
set -euo pipefail
repository=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd -- "$repository"
python3 scripts/test_parity_result.py
python3 scripts/test_replay_state_db.py
for suite in \
    test_run_native_conversion_prepass \
    test_run_distributed_replay_worker \
    test_run_corpus_work_supervised \
    test_run_replay_refill_controller \
    test_run_incremental_eof_checks
do
    printf 'Running %s\n' "$suite"
    bash "scripts/$suite.sh"
done
