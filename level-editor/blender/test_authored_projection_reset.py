"""Blender fixture: rebuilding one part must not preserve its stale authored UVs.

Run with Blender --background --factory-startup --python this_file.py.
"""
import math
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import bpy
from mathutils import Vector
from source_projection_bake import bake


def check():
    collection = bpy.data.collections.new("ResetFixture Working")
    bpy.context.scene.collection.children.link(collection)
    up = Vector((0, math.sin(math.radians(35)), math.cos(math.radians(35))))
    authored = bpy.data.materials.new("Accepted generated surface")
    authored["projection_preserve"] = True
    objects = []
    for index, x in enumerate((0, 8)):
        mesh = bpy.data.meshes.new(f"Part {index}")
        mesh.from_pydata([Vector((x + dx, 0, 0)) + up * (y - 16)
                          for dx, y in ((0, 0), (7, 0), (7, 16), (0, 16))],
                         [], [(0, 1, 2, 3)])
        mesh.update()
        mesh.materials.append(authored)
        obj = bpy.data.objects.new(mesh.name, mesh)
        obj["source_node"] = f"part-{index}"
        obj["asset_group"] = "fixture"
        collection.objects.link(obj)
        objects.append(obj)
    with tempfile.TemporaryDirectory() as temporary:
        output = Path(temporary)
        source = bpy.data.images.new("Observed red", width=16, height=16, alpha=True)
        source.pixels[:] = [1, 0, 0, 1] * 256
        source.filepath_raw = str(output / "source.png")
        source.file_format = "PNG"
        source.save()
        report = bake("ResetFixture", output / "source.png", output / "report.json",
                      reproject_authored_nodes=["part-0"])
        rebuilt, unchanged = objects
        new_material = rebuilt.data.materials[rebuilt.data.polygons[0].material_index]
        assert new_material != authored and new_material.get("source_ownership_bake")
        assert unchanged.data.materials[unchanged.data.polygons[0].material_index] == authored
        assert report["known_texels"] > 0
        assert report["reproject_authored_nodes"] == ["part-0"]
        try:
            bake("ResetFixture", output / "source.png", output / "invalid.json",
                 receiver_nodes=["part-1"], reproject_authored_nodes=["part-0"])
        except ValueError as error:
            assert "receiver nodes" in str(error)
        else:
            raise AssertionError("Reset outside receiver scope was accepted")
    print("PASS: rebuilt part reprojected; unchanged authored material retained; scope guarded")


if __name__ == "__main__":
    check()
