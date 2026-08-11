#!/usr/bin/env python3
"""Report exact-EOF parity and frame progress for a parity sweep.

Usage:
    scripts/report_parity_progress.py output/parity-audits/full-sweep-e1669842d

The audit directory must contain ``traces.snapshot``, ``status/`` and
``logs/`` as produced by ``run_parity_release_sweep.sh``. Passing traces must
have exit status 0 and exactly one anchored exact-EOF marker in their log. For
each failing trace, the script combines the first failure boundary in its log
with the trace header and ``rng_suffix`` terminator to calculate how many
recorded frames matched before the failure.

Reading a trace's terminator requires decompressing it once. Metadata is
cached under the audit directory so subsequent reports are cheap.
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path


STATE_FAILURE_RE = re.compile(r"first parity divergence after frame (\d+)")
RNG_FAILURE_RE = re.compile(r"during original frame (\d+)")
PANIC_FAILURE_RE = re.compile(
    r"^Rust simulation panicked while replaying original frame (\d+) -> (\d+)$",
    re.MULTILINE,
)
CACHE_VERSION = 1
EXACT_EOF_MARKER = "parity trace matched every recorded frame"
INTEGRITY_STATUS = "integrity-eof-marker"


@dataclass(frozen=True)
class TraceEntry:
    source: str
    path: Path
    key: str
    status: str
    log_path: Path


def exact_eof_marker_count(log_path: Path) -> int:
    if not log_path.is_file():
        return 0
    text = log_path.read_text(errors="replace")
    return sum(line == EXACT_EOF_MARKER for line in text.splitlines())


def is_exact_eof(entry: TraceEntry) -> bool:
    return entry.status == "0" and exact_eof_marker_count(entry.log_path) == 1


def is_integrity_failure(entry: TraceEntry) -> bool:
    return entry.status == INTEGRITY_STATUS or (
        entry.status == "0" and exact_eof_marker_count(entry.log_path) != 1
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("audit_dir", type=Path)
    parser.add_argument(
        "--workspace",
        type=Path,
        default=Path.cwd(),
        help="base for relative paths in traces.snapshot (default: cwd)",
    )
    parser.add_argument(
        "--jobs",
        type=int,
        default=min(8, os.cpu_count() or 1),
        help="parallel zstd readers used for uncached failing traces",
    )
    parser.add_argument(
        "--no-cache",
        action="store_true",
        help="do not read or update parity-progress-cache.json",
    )
    parser.add_argument("--json", action="store_true", help="print JSON")
    return parser.parse_args()


def status_key_for_trace(source: str, workspace: Path, known_keys: set[str]) -> str:
    path = Path(source)
    if path.is_absolute():
        try:
            relative = path.relative_to(workspace)
        except ValueError:
            relative = path
    else:
        relative = path

    candidate = relative.as_posix().removeprefix("./").replace("/", "__")
    if candidate in known_keys:
        return candidate

    # Older snapshots sometimes stored an absolute workspace path even though
    # the sweep key was made from its corpus-relative suffix. Resolve that
    # shape without guessing when two status files share the same suffix.
    parts = path.parts
    matches: list[str] = []
    for start in range(len(parts)):
        suffix = "__".join(parts[start:])
        if suffix in known_keys:
            matches.append(suffix)
    if len(matches) == 1:
        return matches[0]
    if not matches:
        raise ValueError(f"no status file corresponds to trace: {source}")
    raise ValueError(f"ambiguous status key for trace {source}: {matches}")


def load_entries(audit_dir: Path, workspace: Path) -> list[TraceEntry]:
    snapshot = audit_dir / "traces.snapshot"
    status_dir = audit_dir / "status"
    logs_dir = audit_dir / "logs"
    if not snapshot.is_file():
        raise ValueError(f"missing trace snapshot: {snapshot}")
    if not status_dir.is_dir() or not logs_dir.is_dir():
        raise ValueError(f"audit must contain status/ and logs/: {audit_dir}")

    status_files = {path.name.removesuffix(".status"): path for path in status_dir.glob("*.status")}
    entries: list[TraceEntry] = []
    for source in snapshot.read_text().splitlines():
        source = source.strip()
        if not source:
            continue
        key = status_key_for_trace(source, workspace, set(status_files))
        status = status_files[key].read_text().strip()
        trace_path = Path(source)
        if not trace_path.is_absolute():
            trace_path = workspace / trace_path
        entries.append(
            TraceEntry(
                source=source,
                path=trace_path,
                key=key,
                status=status,
                log_path=logs_dir / f"{key}.log",
            )
        )
    return entries


def failure_boundary(log_path: Path) -> tuple[str, int] | None:
    if not log_path.is_file():
        return None
    text = log_path.read_text(errors="replace")
    matches = [
        (match.start(), "state", int(match.group(1)))
        for match in STATE_FAILURE_RE.finditer(text)
    ]
    matches.extend(
        (match.start(), "rng", int(match.group(1)))
        for match in RNG_FAILURE_RE.finditer(text)
    )
    for match in PANIC_FAILURE_RE.finditer(text):
        frame_before = int(match.group(1))
        frame_after = int(match.group(2))
        if frame_after == frame_before + 1:
            matches.append((match.start(), "panic", frame_before))
    if not matches:
        return None
    _, kind, frame = min(matches)
    return (kind, frame)


def trace_fingerprint(path: Path) -> dict[str, int]:
    stat = path.stat()
    return {"size": stat.st_size, "mtime_ns": stat.st_mtime_ns}


def read_trace_span(path: Path) -> dict[str, int]:
    header_process = subprocess.Popen(
        ["zstd", "-dc", "--long=31", os.fspath(path)],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    assert header_process.stdout is not None
    first = header_process.stdout.readline()
    header_process.stdout.close()
    header_process.terminate()
    header_process.wait()

    # Let native tools stream the large frame records. Iterating over them in
    # Python creates hundreds of megabytes of short-lived line objects for a
    # single 60-second trace, even though only the terminator is needed.
    zstd_process = subprocess.Popen(
        ["zstd", "-dc", "--long=31", os.fspath(path)],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    assert zstd_process.stdout is not None
    tail_process = subprocess.Popen(
        ["tail", "-n", "1"],
        stdin=zstd_process.stdout,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    zstd_process.stdout.close()
    last, _ = tail_process.communicate()
    zstd_return_code = zstd_process.wait()
    if zstd_return_code != 0 or tail_process.returncode != 0:
        raise RuntimeError(
            f"trace boundary pipeline failed for {path}: "
            f"zstd={zstd_return_code}, tail={tail_process.returncode}"
        )
    try:
        header = json.loads(first)
        terminator = json.loads(last)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"invalid JSON trace boundary in {path}: {error}") from error
    if header.get("type") != "header":
        raise RuntimeError(f"trace does not start with a header: {path}")
    if terminator.get("type") != "rng_suffix":
        raise RuntimeError(f"trace has no rng_suffix terminator: {path}")
    initial_frame = header.get("initial_frame")
    frame_count = terminator.get("frame_count")
    final_frame = terminator.get("final_frame")
    if not all(
        isinstance(value, int) and not isinstance(value, bool) and value >= 0
        for value in (initial_frame, frame_count, final_frame)
    ):
        raise RuntimeError(f"trace has incomplete frame metadata: {path}")
    if final_frame != initial_frame + frame_count:
        raise RuntimeError(
            f"trace frame span is inconsistent in {path}: "
            f"{initial_frame} + {frame_count} != {final_frame}"
        )
    return {
        "initial_frame": initial_frame,
        "frame_count": frame_count,
        "final_frame": final_frame,
    }


def load_cache(path: Path) -> dict[str, dict[str, object]]:
    try:
        data = json.loads(path.read_text())
    except FileNotFoundError:
        return {}
    except json.JSONDecodeError:
        return {}
    if not isinstance(data, dict):
        return {}
    if data.get("version") != CACHE_VERSION or not isinstance(data.get("traces"), dict):
        return {}
    return data["traces"]


def valid_metadata(value: object) -> bool:
    if not isinstance(value, dict):
        return False
    values = [value.get("initial_frame"), value.get("frame_count"), value.get("final_frame")]
    if not all(isinstance(item, int) and not isinstance(item, bool) and item >= 0 for item in values):
        return False
    initial, count, final = values
    return final == initial + count


def save_cache(path: Path, traces: dict[str, dict[str, object]]) -> None:
    lock_path = path.with_suffix(path.suffix + ".lock")
    with lock_path.open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        merged = load_cache(path)
        merged.update(traces)
        with tempfile.NamedTemporaryFile(
            mode="w", dir=path.parent, prefix=f".{path.name}.", delete=False
        ) as temporary:
            json.dump({"version": CACHE_VERSION, "traces": merged}, temporary, indent=1)
            temporary.write("\n")
            temporary_path = Path(temporary.name)
        temporary_path.replace(path)


def progress_for_failure(
    entry: TraceEntry, boundary: tuple[str, int], metadata: dict[str, int]
) -> dict[str, object] | None:
    kind, frame = boundary
    initial = metadata["initial_frame"]
    total = metadata["frame_count"]
    # A state mismatch is reported after executing the divergent frame, so
    # only the preceding frames matched. RNG and wrapped simulation panics are
    # reported at that frame's pre-execution boundary, yielding the count
    # directly.
    reached = frame - initial - (1 if kind == "state" else 0)
    reached = min(total, max(0, reached))
    return {
        "key": entry.key,
        "kind": kind,
        "failure_frame": frame,
        "initial_frame": initial,
        "reached_frames": reached,
        "total_frames": total,
        "progress": reached / total if total else 0.0,
    }


def calculate(args: argparse.Namespace) -> dict[str, object]:
    audit_dir = args.audit_dir.resolve()
    workspace = args.workspace.resolve()
    entries = load_entries(audit_dir, workspace)
    passed = [entry for entry in entries if is_exact_eof(entry)]
    failed = [entry for entry in entries if not is_exact_eof(entry)]
    integrity_failures = [entry for entry in entries if is_integrity_failure(entry)]
    boundaries = {
        entry.key: boundary
        for entry in failed
        if (boundary := failure_boundary(entry.log_path)) is not None
    }

    cache_path = audit_dir / "parity-progress-cache.json"
    cache = {} if args.no_cache else load_cache(cache_path)
    metadata_by_key: dict[str, dict[str, int]] = {}
    missing: list[tuple[TraceEntry, dict[str, int]]] = []
    errors: dict[str, str] = {}
    for entry in failed:
        if not entry.path.is_file():
            errors[entry.key] = f"missing trace: {entry.path}"
            continue
        if entry.key not in boundaries:
            continue
        fingerprint = trace_fingerprint(entry.path)
        cached = cache.get(entry.source)
        if (
            isinstance(cached, dict)
            and cached.get("fingerprint") == fingerprint
            and valid_metadata(cached.get("metadata"))
        ):
            metadata_by_key[entry.key] = cached["metadata"]  # type: ignore[assignment]
        else:
            missing.append((entry, fingerprint))

    if missing:
        unsaved = 0
        with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
            futures = {
                pool.submit(read_trace_span, entry.path): (entry, fingerprint)
                for entry, fingerprint in missing
            }
            for future in as_completed(futures):
                entry, fingerprint = futures[future]
                try:
                    metadata = future.result()
                    current_fingerprint = trace_fingerprint(entry.path)
                except Exception as error:  # Keep the report useful for the remaining corpus.
                    errors[entry.key] = str(error)
                    continue
                if current_fingerprint != fingerprint:
                    errors[entry.key] = f"trace changed while reading: {entry.path}"
                    continue
                metadata_by_key[entry.key] = metadata
                cache[entry.source] = {
                    "fingerprint": fingerprint,
                    "metadata": metadata,
                }
                unsaved += 1
                if not args.no_cache and unsaved >= 16:
                    save_cache(cache_path, cache)
                    unsaved = 0
        if not args.no_cache and unsaved:
            save_cache(cache_path, cache)

    progress_records = [
        record
        for entry in failed
        if (metadata := metadata_by_key.get(entry.key)) is not None
        if (boundary := boundaries.get(entry.key)) is not None
        if (record := progress_for_failure(entry, boundary, metadata)) is not None
    ]
    ratios = [float(record["progress"]) for record in progress_records]
    reached_frames = sum(int(record["reached_frames"]) for record in progress_records)
    total_failure_frames = sum(int(record["total_frames"]) for record in progress_records)
    total = len(entries)
    result: dict[str, object] = {
        "audit": os.fspath(audit_dir),
        "traces": total,
        "exact_eof": len(passed),
        "exact_eof_percent": len(passed) / total * 100.0 if total else 0.0,
        "non_eof": len(failed),
        "integrity_failures": len(integrity_failures),
        "integrity_failure_keys": [entry.key for entry in integrity_failures],
        "non_eof_with_frame_boundary": len(progress_records),
        "non_eof_frame_coverage_percent": (
            len(progress_records) / len(failed) * 100.0 if failed else 100.0
        ),
        "non_eof_average_progress_percent": statistics.fmean(ratios) * 100.0 if ratios else 0.0,
        "non_eof_median_progress_percent": statistics.median(ratios) * 100.0 if ratios else 0.0,
        "non_eof_frame_weighted_progress_percent": (
            reached_frames / total_failure_frames * 100.0 if total_failure_frames else 0.0
        ),
        "matched_failure_frames": reached_frames,
        "recorded_failure_frames": total_failure_frames,
        "metadata_errors": errors,
    }
    return result


def print_human(result: dict[str, object]) -> None:
    print(f"audit: {result['audit']}")
    print(
        "exact EOF: "
        f"{result['exact_eof']}/{result['traces']} "
        f"({result['exact_eof_percent']:.2f}%)"
    )
    print(
        "non-EOF frame progress: "
        f"{result['non_eof_average_progress_percent']:.2f}% average per trace; "
        f"{result['non_eof_frame_weighted_progress_percent']:.2f}% frame-weighted; "
        f"{result['non_eof_median_progress_percent']:.2f}% median"
    )
    print(
        "failure-frame coverage: "
        f"{result['non_eof_with_frame_boundary']}/{result['non_eof']} "
        f"({result['non_eof_frame_coverage_percent']:.2f}%)"
    )
    if result["integrity_failures"]:
        print(f"integrity failures: {result['integrity_failures']}")
    errors = result["metadata_errors"]
    if errors:
        print(f"metadata errors: {len(errors)} (see --json for details)", file=sys.stderr)


def main() -> int:
    args = parse_args()
    if args.jobs < 1:
        print("error: --jobs must be at least 1", file=sys.stderr)
        return 2
    try:
        result = calculate(args)
    except (OSError, ValueError, RuntimeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(result, indent=2))
    else:
        print_human(result)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
