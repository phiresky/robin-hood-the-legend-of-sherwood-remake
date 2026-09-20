"""Deterministic world-space diffuse lighting for source-only review packets.

The reference artwork lights the gate from upper left. Direction is toward the
sun in map world coordinates, not camera coordinates. Heights and light strength
are artistic estimates: the original artwork does not encode a recoverable sun.
"""
from array import array
from pathlib import Path

import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

DEFAULT_LIGHTING = {
    "toward_sun": [-0.65, -0.60, 0.80],
    "ambient": 0.16,
    "diffuse": 0.64,
    "shadow_epsilon": 0.03,
}


def configuration(settings=None):
    result = {**DEFAULT_LIGHTING, **(settings or {})}
    if set(result) != set(DEFAULT_LIGHTING):
        raise ValueError("Unknown review lighting option")
    direction = Vector(result["toward_sun"])
    if direction.length == 0 or result["ambient"] < 0 or result["diffuse"] < 0 or result["shadow_epsilon"] <= 0:
        raise ValueError("Invalid review sunlight configuration")
    result["toward_sun"] = list(direction.normalized())
    return result


def irradiance(normal, toward_sun, ambient, diffuse, blocked=False):
    """Linear neutral reflectance; intentionally independent of camera direction."""
    return ambient + (0 if blocked else diffuse * max(0.0, normal.dot(toward_sun)))


def _surface(objects):
    vertices, triangles, normals = [], [], []
    depsgraph = bpy.context.evaluated_depsgraph_get()
    for obj in objects:
        evaluated = obj.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        try:
            mesh.calc_loop_triangles()
            offset = len(vertices)
            vertices.extend(obj.matrix_world @ vertex.co for vertex in mesh.vertices)
            transform = obj.matrix_world.to_3x3().inverted().transposed()
            for triangle in mesh.loop_triangles:
                triangles.append(tuple(offset + index for index in triangle.vertices))
                normals.append(tuple((transform @ mesh.corner_normals[index].vector).normalized()
                                     for index in triangle.loops))
        finally:
            evaluated.to_mesh_clear()
    if not triangles:
        raise ValueError("No review geometry")
    return BVHTree.FromPolygons(vertices, triangles, all_triangles=True), vertices, triangles, normals


def _interpolated_normal(point, triangle, vertices, normals):
    a, b, c = (vertices[index] for index in triangle)
    ab, ac, ap = b-a, c-a, point-a
    aa, bb, cc = ab.dot(ab), ab.dot(ac), ac.dot(ac)
    determinant = aa*cc-bb*bb
    if abs(determinant) < 1e-20:
        return normals[0]
    u = (cc*ap.dot(ab)-bb*ap.dot(ac))/determinant
    v = (aa*ap.dot(ac)-bb*ap.dot(ab))/determinant
    return (normals[0]*(1-u-v)+normals[1]*u+normals[2]*v).normalized()


def render_solids(scene, cameras, objects, output, *, lighting=None):
    """Trace actual evaluated surfaces and their cast shadows without scene edits.

    Uses existing flat/custom/smooth corner normals exactly. No studio light,
    camera-relative cavity effect, material override or tone-map is involved.
    Other map objects are excluded so review lighting describes this asset.
    """
    settings = configuration(lighting)
    sun = Vector(settings["toward_sun"])
    tree, vertices, triangles, normals = _surface(objects)
    width, height = scene.render.resolution_x, scene.render.resolution_y
    buffers = []
    for index, camera in enumerate(cameras):
        if camera.data.type != "ORTHO":
            raise ValueError("Review sunlight currently requires orthographic cameras")
        frame = camera.data.view_frame(scene=scene)
        left, right = min(p.x for p in frame), max(p.x for p in frame)
        bottom, top = min(p.y for p in frame), max(p.y for p in frame)
        direction = camera.matrix_world.to_3x3() @ Vector((0, 0, -1))
        pixels = array("f")
        for y in range(height):
            for x in range(width):
                origin = camera.matrix_world @ Vector((left+(x+.5)*(right-left)/width,
                                                        bottom+(y+.5)*(top-bottom)/height, 0))
                hit, face_normal, triangle, _ = tree.ray_cast(origin, direction)
                if hit is None:
                    pixels.extend((0, 0, 0, 0))
                    continue
                normal = _interpolated_normal(hit, triangles[triangle], vertices, normals[triangle])
                blocked = False
                if normal.dot(sun) > 0:
                    blocked = tree.ray_cast(hit+sun*settings["shadow_epsilon"], sun)[0] is not None
                value = irradiance(normal, sun, settings["ambient"], settings["diffuse"], blocked)
                pixels.extend((value, value, value, 1))
        image = bpy.data.images.new("World sunlight review", width=width, height=height, alpha=True)
        try:
            image.pixels.foreach_set(pixels)
            image.file_format = "PNG"
            image.filepath_raw = str(Path(output)/f"view-{index}-solid.png")
            image.save()
        finally:
            bpy.data.images.remove(image)
        buffers.append(pixels)
    return buffers
