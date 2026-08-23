#!/usr/bin/env bash
set -euo pipefail

workspace=${PARITY_WATCH_WORKSPACE:-/home/phire/robinhood}
runner=${PARITY_WATCH_RUNNER:?PARITY_WATCH_RUNNER must name a pinned parity runner}
audit_dir=${PARITY_WATCH_AUDIT_DIR:-/home/phire/.cache/sccache/robinhood-parity-audits/autonomous-watch}
poll_seconds=${PARITY_WATCH_POLL_SECONDS:-60}
concurrency=${PARITY_WATCH_CONCURRENCY:-1}
recording_concurrency=${PARITY_WATCH_RECORDING_CONCURRENCY:-1}
stop_file="$audit_dir/.stop"

if [[ ! -x "$runner" ]]; then
    printf 'error: parity runner is not executable: %s\n' "$runner" >&2
    exit 2
fi
if [[ ! "$poll_seconds" =~ ^[1-9][0-9]*$ ]]; then
    printf 'error: PARITY_WATCH_POLL_SECONDS must be a positive integer\n' >&2
    exit 2
fi
if [[ ! "$concurrency" =~ ^[1-9][0-9]*$ ]]; then
    printf 'error: PARITY_WATCH_CONCURRENCY must be a positive integer\n' >&2
    exit 2
fi
if [[ ! "$recording_concurrency" =~ ^[1-9][0-9]*$ ]] \
    || (( recording_concurrency > concurrency )); then
    printf 'error: PARITY_WATCH_RECORDING_CONCURRENCY must be positive and no greater than PARITY_WATCH_CONCURRENCY\n' >&2
    exit 2
fi

mkdir -p -- "$audit_dir" "$audit_dir/logs" "$audit_dir/status"
cd -- "$workspace"

refresh_snapshot() {
    local authoritative_tmp generated_tmp combined_tmp campaign trace marker
    authoritative_tmp=$(mktemp "$audit_dir/authoritative.XXXXXX")
    generated_tmp=$(mktemp "$audit_dir/generated.XXXXXX")
    combined_tmp=$(mktemp "$audit_dir/combined.XXXXXX")

    if ! scripts/build_parity_full_snapshot.sh "$authoritative_tmp"; then
        rm -f -- "$authoritative_tmp" "$generated_tmp" "$combined_tmp"
        return 1
    fi

    # User-driven schema-16 recordings are converted only after the recorder
    # closes them, so every published trace in this directory is replayable.
    # Keep them ahead of batch corpora for immediate interactive feedback.
    # Converted recordings exist only as <identity>.parity.bitcode.zst, so
    # match both spellings and reduce to the logical .jsonl.zst identity.
    if [[ -d parity-save-replays/interactive ]]; then
        while IFS= read -r -d '' trace; do
            printf '%s\n' "$trace" >>"$generated_tmp"
        done < <(
            find parity-save-replays/interactive \
                -type f \( -name '*.jsonl.zst' \
                -o -name '*.jsonl.zst.parity.bitcode.zst' \) -print0 \
                | sed -z 's/\.parity\.bitcode\.zst$//' | sort -zu
        )
    fi

    # Every generated seed campaign carries campaign.env. Include only fully
    # published traces with their adjacent replay completion marker; partial
    # attempts and diagnostic/provisional directories never enter the sweep.
    # Visit newest seed campaigns first so freshly recorded corpora receive
    # parity feedback without waiting behind the entire historical manifest.
    while IFS= read -r -d '' campaign; do
        [[ -f "$campaign/campaign.env" && -d "$campaign/traces" ]] || continue
        while IFS= read -r -d '' trace; do
            marker=${trace%-session-*}.complete
            [[ -f "$marker" || -f "$trace.complete" ]] || continue
            printf '%s\n' "$trace" >>"$generated_tmp"
        done < <(find "$campaign/traces" -type f \( -name '*.jsonl.zst' \
            -o -name '*.jsonl.zst.parity.bitcode.zst' \) -print0 \
            | sed -z 's/\.parity\.bitcode\.zst$//' | sort -zu)
    done < <(
        find parity-save-replays/60s-random-input \
            -mindepth 1 -maxdepth 1 -type d -name 'schema16-seed*' -print0 \
            | sort -zr
    )

    # Preserve the priority order while removing paths already present in the
    # curated authoritative manifest.
    awk '!seen[$0]++' "$generated_tmp" "$authoritative_tmp" >"$combined_tmp"
    mv -f -- "$combined_tmp" "$audit_dir/traces.snapshot"
    rm -f -- "$authoritative_tmp" "$generated_tmp"
}

write_summary() {
    local total completed passed failed temporary status value
    total=$(wc -l <"$audit_dir/traces.snapshot")
    completed=0
    passed=0
    failed=0
    while IFS= read -r -d '' status; do
        completed=$((completed + 1))
        read -r value <"$status" || value=unreadable
        if [[ "$value" == 0 ]]; then
            passed=$((passed + 1))
        else
            failed=$((failed + 1))
        fi
    done < <(find "$audit_dir/status" -type f -name '*.status' -print0)

    temporary=$(mktemp "$audit_dir/summary.env.XXXXXX")
    {
        printf 'UPDATED_UTC=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'RUNNER=%s\n' "$runner"
        printf 'SNAPSHOT_TOTAL=%s\n' "$total"
        printf 'COMPLETED=%s\n' "$completed"
        printf 'PASSED=%s\n' "$passed"
        printf 'FAILED=%s\n' "$failed"
        printf 'PENDING=%s\n' "$((total - completed))"
    } >"$temporary"
    mv -f -- "$temporary" "$audit_dir/summary.env"

    find "$audit_dir/status" -type f -name '*.status' -print0 \
        | while IFS= read -r -d '' status; do
            read -r value <"$status" || value=unreadable
            [[ "$value" == 0 ]] || printf '%s\t%s\n' "$value" "$status"
        done \
        | sort >"$audit_dir/failures.tsv"
}

active_capture_reservations() {
    find parity-save-replays/60s-random-input \
        -path '*/schema16-seed*/.capture-reservations/*.reserve' \
        -type f -print -quit 2>/dev/null | grep -q .
}

printf 'autonomous parity watch started: runner=%s audit=%s\n' "$runner" "$audit_dir"
while [[ ! -e "$stop_file" ]]; do
    if refresh_snapshot; then
        active_concurrency=$concurrency
        if active_capture_reservations; then
            active_concurrency=$recording_concurrency
        fi
        printf '%s sweep concurrency=%s (configured max=%s)\n' \
            "$(date -Is)" "$active_concurrency" "$concurrency"
        sweep_pids=()
        for ((shard = 0; shard < active_concurrency; shard += 1)); do
            PARITY_SWEEP_GLOBAL_CONCURRENCY="$active_concurrency" \
                scripts/run_parity_release_sweep.sh \
                "$workspace" "$audit_dir" "$runner" "$shard" "$active_concurrency" &
            sweep_pids+=("$!")
        done
        for sweep_pid in "${sweep_pids[@]}"; do
            wait "$sweep_pid" || true
        done
        write_summary
    else
        printf 'warning: unable to refresh authoritative snapshot; retrying\n' >&2
    fi

    [[ ! -e "$stop_file" ]] || break
    sleep "$poll_seconds"
done
write_summary 2>/dev/null || true
printf 'autonomous parity watch stopped by %s\n' "$stop_file"
