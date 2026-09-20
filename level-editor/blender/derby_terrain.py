"""Reconstruct Derby's rock banks without moving measured floor supports.

The level records courtyard floors and structural support planes, but contains
no surveyed cliff heights. Exterior bank depth is therefore an explicitly
authored reconstruction, not a recovered height map. All displacement follows
the source camera rays, retaining the cleaned ground texture registration.
"""
import importlib.util
import json
import math
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix

ROOT = Path(__file__).resolve().parents[1]
RECIPE = "derby-supported-rock-banks-v1"
WIDTH, HEIGHT = 1920, 2752
SIN, COS = math.sin(math.radians(35)), math.cos(math.radians(35))


def polygon_distance(x, y, polygon):
    """Vectorized signed containment and Euclidean distance to a polygon."""
    inside = np.zeros(x.shape, dtype=bool)
    distance = np.full(x.shape, np.inf)
    for a, b in zip(polygon, polygon[1:] + polygon[:1]):
        ax, ay = a
        bx, by = b
        dx, dy = bx - ax, by - ay
        if dx == 0 and dy == 0:
            continue
        t = np.clip(((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy), 0, 1)
        distance = np.minimum(distance, np.hypot(x - ax - t * dx, y - ay - t * dy))
        if dy != 0:
            inside ^= ((ay > y) != (by > y)) & (x < ax + (y - ay) * dx / dy)
    return inside, distance


def support_polygons(level):
    polygons = [entry["polygon"]["points"] for entry in level["motion_data"]["layers"][0]]
    polygons += [[[p["x"], p["y"]] for p in obstacle["points"]]
                 for obstacle in level["sight_obstacles"]
                 if min(p["z_bottom"] for p in obstacle["points"]) <= .01]
    return [p for p in polygons if len(p) >= 3]


def terrain_height(x, y, level, background=True, margin=16):
    distance = np.full(x.shape, np.inf)
    supported = np.zeros(x.shape, dtype=bool)
    for polygon in support_polygons(level):
        inside, edge = polygon_distance(x, y, polygon)
        supported |= inside | (edge <= margin)
        distance = np.minimum(distance, np.where(inside, 0, edge))
    # Rock banks descend from the level's measured z=0 support plateaus. The
    # transition has zero slope at its rim; bounded depths avoid vertical folds.
    slope = np.clip((distance - margin) / 320, 0, 1)
    slope = slope * slope * (3 - 2 * slope)
    foreground = np.clip((y - 850) / 350, 0, 1)
    foreground = foreground * foreground * (3 - 2 * foreground)
    height = -150 * slope * foreground
    # Broad rock shelves follow the two visible outer cliff banks; no image
    # brightness displacement is used (painted shadows are not elevation).
    if background:
        path = Path(__file__).with_name("derby_background.py")
        spec = importlib.util.spec_from_file_location("derby_background", path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        distant = np.array([module.height_at(float(a), float(b))
                            for a, b in zip(x.flat, y.flat)]).reshape(x.shape)
        height = np.minimum(height, distant)
    height[supported] = 0
    return height, supported


def refine(level_path=None, spacing=12, background=True):
    """Replace the ground plane once; rebuild our own result on repeated calls."""
    if not 4 <= spacing <= 32:
        raise ValueError("Terrain spacing must be between 4 and 32 source pixels")
    level_path = Path(level_path or ROOT.parent / "datadirs/fullgame_gog_hackable/Data/Levels/Derby.rhp.json")
    level = json.loads(level_path.read_text())
    working = bpy.data.collections["Derby Working"]
    candidates = [o for o in working.objects if o.type == "MESH" and
                  o.get("source_node") == "ground" and not o.hide_render]
    if len(candidates) != 1:
        raise ValueError(f"Expected one visible ground mesh, got {len(candidates)}")
    old = candidates[0]
    if not old.data.materials or not old.data.uv_layers.active:
        raise ValueError("The cleaned ground material and UVs are required")
    xs = np.linspace(0, WIDTH, math.ceil(WIDTH / spacing) + 1)
    ys = np.linspace(0, HEIGHT, math.ceil(HEIGHT / spacing) + 1)
    x, y = np.meshgrid(xs, ys)
    z, supported = terrain_height(x, y, level, background=background, margin=spacing * 2)
    ray_depth_steps = np.diff(y + z, axis=0)
    if np.min(ray_depth_steps) <= 0:
        raise ValueError("Terrain folds in depth; broaden the authored bank transition")
    vertices = np.column_stack((x.ravel(), -(y + z).ravel() / SIN, z.ravel() / COS))
    nx, ny = len(xs), len(ys)
    faces = []
    for row in range(ny - 1):
        for col in range(nx - 1):
            a = row * nx + col
            # Clockwise in image coordinates is upward in the world frame.
            faces.extend(((a, a + nx, a + nx + 1), (a, a + nx + 1, a + 1)))
    mesh = bpy.data.meshes.new("Derby terrain / supported cliff relief")
    mesh.from_pydata(vertices.tolist(), [], faces)
    mesh.materials.clear()
    for material in old.data.materials:
        mesh.materials.append(material)
    uv = mesh.uv_layers.new(name="SourceCamera")
    projected = np.column_stack((x.ravel() / WIDTH, 1 - y.ravel() / HEIGHT))
    for polygon in mesh.polygons:
        polygon.use_smooth = True
        for li in polygon.loop_indices:
            uv.data[li].uv = projected[mesh.loops[li].vertex_index]
    mesh.update()
    obj = bpy.data.objects.new("Derby Terrain / rock banks and distant valley", mesh)
    working.objects.link(obj)
    obj.parent = old.parent
    bpy.context.view_layer.update()
    obj.matrix_world = Matrix.Identity(4)
    for key, value in old.items():
        if not key.startswith("reprojection_"):
            obj[key] = value
    obj["source_node"] = "ground"
    obj["source_obstacle"] = "ground"
    obj["refinement_recipe"] = RECIPE
    obj["terrain_height_evidence"] = "Courtyard supports fixed at level z=0; unsurveyed exterior bank/valley depths are authored approximations"
    obj["terrain_projection"] = "Exact 35-degree source-camera ray displacement; existing cleaned ground texture retained"
    obj["terrain_grid_spacing"] = spacing
    obj["open_surface"] = True
    obj["reprojection_ground_preserved"] = True
    if old.get("refinement_recipe") == RECIPE:
        previous_mesh = old.data
        bpy.data.objects.remove(old, do_unlink=True)
        if previous_mesh.users == 0:
            bpy.data.meshes.remove(previous_mesh)
    else:
        old.hide_render = True
        old.hide_set(True)
        old["replaced_by"] = RECIPE
    measured_error = float(np.max(np.abs(z[supported])))
    projection_error = float(np.max(np.abs(-vertices[:, 1] * SIN - vertices[:, 2] * COS - y.ravel())))
    if measured_error > 1e-8 or projection_error > 1e-8:
        raise AssertionError("Terrain violated support or projection constraints")
    stored_vertices = np.array([v.co[:] for v in mesh.vertices])
    stored_projection_error = float(np.max(np.abs(-stored_vertices[:, 1] * SIN - stored_vertices[:, 2] * COS - y.ravel())))
    if stored_projection_error > .001 or any(p.area <= 0 for p in mesh.polygons):
        raise AssertionError("Stored terrain failed precision or triangle-area validation")
    return {"object": obj.name, "vertices": len(vertices), "triangles": len(faces),
            "supported_vertices": int(supported.sum()), "floor_height_error": measured_error,
            "source_projection_error_pixels": projection_error,
            "stored_mesh_projection_error_pixels": stored_projection_error,
            "minimum_depth_grid_step": float(np.min(ray_depth_steps)),
            "authored_game_height_range": [float(z.min()), float(z.max())],
            "ground_texture": "Existing obstacle-cleaned texture; no building pixels repainted",
            "remaining": "Cliff depths are inferred; fine rock facets and under-structure texture ownership still need visual review"}


if __name__ == "__main__":
    result = refine()
