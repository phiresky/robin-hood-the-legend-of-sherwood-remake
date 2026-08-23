#!/usr/bin/env bash
# Emit a sorted, deduplicated replay manifest using completion markers as the
# sole authority. Paths retain the corpus prefix exactly as supplied.
set -euo pipefail

usage() {
    printf 'usage: %s CORPUS_DIR [CORPUS_DIR ...]\n' "$0" >&2
    printf '       %s --self-test\n' "$0" >&2
}

list_complete_traces() {
    if (( $# == 0 )); then
        usage
        return 2
    fi

    local corpus marker marker_dir marker_name replay_stem trace
    local -a markers=() traces=() matches=() missing=()

    for corpus in "$@"; do
        corpus=${corpus%/}
        if [[ ! -d "$corpus" ]]; then
            printf 'error: replay corpus is not a directory: %s\n' "$corpus" >&2
            return 2
        fi
        while IFS= read -r -d '' marker; do
            markers+=("$marker")
        done < <(find "$corpus" -type f -name '*.complete' -print0)
    done

    for marker in "${markers[@]}"; do
        marker_dir=${marker%/*}
        marker_name=${marker##*/}
        replay_stem=${marker_name%.complete}
        matches=()

        # Current corpora use one marker for every completed replay and one or
        # more session traces beneath it. Accept the pre-session spelling too,
        # but never admit any compressed trace merely because it exists.
        # A trace's logical identity is its .jsonl.zst path even after the
        # recording was converted to the native .parity.bitcode.zst artifact.
        if [[ -f "$marker_dir/$replay_stem.jsonl.zst" \
            || -f "$marker_dir/$replay_stem.jsonl.zst.parity.bitcode.zst" ]]; then
            matches+=("$marker_dir/$replay_stem.jsonl.zst")
        fi
        while IFS= read -r -d '' trace; do
            matches+=("$trace")
        done < <(
            find "$marker_dir" -maxdepth 1 -type f \
                \( -name "$replay_stem-session-*.jsonl.zst" \
                -o -name "$replay_stem-session-*.jsonl.zst.parity.bitcode.zst" \) \
                -print0 | sed -z 's/\.parity\.bitcode\.zst$//' | sort -zu
        )

        if (( ${#matches[@]} == 0 )); then
            missing+=("$marker")
        else
            traces+=("${matches[@]}")
        fi
    done

    if (( ${#missing[@]} != 0 )); then
        printf 'error: %d completion marker(s) have no matching trace:\n' \
            "${#missing[@]}" >&2
        printf '  %s\n' "${missing[@]}" | LC_ALL=C sort >&2
        return 1
    fi

    if (( ${#traces[@]} != 0 )); then
        printf '%s\n' "${traces[@]}" | LC_ALL=C sort -u
    fi
}

self_test() {
    mkdir -p .codex-tmp
    local test_root
    test_root=$(mktemp -d .codex-tmp/complete-trace-list-test.XXXXXX)
    trap 'rm -rf -- "$test_root"' RETURN

    local corpus="$test_root/corpus"
    mkdir -p "$corpus/save-a" "$corpus/save-b"
    touch "$corpus/save-a/replay-001.complete"
    touch "$corpus/save-a/replay-001-session-0001.jsonl.zst"
    touch "$corpus/save-a/replay-002.complete"
    touch "$corpus/save-a/replay-002-session-0001.jsonl.zst"
    touch "$corpus/save-a/replay-002-session-0002.jsonl.zst"
    touch "$corpus/save-b/replay-003-session-0001.jsonl.zst"
    # A converted replay: completion marker plus native artifact, no JSONL.
    touch "$corpus/save-a/replay-005.complete"
    touch "$corpus/save-a/replay-005-session-0001.jsonl.zst.parity.bitcode.zst"

    local actual expected
    actual=$(list_complete_traces "$corpus" "$corpus")
    expected=$(printf '%s\n' \
        "$corpus/save-a/replay-001-session-0001.jsonl.zst" \
        "$corpus/save-a/replay-002-session-0001.jsonl.zst" \
        "$corpus/save-a/replay-002-session-0002.jsonl.zst" \
        "$corpus/save-a/replay-005-session-0001.jsonl.zst")
    if [[ "$actual" != "$expected" ]]; then
        printf 'self-test failure: manifest mismatch\nexpected:\n%s\nactual:\n%s\n' \
            "$expected" "$actual" >&2
        return 1
    fi

    touch "$corpus/save-b/replay-004.complete"
    if list_complete_traces "$corpus" >"$test_root/unexpected-output" 2>"$test_root/error"; then
        printf 'self-test failure: orphan completion marker was accepted\n' >&2
        return 1
    fi
    if [[ -s "$test_root/unexpected-output" ]]; then
        printf 'self-test failure: invalid corpus emitted a partial manifest\n' >&2
        return 1
    fi
    if ! rg -q 'replay-004\.complete' "$test_root/error"; then
        printf 'self-test failure: orphan marker error omitted its path\n' >&2
        return 1
    fi

    printf 'complete-trace manifest self-test passed\n'
}

if [[ ${1:-} == --self-test ]]; then
    if (( $# != 1 )); then
        usage
        exit 2
    fi
    self_test
else
    list_complete_traces "$@"
fi
