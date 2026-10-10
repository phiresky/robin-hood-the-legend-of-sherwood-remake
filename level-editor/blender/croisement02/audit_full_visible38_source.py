"""Independently ray-test every frozen Tree38 wood pixel in a saved model."""
import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement')]
from render_slots import acquire,release
from continuous_wood_field import RAY,SIN,COS


def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--worker',type=Path,required=True);parser.add_argument('--packet',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    if args.output.exists():raise FileExistsError(args.output)
    evidence=json.loads((args.worker/'evidence.json').read_text());model=args.worker/'model.blend';digest=sha(model)
    if digest!=evidence['model_sha256']:raise ValueError('Saved model changed')
    packet=json.loads(args.packet.read_text());record=next(r for r in packet['records']if r['tree']==38);mask=Path(record['source_mask'])
    if sha(mask)!=record['source_mask_sha256']:raise ValueError('Observed source ownership changed')
    observed=np.asarray(Image.open(mask).convert('L'))>0
    if observed.shape!=(1152,1792) or int(observed.sum())!=7044:raise ValueError('Unexpected full native wood domain')
    acquire(slots=2)
    try:
        memory=int(next(line.split()[1]for line in Path('/proc/meminfo').read_text().splitlines()if line.startswith('MemAvailable:')))*1024
        if memory<6*1024**3 or shutil.disk_usage(ROOT).free<10*1024**3:raise RuntimeError('Resource guard stopped worker before scene load')
        bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update()
        wood=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-38' and o.get('projection_component')!='crown']
        if not wood:raise ValueError('No saved Tree38 wood')
        vertices=[];triangles=[]
        for obj in wood:
            if obj.get('source_node')!='building-094':raise ValueError('Unexpected source owner')
            obj.data.calc_loop_triangles();offset=len(vertices);vertices.extend(obj.matrix_world@v.co for v in obj.data.vertices);triangles.extend(tuple(offset+i for i in face.vertices)for face in obj.data.loop_triangles)
        bvh=BVHTree.FromPolygons(vertices,triangles,all_triangles=True,epsilon=0.)
        direction=Vector(RAY.tolist());missing=[];below=[];maximum_error=0.;heights=[]
        for y,x in np.argwhere(observed):
            origin=Vector((float(x)+.5,-(float(y)+.5)/SIN,0.))+direction*5000
            point,_,_,_=bvh.ray_cast(origin,-direction,10000)
            if point is None:missing.append([int(x),int(y)]);continue
            if point.z < -1e-5:below.append([int(x),int(y),float(point.z)])
            heights.append(float(point.z))
            maximum_error=max(maximum_error,float(np.hypot(point.x-(x+.5),-point.y*SIN-point.z*COS-(y+.5))))
        report={'status':'PASS_SAVED_FULL_SOURCE_CENTERS'if not missing and not below and maximum_error<=.005 else'HOLD','model':str(model.resolve()),'model_sha256':digest,'source_mask':str(mask),'source_mask_sha256':sha(mask),'observed_source_centers':7044,'hits':7044-len(missing),'missing':missing,'below_ground_plane':below,'minimum_hit_z':min(heights)if heights else None,'maximum_source_xy_error':float(maximum_error),'scope':'Independent opaque wood BVH at exact native pixel centers in reopened float32 model; z0 is a ground-plane diagnostic','limitations':['Does not establish complete crown/neighbor occlusion or native RGB sampling','Actual terrain contact and visual review remain separate','No geometry or texture approval is implied']}
        args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(report,indent=2)+'\n')
        if sha(model)!=digest:raise ValueError('Audit changed saved model')
        print(json.dumps(report,indent=2))
        if missing or below or maximum_error>.005:raise ValueError('Saved full native source coverage failed')
    finally:release()


if __name__=='__main__':main()
