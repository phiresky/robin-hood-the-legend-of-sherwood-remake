"""Review the saved support candidate in native-first isolated and room views."""
import json,runpy,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];VERSION=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'winch-room-physical-v9';BASE=ROOT/'level-editor/work/york-refinement/restart2'/VERSION
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from PIL import Image
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
sys.path.insert(0,str(Path(__file__).parent))
from restart2_camera_audit import audit_manifest,labeled_copy
for state in ('transition-00','transition-44'):
 bpy.ops.wm.open_mainfile(filepath=str(BASE/state/'model.blend'));scene=bpy.context.scene
 out=BASE/state/'isolated';assert not out.exists();out.mkdir()
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('native_patch')!='patch-004'
 names={f'view-{i}':f'Winch{i}' for i in range(8)};rows=[]
 for i in range(8):
  cam=bpy.data.objects[f'Winch{i}'];rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(r) for r in cam.matrix_world],'ortho_scale':cam.data.ortho_scale})
 (out/'views.json').write_text(json.dumps({'layout':{'columns':4,'rows':2},'views':rows}));audit_manifest(out/'views.json')
 scene.render.resolution_y=384;render_views(scene.name,names,out/'renders',modes=('textured',),width=320)
 sheet=Image.new('RGBA',(1280,768))
 for i in range(8):sheet.paste(Image.open(out/f'renders/view-{i}-textured.png'),((i%4)*320,(i//4)*384))
 sheet.save(out/'solid8.png');labeled_copy(out/'solid8.png',out/'solid8-native-labeled.png')
 sys.argv=['restart2_winch_room_review.py','--',VERSION,state];runpy.run_path(str(Path(__file__).with_name('restart2_winch_room_review.py')),run_name='__main__')
