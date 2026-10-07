"""Resolve broad-phase contact candidates with exact segment-triangle tests."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/winch-complete-chain-motion-v1';OUT=BASE/'depth-contact-sweep.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;motion=json.loads((BASE/'motion.json').read_text());links=[o for o in scene.objects if o.name.startswith('Complete chain loop link')];wood=[o for o in scene.objects if o.type=='MESH' and o.get('native_patch')=='patch-004' and o not in links and not o.name.startswith('Inferred upper')];s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));rows=[]
def tree(objects,shift=Vector((0,0,0))):
 vertices=[];faces=[];owners=[]
 for o in objects:
  offset=len(vertices);vertices.extend(o.matrix_world@v.co+shift for v in o.data.vertices);o.data.calc_loop_triangles()
  for f in o.data.loop_triangles:faces.append(tuple(offset+i for i in f.vertices));owners.append(o.name)
 array=np.asarray(vertices);return BVHTree.FromPolygons(vertices,faces,all_triangles=True),array[np.asarray(faces)],owners
def crosses(a,b):
 e1=b[:,1]-b[:,0];e2=b[:,2]-b[:,0];hit=np.zeros(len(a),dtype=bool)
 for i in range(3):
  origin=a[:,i];direction=a[:,(i+1)%3]-origin;p=np.cross(direction,e2);det=np.einsum('ij,ij->i',e1,p);valid=abs(det)>1e-8;inv=np.zeros_like(det);inv[valid]=1/det[valid];t=origin-b[:,0];u=np.einsum('ij,ij->i',t,p)*inv;q=np.cross(t,e1);v=np.einsum('ij,ij->i',direction,q)*inv;d=np.einsum('ij,ij->i',e2,q)*inv;hit|=valid&(u>=-1e-6)&(v>=-1e-6)&(u+v<=1+1e-6)&(d>1e-6)&(d<1-1e-6)
 return hit
for index in (0,22,36,44):
 scene.frame_set(motion['rows'][index]['tick']);bpy.context.view_layer.update();body,bt,owners=tree(wood)
 for depth in (-12,-9,-6,-3,0,3,6,9,12):
  chain,ct,_=tree(links,back*depth);pairs=chain.overlap(body);counts={}
  if pairs:
   ai,bi=np.asarray(pairs).T;mask=crosses(ct[ai],bt[bi])|crosses(bt[bi],ct[ai])
   for owner in np.asarray(owners)[bi[mask]]:counts[str(owner)]=counts.get(str(owner),0)+1
  rows.append({'frame':index,'view_ray_depth_delta_world':depth,'native_projection_unchanged':True,'minimum_chain_game_z':float(ct[:,:,2].min()*c),'exact_crossing_triangle_pairs':counts})
OUT.write_text(json.dumps({'status':'Private depth diagnostic; exact edge crossings, does not detect complete containment or prove attachment','model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'rows':rows},indent=2)+'\n');print(json.dumps(rows,indent=2))
