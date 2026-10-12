"""Render saved bank beside exact native foreground; audit all phase occlusion rays."""
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as guard

R, B, S, C = guard.R, guard.B, guard.S, guard.C
F = B / 'bank-convex-source-saved-v1'
O = F / 'native-foreground-context-v1'
RAY = Vector((0, -C, S))


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    guard.resources()
    guard.acquire(slots=2)
    try:
        guard.resources()
        assert not O.exists()
        O.mkdir()
        model = F / 'worker.blend'
        audit_path = B / 'bank-foreground-source-audit-v2/receipt.json'
        audit = json.loads(audit_path.read_text())
        pins = {str(model): sha(model), str(audit_path): sha(audit_path)}
        for path, digest in audit['pins'].items():
            assert sha(Path(path)) == digest
        for name in ['shared/src/scene.ts', 'app/src/scenery-frames.ts', 'app/src/scenery-layer.ts']:
            path = R / 'level-editor' / name
            pins[str(path)] = sha(path)
        manifest_path = R / 'datadirs/fullgame_gog_hackable/Data/Animations/Day/Treecr03.rhs.d/manifest.json'
        pins[str(manifest_path)] = sha(manifest_path)
        profile = next(p for p in json.loads(manifest_path.read_text())['profiles'] if p['name'] == 'Croisement03 - Arbre05')
        sprite = audit['native_sprite']
        anchor_y = sprite['position_y'] + profile['center_y'] + sprite['elevation']
        plane_y = -anchor_y / S
        bpy.ops.wm.open_mainfile(filepath=str(model))
        scene = bpy.context.scene
        banks = [obj for obj in scene.objects if obj.type == 'MESH' and obj.name.startswith('Candidate bank')]
        assert len(banks) == 3
        rows = []
        for obj in scene.objects:
            if obj.type == 'MESH':
                obj.hide_render = obj not in banks
        for obj in banks:
            obj.data.calc_loop_triangles()
            rows.append((obj.name, BVHTree.FromPolygons(
                [obj.matrix_world @ v.co for v in obj.data.vertices],
                [tuple(t.vertices) for t in obj.data.loop_triangles], all_triangles=True)))
        union = set()
        frame_domains = []
        for frame in audit['frames']:
            a = np.array(Image.open(frame['source']).convert('RGBA'))
            # The complete native decoder also removes blue shadow keys. None
            # exist here; exact normalized RGBA therefore matches that decoder.
            wide = a.astype(np.uint32)
            packed = ((wide[:, :, 0] >> 3) << 11) | ((wide[:, :, 1] >> 2) << 5) | (wide[:, :, 2] >> 3)
            assert not np.any((packed == 31) & (a[:, :, 3] > 0))
            x0, y0 = frame['bbox'][:2]
            domain = {(int(x + x0), int(y + y0)) for y, x in zip(*np.nonzero(a[:, :, 3]))}
            frame_domains.append(domain)
            union |= domain
        blocked = {}
        for x, y in sorted(union):
            native = Vector((x + .5, plane_y, (anchor_y - y - .5) / C))
            origin = native + RAY * 5000
            hits = []
            for name, bvh in rows:
                point, normal, face, distance = bvh.ray_cast(origin, -RAY)
                if point is not None and distance < 5000 - .001:
                    hits.append([name, int(face), float(distance), list(point)])
            if hits:
                blocked[(x, y)] = min(hits, key=lambda h: h[2])
        phases = [{'phase': i, 'alpha_pixels': len(domain),
                   'blocked': [{'xy': list(p), 'bank_firsthit': blocked[p]} for p in sorted(domain & blocked.keys())]}
                  for i, domain in enumerate(frame_domains)]
        report = {'status': 'HOLD_NATIVE_OCCLUSION' if blocked else 'PASS_EXACT_PLANE_SOURCE_PRIORITY',
                  'pins': pins, 'native_plane_y': plane_y, 'anchor_game_y': anchor_y,
                  'union_pixels': len(union), 'blocked_union_pixels': len(blocked), 'phases': phases,
                  'limits': ['Saved bank and exact full native Arbre05 diagnostic only; not production runtime proof.',
                             'Static source foliage and Tree18 wood are deliberately absent and remain separate unresolved owners.',
                             'No source texture, geometry, gameplay or canonical placement changed.']}
        (O / 'firsthits.json').write_text(json.dumps(report, indent=2) + '\n')
        material = bpy.data.materials.new('Exact native Arbre05 diagnostic')
        material.use_nodes = True
        nodes, links = material.node_tree.nodes, material.node_tree.links
        nodes.clear()
        tex = nodes.new('ShaderNodeTexImage')
        tex.interpolation = 'Closest'
        emission = nodes.new('ShaderNodeEmission')
        transparent = nodes.new('ShaderNodeBsdfTransparent')
        mix = nodes.new('ShaderNodeMixShader')
        output = nodes.new('ShaderNodeOutputMaterial')
        links.new(tex.outputs['Color'], emission.inputs['Color'])
        links.new(tex.outputs['Alpha'], mix.inputs[0])
        links.new(transparent.outputs[0], mix.inputs[1])
        links.new(emission.outputs[0], mix.inputs[2])
        links.new(mix.outputs[0], output.inputs['Surface'])
        mesh = bpy.data.meshes.new('Native Arbre05 full frame')
        foreground = bpy.data.objects.new('Native Arbre05 diagnostic single owner', mesh)
        scene.collection.objects.link(foreground)
        mesh.materials.append(material)

        def frame(index):
            row = audit['frames'][index]
            x, y, w, h = row['bbox']
            verts = [(x, plane_y, (anchor_y-y)/C), (x+w, plane_y, (anchor_y-y)/C),
                     (x+w, plane_y, (anchor_y-y-h)/C), (x, plane_y, (anchor_y-y-h)/C)]
            mesh.clear_geometry()
            mesh.from_pydata(verts, [], [(0, 3, 2, 1)])
            uv = mesh.uv_layers.new(name='Exact native pixel coordinates')
            uvs = [(0, 1), (1, 1), (1, 0), (0, 0)]
            for loop in mesh.loops:
                uv.data[loop.index].uv = uvs[loop.vertex_index]
            tex.image = bpy.data.images.load(row['source'], check_existing=True)

        frame(0)
        points = np.array([list(obj.matrix_world @ v.co) for obj in banks + [foreground] for v in obj.data.vertices])
        camera = bpy.data.objects.new('Full bank native context camera', bpy.data.cameras.new('Full bank native context camera'))
        scene.collection.objects.link(camera)
        camera.data.type = 'ORTHO'
        camera.data.clip_end = 10000
        scene.camera = camera
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 16
        scene.cycles.transparent_max_bounces = 256
        scene.cycles.use_denoising = False
        scene.render.threads_mode = 'FIXED'
        scene.render.threads = 2
        scene.render.resolution_x = scene.render.resolution_y = 768
        scene.render.resolution_percentage = 100
        scene.render.film_transparent = True
        scene.render.image_settings.file_format = 'PNG'
        scene.render.image_settings.color_mode = 'RGBA'
        scene.view_settings.view_transform = 'Standard'
        for name, angle, phase in [('native-phase00', 0, 0), ('native-phase07', 0, 7), ('oblique45', 45, 0), ('oblique180', 180, 0)]:
            frame(phase)
            theta = math.radians(angle)
            direction = Vector((math.sin(theta)*C, -math.cos(theta)*C, S))
            rotation = (-direction).to_track_quat('-Z', 'Y')
            origin = Vector(points.mean(axis=0))
            local = np.array([list(rotation.conjugated() @ (Vector(p)-origin)) for p in points])
            middle = (local[:, :2].min(axis=0) + local[:, :2].max(axis=0)) / 2
            target = origin + rotation @ Vector((*middle, 0))
            camera.location = target + direction * 5000
            camera.rotation_euler = rotation.to_euler()
            camera.data.ortho_scale = max(np.ptp(local[:, 0]), np.ptp(local[:, 1])) * 1.12
            scene.render.filepath = str(O / (name+'.png'))
            bpy.ops.render.render(write_still=True)
        for path, digest in pins.items():
            assert sha(Path(path)) == digest, path
        assert sum(p.stat().st_size for p in O.rglob('*') if p.is_file()) < 32*1024**2
        print(report['status'], len(blocked))
    finally:
        guard.release()


if __name__ == '__main__':
    main()
