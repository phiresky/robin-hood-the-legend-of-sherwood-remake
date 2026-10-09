"""Render saved York texture candidates with unchanged approved neighbor context."""
import hashlib,json,sys,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
import bpy
from mathutils import Matrix
from PIL import Image,ImageDraw
from refinement_workspace import _geometry
from workspace_components import appearance_state
def signature(o):
 a=appearance_state(o)
 def clean(v):
  if isinstance(v,dict):return {k:clean(x)for k,x in v.items()if k not in ('name','filepath','dirty')}
  if isinstance(v,list):return [clean(x)for x in v]
  return v
 return {'vertices':[list(v.co)for v in o.data.vertices],'faces':[list(p.vertices)for p in o.data.polygons],'edges':[list(e.vertices)for e in o.data.edges],'modifiers':[(m.name,m.type,m.show_viewport,m.show_render)for m in o.modifiers],'appearance':clean(a)}
key=sys.argv[sys.argv.index('--')+1];assert key in ('well','stable24')
B=ROOT/'level-editor/work/york-refinement/restart2';E=B/'approved-texture-inputs-v1'/key/'experiment';O=B/f'restart38-{key}-texture-baked-v1';info=json.loads((E.parent/'input-review.json').read_text());names=set(info['object_names']);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();digest=sha(O/'model.blend');source=E/'approved-model.blend'
bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene
if key=='stable24':
 freeze=json.loads((Path(info['approved_scope']['model']).parent/'component-freeze.json').read_text());scene.frame_set(freeze['poses'][44]['tick'])
bpy.context.view_layer.update()
for o in scene.objects:
 if o.type=='MESH'and o.name not in names:
  world=o.matrix_world.copy();o.parent=None;o.matrix_world=world
bpy.context.view_layer.update();context={o.name:{'matrix':[list(r)for r in o.matrix_world],'geometry':signature(o)}for o in scene.objects if o.type=='MESH'and o.name not in names and not o.hide_render}
bpy.ops.wm.open_mainfile(filepath=str(O/'model.blend'));scene=bpy.context.scene;before={o.name:_geometry(o,protect_appearance=True)for o in scene.objects};scene.render.threads_mode='FIXED';scene.render.threads=2
with bpy.data.libraries.load(str(source),link=False)as(fr,to):to.objects=list(context)
for o in to.objects:
 scene.collection.objects.link(o)
bpy.context.view_layer.update()
for o in to.objects:
 o.parent=None;o.matrix_world=Matrix(context[o.name]['matrix']);o.hide_render=False
bpy.context.view_layer.update()
for o in to.objects:
 assert signature(o)==context[o.name]['geometry'],(o.name,[k for k in signature(o)if signature(o)[k]!=context[o.name]['geometry'][k]])
 assert max(abs(o.matrix_world[r][c]-context[o.name]['matrix'][r][c])for r in range(4)for c in range(4))<1e-4
manifest=json.loads((E/'views.json').read_text());folder=O/'contact';folder.mkdir(exist_ok=False);scene.render.resolution_x=320;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.film_transparent=True
cam=bpy.data.objects.new('Saved texture context camera',bpy.data.cameras.new('Saved texture context camera'));scene.collection.objects.link(cam);scene.camera=cam;cam.data.type='ORTHO';cam.data.clip_start=.01;cam.data.clip_end=20000
sheet=Image.new('RGB',(1280,408),(35,40,45));draw=ImageDraw.Draw(sheet)
for j,i in enumerate((0,2,4,6)):
 v=manifest['views'][i];cam.matrix_world=Matrix(v['camera_matrix_world']);cam.data.ortho_scale=v['ortho_scale']*1.3;scene.render.filepath=str(folder/f'view-{i}.png');bpy.ops.render.render(write_still=True);pic=Image.open(scene.render.filepath).convert('RGBA');sheet.paste(pic,(j*320,0),pic);draw.text((j*320+4,388),f'Approved context; view {i}',fill='white')
sheet.save(folder/'contact-four.png')
assert all(_geometry(scene.objects[n],protect_appearance=True)==g for n,g in before.items());assert sha(O/'model.blend')==digest
(folder/'validation.json').write_text(json.dumps({'status':'PASS','saved_model_sha256':digest,'approved_context_model_sha256':sha(source),'context_objects':list(context),'context_geometry_uv_materials_exact':True,'context_id_names_ignored_in_appearance_comparison':True,'context_world_matrix_tolerance':1e-4,'context_world_matrix_max_error':max((abs(o.matrix_world[r][c]-context[o.name]['matrix'][r][c])for o in to.objects for r in range(4)for c in range(4)),default=0),'candidate_unchanged':True,'scope':'Read-only appearance context; no new geometry, mechanism or runtime approval.'},indent=2)+'\n')
