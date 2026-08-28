#!/usr/bin/env python3
"""Import Fabri18's numbered sprite-bank replacements as an additive mod."""

from __future__ import annotations

import argparse
import json
import shutil
import struct
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

from PIL import Image, ImageDraw


@dataclass(frozen=True)
class Unit:
    label: str
    rhs: str
    template: str


@dataclass(frozen=True)
class Archive:
    file: str
    label: str
    slug: str
    units: tuple[str, ...]


UNITS = {
    "Archer": Unit("Archer", "Archer03", "Archer03"),
    "Cavalryman": Unit("Cavalryman", "Knight03", "Knight03"),
    "Crossbowman": Unit("Crossbowman", "Crossbowman03", "Crossbowman03"),
    "Halberdier": Unit("Halberdier", "Guard A03", "Guard A03"),
    "Knight": Unit("Knight", "Soldier B03", "Soldier B03"),
    "Lancer": Unit("Lancer", "Guard B03", "Guard B03"),
    "Officer": Unit("Officer", "Officier B03", "Officier B03"),
    "OfficerCape": Unit("Officer Cape", "Officer03", "Officer03"),
    "Swordsman": Unit("Swordsman", "Soldier A03", "Soldier A03"),
}

ALL_UNITS = tuple(UNITS)
ARCHIVES = (
    Archive("Blu_Version2.zip", "Blue V2", "BlueV2", ALL_UNITS),
    Archive("Dark_blue_sprites.zip", "Dark Blue", "DarkBlue", ALL_UNITS),
    Archive("new_sprite.zip", "New Sprite", "NewSprite", ALL_UNITS),
    Archive("white_sprites.zip", "White", "White", ALL_UNITS),
    Archive("mod_royal_pourple.rar", "Royal Purple", "RoyalPurple", ALL_UNITS),
    Archive("Cavalryman_blu.zip", "Cavalry Blue", "CavalryBlue", ("Cavalryman",)),
    Archive("Cav_blk.rar", "Cavalry Black", "CavalryBlack", ("Cavalryman",)),
    Archive("Cav_gree.rar", "Cavalry Green", "CavalryGreen", ("Cavalryman",)),
    Archive("Off_gree.rar", "Officer Green", "OfficerGreen", ("Officer",)),
    Archive("Officer_cape_blu.zip", "Officer Cape Blue", "OfficerCapeBlue", ("OfficerCape",)),
    Archive(
        "Officer_cape_yellow.zip",
        "Officer Cape Yellow",
        "OfficerCapeYellow",
        ("OfficerCape",),
    ),
)

# Original profile.cpf progression is blue (00), yellow (01), orange (02),
# red (03), black (04), then separate green ally records. Some cavalry and
# cape-officer filenames are duplicated or incorrect in the retail CPF, so
# those tiers use the runtime's explicit `<identifier>__<cpf-index>` form.
STAT_TEMPLATES = {
    "Archer": ("archer00", "archer01", "archer02", "archer03", "archer04", "archer05__17"),
    "Cavalryman": (
        "knight02__52",
        "knight01",
        "knight02__54",
        "knight03",
        "knight02__56",
        "knight02__57",
    ),
    "Crossbowman": (
        "crossbowman00",
        "crossbowman01",
        "crossbowman02",
        "crossbowman03",
        "crossbowman04",
        "crossbowman05__41",
    ),
    "Halberdier": ("guard_a00", "guard_a01", "guard_a02", "guard_a03", "guard_a04", "guard_a05__5"),
    "Knight": ("soldier_b00", "soldier_b01", "soldier_b02", "soldier_b03", "soldier_b04", "soldier_b05__29"),
    "Lancer": ("guard_b00", "guard_b01", "guard_b02", "guard_b03", "guard_b04", "guard_b05__35"),
    "Officer": (
        "officier_b00",
        "officier_b01",
        "officier_b02",
        "officier_b03",
        "officier_b04",
        "officer05__23",
    ),
    "OfficerCape": (
        "officer02__63",
        "officer02__64",
        "officer02__65",
        "officer03",
        "officer04",
        "officer05__23",
    ),
    "Swordsman": (
        "soldier_a00",
        "soldier_a01",
        "soldier_a02",
        "soldier_a03",
        "soldier_a04",
        "soldier_a05__11",
    ),
}

# The two complete blue sets extend the bottom of the retail ladder. Royal
# Purple extends it one tier beyond black; New Sprite and White retain their
# existing red-tier balance because no progression was specified for them.
ARCHIVE_STAT_TIERS = {
    "BlueV2": 0,
    "DarkBlue": 1,
    "NewSprite": 3,
    "White": 3,
    "RoyalPurple": 5,
    "CavalryBlue": 0,
    "CavalryBlack": 4,
    "CavalryGreen": "green",
    "OfficerGreen": "green",
    "OfficerCapeBlue": 0,
    "OfficerCapeYellow": 1,
}


def profile_addition(archive: Archive, unit_key: str) -> dict:
    unit = UNITS[unit_key]
    filename = f"Fabri18 {archive.slug} {unit_key}"
    tier = ARCHIVE_STAT_TIERS[archive.slug]
    templates = STAT_TEMPLATES[unit_key]
    if tier == "green":
        template = templates[5]
        progression_from = None
    elif tier == 5:
        template = templates[4]
        progression_from = templates[3]
    else:
        template = templates[tier]
        progression_from = None
    addition = {
        "template": template,
        "filename": filename,
        "display_name": f"Fabri18 {archive.label} {unit.label}",
        "hostile": False,
    }
    if progression_from is not None:
        addition["progression_from"] = progression_from
    return addition


def all_profile_additions() -> list[dict]:
    return [
        profile_addition(archive, unit_key)
        for archive in ARCHIVES
        for unit_key in archive.units
    ]


def parse_rhs_bank_ids(path: Path) -> tuple[int, list[list[list[int]]]]:
    data = memoryview(path.read_bytes())
    if len(data) < 6:
        raise RuntimeError(f"truncated RHS: {path}")
    signature, num_profiles = struct.unpack_from("<IH", data, 0)
    offset = 6
    profiles: list[list[list[int]]] = []
    for _ in range(num_profiles):
        if offset + 46 > len(data):
            raise RuntimeError(f"truncated profile header: {path}")
        _name, num_rows, _width, _height, _rx, _ry = struct.unpack_from(
            "<32sHHHii", data, offset
        )
        offset += 46
        rows: list[list[int]] = []
        for _ in range(num_rows):
            if offset + 14 > len(data):
                raise RuntimeError(f"truncated row header: {path}")
            num_frames, _done, _hx, _hy, _action = struct.unpack_from(
                "<HHiiH", data, offset
            )
            offset += 14
            ids: list[int] = []
            for _ in range(num_frames):
                if offset + 14 > len(data):
                    raise RuntimeError(f"truncated frame header: {path}")
                bank_id = struct.unpack_from("<I", data, offset)[0]
                offset += 14
                ids.append(bank_id)
            rows.append(ids)
        profiles.append(rows)
    if offset != len(data):
        raise RuntimeError(f"unexpected {len(data) - offset} trailing bytes in {path}")
    return signature, profiles


def load_template(unit: Unit, rhs_root: Path, extracted_root: Path) -> tuple[dict, set[int]]:
    rhs_path = rhs_root / f"{unit.rhs}.rhs"
    manifest_path = extracted_root / f"{unit.rhs}.rhs.d" / "manifest.json"
    signature, bank_profiles = parse_rhs_bank_ids(rhs_path)
    manifest = json.loads(manifest_path.read_text())
    if len(bank_profiles) != len(manifest["profiles"]):
        raise RuntimeError(f"profile count mismatch for {unit.rhs}")
    used: set[int] = set()
    for profile, bank_rows in zip(manifest["profiles"], bank_profiles):
        if len(bank_rows) != len(profile["rows"]):
            raise RuntimeError(f"row count mismatch for {unit.rhs}")
        for row, bank_ids in zip(profile["rows"], bank_rows):
            if len(bank_ids) != len(row["frames"]):
                raise RuntimeError(f"frame count mismatch for {unit.rhs}")
            row["path"] = "frames"
            for frame, bank_id in zip(row["frames"], bank_ids):
                frame["file"] = f"{bank_id}.png"
                used.add(bank_id)
    manifest["signature"] = signature
    return manifest, used


def extract_archive(archive: Path, destination: Path) -> dict[int, Path]:
    subprocess.run(
        ["7z", "x", "-y", f"-o{destination}", str(archive)],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    images: dict[int, Path] = {}
    for path in destination.rglob("*.png"):
        if not path.stem.isdecimal():
            continue
        bank_id = int(path.stem)
        if bank_id in images:
            raise RuntimeError(f"duplicate bank ID {bank_id} in {archive}")
        images[bank_id] = path
    return images


def write_variant(
    destination: Path,
    filename: str,
    label: str,
    manifest: dict,
    bank_ids: set[int],
    archive_images: dict[int, Path],
) -> Path:
    missing = sorted(bank_ids - archive_images.keys())
    if missing:
        raise RuntimeError(
            f"{label}: archive lacks {len(missing)} RHS frames; first missing ID {missing[0]}"
        )
    rhs_dir = destination / "Data/Characters" / f"{filename}.rhs.d"
    frames_dir = rhs_dir / "frames"
    frames_dir.mkdir(parents=True)
    for index, bank_id in enumerate(sorted(bank_ids), 1):
        shutil.copyfile(archive_images[bank_id], frames_dir / f"{bank_id}.png")
        if index % 1000 == 0:
            print(f"  {label}: {index}/{len(bank_ids)} frames")
    manifest = json.loads(json.dumps(manifest))
    manifest["pixel_format"] = "legacy_color_keys"
    manifest["fabri18_variant"] = label
    (rhs_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return frames_dir / f"{min(bank_ids)}.png"


def legacy_key_preview(path: Path) -> Image.Image:
    with Image.open(path) as source:
        sprite = source.convert("RGBA")
    pixels = []
    for r, g, b, alpha in sprite.get_flattened_data():
        rgb565 = ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)
        if rgb565 == 0x07C0:
            pixels.append((0, 0, 0, 0))
        elif rgb565 == 0x001F:
            pixels.append((0, 0, 0, 150))
        else:
            pixels.append((r, g, b, alpha))
    sprite.putdata(pixels)
    return sprite


def write_preview(destination: Path, preview_frames: list[tuple[str, Path]]) -> None:
    cell_w, cell_h = 180, 150
    sheet = Image.new("RGB", (cell_w * 9, cell_h * 6), (28, 31, 26))
    draw = ImageDraw.Draw(sheet)
    for index, (label, path) in enumerate(preview_frames):
        sprite = legacy_key_preview(path)
        sprite.thumbnail((150, 110), Image.Resampling.LANCZOS)
        x = (index % 9) * cell_w
        y = (index // 9) * cell_h
        sheet.paste(sprite, (x + (cell_w - sprite.width) // 2, y + 4), sprite)
        draw.text((x + 5, y + 120), label[:26], fill=(235, 229, 197))
    sheet.save(destination / "preview.png", optimize=True)


def write_gallery(destination: Path, additions: list[dict], preview_frames: list[tuple[str, Path]]) -> None:
    config = destination / "Data/Configuration"
    levels = destination / "Data/Levels"
    config.mkdir(parents=True)
    levels.mkdir(parents=True)
    (destination / "Data/Characters/mission-scoped.json").write_text("{}\n")
    (config / "soldier-profiles.patch.json").write_text(
        json.dumps({"soldiers": additions}, indent=2) + "\n"
    )

    soldiers = []
    for index, addition in enumerate(additions):
        column, row = index % 9, index // 9
        soldiers.append(
            {
                # Keep the grid centered after the original 40% reduction and
                # an additional 20% reduction (220 * 0.6 * 0.8 = 105.6 pixels).
                "position": [
                    round(1180 + (column - 4) * 105.6),
                    round(890 + (row - 2.5) * 105.6),
                ],
                "profile": addition["filename"].lower().replace(" ", "_"),
                "allegiance": 0,
                "direction": (index * 3) % 16,
            }
        )
    enemy_roles = ["officier_b", "archer", "crossbowman", "guard_a", "guard_b", "soldier_a", "soldier_b"]
    enemy_offsets = [(-70, -70), (0, -70), (70, -70), (-70, 0), (0, 0), (70, 0), (0, 70)]
    for squad, grade in enumerate(range(6)):
        center_x = 240 + squad * 390
        for member, (role, offset) in enumerate(zip(enemy_roles, enemy_offsets)):
            if grade == 5:
                # Original levels store CPF indices, not filenames. Grade 05
                # contains friendly/hostile filename pairs, so select the
                # hostile records explicitly. There is no Officier B05 RHS;
                # the grade's hostile cape officer is CPF record 48.
                profile = (
                    "officer05__48"
                    if member == 0
                    else {
                        "archer": "archer05__47",
                        "crossbowman": "crossbowman05__51",
                        "guard_a": "guard_a05__45",
                        "guard_b": "guard_b05__50",
                        "soldier_a": "soldier_a05__46",
                        "soldier_b": "soldier_b05__49",
                    }[role]
                )
            else:
                profile = f"{role}{grade:02d}"
            soldiers.append(
                {
                    "position": [center_x + offset[0], 2140 + offset[1]],
                    "profile": profile,
                    "allegiance": 1,
                    "direction": (8 + member * 2) % 16,
                }
            )
    level = {
        "title": "Fabri18 Complete Sprite Gallery",
        "map_filename": "OpenBattlefield",
        "reveal_all": True,
        "spawn": [1180, 890],
        "spawn_player": True,
        "walkable_polygon": [[80, 80], [2428, 80], [2428, 2428], [80, 2428]],
        "volumes": [],
        "soldiers": soldiers,
        "pcs": [],
    }
    (levels / "Fabri18SpriteGallery.level.json").write_text(json.dumps(level, indent=2) + "\n")
    write_preview(destination, preview_frames)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--archives", type=Path, default=Path("tmp/sprites-by-fabri18"))
    parser.add_argument(
        "--temp-root",
        type=Path,
        default=Path("tmp"),
        help="directory for archive extraction (never uses system /tmp)",
    )
    parser.add_argument("--rhs-root", type=Path, default=Path("datadirs/fullgame_gog/DATA/Characters"))
    parser.add_argument(
        "--extracted-root",
        type=Path,
        default=Path("datadirs/fullgame_gog_hackable/Data/Characters"),
    )
    parser.add_argument("--output", type=Path, default=Path("mods/fabri18-sprite-gallery"))
    parser.add_argument("--preview-only", action="store_true")
    parser.add_argument(
        "--profiles-only",
        action="store_true",
        help="refresh only the generated soldier profile patch",
    )
    args = parser.parse_args()
    if args.profiles_only:
        patch = args.output / "Data/Configuration/soldier-profiles.patch.json"
        if not patch.is_file():
            raise RuntimeError(f"missing generated profile patch: {patch}")
        patch.write_text(json.dumps({"soldiers": all_profile_additions()}, indent=2) + "\n")
        print(f"Refreshed profile stats for {len(all_profile_additions())} profiles")
        return 0
    if args.preview_only:
        additions = json.loads(
            (args.output / "Data/Configuration/soldier-profiles.patch.json").read_text()
        )["soldiers"]
        previews = []
        for addition in additions:
            rhs = args.output / "Data/Characters" / f"{addition['filename']}.rhs.d"
            manifest = json.loads((rhs / "manifest.json").read_text())
            row = manifest["profiles"][0]["rows"][0]
            previews.append(
                (
                    addition["display_name"].removeprefix("Fabri18 "),
                    rhs / row["path"] / row["frames"][0]["file"],
                )
            )
        write_preview(args.output, previews)
        return 0
    if args.output.exists():
        raise SystemExit(f"output already exists: {args.output}")
    args.output.mkdir(parents=True)

    templates = {
        key: load_template(unit, args.rhs_root, args.extracted_root)
        for key, unit in UNITS.items()
    }
    additions: list[dict] = []
    previews: list[tuple[str, Path]] = []
    args.temp_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="fabri18-import-", dir=args.temp_root) as temp:
        temp_root = Path(temp)
        for archive_index, archive in enumerate(ARCHIVES):
            archive_path = args.archives / archive.file
            if not archive_path.is_file():
                raise RuntimeError(f"missing archive: {archive_path}")
            extract_dir = temp_root / str(archive_index)
            print(f"Extracting {archive.file}")
            archive_images = extract_archive(archive_path, extract_dir)
            for unit_key in archive.units:
                unit = UNITS[unit_key]
                filename = f"Fabri18 {archive.slug} {unit_key}"
                label = f"{archive.label} {unit.label}"
                manifest, bank_ids = templates[unit_key]
                preview = write_variant(
                    args.output,
                    filename,
                    label,
                    manifest,
                    bank_ids,
                    archive_images,
                )
                additions.append(profile_addition(archive, unit_key))
                previews.append((label, preview))
    write_gallery(args.output, additions, previews)
    print(f"Imported {len(additions)} new soldier profiles into {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
