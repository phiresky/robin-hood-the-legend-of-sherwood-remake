"""Bounded native-first saved views only after Tree02 first-hit guards pass."""
import sys,json,shutil
from pathlib import Path
import bpy
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def main():
 ver=sys.argv[sys.argv.index('--')+1];out=B/f'tree02-isolated-prototype-{ver}';guard=json.loads((out/'firsthit.json').read_text());assert guard['status'].startswith('PASS');assert guard['model_sha256']==sha(out/'worker.blend');assert shutil.disk_usage(ROOT).free>10*1024**3+128*1024**2;modelroot=out;out=modelroot/'review-unclipped-v1';assert not out.exists();acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(modelroot/'worker.blend'));s=bpy.data.scenes['Tree02 isolated'];s.render.threads_mode='FIXED';s.render.threads=2;s.display.shading.light='STUDIO';s.display.shading.color_type='SINGLE';s.display.shading.single_color=(.6,.6,.6);s.display.shading.show_shadows=True;s.display.shading.show_cavity=True
  s.view_layers.update();all_points=[o.matrix_world@v.co for o in s.objects if o.type=='MESH' for v in o.data.vertices];depth_ranges={}
  for camera in [o for o in s.objects if o.type=='CAMERA']:
   inv=camera.matrix_world.inverted();depths=[-(inv@p).z for p in all_points];camera.data.clip_start=max(.1,min(depths)-25);camera.data.clip_end=max(depths)+25;depth_ranges[camera.name]=[camera.data.clip_start,camera.data.clip_end]
  views={f'view-{i}':f'Tree02 view{i}' for i in range(8)};render_views(s.name,views,out/'actual',modes=('textured','solid'),width=384)
  # Gray Cycles materials retain each foliage alpha mask; no opaque panel substitution.
  for m in bpy.data.materials:
   if not m.use_nodes:continue
   for n in m.node_tree.nodes:
    if n.type=='EMISSION':
     for link in list(n.inputs[0].links):m.node_tree.links.remove(link)
     n.inputs[0].default_value=(.55,.55,.55,1)
  render_views(s.name,views,out/'alpha-gray',modes=('textured',),width=384)
  sheets={}
  for directory,mode,name in [('actual','textured','actual8'),('actual','solid','solid8'),('alpha-gray','textured','alpha-gray8')]:
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    p=Image.open(out/directory/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',p.size,'#333333');bg.alpha_composite(p);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   path=out/f'{name}.png';sheet.save(path);sheets[str(path)]=sha(path)
  total=sum(p.stat().st_size for d in B.glob('tree02-isolated-prototype-*') for p in d.rglob('*') if p.is_file());assert total<128*1024**2;write_json(out/'view-receipt.json',dict(model_sha256=sha(modelroot/'worker.blend'),firsthit_sha256=sha(modelroot/'firsthit.json'),native_view_index=0,camera_depth_bounds_derived_from_vertices=depth_ranges,depth_margin=25,sheets=sheets,total_checkpoint_bytes=total,limits=['Isolated visual review pending; no terrain or shared animation approval.','Alpha-aware gray retains transparency; Workbench solid intentionally shows construction surfaces.']))
 finally:release()
if __name__=='__main__':main()
