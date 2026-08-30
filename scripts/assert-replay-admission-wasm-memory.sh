#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
    echo "usage: $0 replay_admission_bg.wasm" >&2
    exit 2
fi

wasm="$1"
dump="$(wasm-objdump -x "$wasm")"

# The validator must own one non-shared memory. An imported/shared memory can
# couple worker failure to another instance; no maximum permits growth to the
# wasm32 ceiling. WABT prints the bound in 64 KiB pages.
if printf '%s\n' "$dump" | sed -n '/Import\[/,/Function\[/p' | grep -E 'memory\[' >/dev/null; then
    echo "error: replay validator imports memory" >&2
    exit 1
fi
memory_lines="$(printf '%s\n' "$dump" | sed -n '/Memory\[/,/Global\[/p' | grep -E 'memory\[' || true)"
if [ "$(printf '%s\n' "$memory_lines" | grep -c .)" -ne 1 ]; then
    echo "error: replay validator must declare exactly one memory" >&2
    printf '%s\n' "$memory_lines" >&2
    exit 1
fi
if ! printf '%s\n' "$memory_lines" | grep -E 'max=6144([^0-9]|$)' >/dev/null; then
    echo "error: replay validator memory maximum is not 6144 pages (384 MiB)" >&2
    printf '%s\n' "$memory_lines" >&2
    exit 1
fi
if printf '%s\n' "$memory_lines" | grep -E 'shared' >/dev/null; then
    echo "error: replay validator memory is shared" >&2
    exit 1
fi

