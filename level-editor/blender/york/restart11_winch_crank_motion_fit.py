"""Fit eight-spoke crank silhouette phase across every native transition frame."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';OUT=WORK/'restart2/winch-crank-motion-fit-v2'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
import numpy as np
from mathutils import Vector,Matrix,Quaternion
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageOps,ImageFilter
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s))
def world(x,y,z):return Vector((x,-y/s,z/c))
def tree(objects):
 vs=[];fs=[]
 for o in objects:
  off=len(vs);vs.extend(o.matrix_world@v.co for v in o.data.vertices);fs.extend(tuple(off+i for i in f.vertices) for f in o.data.polygons)
 return BVHTree.FromPolygons(vs,fs)
model_path=WORK/'restart2/winch-room-physical-v10/transition-44/model.blend';bpy.ops.wm.open_mainfile(filepath=str(model_path));bpy.context.view_layer.update();scene=bpy.context.scene
parts=[o for o in scene.objects if o.type=='MESH' and not o.hide_render and o.get('native_patch')=='patch-004'];spokes=sorted((o for o in parts if o.name.startswith('Crank spoke')),key=lambda o:o.name);assert len(spokes)==8;original_spokes={o.name:o.matrix_world.copy() for o in spokes}
context=tree([o for o in scene.objects if o.type=='MESH' and not o.hide_render and o not in parts]);fixed=tree([o for o in parts if o not in spokes]);points=[]
# This independent fixed crop includes the exposed crank and its base and
# excludes the travelling part. Static support pixels remain in every score.
for y in range(940,979):
 for x in range(2408,2434):
  origin=Vector((x+.5,-(y+.5)/s,0))+back*10000;ch=context.ray_cast(origin,-back);fh=fixed.ray_cast(origin,-back);depth=ch[3] if ch[0] is not None else 1e20
  points.append((x,y,origin,depth,fh[0] is not None and fh[3]<depth-1e-5))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();lateral=axis.cross(Vector((0,0,1))).normalized();rasters=[]
for phase in range(45):
 rotation=Matrix.Translation(center)@Quaternion(axis,-math.radians(phase+10)).to_matrix().to_4x4()@Matrix.Translation(-center)
 for o in spokes:o.matrix_world=rotation@original_spokes[o.name]
 bpy.context.view_layer.update();mesh=tree(spokes);hits=[]
 for x,y,origin,depth,already in points:
  hit=mesh.ray_cast(origin,-back);hits.append(already or (hit[0] is not None and hit[3]<depth-1e-5))
 rasters.append(hits)
rasters=np.asarray(rasters,dtype=bool);source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');rows=[];costs=[]
for i,f in enumerate(frames):
 fp=source/f['image'];assert hashlib.sha256(fp.read_bytes()).hexdigest()==f['sha256'];alpha=Image.open(fp).getchannel('A');core=ImageOps.expand(alpha,border=1,fill=0).filter(ImageFilter.MinFilter(3)).crop((1,1,alpha.width+1,alpha.height+1));opaque=[];cores=[]
 for x,y,*_ in points:
  xx,yy=x-f['bbox'][0],y-f['bbox'][1];inside=0<=xx<alpha.width and 0<=yy<alpha.height;opaque.append(inside and alpha.getpixel((xx,yy))>=128);cores.append(inside and core.getpixel((xx,yy))>=128)
 opaque=np.array(opaque);cores=np.array(cores);miss=((~rasters)&opaque).sum(axis=1);cmiss=((~rasters)&cores).sum(axis=1);extra=(rasters&~opaque).sum(axis=1);score=miss+extra+3*cmiss;costs.append(score);best=int(np.argmin(score));rows.append({'frame':i,'source_sha256':f['sha256'],'independent_best_phase_mod45':best,'candidates':[{'phase_degrees_mod45':p,'opaque_missed':int(miss[p]),'core_missed':int(cmiss[p]),'empty_filled':int(extra[p]),'score':int(score[p])} for p in range(45)]})
# Limited inter-frame travel resolves eightfold silhouette equivalence without
# pretending that the source gives a unique crank spoke identity.
costs=np.array(costs,dtype=float);dp=costs[0].copy();parents=[]
for i in range(1,45):
 nd=np.full(45,np.inf);parent=np.full(45,-1,dtype=int)
 for q in range(45):
  for p in range(45):
   step=(q-p+22)%45-22
   if abs(step)>12:continue
   value=dp[p]+costs[i,q]+.06*step*step
   if value<nd[q]:nd[q]=value;parent[q]=p
 parents.append(parent);dp=nd
q=int(np.argmin(dp));path=[q]
for parent in reversed(parents):q=int(parent[q]);path.append(q)
path.reverse();unwrapped=[float(path[0])]
for a,b in zip(path,path[1:]):unwrapped.append(unwrapped[-1]+(b-a+22)%45-22)
for r,p,u in zip(rows,path,unwrapped):r['smooth_phase_mod45']=p;r['unwrapped_phase_degrees']=u
OUT.mkdir();(OUT/'fit.json').write_text(json.dumps({'status':'Private source silhouette fit diagnostic; no animation saved or approval','source_model_sha256':hashlib.sha256(model_path.read_bytes()).hexdigest(),'crop':[2408,940,2434,979],'objective':'Opaque missed + empty filled +3*core missed; temporal penalty0.06*phase_step², maximum12degrees/frame. These are inference regularizers, not native physical constraints.','eightfold_phase_ambiguity':True,'frames':rows},indent=2)+'\n');print(json.dumps({'independent':[r['independent_best_phase_mod45'] for r in rows],'smooth':unwrapped}))
