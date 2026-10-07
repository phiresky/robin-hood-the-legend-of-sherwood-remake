"""Inspect a saved bank at the exact native camera and the west shelf contact."""
import sys,math,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(R/'level-editor/refinement')]
from restart2_tree02_shared_ridge_joint_v5 import hit,world
from render_slots import acquire,release
from evidence_io import sha,write_json
B=R/'level-editor/work/croisement03-refinement';O=Path(sys.argv[sys.argv.index('--')+1]).resolve() if '--' in sys.argv else B/'restart2/bank-full-prototype-v2';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=Vector((0,-C,S))

def row(o):
 m=o.data;m.calc_loop_triangles();vs=[o.matrix_world@v.co for v in m.vertices];ts=list(m.loop_triangles)
 return(o,BVHTree.FromPolygons(vs,[list(t.vertices) for t in ts],all_triangles=True),vs,ts,None,None,False,False)

def main():
 output=O/'native-contact-v2';output.mkdir(exist_ok=False);acquire()
 try:
  model=O/'worker.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));s=bpy.context.scene;bank=[row(o) for o in s.objects if o.type=='MESH' and o.name.startswith('Candidate bank')]
  base=B/'baseline/croisement03-baseline.blend';basehash=sha(base)
  with bpy.data.libraries.load(str(base),link=False) as (a,b):b.objects=[f'building-{i:03}.001' for i in (2,3,4,5)]
  objects=[o for o in b.objects if o];assert len(objects)==4
  for o in objects:
   transform=world(o);o.parent=None;o.matrix_world=transform
  rows=[row(o) for o in objects];level=json.loads((B/'baseline/Croisement03.rhp.json').read_text());domain=Image.new('L',(1408,960));domain.paste(Image.open(B/'baseline/masks/000001.png'),tuple(level['masks'][1]['box_top_left']));changes=[];holes=0
  for y,x in zip(*np.nonzero(np.array(domain))):
   before=hit(rows,int(x),int(y));after=hit(rows+bank,int(x),int(y))
   if before is None:holes+=1;continue
   if before!=after:changes.append([int(x),int(y),before,after])
  write_json(output/'tree01-coarse-guard.json',dict(status='PASS' if not changes else 'HOLD coarse source intersections',samples=int(np.count_nonzero(np.array(domain))),baseline_holes=holes,changes=changes,model_sha256=digest,baseline_sha256=basehash))
  s.render.engine='CYCLES';s.cycles.samples=16;s.cycles.use_denoising=False;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_percentage=100;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA'
  camera=bpy.data.objects.new('Exact native bank camera',bpy.data.cameras.new('Exact native bank camera'));s.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.sensor_fit='HORIZONTAL';camera.data.clip_start=.1;camera.data.clip_end=10000;s.camera=camera;source=Image.open(B/'baseline/covered.png').convert('RGBA')
  for label,box in [('full',(0,135,650,510)),('west-contact',(70,290,220,425))]:
   left,top,right,bottom=box;target=Vector(((left+right)/2,-(top+bottom)/2/S,0));camera.location=target+RAY*5000;camera.rotation_euler=(-RAY).to_track_quat('-Z','Y').to_euler();camera.data.ortho_scale=right-left;s.render.resolution_x=right-left;s.render.resolution_y=bottom-top;s.render.filepath=str(output/f'{label}.png');bpy.ops.render.render(write_still=True)
   actual=Image.open(output/f'{label}.png').convert('RGBA');crop=source.crop(box);factor=1 if label=='full' else 3;w,h=crop.size;sheet=Image.new('RGB',(w*factor*2,h*factor+24),'#333333');draw=ImageDraw.Draw(sheet)
   for i,(name,im) in enumerate([('Native source',crop),('Saved geometry; gray remains unclassified',actual)]):
    bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB').resize((w*factor,h*factor),Image.Resampling.NEAREST),(w*factor*i,24));draw.text((w*factor*i+3,5),name,fill='white')
   sheet.save(output/f'{label}-comparison.png')
  assert sha(model)==digest and sha(base)==basehash;write_json(output/'receipt.json',dict(model_sha256=digest,status='Saved native full extent and west contact rendered; visual review required',images={p.name:sha(p) for p in output.glob('*.png')},limits=['West path remains original coarse geometry/material context.','Gray ramp/support morphology is unfinished; rendering is not readiness approval.']))
 finally:release()
if __name__=='__main__':main()
