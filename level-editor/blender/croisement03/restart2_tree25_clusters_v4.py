"""Break the cluster lattice while retaining exact native-ray projection."""
import hashlib,json,math,random,shutil,sys
from pathlib import Path
import bpy
from mathutils import Vector,Quaternion
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from workspace_components import appearance_state
from refinement_workspace import _geometry

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()

def main():
 e=ROOT/'level-editor/work/croisement03-refinement/restart2/texture-batch-v7/croisement03-tree-25/experiment'
 source=e/'cluster-geometry-v3';out=e/'cluster-geometry-v4';assert not out.exists();assert shutil.disk_usage(ROOT).free>25*1024**3
 report=json.loads((source/'construction.json').read_text());assert sha(source/'worker.blend')==report['model_sha256'];acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source/'worker.blend'));bpy.context.preferences.filepaths.save_version=0
  o=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-tree-25');m=o.data
  state=appearance_state(o);outside={x.name:_geometry(x,protect_appearance=True) for x in bpy.data.objects if x.type=='MESH' and x!=o}
  slots={i for i,mat in enumerate(m.materials) if mat and mat.name.startswith('Tree25 small clusters /')}
  fs=[f for f in m.polygons if f.material_index in slots];assert len(fs)%2==0
  wood_indices={v for f in m.polygons if f.material_index not in slots for v in f.vertices};wood={i:tuple(m.vertices[i].co) for i in wood_indices}
  ray=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))));up=Vector((0,-math.sin(math.radians(35)),-math.cos(math.radians(35))))
  uv=m.uv_layers['Foliage UV'];patches=[];minimum={};seen=set()
  for a,b in zip(fs[::2],fs[1::2]):
   ids=sorted(set(a.vertices)|set(b.vertices));assert len(ids)==4 and not seen.intersection(ids);seen.update(ids)
   coords=[uv.data[i].uv for f in (a,b) for i in f.loop_indices]
   key=tuple(round(v,6) for v in (min(t.x for t in coords),min(t.y for t in coords),max(t.x for t in coords),max(t.y for t in coords)))
   center=sum((o.matrix_world@m.vertices[i].co for i in ids),Vector())/4
   normal=o.matrix_world.to_3x3()@a.normal;cross=abs(normal.dot(ray))<1e-5
   patches.append((ids,key,center,cross));minimum[key]=min(minimum.get(key,math.inf),center.y)
  inverse=o.matrix_world.inverted();cross_count=0;front_count=0
  for ids,key,center,cross in patches:
   seed=int(hashlib.sha256(repr(key).encode()).hexdigest()[:12],16);rng=random.Random(seed)
   shift=ray*rng.uniform(0,6) if center.y<=minimum[key]+.15 else Vector()
   if cross:
    rng=random.Random(seed+round(center.y*10));rotation=Quaternion(ray,rng.uniform(-math.pi,math.pi))
    shift+=Vector((rng.uniform(-2.2,2.2),0,0))+up*rng.uniform(-2.2,2.2)+ray*rng.uniform(-2,2)
    for i in ids:m.vertices[i].co=inverse@(center+shift+rotation@(o.matrix_world@m.vertices[i].co-center))
    cross_count+=1
   elif shift.length:
    for i in ids:m.vertices[i].co=inverse@(o.matrix_world@m.vertices[i].co+shift)
    front_count+=1
  m.update();assert wood=={i:tuple(m.vertices[i].co) for i in wood_indices}
  assert appearance_state(o)==state
  assert outside=={x.name:_geometry(x,protect_appearance=True) for x in bpy.data.objects if x.name in outside}
  ps=[o.matrix_world@v.co for v in m.vertices];bounds=[[min(p[i] for p in ps),max(p[i] for p in ps)] for i in range(3)]
  assert bounds[1][1]-bounds[1][0]>=bounds[0][1]-bounds[0][0]
  out.mkdir();bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True)
  shutil.copyfile(source/'native-samples.npz',out/'native-samples.npz');shutil.copyfile(source/'original-source-proof.json',out/'original-source-proof.json')
  report.update(model_sha256=sha(out/'worker.blend'),previous_cluster_model_sha256=sha(source/'worker.blend'),after_bounds=bounds,
   jittered_front_patches=front_count,jittered_crossing_planes=cross_count,
   geometry_polish='Front-depth variation along native rays; inferred crossing-plane rotations about native ray and small centre jitter. Source RGBA/UV/topology/wood unchanged.')
  (out/'construction.json').write_text(json.dumps(report,indent=2)+'\n');print(dict(model_sha256=report['model_sha256'],front=front_count,cross=cross_count,bounds=bounds))
 finally:release()
if __name__=='__main__':main()
