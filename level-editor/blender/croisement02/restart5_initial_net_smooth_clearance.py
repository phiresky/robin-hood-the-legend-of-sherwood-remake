"""Rebuild thin lifting cords along smooth source-ray clearance envelopes."""
import sys,json,shutil,math
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
from restart5_initial_net_candidate import tube_path
ROOT=OUT/'restart5-initial-nets'
def main(key):
 assert shutil.disk_usage(OUT).free>25*1024**3
 parent=ROOT/f'candidate-v2/profile-{key}';prior=ROOT/f'candidate-v4/profile-{key}';report=json.loads((parent/'report.json').read_text());support=report['support'];bpy.ops.wm.open_mainfile(filepath=support['model']);bpy.context.view_layer.update();obs=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==support['asset_id']and o.get('projection_component')!='crown'and 'crown'not in o.name.lower()];tree,_,_=_tree(obs)
 bpy.ops.wm.open_mainfile(filepath=str(prior/'model.blend'));v4=np.array([tuple(v.co)for v in bpy.data.objects['Initial lifting line'].data.vertices]).reshape(-1,8,3).mean(axis=1)
 bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));bpy.context.preferences.filepaths.save_version=0;o=bpy.data.objects['Initial lifting line'];old=np.array([tuple(v.co)for v in o.data.vertices]).reshape(-1,8,3);centers=old.mean(axis=1);req=np.maximum(0,(v4-centers)@np.array(RAY));ground=_geometry(bpy.data.objects['Ground camouflage net'],True);tie=_geometry(bpy.data.objects['Inferred upper fastening loop'],True);materials=list(o.data.materials);native_y=-centers[:,1]*SIN-centers[:,2]*COS;history=[]
 for iteration in range(40):
  shift=np.maximum(0,np.max(req[None,:]-np.abs(native_y[:,None]-native_y[None,:])*1.35,axis=1));shift[0]=0
  for _ in range(4):shift=np.maximum(shift,np.convolve(np.pad(shift,(2,2),mode='edge'),[1/9,2/9,3/9,2/9,1/9],mode='valid'));shift[0]=0
  path=[Vector(c)+RAY*float(s)for c,s in zip(centers,shift)];vs,fs=tube_path(path);vs[:8]=[tuple(v)for v in old[0]];mesh=bpy.data.meshes.new('Smooth circular initial lifting line');mesh.from_pydata(vs,[],fs);mesh.update();bad=set();crossings=[]
  for e in mesh.edges:
   a,b=[mesh.vertices[i].co for i in e.vertices];delta=b-a
   if delta.length<1e-6:continue
   p,n,i,d=tree.ray_cast(a,delta.normalized(),delta.length)
   if p is not None and 1e-5<d<delta.length-1e-5:bad.update(i//8 for i in e.vertices);crossings.append(e.index)
  history.append(dict(iteration=iteration,crossings=len(crossings),max_ray_shift=float(shift.max())))
  if not bad:break
  assert 0 not in bad,'Would disturb fixed upper attachment'
  for i in bad:req[i]=max(req[i],shift[i]+.8)
  bpy.data.meshes.remove(mesh)
 else:raise ValueError('Smooth envelope did not clear wood')
 o.data=mesh
 for m in materials:mesh.materials.append(m)
 bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume();assert volume>0;bm.to_mesh(mesh);bm.free();mesh.update();uv=mesh.uv_layers.new(name='Initial native source projection');ox,oy=report['source']['origin'];w,h=report['source']['size']
 for p in mesh.polygons:
  p.material_index=0 if p.normal.dot(RAY)>1e-6 else 1
  for li in p.loop_indices:
   v=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((v.x-ox)/w,1-(-v.y*SIN-v.z*COS-oy)/h)
 assert _geometry(bpy.data.objects['Ground camouflage net'],True)==ground;assert _geometry(bpy.data.objects['Inferred upper fastening loop'],True)==tie
 newcenters=np.array(path);project=lambda a:np.c_[a[:,0],-a[:,1]*SIN-a[:,2]*COS];drift=float(np.abs(project(newcenters)-project(centers)).max());assert drift<.001
 out=ROOT/f'candidate-v5/profile-{key}';assert not out.exists();out.mkdir(parents=True);shutil.copyfile(parent/'observed-source.png',out/'observed-source.png');bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));h=sha(out/'model.blend');report.update(model_sha256=h,status='Private circular-line clearance revision; final saved/native/contact review pending',parent_model_sha256=sha(parent/'model.blend'),clearance=dict(history=history,center_native_projection_max_drift=drift,ground_and_tie_exact=True,circular_ring_radius=.65,upper_connection_first_ring_exact=True,source_uv_recomputed=True,scope='Only lifting line rebuilt as constant-radius tube around smoothed native-ray clearance envelope.'));write_json(out/'report.json',report)
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
