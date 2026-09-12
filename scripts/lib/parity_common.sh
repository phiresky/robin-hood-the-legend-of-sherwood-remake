#!/usr/bin/env bash
# Shared mechanisms only. Admission, locking and publication policy stay with
# each driver; notably verify_bundle/run_one have different trust contracts.

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
