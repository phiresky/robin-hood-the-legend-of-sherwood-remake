#!/usr/bin/env python3
"""Run explicitly pinned Original traces to EOF with an already built runner.

Usage: python3 scripts/run_parity_fixture_gate.py --runner target/debug/original_parity_replay
  --corpus /path/to/parity-save-replays --datadir /path/to/fullgame_linux
  --output /path/to/new-audit-directory

The manifest records user-provided corpus provenance, byte identities and
expected extents. It is not a signature asserting the original capture host.
The source corpus is read-only; all runner inputs are isolated copies.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
from parity_result import exact_eof, read_result
from validation.runtime_evidence import snapshot_executable


def digest(path: Path) -> str:
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def safe_relative(value: str) -> Path:
    path = Path(value)
    if path.is_absolute() or ".." in path.parts or not path.parts:
        raise ValueError(f"unsafe manifest path: {value}")
    return path


def snapshot_runner(source: Path, target: Path) -> str:
    """Pin one executable across the batch even if Cargo replaces its output."""
    return snapshot_executable(source, target)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runner", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--datadir", type=Path, required=True)
    parser.add_argument("--core-datadir", type=Path,
                        help="CPU core data root (default: this checkout's assets/core-datadir)")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, default=Path(__file__).parent / "parity-campaigns/refactor-eof-gate-20260907.json")
    parser.add_argument("--timeout", type=int, default=900)
    parser.add_argument("--allow-legacy-result", action="store_true",
                        help="baseline comparison only: allow an older runner's exact human EOF marker")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    if args.allow_legacy_result and args.core_datadir is not None:
        parser.error("--core-datadir cannot be passed to an older --allow-legacy-result runner")
    runner = args.runner.resolve(strict=True)
    corpus = args.corpus.resolve(strict=True)
    datadir = args.datadir.resolve(strict=True)
    core_datadir = None if args.allow_legacy_result else (
        args.core_datadir or Path(__file__).resolve().parents[1] / "assets/core-datadir").resolve(strict=True)
    core_input = None if core_datadir is None else dict(
        path=str(core_datadir), audio_durations_sha256=digest(core_datadir / "Data/AudioDurations.json"))

    def check_core_input():
        if core_input is not None and digest(core_datadir / "Data/AudioDurations.json") != core_input["audio_durations_sha256"]:
            raise ValueError("core timing input changed during fixture gate")

    manifest = json.loads(args.manifest.read_text())
    if manifest.get("manifest_version") != 1:
        parser.error("unsupported fixture manifest version")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source_runner = runner
    runner = output / "original_parity_replay"
    runner_sha = snapshot_runner(source_runner, runner)
    records = []
    for entry in manifest["artifacts"]:
        relative = safe_relative(entry["path"])
        source = corpus / relative
        if digest(source) != entry["sha256"]:
            raise ValueError(f"fixture bytes disagree with frozen provenance: {source}")
        target = output / "traces" / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        if digest(target) != entry["sha256"]:
            raise ValueError(f"fixture copy changed bytes: {target}")
    for index, entry in enumerate(manifest["artifacts"]):
        if not entry.get("run"):
            continue
        check_core_input()
        trace = output / "traces" / safe_relative(entry["path"])
        log_path = output / f"{index:02d}.log"
        env = dict(os.environ, ROBINHOOD_DATA_DIR=str(datadir))
        print(f"Replaying {entry['path']} ({entry['frames']} recorded frames)", flush=True)
        with log_path.open("wb") as log:
            try:
                core_args = [] if args.allow_legacy_result else ["--core-datadir", str(core_datadir)]
                status = subprocess.run([str(runner), "--no-auto-dump", *core_args, str(trace)],
                                        env=env, stdout=log, stderr=subprocess.STDOUT,
                                        timeout=args.timeout, check=False).returncode
            except subprocess.TimeoutExpired:
                status = 124
        check_core_input()
        log = log_path.read_text(errors="replace")
        try:
            matched = status == 0 and exact_eof(log, allow_legacy=args.allow_legacy_result, trace=trace)
            result = read_result(log)
            if result is not None:
                matched = matched and result["native_trace_sha256"] == entry["sha256"]
                matched = matched and result["executable_sha256"] == runner_sha
                matched = matched and result["processed_frames"] == entry["frames"]
                matched = matched and result["final_frame"] == entry["final_frame"]
        except ValueError:
            matched, result = False, None
        records.append(dict(trace=entry["path"], status=status, exact_eof=matched,
                            log_sha256=digest(log_path), result=result))
        print(f"  {'exact EOF' if matched else 'FAILED'}; status={status}; log={log_path}", flush=True)
    check_core_input()
    report = dict(gate_version=1, manifest_sha256=digest(args.manifest),
                  runner_sha256=runner_sha, runner=str(runner), source_runner=str(source_runner), datadir=str(datadir),
                  core_datadir=str(core_datadir) if core_datadir is not None else None,
                  core_input=core_input,
                  legacy_result_allowed=args.allow_legacy_result, results=records)
    (output / "gate-result.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if records and all(record["exact_eof"] for record in records) else 1


if __name__ == "__main__":
    raise SystemExit(main())
