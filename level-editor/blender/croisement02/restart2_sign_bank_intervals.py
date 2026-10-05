"""Read-only free depth intervals between target7 sign and bank or rock solids."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,RAY
from render_slots import acquire,release

def main():
    dest=OUT/'restart2-fence/sign7-bank-intervals-v1';dest.mkdir(exist_ok=False)
    details=OUT/'restart2-fence/sign-fragment-bounds-v1/target-7.json';rows=json.loads(details.read_text())['pixels']
    manifest=OUT/'restart2-fence/sign-neighbors-v4/manifest.json';inputs=json.loads(manifest.read_text())['inputs'];trees=[];hashes={}
    for key in ['north-woodland-bank','west-rock-outcrop']:
        row=inputs[key];path=Path(row['worker'])/'model.blend';assert sha(path)==row['model_sha256'];hashes[key]=sha(path)
        bpy.ops.wm.open_mainfile(filepath=str(path));verts=[];tris=[]
        for name in row['objects']:
            obj=bpy.data.objects[name];mesh=obj.data;mesh.calc_loop_triangles();offset=len(verts);verts.extend([tuple(obj.matrix_world@v.co) for v in mesh.vertices]);tris.extend([tuple(offset+i for i in tri.vertices) for tri in mesh.loop_triangles])
        trees.append((key,BVHTree.FromPolygons(verts,tris,all_triangles=True)))
    results=[]
    for row in rows:
        if not row.get('blockers'):continue
        x,y=row['source_pixel'];origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000
        z=row['blockers'][0]['hit_z']-SIN*row['blockers'][0]['required_retreat']
        blockers=[]
        for key,tree in trees:
            hit,normal,index,distance=tree.ray_cast(origin,-RAY,10000)
            if hit is not None:blockers.append(dict(asset=key,first_surface_z=float(hit.z),free_vertical_gap=z-float(hit.z)))
        results.append(dict(phase=row['phase'],source_pixel=[x,y],required_leaf_z_behind_sign=z,solids=blockers,contradictory=any(b['free_vertical_gap']<0 for b in blockers)))
    bad=[r for r in results if r['contradictory']]
    write_json(dest/'report.json',dict(status='Read-only sign-back versus first bank-surface intervals',inputs=hashes,detail_sha256=sha(details),neighbor_manifest_sha256=sha(manifest),sample_count=len(results),contradictory_samples=len(bad),contradictory_unique_pixels=len({tuple(r['source_pixel']) for r in bad}),records=results,limitations=['Four sign poses; sign rear includes0.1worldunit ray clearance.','A negative gap prevents foliage occupying the visible exterior interval between complete sign body and first bank surface at that ray.','This does not exclude rebuilding foliage behind the entire bank, but that would remove its visibility and change source appearance.','Opaque geometry bounds are used for bank/rock surfaces; this is not a complete scene ray ordering proof.']))
    print(dest)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
