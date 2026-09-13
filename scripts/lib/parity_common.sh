#!/usr/bin/env bash
# Shared parity-driver mechanisms. Admission, locking and publication policy
# stay with each driver.
#
# Runner-bundle authentication has exactly one implementation here
# (verify_runner_bundle + verify_runner_bundle_identity). Drivers differ only
# in which loader proof they can check (a path-bound LOADER_LIST.txt, or a live
# relocation check for bundles copied from another host) and in how they react
# to a rejection (`|| exit 2`, or publishing an infrastructure stop first).
#
# Per-trace `run_one` bodies are deliberately NOT shared as a whole: the
# conversion prepass and the reblock snapshot drive different runner modes
# (`--convert` / `--reblock`) with different pre/postconditions, and
# run_incremental_eof_checks.sh publishes content-addressed result directories
# under nonblocking shared admission locks instead of a per-trace status
# ledger. The ledger mechanics the prepass and reblock drivers do share (key
# derivation, status parsing, attempt-log reservation and publication) live in
# the attempt_* helpers below.

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 2
}

sha256_file() {
    local value
    value=$(sha256sum -- "$1") || return 1
    printf '%s\n' "${value%% *}"
}

write_atomic() {
    local destination=$1 temporary
    temporary=$(mktemp "${destination}.tmp.XXXXXX") || return 1
    if ! cat >"$temporary" || ! mv -f -- "$temporary" "$destination"; then
        rm -f -- "$temporary"
        return 1
    fi
}

normalize_bounded_uint() {
    local LC_ALL=C value=$1 limit=$2
    [[ "$value" =~ ^[0-9]+$ && "$limit" =~ ^[0-9]+$ ]] || return 1
    while [[ ${#value} -gt 1 && "$value" == 0* ]]; do value=${value#0}; done
    while [[ ${#limit} -gt 1 && "$limit" == 0* ]]; do limit=${limit#0}; done
    if (( ${#value} > ${#limit} )) \
        || { (( ${#value} == ${#limit} )) && [[ "$value" > "$limit" ]]; }; then
        return 1
    fi
    printf '%s\n' "$value"
}

runner_bundle_digest() {
    local bundle=$1 main_sha lib_sha value
    main_sha=$(sha256_file "$bundle/SHA256SUMS") || return 1
    lib_sha=$(sha256_file "$bundle/LIB_SHA256SUMS") || return 1
    value=$(printf 'schema16-runner-bundle-v1\nSHA256SUMS=%s\nLIB_SHA256SUMS=%s\n' \
        "$main_sha" "$lib_sha" | sha256sum) || return 1
    printf '%s\n' "${value%% *}"
}

# verify_runner_bundle BUNDLE LOADER_PROOF
#
# Authenticates the canonical protocol-2 runner bundle layout: required
# executables and metadata, no symlinks anywhere, checksum manifests whose
# entries are well-formed relative paths without `..` traversal (checked before
# any checksum is evaluated), manifests that exactly cover the root file set
# and the lib tree, `lib` as the only directory, exactly one
# NATIVE_CONVERSION_PROTOCOL=2, and passing checksums. LOADER_PROOF selects the
# loader check:
#   PATH         LOADER_LIST.txt must have been generated at PATH (the
#                authenticated source bundle; a pinned copy passes its source)
#                and every resolved object must lie inside PATH/lib.
#   --relocated  the byte-identical LOADER_LIST.txt was generated on another
#                host at a path unknown here, so it cannot be path-bound.
#                After the checksums pass, the bundled loader resolves the raw
#                runner on this host and every object must lie in BUNDLE/lib.
# Prints an `error:` diagnostic and returns 1 on rejection.
verify_runner_bundle() {
    local bundle=$1 loader_proof=$2 manifest line path loader_output resolved
    local -a protocol_values=()
    [[ -n "$loader_proof" ]] \
        || { printf 'error: verify_runner_bundle requires a loader proof mode\n' >&2; return 1; }
    [[ -x "$bundle/original_parity_replay" \
        && -x "$bundle/original_parity_replay.remote" \
        && -x "$bundle/lib/ld-linux-x86-64.so.2" \
        && -f "$bundle/SHA256SUMS" \
        && -f "$bundle/LIB_SHA256SUMS" \
        && -f "$bundle/PROVENANCE.txt" \
        && -f "$bundle/LOADER_LIST.txt" ]] \
        || { printf 'error: incomplete runner bundle: %s\n' "$bundle" >&2; return 1; }
    if find "$bundle" -type l -print -quit | grep -q .; then
        printf 'error: runner bundle contains a symlink: %s\n' "$bundle" >&2
        return 1
    fi
    for manifest in "$bundle/SHA256SUMS" "$bundle/LIB_SHA256SUMS"; do
        while IFS= read -r line; do
            [[ "$line" =~ ^[0-9a-fA-F]{64}[[:space:]][\ \*](.+)$ ]] \
                || { printf 'error: malformed bundle checksum entry: %s\n' "$manifest" >&2; return 1; }
            path=${BASH_REMATCH[1]}
            [[ "$path" != /* && "$path" != ../* && "$path" != */../* \
                && "$path" != *'/..' && "$path" != *$'\n'* ]] \
                || { printf 'error: unsafe bundle checksum path: %s\n' "$path" >&2; return 1; }
        done <"$manifest"
    done
    mapfile -t protocol_values < <(
        sed -n 's/^NATIVE_CONVERSION_PROTOCOL=//p' "$bundle/PROVENANCE.txt"
    )
    [[ ${#protocol_values[@]} == 1 && "${protocol_values[0]}" == 2 ]] \
        || { printf 'error: runner bundle does not authenticate native conversion protocol 2: %s\n' \
            "$bundle" >&2; return 1; }
    if ! diff -u -- \
        <(find "$bundle/lib" -type f -printf 'lib/%P\n' | LC_ALL=C sort) \
        <(sed -n 's/^[0-9a-fA-F]\{64\} [ *]//p' "$bundle/LIB_SHA256SUMS" \
            | LC_ALL=C sort) >&2
    then
        printf 'error: library manifest does not exactly cover bundle lib tree: %s\n' \
            "$bundle" >&2
        return 1
    fi
    if ! diff -u -- \
        <(find "$bundle" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' \
            | LC_ALL=C sort) \
        <(printf 'lib\n') >&2
    then
        printf 'error: runner bundle has an unexpected root directory: %s\n' \
            "$bundle" >&2
        return 1
    fi
    if ! diff -u -- \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(sed -n 's/^[0-9a-fA-F]\{64\} [ *]//p' "$bundle/SHA256SUMS" \
            | LC_ALL=C sort) >&2
    then
        printf 'error: main manifest does not exactly cover bundle root files: %s\n' \
            "$bundle" >&2
        return 1
    fi
    if ! diff -u -- \
        <(printf '%s\n' LIB_SHA256SUMS LOADER_LIST.txt PROVENANCE.txt SHA256SUMS \
            original_parity_replay original_parity_replay.remote | LC_ALL=C sort) \
        <(find "$bundle" -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort) >&2
    then
        printf 'error: runner bundle root file set is not canonical: %s\n' \
            "$bundle" >&2
        return 1
    fi
    grep -Eq '^[0-9a-fA-F]{64} [ *]original_parity_replay$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]original_parity_replay\.remote$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]LIB_SHA256SUMS$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]PROVENANCE\.txt$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]LOADER_LIST\.txt$' "$bundle/SHA256SUMS" \
        && grep -Eq '^[0-9a-fA-F]{64} [ *]lib/ld-linux-x86-64\.so\.2$' "$bundle/LIB_SHA256SUMS" \
        || { printf 'error: bundle manifests omit required runtime inputs: %s\n' "$bundle" >&2; return 1; }
    if [[ "$loader_proof" != --relocated ]]; then
        grep -Fq -- "=> $loader_proof/lib/ld-linux-x86-64.so.2 " \
            "$bundle/LOADER_LIST.txt" \
            || { printf 'error: runner loader proof is not bound to authenticated bundle path %s: %s\n' \
                "$loader_proof" "$bundle" >&2; return 1; }
        # Strip only ld.so's trailing load address so bundle paths may contain
        # spaces; any `..` component would escape the authenticated prefix.
        if ! awk -v prefix="$loader_proof/lib/" '
            /=>/ {
                resolved=$0
                sub(/^.*=>[[:space:]]*/, "", resolved)
                sub(/[[:space:]]+\(0x[0-9a-fA-F]+\)[[:space:]]*$/, "", resolved)
                if (index(resolved, prefix) != 1 || resolved ~ /(^|\/)\.\.(\/|$)/) exit 1
            }
        ' "$bundle/LOADER_LIST.txt"; then
            printf 'error: runner loader proof resolves outside authenticated lib tree %s/lib: %s\n' \
                "$loader_proof" "$bundle" >&2
            return 1
        fi
    fi
    (cd -- "$bundle" \
        && sha256sum --strict -c SHA256SUMS \
        && sha256sum --strict -c LIB_SHA256SUMS) >/dev/null \
        || { printf 'error: runner bundle checksum verification failed: %s\n' "$bundle" >&2; return 1; }
    if [[ "$loader_proof" == --relocated ]]; then
        # Executed only after every checksum above authenticated the loader,
        # runner and libraries; authenticated metadata is never regenerated.
        loader_output=$("$bundle/lib/ld-linux-x86-64.so.2" \
            --library-path "$bundle/lib" --list "$bundle/original_parity_replay") \
            || { printf 'error: bundled loader cannot resolve the runner: %s\n' "$bundle" >&2; return 1; }
        while IFS= read -r resolved; do
            [[ -n "$resolved" ]] || continue
            resolved=$(realpath -e -- "$resolved") \
                || { printf 'error: bundled loader resolved a missing object: %s\n' "$bundle" >&2; return 1; }
            [[ "$resolved" == "$bundle/lib/"* ]] \
                || { printf 'error: relocated runner resolves outside bundle lib tree: %s\n' \
                    "$resolved" >&2; return 1; }
        done < <(printf '%s\n' "$loader_output" | sed -n \
            -e 's/.* => \([^ ]*\) .*/\1/p' \
            -e 's/^[[:space:]]*\(\/[^ ]*ld-linux[^ ]*\) .*/\1/p')
    fi
}

# verify_runner_bundle_identity BUNDLE TRUST_SHA [RAW_RUNNER_SHA]
#
# Binds an already structurally verified bundle to the caller's pins: the
# composite trust digest over both manifests and, when the caller holds an
# independent raw-runner pin, the raw runner bytes.
verify_runner_bundle_identity() {
    local bundle=$1 expected_trust=$2 expected_raw=${3:-} actual
    if (( $# == 3 )); then
        actual=$(sha256_file "$bundle/original_parity_replay") \
            || { printf 'error: cannot hash raw runner: %s\n' "$bundle" >&2; return 1; }
        [[ "$actual" == "$expected_raw" ]] \
            || { printf 'error: raw runner hash mismatch: expected %s, got %s\n' \
                "$expected_raw" "$actual" >&2; return 1; }
    fi
    actual=$(runner_bundle_digest "$bundle") \
        || { printf 'error: cannot hash runner bundle manifests: %s\n' "$bundle" >&2; return 1; }
    [[ "$actual" == "$expected_trust" ]] \
        || { printf 'error: runner bundle trust digest mismatch: expected %s, got %s\n' \
            "$expected_trust" "$actual" >&2; return 1; }
}

# attempt_key WORKSPACE PATH: the ledger key of PATH relative to WORKSPACE.
attempt_key() {
    local relative=${2#"$1"/} value
    value=$(printf '%s' "$relative" | sha256sum) || return 1
    printf '%s\n' "${value%% *}"
}

# attempt_read_status STATUS_FILE
#
# Parses the single `STATUS<TAB>ATTEMPT<TAB>LOG` ledger line into
# attempt_prior_status, attempt_prior_number and attempt_prior_log (declare
# them local in the caller). A ledger with any other shape returns 1.
attempt_read_status() {
    local -a lines=()
    mapfile -t lines <"$1" || return 1
    (( ${#lines[@]} == 1 )) || return 1
    IFS=$'\t' read -r attempt_prior_status attempt_prior_number attempt_prior_log \
        <<<"${lines[0]}" || return 1
    [[ -n "$attempt_prior_status" && "$attempt_prior_number" =~ ^[0-9]+$ \
        && -n "$attempt_prior_log" ]]
}

# attempt_begin LOGS_DIR KEY FIRST_ATTEMPT STATUS_FILE
#
# Reserves the first unused `KEY.attempt-NNNN.log` at or after FIRST_ATTEMPT
# (an interrupted run can leave a log whose number its status never recorded;
# no prior log is ever reused or overwritten), creates the `.in-progress` log
# with noclobber and publishes a `running` ledger line. Sets attempt_number,
# attempt_log_name, attempt_log_final and attempt_log_in_progress.
attempt_begin() {
    local logs=$1 key=$2 status_file=$4 label
    attempt_number=$3
    while true; do
        printf -v label '%04d' "$attempt_number"
        attempt_log_name="$key.attempt-$label.log"
        attempt_log_final="$logs/$attempt_log_name"
        attempt_log_in_progress="$attempt_log_final.in-progress"
        [[ -e "$attempt_log_final" || -e "$attempt_log_in_progress" ]] || break
        attempt_number=$((attempt_number + 1))
    done
    (set -o noclobber; : >"$attempt_log_in_progress") 2>/dev/null || return 1
    printf 'running\t%s\t%s\n' "$attempt_number" "$attempt_log_name" \
        | write_atomic "$status_file"
}

# attempt_finish STATUS_FILE RC: publishes the attempt log and its final status.
attempt_finish() {
    mv -- "$attempt_log_in_progress" "$attempt_log_final" || return 1
    printf '%s\t%s\t%s\n' "$2" "$attempt_number" "$attempt_log_name" \
        | write_atomic "$1"
}
