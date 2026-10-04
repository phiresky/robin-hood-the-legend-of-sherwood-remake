"""Compare saved northwest cliff depth against the existing observed front."""
import argparse,json,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'refinement/blender'))
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,RAY


def snapshot(worker,node):
    bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update()
    obj=next(o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('source_node')==node and o.get('asset_group')==worker.name)
    vertices=[obj.matrix_world @ v.co for v in obj.data.vertices]
    return BVHTree.FromPolygons(vertices,[tuple(f.vertices) for f in obj.data.polygons])


def main():
    parser=argparse.ArgumentParser();parser.add_argument('worker',type=Path);parser.add_argument('--node',choices=['building-035','building-036'],default='building-035');args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);worker=args.worker.resolve()
    old=OUT/'scenery-round-2/assets/croisement02-northwest-rock-outcrop';before=snapshot(old,args.node);after=snapshot(worker,args.node)
    domain=np.asarray(Image.open(OUT/'northwest-rock-source-revision/domain-380.png'))>0;samples=[];missing=0
    for y in (range(2,88,2) if args.node=='building-035' else range(96,168,2)):
        for x in range(2,154,2):
            if not domain[y,x]:continue
            origin=Vector((x,-y/SIN,0))+RAY*5000;oldpoint=before.ray_cast(origin,-RAY)[0]
            if oldpoint is None:continue
            newpoint=after.ray_cast(origin,-RAY)[0]
            if newpoint is None:missing+=1;continue
            samples.append(dict(source=[x,y],depth_delta=float((newpoint-oldpoint).dot(RAY))))
    values=np.abs([s['depth_delta'] for s in samples]);report=dict(source_node=args.node,model_sha256=sha(worker/'model.blend'),original_model_sha256=sha(old/'model.blend'),samples=len(samples),missing_samples=missing,median_absolute_depth_delta=float(np.median(values)),p95_absolute_depth_delta=float(np.percentile(values,95)),maximum_absolute_depth_delta=float(values.max()),largest_changes=sorted(samples,key=lambda r:abs(r['depth_delta']),reverse=True)[:20],status='Observed front depth comparison; new unsupported source regions measured separately by coverage audit')
    interior=[r for r in samples if r['source'][0]<(100 if args.node=='building-035' else 99) and (r['source'][1]>=20 or args.node!='building-035')]
    interior_values=np.abs([r['depth_delta'] for r in interior])
    report['preserved_interior']=dict(samples=len(interior),median_absolute_depth_delta=float(np.median(interior_values)),p95_absolute_depth_delta=float(np.percentile(interior_values,95)),maximum_absolute_depth_delta=float(interior_values.max()))
    report['edge_corrections']='The obsolete northern/side cut-edge bevel and source-traced middle foot are intentionally corrected; full-sample deltas above retain those changes.'
    write_json(worker/'inspection'/('native-front-depth.json' if args.node=='building-035' else 'native-front-depth-036.json'),report);print(json.dumps(report,indent=2))

if __name__=='__main__':main()
