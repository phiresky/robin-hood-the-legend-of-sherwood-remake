"""Render the saved appearance transfer with the already reviewed source-first cameras."""
import sys,json,hashlib,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];BASE=ROOT/'level-editor/work/york-refinement/restart2';WORK=BASE/'restart59-approved-winch-assembly-v1';OUT=WORK/'review'
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
def main():
 assert not OUT.exists();assert shutil.disk_usage(ROOT).free>=10*1024**3
 assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024>=6*1024**3
 assert sha(WORK/'model.blend')==json.loads((WORK/'report.json').read_text())['model_sha256']
 import bpy
 from mathutils import Matrix
 from PIL import Image
 from render_views import render_views
 bpy.ops.wm.open_mainfile(filepath=str(WORK/'model.blend'));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.resolution_x=224;scene.render.resolution_y=320;scene.render.resolution_percentage=100
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('native_patch')!='patch-004'
 OUT.mkdir();records=[]
 for frame in [0,36,44]:
  scene.frame_set(frame*2);bpy.context.view_layer.update();manifest=BASE/f'winch-supported-motion-v1/review/pose{0 if frame==0 else 36:02d}/views.json';views=json.loads(manifest.read_text())['views'];cams={}
  for row in views:
   camera=scene.objects['Winch'+str(row['index'])];camera.matrix_world=Matrix(row['camera_matrix_world']);camera.data.ortho_scale=row['ortho_scale'];cams['view-'+str(row['index'])]=camera.name
  folder=OUT/f'pose{frame:02d}';render_views(scene.name,cams,folder,modes=('textured',),width=224)
  sheet=Image.new('RGBA',(896,640))
  for i in range(8):sheet.paste(Image.open(folder/f'view-{i}-textured.png'),((i%4)*224,(i//4)*320))
  sheet.save(folder/'textured-eight.png');records.append({'frame':frame,'camera_manifest':str(manifest),'camera_sha256':sha(manifest),'sheet_sha256':sha(folder/'textured-eight.png')})
 (OUT/'report.json').write_text(json.dumps({'model_sha256':sha(WORK/'model.blend'),'poses':records,'source_camera_top_left':True,'scope':'Saved approved stable24 appearance on approved motion; chain/support textures still pending. No new approval inferred.'},indent=2)+'\n')
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
