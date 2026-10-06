"""Reopen scoped bark workers and check source-preserving material composition."""
from pathlib import Path
import os,sys,json,hashlib
import bpy,numpy as np
from PIL import Image,ImageFilter
R=Path.cwd();sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
from render_multiview_asset import render
from restart6_bark_preservation import reachable_images
from restart8_bake_toe_bark import pixels
from bake_texture_candidate import snapshot
O=R/'level-editor/work/croisement02-refinement';B=O/'restart8-toe-bark-fill-v1';read=lambda p:json.loads(p.read_text());h=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main(n):
 F=B/f'tree-{n}-fill-v1';D=F/os.environ.get('C02_TOE_BARK_STAGE','shader-restored-v1');P=D/'preservation.json';proof=read(P);model=D/'worker.blend';assert h(model)==proof['model_sha256'];out=D/'wood-review-v1';out.mkdir(exist_ok=False);auth=next(r for r in read(B/'source-authority.json')['records']if r['asset']==f'croisement02-tree-{n}');meta=read(D/'close-views.json');bpy.ops.wm.open_mainfile(filepath=auth['model']);scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;before=snapshot(scene,set(meta['object_names']));original_uv={name:{uv.name:np.array([x.uv[:]for x in uv.data],np.float32)for uv in scene.objects[name].data.uv_layers}for name in proof['active_receivers']};bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,set(meta['object_names']))==before;records=[]
 for name,layers in original_uv.items():
  for uvname,values in layers.items():assert np.array_equal(values,np.array([x.uv[:]for x in scene.objects[name].data.uv_layers[uvname].data],np.float32))
 for name in proof['active_receivers']:
  ob=scene.objects[name]
  for slot in sorted({p.material_index for p in ob.data.polygons}):
   mat=ob.data.materials[slot];nodes=mat.node_tree.nodes
   for node in reachable_images(mat):
    im=node.image;digest=hashlib.sha256(bytes(im.packed_file.data)).hexdigest()
    if digest in auth['atlases']:known=np.load(auth['atlases'][digest]['path'])['ownership']==1
    elif im.name.startswith(f'native{n}') and'protected original'not in im.name:known=pixels(im)[:,:,3]>0
    else:continue
    uvlink=node.inputs['Vector'].links[0].from_node;lookup=[q for q in nodes if q.type=='TEX_IMAGE'and q.image and'protected original'in q.image.name and q.inputs['Vector'].links and q.inputs['Vector'].links[0].from_node==uvlink];assert lookup,(name,slot,im.name)
    target=np.logical_or.reduce([pixels(q.image)[:,:,0]>.5 for q in lookup if tuple(q.image.size)==tuple(im.size)]);assert target[known].all(),(name,im.name,'unprotected source');records.append(dict(object=name,slot=slot,image=im.name,image_sha256=digest,protected_original_texels=int(known.sum()),protection_guard=True))
 for ob in scene.objects:
  if ob.type=='MESH'and ob.name not in proof['active_receivers']:ob.hide_render=True
 comparisons=[];inputdir=B/f'tree-{n}-input-v1'
 for i in range(8):
  p=D/'close'/f'view-{i}-textured.png';a=np.array(Image.open(p).convert('RGBA'));old=np.array(Image.open(inputdir/f'actual/view-{i}-textured.png').convert('RGBA'));known=np.array(Image.open(inputdir/f'views/view-{i}-known.png'))[:,:,0]>127;interior=np.array(Image.fromarray(np.uint8(known)*255).filter(ImageFilter.MinFilter(5)))>0;interior&=old[:,:,3]==255;delta=np.max(np.abs(a[:,:,:3].astype(int)-old[:,:,:3].astype(int)),axis=2);comparisons.append(dict(view=i,known_interior_pixels=int(interior.sum()),changed_gt1=int((interior&(delta>1)).sum()),max_error=int(delta[interior].max(initial=0)),alpha_equal=bool(np.array_equal(a[:,:,3],old[:,:,3]))))
 result=dict(model_sha256=h(model),preservation_sha256=h(P),reopened_original_uv_exact=True,reopened_geometry_outside_appearance_exact=True,source_shader_guards=records,known_pixel_diagnostics=comparisons,files={str(p):h(p)for p in (D/'close').rglob('*')if p.is_file()});(out/'guard.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(comparisons),flush=True)
if __name__=='__main__':
 acquire()
 try:main(int(sys.argv[sys.argv.index('--')+1]))
 finally:release()
