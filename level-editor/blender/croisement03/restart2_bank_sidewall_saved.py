"""Review the private sidewall correction with unchanged bank and path context."""
import sys,json,math,hashlib,shutil,argparse
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
B=R/'level-editor/work/croisement03-refinement/restart2';I=B/'bank-sidewall-cpu-v1';O=B/'bank-sidewall-saved-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def fingerprint(o):
 m=o.data;return {'vertices':[list(v.co)for v in m.vertices],'faces':[list(f.vertices)for f in m.polygons],'materials':[x.name for x in m.materials],'indices':[f.material_index for f in m.polygons],'uv':[list(v.uv)for v in m.uv_layers.active.data]if m.uv_layers.active else [],'world':[list(row)for row in o.matrix_world],'properties':dict(o.items())}
def resources():
 assert shutil.disk_usage(R).free>10*1024**3;mem={r.split(':')[0]:int(r.split()[1])*1024 for r in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3

def main():
 global O
 ap=argparse.ArgumentParser();ap.add_argument('--output',type=Path,default=O);args=ap.parse_args(sys.argv[sys.argv.index('--')+1:]if '--'in sys.argv else []);O=args.output.resolve()
 resources();acquire(slots=2)
 try:
  resources();O.mkdir(exist_ok=False);plan=json.loads((I/'geometry.json').read_text());assert json.loads((I/'source-firsthit-guard.json').read_text())['status']=='PASS_CPU_SIDEWALL_ANCHORS_AND_BANK_SEEDS';base=B/'bank-whole-prototype-v3/worker.blend';basehash=sha(base);bpy.ops.wm.open_mainfile(filepath=str(base));s=bpy.context.scene;bpy.context.preferences.filepaths.save_version=0;owner=s.objects['Candidate bank 54'];old=owner.data;before={o.name:fingerprint(o)for o in s.objects if o.type=='MESH'and o!=owner};metadata=dict(owner.items());images={im.name:hashlib.sha256(im.packed_file.data).hexdigest()for im in bpy.data.images if im.packed_file};original=json.loads((B/'bank-whole-geometry-plan-v4/geometry.json').read_text())['54'];assert len(old.polygons)==len(original['faces']);assert all(sorted(f.vertices)==sorted(expected)for f,expected in zip(old.polygons,original['faces']))
  revisions={r['old_face']:r['new_faces']for r in plan['sidewall_revision']['changed_faces']};parents=[i for i in range(len(old.polygons))for _ in range(revisions.get(i,1))];d=plan['54'];assert len(parents)==len(d['faces']);vertices=d['vertices'];assert np.max(np.abs(np.array(vertices[:len(old.vertices)])-np.array([v.co for v in old.vertices])))<1e-4
  # Retain old serialized coordinates exactly; new interior controls are inferred.
  vertices=[list(v.co)for v in old.vertices]+vertices[len(old.vertices):];m=bpy.data.meshes.new('Bank54 source-constrained sidewall');m.from_pydata(vertices,[],d['faces']);m.update()
  for mat in old.materials:m.materials.append(mat)
  uv=m.uv_layers.new(name=old.uv_layers.active.name) if old.uv_layers.active else None
  for face,parent in zip(m.polygons,parents):
   ref=old.polygons[parent];face.material_index=ref.material_index
   if uv is None:continue
   oldpoints=np.array([old.vertices[i].co for i in ref.vertices]);projected=np.array([[p[0],-p[1]*S-p[2]*C]for p in oldpoints]);old_uv=np.array([old.uv_layers.active.data[i].uv for i in ref.loop_indices]);direct={vi:old_uv[j]for j,vi in enumerate(ref.vertices)};matrix=np.column_stack([projected[1]-projected[0],projected[2]-projected[0]])
   for li in face.loop_indices:
    vi=m.loops[li].vertex_index
    if vi in direct:value=direct[vi]
    else:
     v=m.vertices[vi].co;p=np.array([v.x,-v.y*S-v.z*C]);a,b=np.linalg.solve(matrix,p-projected[0]);value=old_uv[0]+a*(old_uv[1]-old_uv[0])+b*(old_uv[2]-old_uv[0])
    uv.data[li].uv=value
  bm=bmesh.new();bm.from_mesh(m);assert all(e.is_manifold for e in bm.edges)and all(f.calc_area()>1e-8 for f in bm.faces)and bm.calc_volume()>0;bm.free();owner.data=m;assert dict(owner.items())==metadata
  for name,record in before.items():assert fingerprint(s.objects[name])==record
  model=O/'worker.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));assert model.stat().st_size<8*1024**2;bpy.ops.wm.open_mainfile(filepath=str(model));s=bpy.context.scene
  for name,record in before.items():assert fingerprint(s.objects[name])==record
  assert {im.name:hashlib.sha256(im.packed_file.data).hexdigest()for im in bpy.data.images if im.packed_file}==images
  s.render.threads_mode='FIXED';s.render.threads=2;s.cycles.samples=8;views={f'view-{i}':f'Bank view{i}'for i in range(8)};render_views(s.name,views,O/'views',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(O/'views'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(O/f'{mode}8.png')
  assert sha(base)==basehash;assert sum(p.stat().st_size for p in O.rglob('*')if p.is_file())<32*1024**2;(O/'receipt.json').write_text(json.dumps({'status':'PRIVATE_SAVED_SIDEWALL_REVIEW_PENDING','model_sha256':sha(model),'geometry_sha256':sha(I/'geometry.json'),'baseline_sha256':basehash,'unchanged_objects':sorted(before),'packed_images_exact':True,'gameplay_metadata_exact':True,'source_face_materials_preserved':True,'source_UVs_preserved_by_native_projection':True,'whole_bank_morphology_complete':False},indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
