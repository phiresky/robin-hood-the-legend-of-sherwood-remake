#!/usr/bin/env python3
"""Versioned result protocol shared by sweep admission and ledger import.

Results live inside the already sealed log artifact. Legacy marker admission is
explicit and is only for importing/resuming historical evidence, never new runs.
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

PREFIX = "ROBIN_PARITY_RESULT "
LEGACY_EOF_MARKER = "parity trace matched every recorded frame"
SHA256 = re.compile(r"[0-9a-f]{64}\Z")


def read_result(log: str) -> dict | None:
    lines = [line[len(PREFIX):] for line in log.splitlines() if line.startswith(PREFIX)]
    if not lines:
        return None
    if len(lines) != 1:
        raise ValueError("expected exactly one structured parity result")
    result = json.loads(lines[0])
    if not isinstance(result, dict) or type(result.get("result_version")) is not int or result["result_version"] != 1:
        raise ValueError("unsupported parity result version")
    for field in ("expected_frames", "processed_frames", "expected_final_frame", "final_frame", "divergent_frames"):
        if type(result.get(field)) is not int or not 0 <= result[field] < 2**64:
            raise ValueError(f"invalid parity result {field}")
    for field in ("native_trace_sha256", "executable_sha256"):
        if not isinstance(result.get(field), str) or not SHA256.fullmatch(result[field]):
            raise ValueError(f"invalid parity result {field}")
    for field in ("trace_path", "executable_path"):
        if not isinstance(result.get(field), str) or not result[field] or "\n" in result[field]:
            raise ValueError(f"invalid parity result {field}")
    if type(result.get("terminator_validated")) is not bool:
        raise ValueError("invalid terminator validation")
    if result.get("outcome") not in ("exact_eof", "divergence", "incomplete"):
        raise ValueError("invalid parity outcome")
    policy = result.get("capabilities")
    if not isinstance(policy, dict) or type(policy.get("policy_version")) is not int or policy["policy_version"] != 1:
        raise ValueError("unsupported trace capabilities policy")
    for field in ("trace_schema", "native_version"):
        if type(policy.get(field)) is not int or policy[field] < 1:
            raise ValueError(f"invalid capability {field}")
    if not isinstance(policy.get("exceptions"), list):
        raise ValueError("missing projection limitations")
    for exception in policy["exceptions"]:
        if not isinstance(exception, dict) or any(not isinstance(exception.get(key), str) or not exception[key] for key in ("id", "scope", "removal_condition")):
            raise ValueError("invalid projection limitation")
    return result


def exact_eof(log: str, *, allow_legacy: bool = False, trace: Path | None = None) -> bool:
    result = read_result(log)
    if result is None:
        return allow_legacy and log.splitlines().count(LEGACY_EOF_MARKER) == 1
    if trace is not None:
        # resolve() also handles logical JSONL identities whose source was
        # quarantined after conversion. The parent directory must still exist.
        if Path(result["trace_path"]).resolve() != trace.resolve():
            raise ValueError("structured result belongs to a different trace")
    return (result["outcome"] == "exact_eof"
            and result["terminator_validated"]
            and result["expected_frames"] == result["processed_frames"]
            and result["expected_final_frame"] == result["final_frame"]
            and result["divergent_frames"] == 0)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--trace", type=Path)
    parser.add_argument("--allow-legacy", action="store_true")
    args = parser.parse_args()
    try:
        return 0 if exact_eof(args.log.read_text(), allow_legacy=args.allow_legacy, trace=args.trace) else 1
    except (OSError, ValueError) as error:
        parser.exit(1, f"invalid parity evidence: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
