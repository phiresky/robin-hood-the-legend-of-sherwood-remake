"""Restore exact original bark texels on the inferred atlas without geometry changes."""
import hashlib,json,shutil,sys
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from review_evidence import sha
from refinement_workspace import _geometry
from render_multiview_asset import render
from refinement_review import _tile
from array import array
R=ROOT/'level-editor/work/active/croisement01/restart2';W=R/'approved-tree08-bark-fill-v3/croisement01-tree-08';src=W/'baked-v1';out=W/'baked-v3-native-exact'
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 assert not out.exists()
 guard=json.loads((src/'native-texel-guard.json').read_text());assert not guard['misses'];assert guard['model_sha256']==sha(src/'worker.blend')
 samples=json.loads((src/'native-texel-samples.json').read_text());assert len(samples)==6276
 by_image={}
 for row in samples:
  key=tuple(row['texel']);mapping=by_image.setdefault(row['image'],{})
  assert key not in mapping or mapping[key]==row['expected'],'Conflicting original colors share an atlas texel; higher resolution required'
  mapping[key]=row['expected']
 bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));scene=bpy.context.scene;before={o.name:_geometry(o) for o in scene.objects}
 out.mkdir();changes=[]
 for name,mapping in by_image.items():
  im=bpy.data.images[name];pixels=np.empty(len(im.pixels),dtype=np.float32);im.pixels.foreach_get(pixels);pixels=pixels.reshape(im.size[1],im.size[0],4)
  count=0
  for (x,y),rgba in mapping.items():
   # This generated ownership atlas stores its packed byte values directly.
   # Preserve that representation; converting to linear would darken source RGB twice.
   value=np.array(rgba,dtype=np.float32)/255
   count+=int(not np.array_equal(pixels[im.size[1]-1-y,x],value.astype(np.float32)))
   pixels[im.size[1]-1-y,x]=value
  im.pixels.foreach_set(pixels.ravel());im.update();im.pack();changes.append({'image':name,'native_sample_texels':len(mapping),'changed':count})
 for o in scene.objects:
  if o.type!='MESH':continue
  for mat in o.data.materials:
   if not mat:continue
   for n in mat.node_tree.nodes:
    if n.type=='TEX_IMAGE' and n.image and n.image.name in by_image:n.interpolation='Closest'
 assert before=={o.name:_geometry(o) for o in scene.objects}
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True)
 report={'status':'RESTORED_AWAITING_REOPENED_NATIVE_GUARD','parent_model_sha256':sha(src/'worker.blend'),'model_sha256':sha(out/'worker.blend'),'samples_sha256':sha(src/'native-texel-samples.json'),'changes':changes,'geometry_exact':True,'policy':'Original source RGBA restored only at actual native first-hit atlas texels. Closest filtering retains exact original sampled colors. Inferred generated bark elsewhere unchanged.'}
 (out/'native-restoration.json').write_text(json.dumps(report,indent=2)+'\n')
 manifest=W/'experiment/baked-v1-sampling-views.json';render(manifest,out/'actual',width=384)
 buffers=[]
 for i in range(8):
  im=bpy.data.images.load(str(out/'actual'/f'view-{i}-textured.png'),check_existing=False);v=array('f',[0])*len(im.pixels);im.pixels.foreach_get(v);buffers.append(v);bpy.data.images.remove(im)
 _tile(buffers,384,384,out/'actual/textured.png')
 print(report,flush=True)
finally:release()
