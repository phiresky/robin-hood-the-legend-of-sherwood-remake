"""Actual-triangle guards for the connected room and both approved mechanisms."""
import sys,json,hashlib,shutil,math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'restart61-room-connected-contact-v19';assert not OUT.exists();sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
def geometry(o):
 o.data.calc_loop_triangles();v=[o.matrix_world@p.co for p in o.data.vertices];f=[tuple(t.vertices)for t in o.data.loop_triangles];return pack(v,f)
def pack(v,f):
 a=np.array(v);return {'v':a,'f':f,'t':a[f],'lo':a.min(axis=0),'hi':a.max(axis=0),'tree':BVHTree.FromPolygons([Vector(p)for p in v],f,all_triangles=True)}
def seg(p,q,t):
 e1=t[1]-t[0];e2=t[2]-t[0];d=q-p;h=np.cross(d,e2);det=np.dot(e1,h)
 if abs(det)<1e-9:return False
 ss=p-t[0];u=np.dot(ss,h)/det;r=np.cross(ss,e1);v=np.dot(r,d)/det;tt=np.dot(e2,r)/det
 return u>=-1e-7 and v>=-1e-7 and u+v<=1+1e-7 and 1e-6<tt<1-1e-6
def check(a,b):
 if min(np.minimum(a['hi'],b['hi'])-np.maximum(a['lo'],b['lo']))<=.005:return 0
 count=0
 for i,j in a['tree'].overlap(b['tree']):
  x,y=a['t'][i],b['t'][j]
  if any(seg(x[k],x[(k+1)%3],y)or seg(y[k],y[(k+1)%3],x)for k in range(3)):count+=1
 return count
model=W/'restart61-room-interior-v19/model.blend';bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();parts={o.name:geometry(o)for o in bpy.context.scene.objects if o.get('room_receiver')};rows=[]
winch=W/'restart59-approved-winch-assembly-v1/model.blend';bpy.ops.wm.open_mainfile(filepath=str(winch));scene=bpy.context.scene
for frame in range(45):
 scene.frame_set(frame);bpy.context.view_layer.update();deps=bpy.context.evaluated_depsgraph_get();hits=[];count=0
 for obj in scene.objects:
  if obj.type!='MESH'or obj.get('native_patch')!='patch-004':continue
  geo=geometry(obj.evaluated_get(deps));count+=1
  for name,p in parts.items():
   n=check(geo,p)
   if n:hits.append({'object':obj.name,'receiver':name,'triangle_crossings':n})
 rows.append({'mechanism':'winch','frame':frame,'components':count,'hits':hits})
audit=json.loads((W/'gate-saved-contact-audit-v1/report.json').read_text());gv=np.array(audit['geometry_world']['gate_vertices']);gf=audit['geometry_world']['gate_faces'];gids=[(f[0],f[i],f[i+1])for f in gf for i in range(1,len(f)-1)]
for pose in json.loads((W/'jamb-clearance-candidate-v1/motion-proposal.json').read_text())['rows']:
 geo=pack(gv+[0,0,pose['nominal_lift_world_z']],gids);hits=[]
 for name,p in parts.items():
  n=check(geo,p)
  if n:hits.append({'receiver':name,'triangle_crossings':n})
 rows.append({'mechanism':'gate','frame':pose['frame'],'hits':hits})
OUT.mkdir();report={'status':'PASS_NO_TRIANGLE_CROSSINGS'if not any(r['hits']for r in rows)else'HOLD_MECHANISM_CROSSINGS','model_sha256':hashlib.sha256(model.read_bytes()).hexdigest(),'winch_sha256':hashlib.sha256(winch.read_bytes()).hexdigest(),'rows':rows,'limits':'Tests actual triangle crossings after AABB and BVH filtering. Coplanar contact and penetrations <=0.005 world units excluded. Complete solid containment not independently certified.'};(OUT/'report.json').write_text(json.dumps(report,indent=2));print(report['status']);print([r for r in rows if r['hits']][:3])
