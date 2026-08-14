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
    full_key=${trace//\//__}
    relative=${trace#parity-save-replays/}
    short_key=${relative//\//__}
    status_files=()
    for key in "$full_key" "$short_key"; do
        status_file="$current_audit/status/$key.status"
        if [[ -f "$status_file" ]]; then
            status_files+=("$status_file")
        fi
        [[ "$short_key" != "$full_key" ]] || break
    done

    if (( ${#status_files[@]} == 0 )); then
        # An older revision's pass is not evidence for this revision: later
        # commits can regress a previously matching trace. Include every path
        # not yet tested by this exact sweep so the result remains proof-safe.
        printf '%s\n' "$trace" >> "$temporary"
        ((untested += 1))
        continue
    fi

    status=$(<"${status_files[0]}")
    status_conflict=false
    for status_file in "${status_files[@]:1}"; do
        if [[ "$(<"$status_file")" != "$status" ]]; then
            status_conflict=true
            break
        fi
    done
    if [[ "$status_conflict" == true ]]; then
        printf '%s\n' "$trace" >> "$temporary"
        ((integrity_unknown += 1))
        continue
    fi

    if [[ "$status" == 0 ]]; then
        proof_valid=true
        for status_file in "${status_files[@]}"; do
            key=${status_file##*/}
            key=${key%.status}
            log_file="$current_audit/logs/$key.log"
            marker_count=0
            if [[ -f "$log_file" ]]; then
                marker_count=$(grep -Fxc -- 'parity trace matched every recorded frame' "$log_file" || true)
            fi
            if [[ "$marker_count" != 1 ]] \
                || grep -Fq -- 'first parity divergence' "$log_file" \
                || grep -Fq -- 'panicked at' "$log_file"
            then
                proof_valid=false
                break
            fi
        done
        if [[ "$proof_valid" == true ]]; then
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
