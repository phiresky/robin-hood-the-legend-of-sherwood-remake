"""Move one inferred support behind the observed stem without changing source geometry."""
import sys,math,json,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2'
def main():
 out=B/'tree06-crown-prototype-v2';assert shutil.disk_usage(ROOT).free>25*1024**3;out.mkdir(exist_ok=False);acquire()
 try:
  src=B/'tree06-crown-prototype-v1/worker.blend';bpy.ops.wm.open_mainfile(filepath=str(src));scene=bpy.data.scenes['Tree13 isolated wood'];o=scene.objects['Inferred cluster support 4'];assert o.get('inferred_branch');ray=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))))
  images={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data}
  unchanged={x.name:hashlib.sha256(np.array([tuple(v.co) for v in x.data.vertices],np.float32).tobytes()).hexdigest() for x in scene.objects if x.type=='MESH' and x!=o}
  for v in o.data.vertices:v.co-=ray*4
  o.data.update()
  assert unchanged=={x.name:hashlib.sha256(np.array([tuple(v.co) for v in x.data.vertices],np.float32).tobytes()).hexdigest() for x in scene.objects if x.type=='MESH' and x!=o}
  assert images=={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data}
  bpy.data.libraries.write(str(out/'worker.blend'),{scene},fake_user=True,compress=True)
  render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},out/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(out/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(out/'actual'/f'{mode}.png')
  d=json.loads((src.parent/'receipt.json').read_text());d.update(status='PRIVATE local support clearance; native audit required',model_sha256=sha(out/'worker.blend'),prior_model_sha256=sha(src),correction={'object':o.name,'translation':list(-ray*4),'reason':'Support4 occluded two accepted bark rays583,65 and584,65. Move4 units behind along unchanged source rays; root remains inside the broader supporting stem cross section.','all_other_mesh_vertices_exact':True,'all_image_rgba_exact':True});write_json(out/'receipt.json',d)
 finally:release()
if __name__=='__main__':main()
