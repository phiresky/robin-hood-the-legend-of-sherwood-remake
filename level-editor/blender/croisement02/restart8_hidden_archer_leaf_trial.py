"""Private source-preserving leaf-packet trials for two reveal endpoints."""
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from PIL import Image

HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE), str(HERE.parents[1] / 'refinement'),
               str(HERE.parents[1] / 'refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from tree_geometry import SIN, COS, RAY, leaf_cluster_geometry


def main():
    audit = OUT / 'restart8-hidden-archer-receiver-audit-v1'
    output = OUT / 'restart8-hidden-archer-leaf-trial-v1'
    output.mkdir(exist_ok=False)
    bounds = json.loads((audit / 'sprite-depth-feasibility-v1.json').read_text())
    hits = json.loads((audit / 'current-first-hit-v1/report.json').read_text())
    for profile in bounds['profiles']:
        number = int(profile['profile'][-2:])
        samples = next(p['samples'] for p in hits['profiles'] if p['profile'] == profile['profile'])
        for state in profile['states']:
            folder = output / f'profile-{number:02d}-{state["state"]}'
            folder.mkdir()
            source = Path(state['sprite_source'])
            assert sha(source) == state['sprite_sha256']
            # The full original sprite is retained, including every transparent
            # hole. All hidden depth is an explicitly unapproved hypothesis.
            image = np.asarray(Image.open(source).convert('RGBA'))
            Image.fromarray(image).save(folder / 'complete-source.png')
            h, w = image.shape[:2]
            x, y = state['native_top_left']
            bpy.ops.wm.read_factory_settings(use_empty=True)
            scene = bpy.context.scene
            scene.name = 'Croisement02 Refinement'
            obj = bpy.data.objects.new(f'Hidden archer {number:02d} {state["state"]} foliage',
                                       bpy.data.meshes.new('Leaf packet scaffold'))
            scene.collection.objects.link(obj)
            asset = f'croisement02-hidden-archer-{number:02d}-{state["state"]}'
            obj['asset_group'] = asset
            obj['source_node'] = f'mission-hidden-archer-{number:02d}'
            packet = dict(lobes=[dict(image=str(folder / 'complete-source.png'))],
                          native_bbox=[x, y, w, h], bbox=[x, y, w, h], native_mask=number)
            shape = leaf_cluster_geometry(obj, packet, y+h+8, depth_ratio=.85)
            vertices = obj.data.vertices
            assert len(vertices) % 16 == 0
            uv = obj.data.uv_layers['Foliage UV']
            vertex_uv = {}
            for loop in obj.data.loops:
                vertex_uv[loop.vertex_index] = uv.data[loop.index].uv.copy()
            shifts = []
            for start in range(0, len(vertices), 16):
                front = [vertices[start+i].co.copy() for i in range(4)]
                sx0, sx1 = min(p.x for p in front), max(p.x for p in front)
                sy0, sy1 = min(-p.y*SIN-p.z*COS for p in front), max(-p.y*SIN-p.z*COS for p in front)
                required = 0.
                for sample in samples:
                    px, py = sample['pixel']
                    if sample['world'] is None or not (sx0 <= px+.5 <= sx1 and sy0 <= py+.5 <= sy1):
                        continue
                    ix, iy = px-x, py-y
                    if not (0 <= ix < w and 0 <= iy < h and image[iy, ix, 3] >= 128):
                        continue
                    # Parallel source-facing quads have a constant ray depth.
                    plane_depth = front[0].dot(RAY)
                    required = max(required, Vector(sample['world']).dot(RAY)+2-plane_depth)
                if required > 0:
                    for index in range(start, start+16):
                        vertices[index].co += RAY*required
                shifts.append(required)
            obj.data.update()
            obj['state_endpoint'] = state['state']
            obj['geometry_status'] = 'Private leaf-volume trial; support and complete source audit pending'
            bpy.context.preferences.filepaths.save_version = 0
            bpy.ops.wm.save_as_mainfile(filepath=str(folder / 'model.blend'), compress=True)
            points = np.array([v.co for v in vertices])
            write_json(folder / 'construction.json', dict(
                status='Private geometry trial; not ready for user review',
                model_sha256=sha(folder / 'model.blend'), asset_id=asset,
                source=str(source), source_sha256=state['sprite_sha256'],
                source_top_left=[x,y], source_opaque_centers=int((image[:,:,3]>=128).sum()),
                shape=shape, rigid_packet_count=len(shifts), shifted_packets=sum(d>0 for d in shifts),
                maximum_ray_shift=max(shifts), world_min=points.min(0).tolist(), world_max=points.max(0).tolist(),
                evidence=dict(first_hit_report=str(audit / 'current-first-hit-v1/report.json'),
                              first_hit_report_sha256=sha(audit / 'current-first-hit-v1/report.json')),
                limitations=['Inferred packet depth and placement require visual review.',
                    'Clearance uses changed native centers only; unchanged centers need a full audit.',
                    'Individual elevated packets are not yet proven supported; no endpoint completion claim.',
                    'All rear/crossed leaf RGB reuses this exact endpoint sprite, not observed rear artwork.',
                    'Existing trunks, kindling, ground and source contracts have not been modified.']))
            print(asset, 'saved private trial', flush=True)


if __name__ == '__main__':
    acquire()
    try:
        main()
    finally:
        release()
