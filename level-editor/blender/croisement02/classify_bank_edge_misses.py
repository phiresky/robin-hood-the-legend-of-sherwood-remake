"""Read-only native bank boundary classification using saved physical geometry."""
import json
from pathlib import Path
import sys
import bpy
import numpy as np
from PIL import Image, ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(Path(__file__).parent), str(ROOT/'level-editor/refinement'), str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha, write_json
from render_slots import acquire, release
from tree_geometry import SIN, COS, RAY


def main():
    output = OUT/'restart2-bank321/classification-v2'
    if output.exists():
        raise FileExistsError(output)
    output.mkdir(parents=True)
    bank = OUT/'texture-fill-round-2/croisement02-north-woodland-bank/complete-preparation/experiment-ground-retry-v2/bake-v1/worker.blend'
    original = OUT/'terrain-bank-candidate'
    missing = np.asarray(Image.open(original/'integration/bank-missing-domain.png').convert('L')) > 0
    known = np.asarray(Image.open(original/'bank-source-domain.png').convert('L')) > 0
    if missing.sum() != 321 or (missing & ~known).any():
        raise ValueError('Historical bank miss authority changed')
    digest = sha(bank)
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(bank))
        objects = [o for o in bpy.data.objects if o.type == 'MESH' and o.get('asset_group') == 'croisement02-north-woodland-bank']
        verts, triangles, names = [], [], []
        for obj in objects:
            base = len(verts)
            verts.extend(obj.matrix_world @ v.co for v in obj.data.vertices)
            obj.data.calc_loop_triangles()
            for t in obj.data.loop_triangles:
                triangles.append(tuple(base+i for i in t.vertices)); names.append(obj.name)
        if not triangles:
            raise ValueError('No selected bank triangles')
        tree = BVHTree.FromPolygons(verts, triangles, all_triangles=True)
        projected = np.array([(p.x, -p.y*SIN-p.z*COS) for p in verts])
        edges = np.array([(t[i], t[(i+1)%3]) for t in triangles for i in range(3)])
        edge_a, edge_b = projected[edges[:,0]], projected[edges[:,1]]
        edge_delta = edge_b-edge_a
        edge_norm = np.maximum((edge_delta*edge_delta).sum(axis=1), 1e-20)
        ray = Vector(RAY)
        def hit(x, y):
            origin = Vector((x, -y/SIN, 0)) + ray*10000
            p, _, face, distance = tree.ray_cast(origin, -ray)
            if p is None:
                return False, None
            # Native flat receiver is world z=0; compare the same camera ray.
            ground_distance = origin.z/ray.z
            return distance <= ground_distance + .001, names[face]
        rows = []
        offsets = np.arange(.1, 1, .2)
        for y, x in np.argwhere(missing):
            center, owner = hit(x+.5, y+.5)
            covered = sum(hit(x+dx, y+dy)[0] for dx in offsets for dy in offsets)
            nearby = any(hit(x+.5+dx,y+.5+dy)[0] for dx,dy in [(0,-1),(0,1),(-1,0),(1,0),(-1,-1),(1,-1),(-1,1),(1,1)])
            category = 'center-hit-raster-disagreement' if center else 'subpixel-boundary' if covered else 'within-one-pixel-boundary' if nearby else 'beyond-one-pixel-needs-source-review'
            p2 = np.array([x+.5,y+.5])
            t = np.clip(((p2-edge_a)*edge_delta).sum(axis=1)/edge_norm,0,1)
            nearest = float(np.linalg.norm(edge_a+t[:,None]*edge_delta-p2,axis=1).min())
            rows.append(dict(nearest_projected_triangle_edge=nearest, x=int(x), y=int(y), center_hit=center, center_owner=owner, subpixel_hits=covered, samples=25, adjacent_center_hit=nearby, category=category))
        source = Image.open(OUT/'ground-receiver-review-v5/reference/source.png').convert('RGB')
        colors = {'center-hit-raster-disagreement':(0,230,210), 'subpixel-boundary':(255,170,0), 'within-one-pixel-boundary':(230,50,210), 'beyond-one-pixel-needs-source-review':(255,30,30)}
        marked=np.array(source)
        counts={c:0 for c in colors}
        for row in rows:
            marked[row['y'],row['x']]=colors[row['category']]; counts[row['category']]+=1
        Image.fromarray(marked).save(output/'classified-source.png')
        boxes=[(240,210,560,430),(540,400,850,640),(875,240,1180,350),(1150,205,1240,295)]
        for i,box in enumerate(boxes):
            w,h=box[2]-box[0],box[3]-box[1]
            sheet=Image.new('RGB',(w*4,h*2+32),(30,30,30)); draw=ImageDraw.Draw(sheet)
            draw.text((8,8),'Native RGB | classified misses (2x nearest)',fill='white')
            sheet.paste(source.crop(box).resize((w*2,h*2),Image.Resampling.NEAREST),(0,32))
            sheet.paste(Image.fromarray(marked).crop(box).resize((w*2,h*2),Image.Resampling.NEAREST),(w*2,32))
            sheet.save(output/f'context-{i}.png')
        write_json(output/'classification.json', dict(model=str(bank), model_sha256=digest, historical_missing_sha256=sha(original/'integration/bank-missing-domain.png'), counts=counts, rows=rows, method='BVH physical bank against z=0 receiver; pixel center and 5x5 subpixel samples; adjacent pixel centers. Boundary categories measure coverage, not ownership or automatic geometry authority.', colors=colors, geometry_changed=False, api_calls=0))
        if sha(bank)!=digest:
            raise ValueError('Bank changed during read-only audit')
        print(json.dumps(counts))
    finally:
        release()


if __name__ == '__main__':
    main()
