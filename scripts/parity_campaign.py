#!/usr/bin/env python3
"""Read a frozen versioned campaign manifest; never silently refresh its hashes."""
from __future__ import annotations
import argparse
import hashlib
import json
import re
import shlex
from pathlib import Path

SCRIPT_KEYS = {
    "expected_prepass_script_sha": "scripts/run_native_conversion_prepass.sh",
    "expected_final_script_sha": "scripts/run_schema16_final_validation.sh",
    "expected_sweep_script_sha": "scripts/run_parity_release_sweep.sh",
    "expected_controller_sha": "scripts/run_schema16_onward_corpus_controller.sh",
    "expected_helper_sha": "scripts/run_schema16_existing_corpora_orchestrator.sh",
    "expected_capture_sha": "original-game/scripts/capture_parity_save_replays.sh",
    "expected_prepass_sha": "scripts/run_native_conversion_prepass.sh",
    "expected_final_sha": "scripts/run_schema16_final_validation.sh",
    "expected_sweep_sha": "scripts/run_parity_release_sweep.sh",
}
DEPENDENCIES = ("parity_campaign.py", "parity_result.py", "lib/parity_common.sh")


def load(path: Path) -> dict:
    value = json.loads(path.read_text())
    if type(value.get("manifest_version")) is not int or value["manifest_version"] != 1:
        raise ValueError("unsupported parity campaign manifest version")
    profiles = value.get("profiles")
    if not isinstance(profiles, dict) or set(profiles) != {"existing_corpora", "onward_handoff"}:
        raise ValueError("campaign manifest must contain both orchestration profiles")
    for profile in profiles.values():
        if not isinstance(profile, dict) or not profile:
            raise ValueError("empty campaign profile")
        for key, digest in profile.items():
            if key not in SCRIPT_KEYS or not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
                raise ValueError(f"invalid campaign script identity: {key}")
    expected = {"existing_corpora": set(list(SCRIPT_KEYS)[:3]),
                "onward_handoff": set(list(SCRIPT_KEYS)[3:])}
    if any(set(profiles[key]) != keys for key, keys in expected.items()):
        raise ValueError("incomplete campaign script identities")
    dependencies = value.get("script_dependencies")
    if dependencies is not None:
        if not isinstance(dependencies, dict) or set(dependencies) != set(DEPENDENCIES):
            raise ValueError("incomplete campaign dependency identities")
        for name, digest in dependencies.items():
            if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
                raise ValueError(f"invalid dependency identity: {name}")
    return value


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--profile", choices=("existing_corpora", "onward_handoff"))
    parser.add_argument("--freeze-workspace", type=Path,
                        help="explicitly output a NEW manifest for this workspace; does not modify the input")
    args = parser.parse_args()
    value = load(args.manifest)
    if args.freeze_workspace:
        selected = [value["profiles"][args.profile]] if args.profile else value["profiles"].values()
        for profile in selected:
            for key in profile:
                profile[key] = hashlib.sha256((args.freeze_workspace / SCRIPT_KEYS[key]).read_bytes()).hexdigest()
        value["description"] = "Explicit workspace script snapshot; preserve with campaign evidence"
        value["script_dependencies"] = {
            name: hashlib.sha256((Path(__file__).parent / name).read_bytes()).hexdigest()
            for name in DEPENDENCIES
        }
        print(json.dumps(value, indent=2))
    elif args.profile:
        for name, expected in value.get("script_dependencies", {}).items():
            actual = hashlib.sha256((Path(__file__).parent / name).read_bytes()).hexdigest()
            if actual != expected:
                parser.error(f"campaign dependency hash mismatch: {name}")
        print("campaign_manifest_sha256=" + hashlib.sha256(args.manifest.read_bytes()).hexdigest())
        for key, digest in value["profiles"][args.profile].items():
            print(f"{key}={shlex.quote(digest)}")
    else:
        parser.error("select --profile or --freeze-workspace")


if __name__ == "__main__":
    main()
