"""Capped immutable-motion review supplement with fixed solid cameras."""
from pathlib import Path
import sys,json,hashlib,shutil,resource
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';TRIAL=BASE/'tree42-motion-v5';DEST=TRIAL/'review-supplement-v1'
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def guard(n=2**20):
 assert shutil.disk_usage(BASE).free-n>=10*2**30
 total=sum(p.stat().st_size for p in TRIAL.rglob('*') if p.is_file());extra=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()) if DEST.exists() else 0
 assert total+n<=256*2**20 and extra+n<=16*2**20,(total,extra,n)
def main():
 import bpy
 from mathutils import Matrix
 from PIL import Image,ImageDraw
 guard();DEST.mkdir(exist_ok=True);out=DEST/'solid';out.mkdir(exist_ok=False);model=TRIAL/'prototype.blend';assert sha(model)=='b569c53628404fd640c582402306ba265fead93a032fd43d1c07d62e03c6eb9b'
 bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;scene.frame_set(29)
 mat=bpy.data.materials.new('Private solid review override');mat.diffuse_color=(.55,.55,.55,1);mat.use_nodes=True;p=mat.node_tree.nodes.get('Principled BSDF');p.inputs['Base Color'].default_value=(.55,.55,.55,1);p.inputs['Roughness'].default_value=.8
 for o in scene.objects:
  if o.type=='MESH':
   o.hide_render=o.get('asset_group')!='croisement02-tree-42'
   if not o.hide_render:
    for i in range(len(o.data.materials)):o.data.materials[i]=mat
 scene.render.engine='CYCLES';scene.cycles.samples=16;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard'
 data=bpy.data.cameras.new('Solid review camera');data.type='ORTHO';data.clip_end=20000;cam=bpy.data.objects.new('Solid review camera',data);scene.collection.objects.link(cam);scene.camera=cam
 cameras=json.loads((TRIAL/'review-v3/report.json').read_text())['fixed_cameras'];sheet=Image.new('RGB',(1536,808),'#ddd');d=ImageDraw.Draw(sheet);images=[]
 for i,c in enumerate(cameras):
  cam.matrix_world=Matrix(c['matrix']);data.ortho_scale=c['ortho_scale'];path=out/f'view-{i}.png';guard();scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);im=Image.open(path);x=i%4*384;y=i//4*404;sheet.paste(im,(x,y+20),im);d.text((x+5,y+4),f'Phase7 solid view{i}',fill='black');images.append({'path':str(path),'sha256':sha(path)})
 guard();sheet.save(out/'solid-eight.png');guard(65536);(out/'report.json').write_text(json.dumps({'status':'RENDERED_FOR_REVIEW','model_sha256':sha(model),'fixed_cameras':cameras,'phase':7,'images':images,'sheet_sha256':sha(out/'solid-eight.png'),'peak_rss_kib':resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,'threads':2,'scope':'Private opaque solid override only; approved file unchanged.'},indent=2)+'\n')
if __name__=='__main__':
 from render_slots import acquire,release
 acquire()
 try:main()
 finally:release()
