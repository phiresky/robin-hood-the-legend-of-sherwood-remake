"""Correct only lifting-line depth clearance against its unchanged supporting wood."""
import sys,json,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import RAY,SIN,COS
from refinement_review import _tree
from refinement_workspace import _geometry
from render_slots import acquire,release
ROOT=OUT/'restart5-initial-nets'
def main(key):
 assert shutil.disk_usage(OUT).free>25*1024**3
 parent=ROOT/f'candidate-v2/profile-{key}';report=json.loads((parent/'report.json').read_text());assert sha(parent/'model.blend')==report['model_sha256'];support=report['support'];bpy.ops.wm.open_mainfile(filepath=support['model']);bpy.context.view_layer.update();obs=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==support['asset_id']and o.get('projection_component')!='crown'and 'crown'not in o.name.lower()];tree,_,_=_tree(obs)
 bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));bpy.context.preferences.filepaths.save_version=0;o=bpy.data.objects['Initial lifting line'];old=np.array([tuple(v.co)for v in o.data.vertices]);ground=_geometry(bpy.data.objects['Ground camouflage net'],True);tie=_geometry(bpy.data.objects['Inferred upper fastening loop'],True);uvs={u.name:[list(x.uv)for x in u.data]for u in o.data.uv_layers}
 def clearance(p):
  native=-p.y*SIN-p.z*COS;origin=Vector((p.x,-native/SIN,0))+RAY*6000;hit,n,i,d=tree.ray_cast(origin,-RAY)
  return None if hit is None else (p-hit).dot(RAY)
 for v in o.data.vertices:
  if v.index<8:continue
  d=clearance(v.co)
  if d is not None and d<.4:v.co+=RAY*(.4-d)
 history=[]
 for iteration in range(60):
  crossing=[];bad=set()
  for e in o.data.edges:
   a,b=[o.data.vertices[i].co for i in e.vertices];delta=b-a
   if delta.length<1e-6:continue
   p,n,i,dist=tree.ray_cast(a,delta.normalized(),delta.length)
   if p is not None and 1e-5<dist<delta.length-1e-5:crossing.append(e.index);bad.update(e.vertices)
  for face in o.data.polygons:
   p=sum((o.data.vertices[i].co for i in face.vertices),Vector())/len(face.vertices);d=clearance(p)
   if d is not None and d<.15:bad.update(face.vertices)
  history.append(dict(iteration=iteration,edge_crossings=len(crossing),vertices_needing_clearance=len(bad)))
  if not bad:break
  assert not any(i<8 for i in bad),'Correction would detach exact reviewed tie contact'
  for i in bad:o.data.vertices[i].co+=RAY*.35
 else:raise ValueError('Bounded clearance did not converge')
 new=np.array([tuple(v.co)for v in o.data.vertices]);native=lambda a:np.c_[a[:,0],-a[:,1]*SIN-a[:,2]*COS];drift=float(np.abs(native(new)-native(old)).max());assert drift<.001
 assert _geometry(bpy.data.objects['Ground camouflage net'],True)==ground;assert _geometry(bpy.data.objects['Inferred upper fastening loop'],True)==tie;assert {u.name:[list(x.uv)for x in u.data]for u in o.data.uv_layers}==uvs
 bm=bmesh.new();bm.from_mesh(o.data);assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume();assert volume>0;bm.free();o.data.update();out=ROOT/f'candidate-v3/profile-{key}';assert not out.exists();out.mkdir(parents=True);shutil.copyfile(parent/'observed-source.png',out/'observed-source.png');bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));h=sha(out/'model.blend')
 report.update(model_sha256=h,status='Private source-preserving lifting-line clearance derivative; actual contact review pending',parent_model_sha256=sha(parent/'model.blend'),clearance=dict(history=history,native_projection_max_drift=drift,max_vertex_shift=float(np.linalg.norm(new-old,axis=1).max()),ground_and_tie_exact=True,original_line_uv_exact=True,scope='Only lifting line vertices move along native source rays. Exact first ring remains attached to unchanged upper loop.'));write_json(out/'report.json',report)
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
