"""Measure saved tree wood contact against the selected bank geometry."""
import argparse,json,sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace,scenery_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS


def audit(mask):
    worker=tree_workspace(mask);bank=scenery_workspace('croisement02-north-woodland-bank')
    hashes={str(w):sha(w/'model.blend') for w in (worker,bank)}
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update()
    wood=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown']
    with bpy.data.libraries.load(str(bank/'model.blend'),link=False) as (source,loaded):loaded.collections=['Croisement02 Working']
    collection=loaded.collections[0];bpy.context.scene.collection.children.link(collection);bpy.context.view_layer.update()
    surfaces=[o for o in collection.all_objects if o.type=='MESH' and o.get('asset_group')==bank.name]
    bvhs=[(o,BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons])) for o in surfaces]
    rows=[]
    for obj in wood:
        points=[obj.matrix_world@v.co for v in obj.data.vertices];low=min(p.z for p in points);lower=[p for p in points if p.z<low+3]
        samples=[]
        for p in lower:
            hits=[]
            for surface,bvh in bvhs:
                hit=bvh.ray_cast(Vector((p.x,p.y,500)),Vector((0,0,-1)),1000)
                if hit[0] is not None:hits.append((hit[0].z,surface.get('source_node')))
            support=max(hits,default=(0,'diagnostic-Z0'),key=lambda r:r[0])
            samples.append(dict(point=list(p),source_pixel=[p.x,-p.y*SIN-p.z*COS],support_z=support[0],support_node=support[1],vertical_gap=p.z-support[0]))
        rows.append(dict(object=obj.name,source_node=obj.get('source_node'),vertices=len(points),bounds_min=[min(p[i] for p in points) for i in range(3)],bounds_max=[max(p[i] for p in points) for i in range(3)],lower_samples=samples))
    for path,digest in hashes.items():
        if sha(Path(path)/'model.blend')!=digest:raise ValueError('Source worker changed during audit')
    target=OUT/f'tree{mask:02}-root-research/contact-audit.json'
    target.parent.mkdir(parents=True,exist_ok=True)
    write_json(target,dict(mask=mask,workers=hashes,wood=rows,limitation='Lower three world units of each wood object. Branch bottoms are not roots; inspect topology/source before interpreting. Outside bank footprint Z0 is a diagnostic datum, not a terrain proof.'))
    print(target)


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('masks',type=int,nargs='+');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    acquire()
    try:
        for mask in args.masks:audit(mask)
    finally:release()
