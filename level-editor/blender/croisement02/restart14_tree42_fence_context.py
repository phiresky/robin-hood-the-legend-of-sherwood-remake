"""Compare immutable canopy motion against a fixed fence in small context views."""
from pathlib import Path
import sys,json,hashlib,shutil,resource,math
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
import bpy,numpy as np
from mathutils import Vector,Matrix
from PIL import Image,ImageDraw
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
BASE=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';TRIAL=BASE/'tree42-motion-v5';DEST=TRIAL/'fence-context-v1';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def guard(n=524288):
 extra=sum(p.stat().st_size for p in DEST.rglob('*') if p.is_file()) if DEST.exists() else 0;total=sum(p.stat().st_size for p in TRIAL.rglob('*') if p.is_file());assert extra+n<=4*2**20 and total+n<=256*2**20;assert shutil.disk_usage(BASE).free-n>=10*2**30

def main():
 guard();DEST.mkdir(exist_ok=False);pin=next(p for p in json.loads((BASE/'tree42-alpha-neighbors-coherent-v1/report.json').read_text())['pins'] if p['id']=='croisement02-south-field-wattle-fence');mp=Path(pin['model']);assert sha(mp)==pin['model_sha256'];assert all(sha(Path(r['path']))==r['sha256'] for r in pin['resources']);images=[];cameras=[]
 for version in(4,5):
  model=BASE/f'tree42-motion-v{version}/prototype.blend';expected={4:'ed90774d18790d35b23b2d20941c609b2420004ee4b4a1b293872ba934150376',5:'b569c53628404fd640c582402306ba265fead93a032fd43d1c07d62e03c6eb9b'}[version];assert sha(model)==expected;bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;scene.frame_set(29)
  tree=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-tree-42'];before=set(bpy.data.objects);bpy.ops.import_scene.gltf(filepath=str(mp));added=[o for o in bpy.data.objects if o not in before];roots=[o for o in added if o.parent not in added];assert len(roots)==1;t=pin['placement']['transform'];assert t['rot_deg']==0;roots[0].location+=Vector((t['dx'],-t['dy']/SIN,t['dz']));fence=[o for o in added if o.type=='MESH'];bpy.context.view_layer.update()
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o not in tree+fence
  # The delivery is unlit; preserve its color/alpha shader instead of adding lights.
  material_nodes={m.name:[n.type for n in m.node_tree.nodes] for o in fence for m in o.data.materials if m and m.use_nodes}
  scene.render.engine='CYCLES';scene.cycles.samples=32;scene.cycles.transparent_max_bounces=512;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=512;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard'
  data=bpy.data.cameras.new('Fixed fence context');data.type='ORTHO';data.clip_end=20000;cam=bpy.data.objects.new('Fixed fence context',data);scene.collection.objects.link(cam);scene.camera=cam
  if version==4:
   from refinement_review import _tree
   # Use physical surface positions only to set one shared oblique framing.
   origin=Vector((890.5,-817.5/SIN,0))+RAY*5000;ct=_tree(tree)[0];ft=_tree(fence)[0];ch=ct.ray_cast(origin,-RAY);fh=ft.ray_cast(origin,-RAY);assert fh[0] is not None
   crown=ch[0] if ch[0] is not None else origin-RAY*(fh[3]-480);mid=(crown+fh[0])*.5;native_center=Vector((890.5,-817.5/SIN,0));oblique=Vector((.45,-COS,SIN)).normalized()
   for center,direction,scale in [(native_center,RAY,84),(mid,oblique,520)]:
    pos=center+direction*5000;matrix=(center-pos).to_track_quat('-Z','Y').to_matrix().to_4x4();matrix.translation=pos;cameras.append({'matrix':[list(row) for row in matrix],'ortho_scale':scale})
  for i,c in enumerate(cameras):
   cam.matrix_world=Matrix(c['matrix']);data.ortho_scale=c['ortho_scale'];path=DEST/f'v{version}-{("native","oblique")[i]}.png';guard();scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);images.append({'path':str(path),'sha256':sha(path),'version':version,'view':i})
  assert sha(model)==expected
 sheet=Image.new('RGB',(1024,808),'#ddd');draw=ImageDraw.Draw(sheet)
 for vi,version in enumerate((4,5)):
  for ci,name in enumerate(('native','oblique')):
   im=Image.open(DEST/f'v{version}-{name}.png');x=ci*512;y=vi*404;sheet.paste(im,(x,y+20),im);draw.text((x+6,y+4),f'v{version} phase7 - {name} - fixed south-field fence',fill='black')
 guard(2*2**20);sheet.save(DEST/'comparison.png');f=json.loads((BASE/'source-reconciliation-v1/report.json').read_text())['groups'][1]['frames'][7];x,y,w,h=f['bbox'];source=Image.open(f['path']);crop=source.crop((890-x-12,817-y-12,890-x+13,817-y+13)).resize((200,200),Image.Resampling.NEAREST);panel=Image.new('RGB',(420,250),'#292929');panel.paste(crop,(12,32),crop);d=ImageDraw.Draw(panel);d.text((8,8),'Original phase7 coordinate (890,817)',fill='white');d.rectangle((108,128,115,135),outline='#ff5555');d.text((222,45),'Marker only here.\nComparison cameras\nare unchanged.\nNo fence art added\nto source canopy.',fill='white');guard();panel.save(DEST/'source-coordinate.png');guard(65536);(DEST/'report.json').write_text(json.dumps({'status':'RENDERED_FOR_AUTHOR_ROOT_REVIEW','phase':7,'models':{str(v):sha(BASE/f'tree42-motion-v{v}/prototype.blend') for v in(4,5)},'fence_pin':pin,'fixed_cameras':cameras,'images':images,'material_nodes':material_nodes,'peak_rss_kib':resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,'threads':2,'scope':'Same immutable v4/v5 tree and fixed fence only. Native tight84px frame and oblique depth context520px; marker separate. No saved model or material changes.'},indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
