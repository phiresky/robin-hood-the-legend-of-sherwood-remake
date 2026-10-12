"""Fill only audited visible unknown samples from nearby generated bark on the same face."""
import io,json,shutil,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from review_evidence import sha
from refinement_workspace import _geometry
from render_multiview_asset import render
from refinement_review import _tile
from array import array
R=ROOT/'level-editor/work/active/croisement01/restart2';W=R/'approved-tree08-bark-fill-v3/croisement01-tree-08';src=W/'baked-v3-native-exact';out=W/'baked-v4-visible-gaps'
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 bpy.ops.wm.open_mainfile(filepath=str(src/'worker.blend'));obj=next(o for o in bpy.context.scene.objects if o.type=='MESH');mesh=obj.data;before=_geometry(obj)
 material=next(m for m in mesh.materials if m and m.get('source_ownership_bake'));texture=next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE');im=texture.image;uv=mesh.uv_layers[next(n.uv_map for n in material.node_tree.nodes if n.type=='UVMAP')]
 layer=json.loads((W/'baked-v1/layer-0.json').read_text())['objects'][0];ownership=np.load(layer['texel_provenance']['path'])['ownership'];pixels=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(pixels);pixels=pixels.reshape(im.size[1],im.size[0],4);old=pixels.copy();repairs={};unresolved=[]
 report=json.loads((src/'visible-fill-audit/report.json').read_text());assert report['model_sha256']==sha(src/'worker.blend')
 for view in report['views']:
  for _,_,fid,x,y in view['unfilled']:
   if (x,y) in repairs:continue
   assert ownership[y,x]==0
   face=mesh.polygons[fid];coords=np.array([uv.data[i].uv[:] for i in face.loop_indices])*[im.size[0],im.size[1]]
   low=np.maximum(0,np.floor(coords.min(0)).astype(int)-2);high=np.minimum([im.size[0],im.size[1]],np.ceil(coords.max(0)).astype(int)+2)
   candidates=np.argwhere(ownership[low[1]:high[1],low[0]:high[0]]==2)
   if len(candidates):
    candidates+=low[::-1];dist=((candidates-[y,x])**2).sum(1);choice=dist.argmin();dy,dx=candidates[choice]
   if not len(candidates) or dist[choice]>16:
    unresolved.append([fid,x,y]);continue
   pixels[y,x,:3]=old[dy,dx,:3];repairs[x,y]={'face':fid,'texel':[x,y],'donor':[int(dx),int(dy)],'distance_texels':float(dist[choice]**.5)}
 assert np.array_equal(pixels[ownership!=0],old[ownership!=0]);assert np.array_equal(pixels[:,:,3],old[:,:,3]);assert len(repairs)<=1000
 assert not out.exists();out.mkdir()
 if unresolved:
  (out/'unresolved.json').write_text(json.dumps({'safe_repairs':len(repairs),'unresolved':unresolved},indent=2)+'\n')
  print('HOLD',len(repairs),len(unresolved),flush=True)
 else:
  im.pixels.foreach_set(pixels.ravel());im.update();im.pack();assert _geometry(obj)==before;bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'worker.blend'),compress=True)
  record={'status':'PASS_SCOPED_SAME_FACE_GENERATED_FILL','model_sha256':sha(out/'worker.blend'),'parent_model_sha256':sha(src/'worker.blend'),'repairs':list(repairs.values()),'protected_source_and_existing_generated_rgba_exact':True,'alpha_exact':True,'geometry_exact':True,'policy':'Only class-zero samples explicitly observed in the eight reviewed cameras; generated donors from the same triangle atlas island, at most four texels away; original source never edited.'}
  (out/'gap-repair.json').write_text(json.dumps(record,indent=2)+'\n');updated=ownership.copy()
  for x,y in repairs:updated[y,x]=3
  np.savez_compressed(out/'ownership.npz',ownership=updated)
  render(W/'experiment/baked-v1-sampling-views.json',out/'actual',width=384)
  buffers=[]
  for i in range(8):
   image=bpy.data.images.load(str(out/'actual'/f'view-{i}-textured.png'),check_existing=False);v=array('f',[0])*len(image.pixels);image.pixels.foreach_get(v);buffers.append(v);bpy.data.images.remove(image)
  _tile(buffers,384,384,out/'actual/textured.png');print('PASS',len(repairs),flush=True)
finally:release()
