"""Transfer only approved stable-component appearance into the approved moving assembly."""
import sys,json,hashlib,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2';OUT=BASE/'restart59-approved-winch-assembly-v1'
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
 assert not OUT.exists();assert shutil.disk_usage(ROOT).free>=10*1024**3
 assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024>=6*1024**3
 motion=BASE/'winch-supported-motion-v1/model.blend';texture=BASE/'restart38-stable24-texture-baked-v1/model.blend'
 assert sha(motion)=='ebe41b23e0b4f3695eace56ed8fdaf57b4f03eb193932a737b66cee18e2f2422'
 assert sha(texture)=='e10576da59903ad099fbe0382ba3e28f07388458204e28ae20d086312675b8b6'
 names=json.loads((BASE/'approved-texture-inputs-v1/stable24/input-review.json').read_text())['object_names'];assert len(names)==24
 import bpy
 from refinement_workspace import _geometry
 bpy.ops.wm.open_mainfile(filepath=str(motion));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2
 originals={o.name:o for o in scene.objects};assert all(n in originals for n in names)
 def poses():
  result=[]
  for frame in range(45):
   scene.frame_set(frame*2);bpy.context.view_layer.update();result.append({n:_geometry(o) for n,o in originals.items()})
  return result
 before=poses();scene.frame_set(88);bpy.context.view_layer.update()
 outside={n:_geometry(o,protect_appearance=True) for n,o in originals.items() if n not in names}
 with bpy.data.libraries.load(str(texture),link=False) as (source,dest):
  assert all(n in source.objects for n in names);dest.objects=list(names)
 donors=dict(zip(names,dest.objects));rows=[]
 for name in names:
  target,donor=originals[name],donors[name];assert target.type==donor.type=='MESH'
  a,b=target.data,donor.data
  assert [tuple(v.co) for v in a.vertices]==[tuple(v.co) for v in b.vertices],name+' local vertices differ'
  assert [tuple(p.vertices) for p in a.polygons]==[tuple(p.vertices) for p in b.polygons],name+' topology differs'
  assert len(a.loops)==len(b.loops)
  a.materials.clear()
  for mat in b.materials:a.materials.append(mat)
  for dst,src in zip(a.polygons,b.polygons):dst.material_index=src.material_index
  while a.uv_layers:a.uv_layers.remove(a.uv_layers[0])
  for layer in b.uv_layers:
   copied=a.uv_layers.new(name=layer.name)
   for dst,src in zip(copied.data,layer.data):dst.uv=src.uv
   copied.active_render=layer.active_render
  if b.uv_layers.active:a.uv_layers.active_index=b.uv_layers.active_index
  rows.append({'name':name,'vertices':len(a.vertices),'faces':len(a.polygons),'materials':[m.name for m in a.materials],'uv_layers':[l.name for l in a.uv_layers]})
 for donor in donors.values():bpy.data.objects.remove(donor,do_unlink=True)
 assert before==poses(),'Approved motion geometry changed'
 scene.frame_set(88);bpy.context.view_layer.update();assert outside=={n:_geometry(o,protect_appearance=True) for n,o in originals.items() if n not in names},'Other appearance changed'
 OUT.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True)
 (OUT/'report.json').write_text(json.dumps({'status':'APPROVED_STABLE24_TRANSFERRED_INTO_APPROVED_MOTION_PRIVATE','model_sha256':sha(OUT/'model.blend'),'motion_source':{'path':str(motion),'sha256':sha(motion)},'texture_source':{'path':str(texture),'sha256':sha(texture)},'components':rows,'geometry_all45poses_exact':True,'outside_appearance_exact':True,'remaining':['Chain/support hidden textures remain pending','All-view saved appearance and native source comparison required','Independent room cover and engine integration excluded']},indent=2)+'\n')
 print(str(OUT),flush=True)
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
