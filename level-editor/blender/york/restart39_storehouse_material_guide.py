"""Render a diagnostic material-region guide from exact approved storehouse geometry."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
import bpy
from mathutils import Matrix
from PIL import Image
from refinement_workspace import _geometry
B=ROOT/'level-editor/work/york-refinement/restart2/approved-texture-inputs-v1/storehouse';E=B/'experiment';O=B/'material-guide-v4';O.mkdir(exist_ok=False);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();a=json.loads((E/'approval.json').read_text());m=json.loads((E/'views.json').read_text());model=E/'approved-model.blend';assert sha(model)==a['saved_model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;before={o.name:_geometry(o)for o in scene.objects};own=set(m['render_object_names']);roles={}
for o in scene.objects:
 if o.type!='MESH':continue
 o.hide_render=o.name not in own
 if o.name not in own:continue
 role='roof'if ('roof shell'in o.name or 'hip shell'in o.name) else 'chimney'if 'chimney'in o.name else 'wall';roles[o.name]=role
 mat=bpy.data.materials.new('DIAGNOSTIC '+role);mat.diffuse_color={'roof':(.8,.08,.12,1),'wall':(.05,.65,.2,1),'chimney':(.1,.25,.9,1)}[role];o.color=mat.diffuse_color;o.data.materials.clear();o.data.materials.append(mat)
 for p in o.data.polygons:p.material_index=0
assert list(roles.values()).count('roof')==4 and list(roles.values()).count('wall')==1 and list(roles.values()).count('chimney')==1
scene.render.engine='BLENDER_WORKBENCH';scene.display.shading.light='FLAT';scene.display.shading.color_type='OBJECT';scene.display.shading.show_shadows=False;scene.display.shading.show_cavity=False;scene.display.shading.show_specular_highlight=False;scene.render.film_transparent=True;scene.render.resolution_x=320;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.view_settings.view_transform='Standard';scene.view_settings.look='None'
cam=bpy.data.objects.new('Approved guide camera',bpy.data.cameras.new('Approved guide camera'));scene.collection.objects.link(cam);scene.camera=cam;cam.data.type='ORTHO';cam.data.clip_start=.01;cam.data.clip_end=20000;sheet=Image.new('RGB',(1280,768),(0,0,0))
for v in m['views']:
 cam.matrix_world=Matrix(v['camera_matrix_world']);cam.data.ortho_scale=v['ortho_scale'];scene.render.filepath=str(O/f"view-{v['index']}.png");bpy.ops.render.render(write_still=True);im=Image.open(scene.render.filepath).convert('RGBA');sheet.paste(im,(v['crop']['left'],v['crop']['top']),im)
sheet.save(O/'region-guide.png');assert sha(model)==a['saved_model_sha256'];(O/'guide.json').write_text(json.dumps({'status':'DIAGNOSTIC_GUIDE_ROOT_REVIEW_PENDING','approved_model_sha256':sha(model),'approved_views_sha256':sha(E/'views.json'),'dimensions':[1280,768],'roles':roles,'legend':{'red':'All four sloped hip-roof shells: terracotta roof tiles, never masonry.','green':'Closed masonry body: uninterrupted blank stone on unknown walls; keep only existing protected openings.','blue':'Existing closed chimney: masonry.'},'region_guide_sha256':sha(O/'region-guide.png'),'scope':'Ordinary supplementary image only; original input, lighting, editable mask, approved geometry and source protection remain unchanged. No model saved.'},indent=2)+'\n')
