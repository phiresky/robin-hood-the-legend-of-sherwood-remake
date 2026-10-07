"""Inspect leaf endpoints with the pinned static neighbors that share their native footprint."""
from pathlib import Path
import sys,json,hashlib,math
import bpy,numpy as np
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from leaf_state_scene_context import load_scene
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 root=OUT/'restart9-hiding-scatter';out=root/'neighborhood-context-v2';out.mkdir(exist_ok=False);mound=root/'mound-support-variants-v2';scatter=root/'scatter-surfaces-v2';mr=json.loads((mound/'validation.json').read_text());sr=json.loads((scatter/'manifest.json').read_text())
 for worker in [mound,scatter]:assert json.loads((worker/'saved-pixel-guard-v1/report.json').read_text())['status']=='PASS'
 scene,static,pins,base=load_scene();bpy.context.view_layer.update();bounds={}
 for obj in static:
  p=np.array([obj.matrix_world@Vector(v)for v in obj.bound_box]);q=np.column_stack((p[:,0],-p[:,1]*SIN-p[:,2]*COS));bounds[obj]=(q.min(0),q.max(0))
 mapping={};state_objects=[]
 for worker,rec,names in [(mound,mr,[r['object']for r in mr['records']]),(scatter,sr,[n for r in sr['records']for n in r['objects']])]:
  path=worker/'model.blend';assert sha(path)==rec['model_sha256']
  with bpy.data.libraries.load(str(path),link=False)as(src,dst):dst.objects=list(names)
  for name,obj in zip(names,dst.objects):scene.collection.objects.link(obj);mapping[(worker.name,name)]=obj;state_objects.append(obj)
 bpy.context.view_layer.update();scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';images=[]
 ids=['mission-Emb05_FoB_MP-patch-013','mission-Tac02_FoB_EC-patch-022','mission-Tac21_FoB_EC-patch-010','mission-Tac19_FoB_EC-patch-000']
 for instance in ids:
  initial=next((r for r in mr['records']if instance in r['instances']),None);applied=next(r for r in sr['records']if instance in r['instances']);x,y,w,h=applied['bbox'];neighbors=[obj for obj,(lo,hi)in bounds.items()if hi[0]>=x-60 and lo[0]<=x+w+60 and hi[1]>=y-60 and lo[1]<=y+h+60]
  for state in ['initial','applied']:
   own=([mapping[(mound.name,initial['object'])]]if initial else [])if state=='initial'else[mapping[(scatter.name,n)]for n in applied['objects']];framing=own or [mapping[(scatter.name,n)]for n in applied['objects']]
   for obj in scene.objects:
    if obj.type=='MESH':obj.hide_render=obj not in neighbors and obj not in own
   for view,direction in [('native',RAY),('side',Vector((math.cos(math.radians(12)),0,math.sin(math.radians(12)))) )]:
    camera=frame(scene,framing,direction,512,1.8);file=out/f'{instance}-{state}-{view}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);images.append(dict(instance=instance,state=state,view=view,path=file.name,sha256=sha(file),camera_matrix=[list(r)for r in camera.matrix_world],neighbors=[o.name for o in neighbors]))
 sheet([out/r['path']for r in images],out/'sixteen-contexts.png');(out/'report.json').write_text(json.dumps(dict(base_sha256=sha(base),substitutions=pins,mound_sha256=mr['model_sha256'],scatter_sha256=sr['model_sha256'],images=images,scope='All native-overlap neighbors plus60pixels margin; native first-hit audit covers complete frozen scene. Side12degree diagnostic is a neighborhood view, not whole-scene oblique parity. Original artwork remains separate native presentation authority.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
