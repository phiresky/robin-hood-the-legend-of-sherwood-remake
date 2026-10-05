"""Inspect the remaining foliage reservation against the approved tree45 receiver."""
import sys,json
from pathlib import Path
from collections import Counter
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from restart3_initial_fence_contact import link
from restore_ground75_source import geometry
from review_bank_candidate import camera
from refinement_review import _tree
from tree_geometry import SIN,RAY
GROUND=OUT/'restart3-initial-fence/floor-fill-v1/bake-v1/model.blend'
FENCE=OUT/'restart3-initial-fence/geometry-v6/model.blend'
TREE=OUT/'restart2-textures/approved7-combined-fill-v1/croisement02-tree-45/native-front-preparation/experiment/bake-v1/worker.blend'

def main():
 out=OUT/'restart3-initial-fence/tree45-reservation-context-v1';out.mkdir(exist_ok=False)
 expected={GROUND:'4e2c98fbc63743af3c6539c40c60b96eb759d3a9075c13edd6e1bf16fd19eb18',FENCE:'46579f5398d1495b13e8d8433fa1a0687be447e025cd9ba745f5d7251076c5a0',TREE:'859ab8649ecba8b1059819438b1c30b935ca78c99abf4730eef381ef54a3844c'}
 for p,h in expected.items():assert sha(p)==h
 bpy.ops.wm.open_mainfile(filepath=str(GROUND));scene=bpy.data.scenes.new('Read-only fence and approved tree45');bpy.context.window.scene=scene
 ground=bpy.data.objects['Croisement02 Terrain'];link(scene,ground);ground.hide_render=False;objects=[ground];records=[]
 for model,group in [(FENCE,'croisement02-south-field-wattle-fence'),(TREE,'croisement02-tree-45')]:
  with bpy.data.libraries.load(str(model),link=False)as(src,dst):dst.objects=list(src.objects)
  chosen=[o for o in dst.objects if o and o.type=='MESH' and o.get('asset_group')==group];assert chosen,(model,[(o.name,o.get('asset_group'))for o in dst.objects if o and o.type=='MESH'])
  for o in chosen:link(scene,o);o.hide_render=False
  objects+=chosen;bpy.context.view_layer.update();records.append(dict(model=str(model),sha256=sha(model),asset_group=group,objects={o.name:geometry(o)for o in chosen}))
 bpy.context.view_layer.update();tree,owners,_=_tree(objects)
 old=json.loads((OUT/'restart3-initial-fence/first-hit-v1/audit.json').read_text());points=[r['pixel']for r in old['samples']if 128 in r['native_masks']];assert len(points)==2441
 counts=Counter();samples=[]
 for x,y in points:
  hit,normal,index,distance=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY)
  obj=owners[index]if hit is not None else None;asset=(obj.get('asset_group')or obj.get('source_node')or obj.name)if obj else '<none>';counts[asset]+=1;samples.append(dict(pixel=[x,y],asset=asset,object=obj.name if obj else None,hit=list(hit)if hit is not None else None))
 write_json(out/'first-hit.json',dict(status='Read-only foliage reservation classification',domain_pixels=len(points),first_hits=dict(counts),samples=samples,models=records,ground_candidate_sha256=sha(GROUND),ground_appearance_pending_user=True,shared_scene_unchanged=True,source_authority=str(OUT/'restart3-initial-fence/first-hit-v1/audit.json'),source_authority_sha256=sha(OUT/'restart3-initial-fence/first-hit-v1/audit.json'),method='Alpha-aware one-sided BVH at exact native pixel centers after hierarchy linking and dependency graph update.',limitations=['Small context includes ground, initial fence and tree45 only; it is not a new whole-scene assembly.','Native occlusion does not establish unseen floor appearance at arbitrary oblique cameras.','Pixel-center boundary tests differ from multisample edge coverage.']))
 print('FIRST_HIT',dict(counts),flush=True)
 for label,direction in [('native',RAY),('oblique',Vector((-.55,.7,.45)).normalized())]:
  target=Vector((1094,-887/SIN,0))if label=='native' else Vector((1094,-1555,25));camera(scene,target,direction,704,512,220 if label=='native' else 255);scene.cycles.transparent_max_bounces=1024;scene.render.filepath=str(out/(label+'.png'));bpy.ops.render.render(write_still=True,scene=scene.name)
 source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop((984,807,1204,967)).resize((704,512),Image.Resampling.NEAREST);before=Image.open(OUT/'restart3-initial-fence/floor-fill-v1/bake-v1/candidate-native.png').convert('RGB');after=Image.open(out/'native.png').convert('RGB');sheet=Image.new('RGB',(2112,544),'#303030');draw=ImageDraw.Draw(sheet)
 for i,(label,image)in enumerate([('Original source',source),('Floor / fence; tree omitted',before),('Floor / fence / approved tree45',after)]):sheet.paste(image,(i*704,32));draw.text((i*704+5,8),label,fill='white')
 sheet.save(out/'source-context.png')
 assert all(sha(p)==h for p,h in expected.items());print(out,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
