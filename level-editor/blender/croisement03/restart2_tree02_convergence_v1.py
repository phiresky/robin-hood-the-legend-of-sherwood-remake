"""Prove native and reverse foliage transparency budgets converge on saved geometry."""
import hashlib,json,sys,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 e=ROOT/'level-editor/work/croisement03-refinement/restart2';p=e/sys.argv[sys.argv.index('--')+1];out=p/'transparency-convergence';assert not out.exists();assert shutil.disk_usage(ROOT).free>10*1024**3+32*1024**2
 acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(p/'worker.blend'));manifest={};scene=bpy.data.scenes['Tree02 isolated'];assert scene.render.engine=='CYCLES';scene.render.threads_mode='FIXED';scene.render.threads=2;scene.view_layers.update();points=[o.matrix_world@v.co for o in scene.objects if o.type=='MESH' for v in o.data.vertices]
  for c in [o for o in scene.objects if o.type=='CAMERA']:
   inv=c.matrix_world.inverted();depth=[-(inv@v).z for v in points];c.data.clip_start=max(.1,min(depth)-25);c.data.clip_end=max(depth)+25
  old=scene.cycles.transparent_max_bounces;out.mkdir();mp=out/'views.json';mp.write_text(json.dumps(manifest));scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA';records=[];previous=None
  for budget in (32,64,128,256):
   scene.cycles.transparent_max_bounces=budget;dest=out/f'budget-{budget}';render_views(scene.name,{'view-0':'Tree02 view0','view-4':'Tree02 view4'},dest,modes=('textured',),width=384);current={i:np.array(Image.open(dest/f'view-{i}-textured.png').convert('RGBA')).astype(int) for i in (0,4)};diff={}
   if previous is not None:
    for i in current:
     d=np.abs(current[i]-previous[i]);diff[str(i)]={'changed_pixels':int(np.any(d,axis=2).sum()),'max_channel_delta':int(d.max()),'mean_channel_delta':float(d.mean())}
   records.append({'budget':budget,'images':{str(i):sha(dest/f'view-{i}-textured.png') for i in current},'previous_comparison':diff});previous=current
   if diff and all(v['max_channel_delta']==0 for v in diff.values()):break
  passed=bool(diff) and all(v['max_channel_delta']==0 for v in diff.values());(out/'receipt.json').write_text(json.dumps({'status':'PASS' if passed else 'HOLD','model_sha256':sha(p/'worker.blend'),'saved_scene_budget':old,'views':[0,4],'native_view_index':0,'records':records,'criterion':'Byte-identical RGBA for native and reverse view at consecutive increasing budgets','scope':'Same saved geometry/materials/cameras; transparent film diagnostic only, original approved evidence unchanged.'},indent=2)+'\n');print('CONVERGENCE',passed,old,records[-1]);assert passed
 finally:release()
if __name__=='__main__':main()
