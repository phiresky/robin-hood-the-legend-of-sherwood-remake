"""Test ordinary straight-sided oval chain links against the tight hidden return."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2/winch-chain-loop-prototype-v5';OUT=BASE/'capsule-profile-sweep.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));links=sorted((o for o in bpy.context.scene.objects if o.name.startswith('Complete chain loop link')),key=lambda o:o.name);rows=[];count=512
def capsule(t,rx,ry):
 arc=math.pi*rx;straight=2*(ry-rx)
 if t<arc:
  a=t/rx;return Vector((rx*math.cos(a),ry-rx+rx*math.sin(a),0))
 t-=arc
 if t<straight:return Vector((-rx,ry-rx-t,0))
 t-=straight
 if t<arc:
  a=math.pi+t/rx;return Vector((rx*math.cos(a),-ry+rx+rx*math.sin(a),0))
 t-=arc;return Vector((rx,-ry+rx+t,0))
for rx in (1.2,1.5,1.8):
 for ry in (2.7,3.0,3.3):
  perimeter=2*math.pi*rx+4*(ry-rx);samples=[np.array([tuple(o.matrix_world@capsule(i*perimeter/count,rx,ry)) for i in range(count)]) for o in links];distances=[]
  for a,b in zip(samples,samples[1:]+samples[:1]):distances.append(float(np.sqrt(((a[:,None,:]-b[None,:,:])**2).sum(axis=2).min())))
  lower=min(distances)-perimeter/count;rows.append({'rx':rx,'ry':ry,'minimum_centerline_distance_bound':lower,'max_wire_radius_bound':lower/2})
OUT.write_text(json.dumps({'status':'Static capsule profile diagnostic, no model written','model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'rows':rows},indent=2)+'\n');print(json.dumps(rows,indent=2))
