"""Attribute leaf endpoint pixels to the pinned physical scene before integration."""
from pathlib import Path
from collections import Counter
import sys,json,hashlib
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from refinement_review import _tree
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 out=OUT/'restart9-hiding-scatter/scene-receivers-v1';out.mkdir(exist_ok=False);base=OUT/'restart2-textures/batch10-linked-static-v1/scene.blend';assert sha(base)=='493eb8afa1f3e60f433ee2faa5e018d65552292fe5e2dfb63e96a5eb32acfd68';bpy.ops.wm.open_mainfile(filepath=str(base));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'and not o.hide_render];substitutions=[('croisement02-north-woodland-bank','restart4-bank103-source-overlay-v2/model.blend','c0b8458ed39b0412fae16a03384c5585c3271cc5eb48072f0354380ab8f0072e'),('croisement02-tree-07','restart6-tree07-bark-fill-v1/native-restored-v1/worker.blend','6ba5b31c8e2f625e82535152ccea291f658e6f2e12cb53a1b539c488d8bc9189'),('croisement02-east-stone-wall-and-gate','restart7-fence-residual/wall101-appearance-reuse-v1/model.blend','3204d38f8b94a3ef78662b4633b7219b131ff8d61450d0fa8b8c562be64b0475'),('GROUND','restart4-fence14-ground-candidate-v1/model.blend','4f4875bc62b5602417830eb8b458bfbe8dcc9244095699096616fdb4dfb58bf8')];pins=[]
 for group,relative,digest in substitutions:
  path=OUT/relative;assert sha(path)==digest
  with bpy.data.libraries.load(str(path),link=False)as(src,dst):dst.scenes=src.scenes
  candidates=[]
  for imported_scene in dst.scenes:
   bpy.context.window.scene=imported_scene;bpy.context.view_layer.update()
   for obj in imported_scene.objects:
    if obj.type=='MESH'and ((group=='GROUND'and obj.name.startswith('Croisement02 Terrain'))or obj.get('asset_group')==group):candidates.append((obj,obj.matrix_world.copy()))
  assert candidates,(group,path);bpy.context.window.scene=scene;objects=[o for o in objects if not((group=='GROUND'and o.name.startswith('Croisement02 Terrain'))or o.get('asset_group')==group)]
  for obj,matrix in candidates:
   scene.collection.objects.link(obj);obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False;objects.append(obj)
  bpy.context.view_layer.update();pins.append(dict(group=group,model=str(path),sha256=digest,objects=[dict(name=o.name,matrix_world=[list(r)for r in m])for o,m in candidates]))
 boxes={}
 for obj in objects:
  p=np.array([obj.matrix_world@Vector(v)for v in obj.bound_box]);q=np.column_stack((p[:,0],-p[:,1]*SIN-p[:,2]*COS));boxes[obj]=(q.min(0),q.max(0))
 audit=json.loads((OUT/'restart9-hiding-scatter/terrain-receivers-v2/report.json').read_text());manifest=json.loads((OUT/'restart7-source-patch-delivery/contracts-v1/manifest.json').read_text());resources={r['path']:Path(r['source'])for r in manifest['resources']};cache={};results=[]
 for r in audit['records']:
  for state in ['initial','applied']:
   f=r[state]
   if f is None:continue
   key=tuple(r['display_position'])+(f['sha256'],)
   if key in cache:results.append(dict(instance=r['id'],state=state,reuse=cache[key]));continue
   a=np.array(Image.open(resources[f['path']]).convert('RGBA'));x0,y0=np.array(r['display_position'])+f['offset'];h,w=a.shape[:2];selected=[o for o,(lo,hi)in boxes.items()if hi[0]>=x0 and lo[0]<=x0+w and hi[1]>=y0 and lo[1]<=y0+h];tree,owners,_=_tree(selected);samples=[];counts=Counter()
   for y,x in np.argwhere(a[:,:,3]>0):
    p,n,i,d=tree.ray_cast(Vector((x0+x+.5,-(y0+y+.5)/SIN,0))+RAY*6000,-RAY);obj=owners[i]if p is not None else None;asset=(obj.get('asset_group')or obj.name)if obj else '<none>';counts[asset]+=1;samples.append(dict(pixel=[int(x0+x),int(y0+y)],receiver=asset,object=obj.name if obj else None,point=list(p)if p is not None else None))
   cache[key]=len(results);results.append(dict(instance=r['id'],state=state,source_sha256=f['sha256'],counts=dict(counts),samples=samples));print(r['id'],state,dict(counts),flush=True)
 (out/'report.json').write_text(json.dumps(dict(base_sha256=sha(base),substitutions=pins,records=results,scope='Current frozen publication candidate visual receiver attribution. Later pending bark and19/25 substitutions excluded. Endpoint geometry absent; no visibility parity claim.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
