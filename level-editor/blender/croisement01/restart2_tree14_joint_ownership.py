"""Measure joint candidate first hits; geometric visibility is not source semantics."""
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
r=ROOT/'level-editor/work/active/croisement01/restart2'
w=r/'tree14-crown94-geometry-v2';out=w/'joint-ownership-v1';assert not out.exists();out.mkdir()
model=w/'model.blend';before=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model))
wood=next(o for o in bpy.context.scene.objects if o.get('source_node')=='scenery-tree14-wood')
crown=next(o for o in bpy.context.scene.objects if o.get('source_node')=='foliage-tree14-crown94')
def tree(obj):return BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(f.vertices) for f in obj.data.polygons])
wt,ct=tree(wood),tree(crown)
textures=[]
for mat in crown.data.materials:
    image=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE')
    textures.append(np.asarray(Image.open(image.filepath).convert('RGBA')))
uv=crown.data.uv_layers.active.data
native=np.asarray(Image.open(r.parent/'baseline/masks/000014.png').convert('L'))>0
known=np.asarray(Image.open(r/'tree14-wood-v13/wood-domain.png').convert('L'))>0
pending=native&~known
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s))
records=[];classified=np.zeros((*native.shape,4),dtype=np.uint8)
for y,x in zip(*np.nonzero(pending)):
    origin=Vector((float(689+x)+.5,-(float(122+y)+.5)/s,0))+ray*5000
    hit,_,_,wd=wt.ray_cast(origin,-ray,10000)
    wd=wd if hit is not None else 1e20
    start=origin.copy();travel=0.;cd=1e20
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
            cd=travel+distance;break
        step=distance+.001;travel+=step;start=origin-ray*travel
    else:raise RuntimeError('Transparent intersection limit reached')
    state='wood' if wd<cd else 'crown' if cd<1e20 else 'uncovered'
    records.append(dict(pixel=[int(689+x),int(122+y)],first=state,
        wood_distance=wd if wd<1e20 else None,crown_distance=cd if cd<1e20 else None))
    classified[y,x]=dict(wood=(240,70,50,255),crown=(60,210,100,255),uncovered=(230,70,230,255))[state]
Image.fromarray(classified).resize((native.shape[1]*3,native.shape[0]*3),Image.Resampling.NEAREST).save(out/'deferred-first-hit.png')
report=dict(status='MEASURED_CANDIDATE_VISIBILITY_NOT_SEMANTIC_OWNERSHIP',model_sha256=before,
    counts={k:sum(x['first']==k for x in records) for k in ('wood','crown','uncovered')},
    source_pixel_count=len(records),records=records,
    limitation='Inferred crown depth can hide observed bark. These hits locate conflicts for artwork review; they do not authorize removing observed bark or filling it synthetically.')
assert sha(model)==before
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n')
print(report['counts'])
