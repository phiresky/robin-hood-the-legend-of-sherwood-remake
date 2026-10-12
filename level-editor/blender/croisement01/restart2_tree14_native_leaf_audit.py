"""Audit actual native leaf texels against saved joint geometry and alpha visibility."""
import json
import math
import shutil
import sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
from evidence_io import sha
acquire(slots=2)
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
r=ROOT/'level-editor/work/croisement01-refinement/restart2'
w=r/'tree14-crown94-geometry-v6';out=w/'native-first-hit-v1';assert not out.exists();out.mkdir()
model=w/'model.blend';before=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model))
packet=json.loads((w/'source-partition.json').read_text());frame=packet['frame_records'][0]
source=ROOT/frame['path'];assert sha(source)==frame['sha256']
rgba=np.asarray(Image.open(source).convert('RGBA'));left,top,width,height=frame['bbox']
assert rgba.shape==(height,width,4)
wood=next(o for o in bpy.context.scene.objects if o.get('source_node')=='scenery-tree14-wood')
crown=next(o for o in bpy.context.scene.objects if o.get('source_node')=='foliage-tree14-crown94')
def tree(obj):
    assert all(len(f.vertices)==3 for f in obj.data.polygons),obj.name
    return BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(f.vertices) for f in obj.data.polygons],all_triangles=True)
# Retained wood has quads; its BVH does not require texture lookup.
wt=BVHTree.FromPolygons([wood.matrix_world@v.co for v in wood.data.vertices],[list(f.vertices) for f in wood.data.polygons])
ct=tree(crown);uv=crown.data.uv_layers.active.data
textures=[]
for mat in crown.data.materials:
    image=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE')
    textures.append(np.asarray(Image.open(bpy.path.abspath(image.filepath)).convert('RGBA')))
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s))
records=[];classified=np.zeros(rgba.shape,dtype=np.uint8)
for y,x in zip(*np.nonzero(rgba[:,:,3]>127)):
    origin=Vector((float(left+x)+.5,-(float(top+y)+.5)/s,0))+ray*5000
    wh,_,_,wd=wt.ray_cast(origin,-ray,10000);wd=wd if wh is not None else float('inf')
    start=origin.copy();travel=0.;cd=float('inf');actual=None;known=False
    for attempt in range(128):
        point,normal,face,distance=ct.ray_cast(start,-ray,10000-travel)
        if point is None:break
        poly=crown.data.polygons[face]
        vs=[crown.matrix_world@crown.data.vertices[i].co for i in poly.vertices]
        coords=[Vector((*uv[i].uv,0.)) for i in poly.loop_indices]
        texcoord=barycentric_transform(point,*vs,*coords)
        arr=textures[poly.material_index];h,ww=arr.shape[:2]
        tx=min(ww-1,max(0,int(texcoord.x*ww)));ty=min(h-1,max(0,int((1-texcoord.y)*h)))
        if normal.dot(ray)>0 and arr[ty,tx,3]>127:
            cd=travel+distance;actual=arr[ty,tx].tolist();known=bool(crown.data.materials[poly.material_index].get('foliage_observed'));break
        travel+=distance+.001;start=origin-ray*travel
    else:raise RuntimeError('Transparent intersection limit reached')
    if wd<cd:state='occluded_by_wood'
    elif actual is None:state='missing_leaf'
    elif not known:state='unknown_in_front'
    elif actual!=rgba[y,x].tolist():state='source_color_mismatch'
    else:state='exact_native_leaf'
    records.append(dict(pixel=[int(left+x),int(top+y)],state=state,wood_distance=wd if math.isfinite(wd) else None,
                        leaf_distance=cd if math.isfinite(cd) else None,actual_rgba=actual))
    classified[y,x]=dict(exact_native_leaf=(60,210,100,255),occluded_by_wood=(240,70,50,255),
        missing_leaf=(230,70,230,255),unknown_in_front=(245,200,70,255),source_color_mismatch=(70,120,240,255))[state]
native_wood=np.asarray(Image.open(r.parent/'baseline/masks/000014.png').convert('L'))>0
known_wood=np.asarray(Image.open(r/'tree14-source-restoration-proposal-v1/wood-domain.png').convert('L'))>0
edge_records=[]
for y,x in zip(*np.nonzero(native_wood&~known_wood)):
    origin=Vector((float(689+x)+.5,-(float(122+y)+.5)/s,0))+ray*5000
    hit,_,_,_=wt.ray_cast(origin,-ray,10000)
    edge_records.append(dict(pixel=[int(689+x),int(122+y)],wood_geometry_hit=hit is not None))
assert len(edge_records)==40
counts={key:sum(x['state']==key for x in records) for key in ('exact_native_leaf','occluded_by_wood','missing_leaf','unknown_in_front','source_color_mismatch')}
assert len(records)==16719
Image.fromarray(classified).resize((width*3,height*3),Image.Resampling.NEAREST).save(out/'native-first-hit.png')
report=dict(status='MEASURED_NATIVE_FRAME0_VISIBILITY',model_sha256=before,source_sha256=frame['sha256'],counts=counts,records=records,
    pending_wood_edge_geometry=edge_records,pending_wood_edge_source_status='Observed edge fragments unresolved; not eligible for synthesis.',
    limitations=['Checks retained Tree14 wood and crown front/back geometry. Edge-on inferred cross sections and neighboring objects require actual saved camera review.',
                 'Frame0 source only. Remaining13frames and native state ordering need integration proof.'])
assert sha(model)==before
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(counts)
