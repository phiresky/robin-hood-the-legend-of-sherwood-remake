#!/usr/bin/env python3
"""Generate the local Ferris overlay datadir from the Blender source file."""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


RENDER_SIZE = 300
PROFILE_NAME = "Ferris"
ACTION_WAITING = "WaitingUpright"
ACTION_WALKING = "WalkingUpright"
WAITING_FRAME_COUNT = 12
WAITING_SOURCE_FRAMES = [1] * WAITING_FRAME_COUNT
WALKING_SOURCE_FRAMES = [1, 5, 9, 13, 17, 21, 25, 29, 33, 37, 41, 45]

# Game direction folders use sector 0 as north/up, 4 as east/right,
# 8 as south/down, and 12 as west/left. Ferris's Blender yaw is mirrored
# on the vertical axis relative to those sectors: east/west match, while
# north/south need swapping. The one-sector phase shift aligns generated
# dir_00 with the pose that previously landed in dir_01.
DIRECTION_ROTATION_OFFSET = 7


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--blend",
        type=Path,
        default=repo_root() / "assets/ferris/ferris3d_v1.0.blend",
        help="Ferris Blender source file.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=repo_root() / "datadirs/ferris_overlay",
        help="Overlay datadir to recreate.",
    )
    parser.add_argument(
        "--base-profile",
        type=Path,
        default=repo_root()
        / "datadirs/demo_leicester_ecoste_hackable/Data/Configuration/profile.cpf.json",
        help="Base profile JSON copied and extended with Ferris.",
    )
    parser.add_argument(
        "--blender",
        default=os.environ.get("BLENDER", "blender"),
        help="Blender executable.",
    )
    parser.add_argument(
        "--worker",
        action="store_true",
        help=argparse.SUPPRESS,
    )
    parser.add_argument(
        "--tmp-dir",
        type=Path,
        default=None,
        help=argparse.SUPPRESS,
    )
    return parser.parse_args(argv)


def main() -> int:
    argv = sys.argv[1:]
    if "--" in argv:
        argv = argv[argv.index("--") + 1 :]
    args = parse_args(argv)
    if args.worker:
        return blender_worker(args)
    return run_blender(args)


def run_blender(args: argparse.Namespace) -> int:
    if not args.blend.exists():
        raise SystemExit(f"Missing Blender source: {args.blend}")
    if not args.base_profile.exists():
        raise SystemExit(f"Missing base profile JSON: {args.base_profile}")

    with tempfile.TemporaryDirectory(prefix="ferris-render-") as tmp:
        cmd = [
            args.blender,
            "--background",
            str(args.blend),
            "--python",
            str(Path(__file__).resolve()),
            "--",
            "--worker",
            "--blend",
            str(args.blend),
            "--output",
            str(args.output),
            "--base-profile",
            str(args.base_profile),
            "--tmp-dir",
            tmp,
        ]
        return subprocess.call(cmd)


def blender_worker(args: argparse.Namespace) -> int:
    import bpy  # type: ignore

    output = args.output.resolve()
    tmp_dir = args.tmp_dir.resolve()
    char_dir = output / "Data/Characters/Ferris.rhs.d"
    profile_path = output / "Data/Configuration/profile.cpf.json"

    if char_dir.exists():
        shutil.rmtree(char_dir)
    char_dir.mkdir(parents=True, exist_ok=True)
    profile_path.parent.mkdir(parents=True, exist_ok=True)

    configure_scene(bpy)
    rows = []
    for direction in range(16):
        rows.append(
            render_action(
                bpy=bpy,
                char_dir=char_dir,
                tmp_dir=tmp_dir,
                action=ACTION_WAITING,
                action_id=3,
                action_done=WAITING_FRAME_COUNT - 1,
                direction=direction,
                source_frames=WAITING_SOURCE_FRAMES,
                delay=4,
                distance=0,
                average_speed=0.0,
            )
        )
    for direction in range(16):
        rows.append(
            render_action(
                bpy=bpy,
                char_dir=char_dir,
                tmp_dir=tmp_dir,
                action=ACTION_WALKING,
                action_id=6,
                action_done=len(WALKING_SOURCE_FRAMES) - 1,
                direction=direction,
                source_frames=WALKING_SOURCE_FRAMES,
                delay=0,
                distance=3,
                average_speed=2.5,
            )
        )

    manifest = {
        "signature": "RHSHACK1",
        "profiles": [
            {
                "name": PROFILE_NAME,
                "width": float(RENDER_SIZE),
                "height": float(RENDER_SIZE),
                "center_x": RENDER_SIZE / 2.0,
                "center_y": RENDER_SIZE / 2.0,
                "rows": rows,
            }
        ],
    }
    (char_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    write_profile(args.base_profile, profile_path)
    print(f"Generated {char_dir}")
    return 0


def configure_scene(bpy) -> None:
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE"
    scene.eevee.taa_render_samples = 64
    scene.render.resolution_x = RENDER_SIZE
    scene.render.resolution_y = RENDER_SIZE
    scene.render.film_transparent = True
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "Medium High Contrast"
    scene.view_settings.exposure = -1.0
    scene.world.color = (0.025, 0.025, 0.025)

    camera = bpy.data.objects.get("FerrisRenderCamera")
    if camera is None:
        raise RuntimeError("Missing FerrisRenderCamera")
    scene.camera = camera

    ferris_collection = bpy.data.collections.get("Ferris")
    ferris_objects = set(ferris_collection.objects) if ferris_collection else set()
    for obj in bpy.data.objects:
        if obj.type == "LIGHT":
            obj.hide_render = obj.name != "FerrisRenderSun"
        elif obj.type not in {"CAMERA"} and obj not in ferris_objects and obj.name != "FerrisShadowKey":
            obj.hide_render = True

    configure_shadow_mesh(bpy)

    sun = bpy.data.objects.get("FerrisRenderSun")
    if sun is not None:
        sun.data.energy = 0.85
        sun.data.angle = 0.45
        sun.rotation_euler = (math.radians(45.0), 0.0, math.radians(35.0))

    for mat in bpy.data.materials:
        if not mat.use_nodes:
            continue
        bsdf = mat.node_tree.nodes.get("Principled BSDF")
        if bsdf is None:
            continue
        set_input(bsdf, "Roughness", 0.92)
        set_input(bsdf, "Specular IOR Level", 0.05)
        set_input(bsdf, "Specular Tint", (0.25, 0.25, 0.25, 1.0))
        set_input(bsdf, "Emission Strength", 0.0)



def configure_shadow_mesh(bpy) -> None:
    shadow = bpy.data.objects.get("FerrisShadowKey")
    if shadow is None:
        return
    shadow.hide_render = False
    shadow.hide_viewport = False
    # Baked character overlays do not go through the legacy character-shadow
    # blit path, so render Ferris's soft contact shadow directly into each
    # generated PNG. The material is transparent black and participates in
    # the normal alpha crop/offset calculation.
    mat = bpy.data.materials.get("FerrisRenderedShadow")
    if mat is None:
        mat = bpy.data.materials.new("FerrisRenderedShadow")
    mat.use_nodes = True
    mat.blend_method = "BLEND"
    mat.show_transparent_back = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    if bsdf is not None:
        set_input(bsdf, "Base Color", (0.0, 0.0, 0.0, 0.32))
        set_input(bsdf, "Alpha", 0.32)
        set_input(bsdf, "Roughness", 1.0)
        set_input(bsdf, "Specular IOR Level", 0.0)
    shadow.data.materials.clear()
    shadow.data.materials.append(mat)


def set_input(node, name: str, value) -> None:
    socket = node.inputs.get(name)
    if socket is not None:
        socket.default_value = value


def render_action(
    *,
    bpy,
    char_dir: Path,
    tmp_dir: Path,
    action: str,
    action_id: int,
    action_done: int,
    direction: int,
    source_frames: list[int],
    delay: int,
    distance: int,
    average_speed: float,
) -> dict:
    path = Path(action) / f"dir_{direction:02d}"
    out_dir = char_dir / path
    out_dir.mkdir(parents=True, exist_ok=True)

    head = bpy.data.objects.get("head")
    if head is None:
        raise RuntimeError("Missing head object")
    render_direction = (DIRECTION_ROTATION_OFFSET - direction) % 16

    frames = []
    for index, source_frame in enumerate(source_frames):
        bpy.context.scene.frame_set(source_frame)
        head.rotation_euler.z = render_direction * math.tau / 16.0
        saved_transforms = save_render_transforms(bpy)
        full_path = tmp_dir / f"{action}_{direction:02d}_{index:02d}_full.png"
        crop_path = out_dir / f"{index:02d}.png"
        try:
            if action == ACTION_WAITING:
                apply_waiting_pose(bpy, index, len(source_frames))
            bpy.context.scene.render.filepath = str(full_path)
            bpy.ops.render.render(write_still=True)
        finally:
            restore_render_transforms(saved_transforms)
        bbox = alpha_bbox(full_path)
        crop_png(full_path, crop_path, bbox)
        frames.append(
            {
                "file": crop_path.name,
                "delay": delay,
                "distance": distance,
                "offset_x": float(bbox[2]),
                "offset_y": float(bbox[3]),
                "sound_id": 0,
            }
        )
        full_path.unlink(missing_ok=True)

    return {
        "action": action,
        "action_id": action_id,
        "action_done": action_done,
        "average_speed": average_speed,
        "direction": direction,
        "hotspot_x": RENDER_SIZE / 2.0,
        "hotspot_y": RENDER_SIZE / 2.0,
        "path": path.as_posix(),
        "frames": frames,
    }


def save_render_transforms(bpy) -> list[tuple[object, object, object, object]]:
    names = ("head", "armL", "armR", "thumbL", "thumbR")
    saved = []
    for name in names:
        obj = bpy.data.objects.get(name)
        if obj is not None:
            saved.append(
                (obj, obj.location.copy(), obj.rotation_euler.copy(), obj.scale.copy())
            )
    return saved


def restore_render_transforms(saved: list[tuple[object, object, object, object]]) -> None:
    for obj, location, rotation, scale in saved:
        obj.location = location
        obj.rotation_euler = rotation
        obj.scale = scale


def apply_waiting_pose(bpy, frame_index: int, frame_count: int) -> None:
    phase = math.tau * frame_index / frame_count
    inhale = (1.0 - math.cos(phase)) * 0.5
    sway = math.sin(phase)

    head = bpy.data.objects.get("head")
    if head is None:
        raise RuntimeError("Missing head object")
    head.location.z += 0.014 * inhale
    head.rotation_euler.x += math.radians(0.45) * sway
    head.scale.x *= 1.0 + 0.008 * inhale
    head.scale.y *= 1.0 + 0.006 * inhale
    head.scale.z *= 1.0 + 0.012 * inhale

    for side, sign in (("L", 1.0), ("R", -1.0)):
        arm = bpy.data.objects.get(f"arm{side}")
        thumb = bpy.data.objects.get(f"thumb{side}")
        if arm is not None:
            arm.location.z += 0.01 * inhale
            arm.rotation_euler.y += sign * math.radians(0.5) * inhale
        if thumb is not None:
            thumb.location.z += 0.012 * inhale
            thumb.rotation_euler.y -= sign * math.radians(0.8) * inhale


def alpha_bbox(path: Path) -> tuple[int, int, int, int]:
    output = subprocess.check_output(
        ["magick", str(path), "-alpha", "extract", "-format", "%@", "info:"],
        text=True,
    ).strip()
    match = re.fullmatch(r"(\d+)x(\d+)\+(-?\d+)\+(-?\d+)", output)
    if not match:
        raise RuntimeError(f"Could not parse alpha bbox for {path}: {output!r}")
    width, height, x, y = (int(value) for value in match.groups())
    return width, height, x, y


def crop_png(src: Path, dst: Path, bbox: tuple[int, int, int, int]) -> None:
    width, height, x, y = bbox
    subprocess.check_call(
        [
            "magick",
            str(src),
            "-crop",
            f"{width}x{height}+{x}+{y}",
            "+repage",
            str(dst),
        ]
    )


def write_profile(base_profile_path: Path, output_path: Path) -> None:
    profile = json.loads(base_profile_path.read_text())
    characters = profile["characters"]
    if not isinstance(characters, dict):
        raise ValueError("base profile must use canonical named JSON; re-export the original CPF")
    characters["Ferris"] = {
        "filename": "Ferris",
        "profile_name": PROFILE_NAME,
        "alternative_profile_name": "",
        "valid_alternative_profile": False,
        "vip": False,
        "shooting": 0,
        "fighting": 1,
        "endurance": 20,
        "exclamation_id": 1129136976,
        "hth_weapon_id": 10,
        "shooting_weapon_id": 0,
        "actions": ["NoAction", "NoAction", "NoAction"],
        "action_max_ammo": [0, 0, 0],
        "contextual_actions": ["NoAction", "NoAction", "NoAction", "NoAction"],
        "pathfinder_index": 0,
        "box_move": {
            "min": {"x": -4.0, "y": -3.0},
            "max": {"x": 4.0, "y": 3.0},
        },
        "center": {"x": 150.0, "y": 150.0},
        "priority": 1,
        "wake_up": 64,
        "detection_speed_in_city": 100,
        "detection_speed_in_forest": 100,
        "weapon_material": "Wood",
        "armor_material": "Leather",
    }
    # Unlisted characters append after the existing numeric slots.
    output_path.write_text(json.dumps(profile, indent=2) + "\n")


if __name__ == "__main__":
    raise SystemExit(main())
