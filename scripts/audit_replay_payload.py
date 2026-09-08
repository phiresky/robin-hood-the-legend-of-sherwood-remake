#!/usr/bin/env python3
"""Measure an existing corpus using mission_sprite_audit's two JSONL streams.

This reports download bytes, not decoded sprite memory or runtime reachability.
Character profile arguments augment the authored mission closure explicitly;
they do not infer a replay's selected team or reinforcement candidates.
"""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path


def audit(data, inventory, roots, mission, characters):
    plan = json.loads((data / "conversion-plan.json").read_text())
    if not plan["completed"]:
        raise ValueError("conversion plan is incomplete")
    parts = {}
    for line in inventory.read_text().splitlines():
        row = json.loads(line)
        if row["file"] in parts:
            raise ValueError(f"duplicate inventory entry: {row['file']}")
        parts[row["file"]] = row
    manifests = [json.loads(line) for line in roots.read_text().splitlines()]
    manifests = [row for row in manifests if "missions" in row]
    if len(manifests) != 1:
        raise ValueError("expected exactly one mission manifest in roots JSONL")
    manifest = manifests[0]
    names = set(manifest["missions"][mission]["files"])
    for character in characters:
        names.update(manifest["characters"][str(character)])
    requirements = defaultdict(set)
    for name, row in plan["rhs"].items():
        requirements[name.lower()].update(row["profiles"])
    categories = Counter()
    digests = defaultdict(list)
    excess_profiles = []
    selected = []
    for name in sorted(names):
        path = data / name
        if not path.resolve().is_relative_to(data.resolve()):
            raise ValueError(f"payload path escapes Data: {name}")
        contents = path.read_bytes()
        row = parts[name]
        if len(contents) != row["bytes"]:
            raise ValueError(f"inventory size differs from actual file: {name}")
        categories[name.split("/")[0]] += len(contents)
        digest = hashlib.sha256(contents).hexdigest()
        digests[digest].append(name)
        selected.append({"file": name, "bytes": len(contents), "sha256": digest})
        for rhs in row["profiles"]:
            key = rhs["rhs"].lower()
            if key not in requirements:
                raise ValueError(f"RHS absent from conversion plan: {rhs['rhs']}")
            required = requirements[key]
            if "" not in required:
                for profile in rhs["profiles"]:
                    if profile["profile"] not in required:
                        excess_profiles.append({"rhs": rhs["rhs"], "profile": profile["profile"]})
    duplicates = [group for group in digests.values() if len(group) > 1]
    duplicate_bytes = sum((len(group) - 1) * parts[group[0]]["bytes"] for group in duplicates)
    object_bytes = sum(row["bytes"] for row in selected if row["file"].startswith((
        "rhs/characters_accessories_", "rhs/characters_bonus_", "rhs/characters_relic_")))
    return {
        "mission": mission,
        "additional_character_profiles": characters,
        "scope": "authored mission plus explicit character RHS dependencies; excludes boot and external audio",
        "files": len(names),
        "bytes": sum(categories.values()),
        "categories": dict(sorted(categories.items())),
        "identical_payload_groups": duplicates,
        "identical_payload_redundant_bytes": duplicate_bytes,
        "profiles_outside_global_conversion_plan": excess_profiles,
        "all_object_master_payload_bytes": object_bytes,
        "inputs": {str(path): hashlib.sha256(path.read_bytes()).hexdigest() for path in
                   (data / "conversion-plan.json", inventory, roots, data / "datadir.bin")},
        "payloads": selected,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("data", type=Path)
    parser.add_argument("inventory", type=Path, help="mission_sprite_audit stdout JSONL")
    parser.add_argument("roots", type=Path, help="mission_sprite_audit stderr JSONL")
    parser.add_argument("mission")
    parser.add_argument("--character", type=int, action="append", default=[])
    args = parser.parse_args()
    print(json.dumps(audit(args.data, args.inventory, args.roots, args.mission,
                           args.character), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
