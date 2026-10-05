"""Prepare cap-only texture inputs while protecting all existing fence appearance."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_review import render_review
from restart2_prepare_log_pile_inputs import signature
from prepare_private_texture_inputs import prepare
DEST=OUT/'restart2-state/cleared-fence-inputs-v1';ASSET='croisement02-south-field-wattle-fence-cleared-state'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 base=OUT/'fence-state-candidate-v2';model=base/'worker.blend';expected='ac896c0a'
 digest=sha(model)
 if not digest.startswith(expected):raise ValueError('Approved fence changed')
 acquire()
 try:
  DEST.mkdir();bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();objects=sorted([o for o in scene.objects if o.type=='MESH' and o.name.endswith(' / cleared') and o.get('asset_group')=='croisement02-south-field-wattle-fence'],key=lambda o:o.name)
  if len(objects)!=2:raise ValueError('Expected only two approved runs')
  before=signature(objects);collection=bpy.data.collections.new('Cleared fence input Working');scene.collection.children.link(collection)
  for obj in scene.objects:
   if obj.type=='MESH':obj.hide_render=obj not in objects
  for i,obj in enumerate(objects):collection.objects.link(obj);obj['asset_group']=ASSET;obj['source_node']=f'cleared-fence-{i}';obj.hide_render=False
  original=OUT/'scenery-round-2/assets/croisement02-south-field-wattle-fence';workspace=json.loads((original/'workspace.json').read_text());source=Path(workspace['source_path']);shutil.copyfile(source,DEST/'source.png');masks=original/'source-masks.json'
  # Native ownership is retained separately; only inferred cut-face pixels will be editable.
  frames=render_review(DEST/'modified',scene_name=scene.name,collection_name=collection.name,asset_id=ASSET,source_path=DEST/'source.png',width=384,height=384,framing_padding=1.15)
  if signature(objects)!=before:raise ValueError('Approved fence geometry/UV/material changed')
  bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'model.blend'))
  points=[];triangles=[];caps=[]
  for obj in objects:
   obj.data.calc_loop_triangles();offset=len(points);points.extend([obj.matrix_world@v.co for v in obj.data.vertices])
   for triangle in obj.data.loop_triangles:
    ids=[offset+i for i in triangle.vertices];triangles.append(ids);xs=[points[i].x for i in ids];caps.append(any(max(abs(x-cut) for x in xs)<.002 for cut in [1018,1170]))
  tree=BVHTree.FromPolygons(points,triangles,all_triangles=True);path=DEST/'modified/views.json';meta=json.loads(path.read_text());w,h=meta['tile_size'];records=[]
  for view in meta['views']:
   matrix=Matrix(view['camera_matrix_world']);right=matrix.col[0].to_3d();up=matrix.col[1].to_3d();direction=-matrix.col[2].to_3d();center=matrix.translation;unit=view['ortho_scale']/w;editable=np.zeros((h,w),bool)
   for y in range(h):
    for x in range(w):
     origin=center+right*((x+.5-w/2)*unit)+up*((h/2-y-.5)*unit);hit,normal,face,distance=tree.ray_cast(origin,direction)
     if hit is not None and caps[face]:editable[y,x]=True
   known=np.full((h,w,4),255,np.uint8);known[editable,:3]=0;name=DEST/'modified/views'/f"view-{view['index']}-known.png";Image.fromarray(known).save(name);view['ownership_sha256']=sha(name);records.append({'view':view['index'],'editable_cap_pixels':int(editable.sum())})
  meta['known_rule']='Protect all prior stored appearance and background. Only exact first-hit cut-cap triangles are editable; this is not a claim of native observation.';meta['source_mask_evidence']={str(source):sha(source),str(masks):sha(masks)};write_json(path,meta)
  write_json(DEST/'derivation.json',{'status':'Approved geometry derivative; no synthesis performed','source_model':str(model),'source_model_sha256':digest,'prepared_model_sha256':sha(DEST/'model.blend'),'geometry_uv_material_signature':before,'geometry_uv_materials_unchanged':True,'protected_prior_appearance':True,'cap_triangles':sum(caps),'views':records,'scope':'Only newly cut end faces; whole terminal ground patch owned separately.','approval':'restart2-state/user-state-geometry-decisions-v2.json'})
  prepare(DEST,sha(DEST/'model.blend'),DEST/'private-inputs')
 finally:release()
if __name__=='__main__':main()
