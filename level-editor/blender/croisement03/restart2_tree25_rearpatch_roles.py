"""Separate visible rear shells from front backings with material-sidedness rays."""
import collections
import hashlib
import json
import re
from pathlib import Path
import sys
import bpy
import numpy as np
from PIL import Image
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'level-editor/refinement'))
from render_slots import acquire, release


def main():
    e = ROOT / 'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
    out = e / 'rearpatch-geometry-v1'
    assert not (out / 'role-attribution.json').exists()
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(out / 'worker.blend'))
        obj = next(o for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == 'croisement03-tree-25')
        mesh = obj.data; mesh.calc_loop_triangles()
        points = [obj.matrix_world @ v.co for v in mesh.vertices]
        tris = list(mesh.loop_triangles)
        tree = BVHTree.FromPolygons(points, [list(t.vertices) for t in tris], all_triangles=True)
        uv = mesh.uv_layers['Foliage UV']
        key = lambda f: tuple(sorted(tuple(round(c,6) for c in uv.data[i].uv) for i in f.loop_indices))
        fronts = {slot: {key(f): f for f in mesh.polygons if f.material_index == slot} for slot in range(1,34,4)}
        roles = {}
        for face in mesh.polygons:
            slot = face.material_index
            if slot in fronts:
                role = 'native-front'
            elif slot-1 in fronts:
                counterpart = fronts[slot-1].get(key(face))
                role = 'front-backing' if counterpart and (face.center-counterpart.center).length < .1 else 'rear-shell'
            elif slot in (3,4,7,8,11,12,15,16,19,20,23,24,27,28,31,32,35,36):
                role = 'transverse'
            else:
                role = 'wood-or-offmap'
            roles[face.index] = role
        atlas = {}
        for slot, mat in enumerate(mesh.materials):
            if not mat or not mat.get('foliage_physical_opacity'):
                continue
            node = next(n for n in mat.node_tree.nodes if n.type == 'TEX_IMAGE' and n.image)
            data = np.empty(len(node.image.pixels), np.float32); node.image.pixels.foreach_get(data)
            atlas[slot] = dict(alpha=data.reshape(node.image.size[1],node.image.size[0],4)[...,3],
                uv=mesh.uv_layers[node.inputs['Vector'].links[0].from_node.uv_map],
                lobe=int(re.search(r'lobe(\d+)',mat.name)[1]),
                one_sided=mat.get('foliage_card_sides') == 'paired-one-sided')
        views = json.loads((e / 'views-grid8-v4.json').read_text())['views']
        reports = []
        for index, box in [(4,(120,160,280,320)), (5,(80,120,280,320))]:
            camera = Matrix(views[index]['camera_matrix_world']); scale = views[index]['ortho_scale']
            direction = camera.to_3x3() @ Vector((0,0,-1)); direction.normalize()
            counts = collections.Counter(); colors = {'native-front':(80,180,80), 'front-backing':(230,50,40),
                'rear-shell':(50,140,240), 'transverse':(240,200,50), 'wood-or-offmap':(140,140,140)}
            picture = Image.new('RGB',(384,384),(25,25,25)); limited = 0; sampled = 0
            for y in range(box[1],box[3]):
                for x in range(box[0],box[2]):
                    sampled += 1
                    origin = camera @ Vector((((x+.5)/384-.5)*scale,(.5-(y+.5)/384)*scale,0))
                    for step in range(256):
                        p, normal, tid, distance = tree.ray_cast(origin,direction)
                        if p is None:
                            counts['transparent'] += 1; break
                        tri = tris[tid]; face = mesh.polygons[tri.polygon_index]; slot = face.material_index
                        role = roles[face.index]
                        if slot not in atlas:
                            counts[role] += 1; picture.putpixel((x,y), colors[role]); break
                        data = atlas[slot]
                        if not (data['one_sided'] and normal.dot(direction) >= 0):
                            layer = data['uv']; alpha = data['alpha']
                            mapped = barycentric_transform(p,*[points[v] for v in tri.vertices],
                                *[Vector((*layer.data[i].uv,0)) for i in tri.loops])
                            tx = min(alpha.shape[1]-1,int((mapped.x%1)*alpha.shape[1]))
                            ty = min(alpha.shape[0]-1,int((mapped.y%1)*alpha.shape[0]))
                            if alpha[ty,tx] >= .5:
                                counts[f'lobe{data["lobe"]:02}/{role}'] += 1
                                picture.putpixel((x,y),colors[role]); break
                        origin = p+direction*.002
                    else:
                        limited += 1
            picture.save(out / f'view-{index}-surface-role.png')
            reports.append(dict(view=index, box=box, sampled=sampled, depth_limits=limited, first_visible=dict(counts)))
            print(reports[-1],flush=True)
        (out / 'role-attribution.json').write_text(json.dumps(dict(
            model_sha256=hashlib.sha256((out / 'worker.blend').read_bytes()).hexdigest(),
            reports=reports, colors=colors, method='Every pixel centre in the two reverse-view crop boxes; nearest alpha plus explicit paired-one-sided material culling. Unknown faces within0.1unit of same-UV native front are classified as front backings.',
            limitations=['Prior role diagnostics did not enforce shader sidedness and conflated rear shells with front backings; use this bounded report for visible-surface attribution.',
                        'Pixel-centre test does not reproduce render antialiasing.']),indent=2)+'\n')
    finally:
        release()


if __name__ == '__main__':
    main()
