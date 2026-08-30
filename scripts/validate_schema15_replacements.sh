#!/usr/bin/env bash
set -euo pipefail

workspace_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
runner="${PARITY_RUNNER:-$workspace_dir/target/parity/original_parity_replay}"
data_dir="${ROBINHOOD_DATA_DIR:-$workspace_dir/datadirs/fullgame_linux}"
validation_jobs="${VALIDATION_JOBS:-3}"
timeout_seconds="${VALIDATION_TIMEOUT_SECONDS:-900}"
result_root="${VALIDATION_RESULT_ROOT:-$workspace_dir/parity-save-replays/schema15-replacement-validation-20260817}"
require_exact="${VALIDATION_REQUIRE_EXACT:-1}"
status_dir="$result_root/status"
log_dir="$result_root/logs"

replacement_roots=(
    "$workspace_dir/parity-save-replays/60s-random-input/schema15-replacements-20260817/traces"
    "$workspace_dir/parity-save-replays/30s-random-input/schema15-replacements-20260817/traces"
    "$workspace_dir/parity-save-replays-legacy/10s-no-input-schema15-replacements-20260817/traces"
)

trace_id() {
    local trace="$1"
    local digest base
    digest="$(printf '%s' "${trace#"$workspace_dir/"}" | sha256sum | cut -c1-16)"
    base="$(basename -- "$trace" .jsonl.zst)"
    printf '%s-%s\n' "$base" "$digest"
}

validate_one() {
    local trace="$1"
    local id log_file status_file runner_exit status detail
    id="$(trace_id "$trace")"
    log_file="$log_dir/$id.log"
    status_file="$status_dir/$id.tsv"
    mkdir -p -- "$status_dir" "$log_dir"
    if [[ -f "$status_file" ]]; then
        return 0
    fi

    set +e
    env ROBINHOOD_DATA_DIR="$data_dir" \
        timeout --signal=TERM --kill-after=5s "$timeout_seconds" \
        "$runner" --no-auto-dump "$trace" >"$log_file" 2>&1
    runner_exit=$?
    set -e

    if rg -q 'parity trace matched every recorded frame' "$log_file"; then
        status=exact
        detail=exact_eof
    elif rg -q 'first parity divergence after frame' "$log_file"; then
        status=divergent
        detail="$(rg -m1 'first parity divergence after frame' "$log_file" | sed 's/^[[:space:]]*//')"
    elif [[ "$runner_exit" -eq 124 || "$runner_exit" -eq 137 ]]; then
        status=timeout
        detail="exit=$runner_exit"
    else
        status=error
        detail="exit=$runner_exit"
    fi

    printf '%s\t%s\t%s\n' \
        "$status" \
        "${trace#"$workspace_dir/"}" \
        "$detail" >"$status_file"
}

if [[ "${1:-}" == --worker ]]; then
    [[ $# -eq 2 ]] || exit 2
    validate_one "$2"
    exit
fi

[[ $# -eq 0 ]] || exit 2
[[ -x "$runner" ]] || { printf 'missing parity runner: %s\n' "$runner" >&2; exit 2; }
[[ "$validation_jobs" =~ ^[1-9][0-9]*$ ]] || exit 2
[[ "$require_exact" == 0 || "$require_exact" == 1 ]] || exit 2
mkdir -p -- "$status_dir" "$log_dir"

trace_list="$(mktemp)"
trap 'rm -f -- "$trace_list"' EXIT
# Logical .jsonl.zst identities; converted recordings only exist as
# <identity>.parity.bitcode.zst on disk.
find "${replacement_roots[@]}" -type f \( -name '*.jsonl.zst' \
    -o -name '*.jsonl.zst.parity.bitcode.zst' \) -print \
    | sed 's/\.parity\.bitcode\.zst$//' | sort -u >"$trace_list"
trace_count="$(wc -l <"$trace_list")"
if [[ "$trace_count" -ne 98 ]]; then
    printf 'refusing unexpected replacement count: got %s, expected 98\n' "$trace_count" >&2
    exit 2
fi

xargs -P "$validation_jobs" -n 1 "$0" --worker <"$trace_list"
{
    printf 'status\treplacement_trace\tdetail\n'
    cat "$status_dir"/*.tsv | sort -k2,2
} >"$result_root/results.tsv"
exact_count="$(awk -F '\t' '$1 == "exact" { count++ } END { print count + 0 }' "$status_dir"/*.tsv)"
printf 'schema-15 replacement validation complete: %s/%s exact traces\n' \
    "$exact_count" "$trace_count"
if [[ "$require_exact" == 1 && "$exact_count" -ne "$trace_count" ]]; then
    exit 1
fi
