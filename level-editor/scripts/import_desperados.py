"""Build basic, unrefined Desperados scenes in a separate editor library.

Usage: python3 level-editor/scripts/import_desperados.py [--levels 1 2 ...]
Requires Pillow and the installed level-editor Node dependencies.
Only sight volumes and RGB565 backgrounds are imported, not gameplay.
"""

import argparse
import bz2
import copy
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import zipfile

from PIL import Image


EDITOR = Path(__file__).resolve().parents[1]


def sight_chunk(dvd):
    offset = 0
    while offset + 8 <= len(dvd):
        tag, size = struct.unpack_from("<4sI", dvd, offset)
        end = offset + 8 + size
        if end > len(dvd):
            raise ValueError(f"Truncated DVD chunk {tag!r}")
        if tag == b"SGHT":
            return dvd[offset + 8:end]
        offset = end
    raise ValueError("Missing SGHT section")


def read_sight(data):
    version, count = struct.unpack_from("<IH", data)
    if version != 6:
        raise ValueError(f"Unsupported SGHT version {version}")
    offset = 6
    obstacles = []
    for index in range(count):
        n, = struct.unpack_from("<H", data, offset)
        offset += 2
        if not 2 <= n <= 1000:
            raise ValueError(f"Obstacle {index}: invalid vertex count {n}")
        points = []
        for _ in range(n):
            coords = struct.unpack_from("<4f", data, offset)
            offset += 16
            if not all(math.isfinite(v) for v in coords) or coords[2] > coords[3] + 0.01:
                raise ValueError(f"Obstacle {index}: invalid coordinates {coords}")
            points.append(dict(zip(("x", "y", "z_bottom", "z_top"), coords)))
        # Six bounding-box floats, then a conditional projection-area reference.
        bounds = struct.unpack_from("<6f", data, offset)
        offset += 24
        projection = data[offset]
        offset += 1
        if projection not in (0, 1):
            raise ValueError(f"Obstacle {index}: invalid projection flag")
        area = list(struct.unpack_from("<HH", data, offset)) if projection else None
        if projection:
            offset += 4
        flags = data[offset:offset + 4]
        # Legacy serialized booleans include noncanonical nonzero true values.
        if len(flags) != 4:
            raise ValueError(f"Obstacle {index}: invalid flags")
        offset += 4
        # TODO: interpret the remaining two floats, integer and byte before
        # supporting gameplay/material export. Preserve them in the audit.
        extra = data[offset:offset + 13]
        if len(extra) != 13:
            raise ValueError("Truncated sight record")
        offset += 13
        obstacles.append({
            "points": points, "projection_area": area,
            "opaque": bool(flags[0]), "solid": bool(flags[1]),
            "mouse": bool(flags[2]), "show_shadow_polygon": bool(flags[3]),
            "default_material": 0, "material_indices": [],
            "desperados_record": {"bounds": bounds, "flags": list(flags), "tail_hex": extra.hex()},
        })
    if offset != len(data):
        raise ValueError(f"SGHT has {len(data) - offset} unconsumed bytes")
    return obstacles


def read_background(data):
    width, height, encoding, size = struct.unpack_from("<HHII", data)
    if encoding != 2 or size != len(data) - 12:
        raise ValueError("Unsupported DVM header")
    pixels = bz2.decompress(data[12:])
    if len(pixels) != width * height * 2:
        raise ValueError("DVM pixel count does not match dimensions")
    return Image.frombytes("RGB", (width, height), pixels, "raw", "BGR;16")


def geometry_adapter(obstacles):
    # Geometry-only interchange for the existing volume pipeline. Empty arrays
    # explicitly omit unimported systems; this is not a playable level export.
    obstacles = copy.deepcopy(obstacles)
    for index, obstacle in enumerate(obstacles):
        for point in obstacle["points"]:
            delta = point["z_bottom"] - point["z_top"]
            if delta > 0:
                if delta > 0.00001:
                    raise ValueError(f"Obstacle {index}: inverted height by {delta}")
                obstacle["desperados_record"].setdefault("rounded_points", []).append(dict(point))
                point["z_top"] = point["z_bottom"]
                print(f"Obstacle {index}: rounded {delta:g}-unit float height inversion", flush=True)
    return {
        "format": "Demo",
        "misc": {"control_crc": 0, "forest_level": False, "default_material": 0},
        "sight_obstacles": obstacles,
        "motion_data": {"layers": [], "graph_bytes": []},
        **{key: [] for key in (
            "patches", "animations", "material_sectors", "sight_material_indices",
            "light_sectors", "elevation_lines", "masks", "sound_sources",
            "jump_zones", "jump_line_pairs", "lifts", "buildings",
        )},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, default=EDITOR.parent / "datadirs/desperados/unisoed/Game/Data/Levels.pac")
    parser.add_argument("--output", type=Path, default=EDITOR / "work/desperados")
    parser.add_argument("--levels", type=int, nargs="+", default=list(range(1, 26)))
    parser.add_argument("--extract-only", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    library = output / "library"
    datadir = library / "game-data"
    levels = datadir / "Data/Levels"
    (levels / "Day").mkdir(parents=True, exist_ok=True)
    (library / "scenes").mkdir(parents=True, exist_ok=True)
    (library / "3d-assets").mkdir(parents=True, exist_ok=True)
    (library / "3d-assets/index.json").write_text(json.dumps({"version": 1, "assets": []}))
    print("Geometry-only import: gameplay and dynamic patches are not imported.", flush=True)
    with zipfile.ZipFile(args.archive) as archive:
        for number in args.levels:
            if not 1 <= number <= 25:
                raise ValueError(f"Level out of range: {number}")
            source = f"Levels/Level_{number:02}"
            name = f"desperados-{number:02}"
            obstacles = read_sight(sight_chunk(archive.read(source + ".dvd")))
            image = read_background(archive.read(source + ".dvm"))
            (levels / f"{name}.rhp.json").write_text(json.dumps(geometry_adapter(obstacles)))
            image.save(levels / "Day" / f"{name}.map.png")
            image.thumbnail((480, 300))
            image.save(library / "scenes" / f"{name}.png")
            files = sorted(str(p.relative_to(datadir)) for p in levels.rglob("*") if p.is_file())
            temporary_index = datadir / "index.json.tmp"
            temporary_index.write_text(json.dumps({"version": 1, "files": files}))
            temporary_index.replace(datadir / "index.json")
            print(f"{name}: {len(obstacles)} sight volumes, background decoded", flush=True)
            if not args.extract_only:
                subprocess.run([
                    "node", "src/volumes.ts", "--map", name, "--fill", "proc",
                    "--out", str(library / "scenes"),
                ], cwd=EDITOR / "pipeline", env={**os.environ, "HACKABLE_DATADIR": str(datadir)}, check=True)
    print(f"Ready: EDITOR_LIBRARY={library} pnpm --dir {EDITOR} dev --host 127.0.0.1 --port 5193", flush=True)


if __name__ == "__main__":
    main()
