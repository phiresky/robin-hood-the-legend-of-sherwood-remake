"""Author standard profile JSON Patches against a canonical profile.cpf.json document."""

from __future__ import annotations

import json
from pathlib import Path


CAPACITIES = ("intelligence", "courage", "initiative", "pride", "shooting", "fighting", "endurance")
ARCHETYPE = ("rank", "rider", "heavy", "pathfinder_index", "hth_weapon_id", "shooting_weapon_id")


def identifier(filename: str) -> str:
    result = []
    separator = False
    for character in filename.lower():
        if character.isascii() and character.isalnum():
            if separator and result:
                result.append("_")
            result.append(character)
            separator = False
        else:
            separator = True
    return "".join(result)


def pointer(key: str) -> str:
    return key.replace("~", "~0").replace("/", "~1")


def load_catalog(path: Path) -> dict:
    catalog = json.loads(path.read_text())
    if not isinstance(catalog.get("soldiers"), dict):
        raise ValueError(f"{path}: expected a canonical profile.cpf.json catalog; regenerate the hackable datadir or re-export the original CPF with cpf_to_json")
    return catalog


def resolve_soldier(catalog: dict, reference: str) -> tuple[str, dict]:
    soldiers = catalog["soldiers"]
    if reference in soldiers:
        return reference, soldiers[reference]
    matches = []
    for key, profile in soldiers.items():
        name = identifier(profile["filename"])
        if key != profile["filename"]:
            name += "__" + key.rsplit("#", 1)[1]
        if name == reference:
            matches.append((key, profile))
    if len(matches) != 1:
        raise ValueError(f"unknown or ambiguous soldier reference: {reference!r}")
    return matches[0]


def soldier_copy_patch(
    catalog: dict,
    template: str,
    filename: str,
    display_name: str,
    *,
    hostile: bool | None = None,
    profile_name: str | None = None,
    progression_from: str | None = None,
) -> list[dict]:
    """Compile authoring choices and optional arithmetic to RFC 6902 operations.

    Arithmetic runs here at generation time, never in the mod loader. Tests
    pin every source stat used by a formula so a different CPF cannot silently
    give the authored values a different meaning.
    """
    if filename in catalog["soldiers"]:
        raise ValueError(f"new soldier already exists: {filename!r}")
    key, source = resolve_soldier(catalog, template)
    source_path = "/soldiers/" + pointer(key)
    target_path = "/soldiers/" + pointer(filename)
    operations = [{"op": "copy", "from": source_path, "path": target_path}]
    overrides = {"filename": filename, "display_name": display_name}
    if hostile is not None:
        overrides["hostile"] = hostile
    if profile_name is not None:
        overrides["profile_name"] = profile_name
    if progression_from is not None:
        previous_key, previous = resolve_soldier(catalog, progression_from)
        if any(source[field] != previous[field] for field in ARCHETYPE):
            raise ValueError(f"{template!r} and {progression_from!r} are different soldier archetypes")
        for field in ("life_point", *CAPACITIES):
            maximum = 65535 if field == "life_point" else 100
            overrides[field] = min(maximum, max(0, 2 * source[field] - previous[field]))
        for source_key, profile in [(key, source), (previous_key, previous)]:
            for field in ("life_point", *CAPACITIES, *ARCHETYPE):
                operations.append({
                    "op": "test", "path": f"/soldiers/{pointer(source_key)}/{field}",
                    "value": profile[field],
                })
    operations.extend(
        {"op": "replace", "path": f"{target_path}/{field}", "value": value}
        for field, value in overrides.items()
    )
    return operations
