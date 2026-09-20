"""Run with Blender --background --python this_file.py."""
import sys
import tempfile
import unittest
from pathlib import Path

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from review_sunlight import configuration, irradiance, render_solids


class ReviewSunlightTests(unittest.TestCase):
    def test_world_surface_brightness_and_scene_restoration(self):
        scene = bpy.data.scenes.new("Sunlight invariant fixture")
        scene.render.resolution_x = scene.render.resolution_y = 17
        mesh = bpy.data.meshes.new("Horizontal sample")
        mesh.from_pydata([(-10,-10,0),(10,-10,0),(10,10,0),(-10,10,0)], [], [(0,1,2,3)])
        obj = bpy.data.objects.new("Horizontal sample", mesh)
        scene.collection.objects.link(obj)
        cameras = []
        old_scene = bpy.context.window.scene
        bpy.context.window.scene = scene
        try:
            for location in ((0,-20,20),(20,0,20),(0,20,20)):
                data = bpy.data.cameras.new("Invariant camera")
                data.type = "ORTHO"
                data.ortho_scale = 15
                camera = bpy.data.objects.new(data.name,data)
                scene.collection.objects.link(camera)
                camera.location = location
                camera.rotation_euler = (-Vector(location)).to_track_quat('-Z','Y').to_euler()
                cameras.append(camera)
            bpy.context.view_layer.update()
            scene.camera = cameras[1]
            before = (scene.camera, scene.render.engine, scene.render.filepath,
                      obj.hide_render, len(mesh.materials), len(bpy.data.images))
            with tempfile.TemporaryDirectory(dir=Path(__file__).parent) as directory:
                buffers = render_solids(scene, cameras, [obj], directory)
            after = (scene.camera, scene.render.engine, scene.render.filepath,
                     obj.hide_render, len(mesh.materials), len(bpy.data.images))
            self.assertEqual(before, after)
            light = configuration()
            expected = irradiance(Vector((0,0,1)), Vector(light['toward_sun']),
                                  light['ambient'], light['diffuse'])
            for buffer in buffers:
                self.assertAlmostEqual(buffer[(8*17+8)*4], expected, places=6)
                self.assertEqual(buffer[(8*17+8)*4+3], 1)
        finally:
            bpy.context.window.scene = old_scene
            for camera in cameras:
                data = camera.data
                bpy.data.objects.remove(camera, do_unlink=True)
                bpy.data.cameras.remove(data)
            bpy.data.objects.remove(obj, do_unlink=True)
            bpy.data.meshes.remove(mesh)
            bpy.data.scenes.remove(scene)


if __name__ == '__main__':
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(ReviewSunlightTests)
    if not unittest.TextTestRunner().run(suite).wasSuccessful():
        raise RuntimeError("World sunlight regression")
