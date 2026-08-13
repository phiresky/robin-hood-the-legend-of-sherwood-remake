#!/usr/bin/env bash
set -euo pipefail

if (( $# < 2 || $# > 3 )); then
    printf 'usage: %s FULL_SNAPSHOT CURRENT_AUDIT [OUTPUT]\n' "$0" >&2
    exit 2
fi

full_snapshot=$1
current_audit=$2
output=${3:-parity-failures.snapshot}

for required in "$full_snapshot" "$current_audit/status" "$current_audit/logs"; do
    if [[ ! -e "$required" ]]; then
        printf 'error: required input does not exist: %s\n' "$required" >&2
        exit 2
    fi
done

temporary=$(mktemp "$output.tmp.XXXXXX")
trap 'rm -f "$temporary"' EXIT

failures=0
untested=0
integrity_unknown=0
passes=0

while IFS= read -r trace; do
    [[ -n "$trace" ]] || continue
    key=${trace//\//__}
    status_file="$current_audit/status/$key.status"
    if [[ ! -f "$status_file" ]]; then
        # An older revision's pass is not evidence for this revision: later
        # commits can regress a previously matching trace. Include every path
        # not yet tested by this exact sweep so the result remains proof-safe.
        printf '%s\n' "$trace" >> "$temporary"
        ((untested += 1))
        continue
    fi

    status=$(<"$status_file")
    if [[ "$status" == 0 ]]; then
        log_file="$current_audit/logs/$key.log"
        marker_count=0
        if [[ -f "$log_file" ]]; then
            marker_count=$(grep -Fxc -- 'parity trace matched every recorded frame' "$log_file" || true)
        fi
        if [[ "$marker_count" == 1 ]]; then
            ((passes += 1))
        else
            # A zero process status without exactly one explicit EOF marker is
            # not proof of parity. Re-run it instead of silently excluding it.
            printf '%s\n' "$trace" >> "$temporary"
            ((integrity_unknown += 1))
        fi
    else
        printf '%s\n' "$trace" >> "$temporary"
        ((failures += 1))
    fi
done < "$full_snapshot"

mv "$temporary" "$output"
trap - EXIT

printf 'wrote %s: %d failures, %d untested, %d integrity-unknown, %d proven passes excluded (%d total)\n' \
    "$output" "$failures" "$untested" "$integrity_unknown" "$passes" \
    "$((failures + untested + integrity_unknown))"
