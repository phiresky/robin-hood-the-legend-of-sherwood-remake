#!/usr/bin/env bash
# Runs a hand-picked list of parity traces against the worktree build and
# summarises the first divergence of each. Usage: validate_motionstate.sh OUTDIR LISTFILE
set -u

outdir=$1
list=$2
wt=/home/phire/data/dev/2026/robin-hood-the-legend-of-sherwood/.claude/worktrees/fix-motionstate
corpus=/home/phire/data/dev/2026/robin-hood-the-legend-of-sherwood
runner=$wt/target/release/original_parity_replay
mkdir -p "$outdir"

run_one() {
    local rel=$1
    local key=${rel//\//__}
    timeout --signal=TERM --kill-after=10s 900s \
        env ROBINHOOD_DATA_DIR="$wt/datadirs/fullgame_linux" \
        "$runner" --no-auto-dump "$corpus/$rel" > "$outdir/$key.log" 2>&1
    printf '%s\n' "$?" > "$outdir/$key.status"
}
export -f run_one
export outdir wt corpus runner

xargs -a "$list" -P 5 -I{} bash -c 'run_one "$@"' _ {}
