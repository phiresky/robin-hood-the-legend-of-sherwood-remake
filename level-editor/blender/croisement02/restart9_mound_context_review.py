"""Verify saved leaf-volume pixels and inspect exact terrain support variants."""
from pathlib import Path
import sys,json,hashlib
import bpy
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from restart9_mound_saved_pixel_guard import main as pixel_guard
from restart4_stump_final_contact import frame,sheet
from sign_context_import import append_verified
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 pixel_guard();root=OUT/'restart9-hiding-scatter';worker=root/'mound-support-variants-v1';out=worker/'terrain-context-v1';out.mkdir(exist_ok=False);rec=json.loads((worker/'validation.json').read_text());model=worker/'model.blend';assert sha(model)==rec['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();mounds=[bpy.data.objects[r['object']]for r in rec['records']];pins=[]
 for pin in rec['terrain_models']:
  path=Path(pin['path']);assert sha(path)==pin['sha256'];names=[r['name']for r in pin['objects']];objects,proof=append_verified(scene,path,names,{r['name']:r for r in pin['objects']});pins.append(dict(path=str(path),sha256=sha(path),objects=proof))
 scene.cycles.samples=16;scene.cycles.transparent_max_bounces=512;images=[]
 for instance in ['mission-Emb05_FoB_MP-patch-015','mission-Tac21_FoB_EC-patch-009','mission-Tac21_FoB_EC-patch-010','mission-Tac19_FoB_EC-patch-015']:
  row=next(r for r in rec['records']if instance in r['instances']);obj=bpy.data.objects[row['object']]
  for m in mounds:m.hide_render=m!=obj
  for view,direction in [('native',RAY),('oblique',Vector((COS,0,SIN)))]:
   frame(scene,[obj],direction,512,1.6);file=out/f'{instance}-{view}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);images.append(dict(instance=instance,view=view,path=file.name,sha256=sha(file)))
 sheet([out/r['path']for r in images],out/'eight-contexts.png');(out/'report.json').write_text(json.dumps(dict(model_sha256=sha(model),receivers=pins,images=images,scope='Exact terrain contact views. Foreground receiver audit remains separate.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
