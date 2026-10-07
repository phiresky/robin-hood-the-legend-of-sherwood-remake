"""Break regular static-cell depth ribs while preserving exact native source projections."""
import sys,math,json,hashlib,random,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def geom(o):return hashlib.sha256(np.asarray([tuple(v.co) for v in o.data.vertices],np.float32).tobytes()).hexdigest()
def main(tree):
 assert tree in (12,14);assert shutil.disk_usage(ROOT).free>25*1024**3;src=B/f'tree{tree}-static-leaf-crown-v1';out=B/f'tree{tree}-static-leaf-crown-v3';out.mkdir(exist_ok=False);acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));scene=bpy.data.scenes['Croisement03 Refinement'];o=next(o for o in scene.objects if o.get('source_role','').startswith('NEW proposed STATIC'));old={x.name:geom(x) for x in scene.objects if x.type=='MESH' and x!=o};images={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};before=np.array([tuple(v.co) for v in o.data.vertices]);sin,cos=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-cos,sin));rng=random.Random(9200+tree)
  # Every independent native texel keeps its projected boundary and UV. Small
  # irregular local depth offsets and planar tilts prevent smooth parallel ribs.
  assert len(o.data.vertices)==4*len(o.data.polygons)
  for f in o.data.polygons:
   assert len(f.vertices)==4;center=rng.uniform(-10,10);sx,sy=rng.uniform(-.35,.35),rng.uniform(-.35,.35)
   for vi,(u,v) in zip(f.vertices,((-1,-1),(1,-1),(1,1),(-1,1))):o.data.vertices[vi].co+=ray*(center+sx*u+sy*v)
  o.data.update();after=np.array([tuple(v.co) for v in o.data.vertices]);projection=lambda a:np.column_stack((a[:,0],-a[:,1]*sin-a[:,2]*cos));error=float(np.abs(projection(before)-projection(after)).max());assert error<.0001
  assert old=={x.name:geom(x) for x in scene.objects if x.type=='MESH' and x!=o};assert images=={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data}
  bpy.data.libraries.write(str(out/'worker.blend'),{scene},fake_user=True,compress=True);render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},out/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(out/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(out/'actual'/f'{mode}.png')
  d=json.loads((src/'receipt.json').read_text());d.update(status='PRIVATE irregular static-cell depth correction; native and visual review required',model_sha256=sha(out/'worker.blend'),prior_model_sha256=sha(src/'worker.blend'),correction=dict(object=o.name,max_native_projection_error=error,max_local_ray_shift=float(np.linalg.norm(after-before,axis=1).max()),original_approved_geometry_exact=True,all_images_rgba_exact=True,uv_alpha_source_ownership_unchanged=True));write_json(out/'receipt.json',d)
 finally:release()
if __name__=='__main__':main(int(sys.argv[sys.argv.index('--')+1]))
