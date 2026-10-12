"""Compare current neighboring wood visibility against original and inferred soil."""
import bpy,math,json,hashlib,argparse,sys
from pathlib import Path
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
p=argparse.ArgumentParser();p.add_argument('worker',type=Path);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);acquire(slots=2)
root=ROOT/'level-editor/work/active/croisement01/restart2';wood=root/'approved-tree08-bark-fill-v3/croisement01-tree-08/baked-v8-connected-gaps/worker.blend';bank=a.worker.resolve()/'model.blend';nodes=json.loads((bank.parents[2]/'construction.json').read_text())['changed_nodes']
def tree(objects):
 p=[];f=[]
 for o in objects:
  off=len(p);p.extend(o.matrix_world@v.co for v in o.data.vertices);f.extend(tuple(off+k for k in poly.vertices) for poly in o.data.polygons)
 return BVHTree.FromPolygons(p,f)
assert hashlib.sha256(wood.read_bytes()).hexdigest()=='01dedc44df9d06962a76b6bbf6d4fc96715f219ee0af7eb782ba22677892767a'
bpy.ops.wm.open_mainfile(filepath=str(wood));wb=tree([o for o in bpy.context.scene.objects if o.type=='MESH'])
bpy.ops.wm.open_mainfile(filepath=str(root/'tree05-v7/assets/croisement01-tree-05/model.blend'));oldbb=tree([o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('source_node') in nodes])
bpy.ops.wm.open_mainfile(filepath=str(bank));bb=tree([o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('source_node') in nodes]);s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s));mask=Image.open(root.parent/'baseline/masks/000008.png').convert('L');hits=0;hidden=[];new_hidden=[];old_hidden=0
for y in range(mask.height):
 for x in range(mask.width):
  if not mask.getpixel((x,y)):continue
  origin=Vector((331+x+.5,-(11+y+.5)/s,0))+ray*5000;hit,_,_,wd=wb.ray_cast(origin,-ray,10000)
  if hit is None:continue
  hits+=1;bh,_,_,bd=bb.ray_cast(origin,-ray,10000)
  oh,_,_,od=oldbb.ray_cast(origin,-ray,10000);was_hidden=oh is not None and od<wd-.1;old_hidden+=int(was_hidden)
  if bh is not None and bd<wd-.1:
   hidden.append([x,y,wd-bd])
   if not was_hidden:new_hidden.append([x,y,wd-bd])
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();out=dict(status='Current approved wood geometry source-camera visibility, not approval',wood_sha256=sha(wood),bank_sha256=sha(bank),original_soil_model_sha256=sha(root/'tree05-v7/assets/croisement01-tree-05/model.blend'),native_mask_sha256=sha(root.parent/'baseline/masks/000008.png'),current_wood_native_hits=hits,hidden=hidden,newly_hidden=new_hidden,original_terrain_hidden=old_hidden);(bank.parent/'inspection/current-tree08-bank-visibility.json').write_text(json.dumps(out,indent=2)+'\n');print(hits,len(hidden),old_hidden,len(new_hidden))
