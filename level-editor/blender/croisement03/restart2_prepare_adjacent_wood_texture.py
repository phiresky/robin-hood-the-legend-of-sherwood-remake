"""Prepare unchanged approved woody surfaces with exact source-part normalization."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_workspace import prepare,modified,_geometry
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';TREE=int(sys.argv[sys.argv.index('--')+1]);assert TREE in (12,14);ASSET=f'croisement03-tree-{TREE}'
def triangles(objects):
 rows=[]
 for obj in objects:
  obj.data.calc_loop_triangles()
  for tri in obj.data.loop_triangles:
   points=[tuple(round(float(x),5) for x in obj.matrix_world@obj.data.vertices[i].co) for i in tri.vertices];rows.append(tuple(sorted(points)))
 return sorted(rows)
def main():
 decision=json.loads((B/f'user-approval-v14/croisement03-tree-{TREE}-shared-crown-fragment.json').read_text());source=Path(decision['model']);assert sha(source)==decision['model_sha256'];out=B/f'tree{TREE}-approved-wood-texture-v1';out.mkdir(exist_ok=False);assert shutil.disk_usage(ROOT).free>25*1024**3;acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.preferences.filepaths.save_version=0;scene=bpy.data.scenes['Tree13 isolated wood'];scene.name='Croisement03 Refinement';bpy.context.window.scene=scene;collection=bpy.data.collections.new('Croisement03 Working');scene.collection.children.link(collection);wood=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==ASSET];assert len(wood)>=(9 if TREE==12 else 6);foliage=[o for o in scene.objects if o.type=='MESH' and o not in wood];assert len(foliage)==1;protected={o.name:_geometry(o,protect_appearance=True) for o in foliage};before=triangles(wood);images={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};joined=[]
  groups={node:[o for o in wood if o.get('source_node')==node] for node in sorted({o['source_node'] for o in wood})};assert len(groups)==(3 if TREE==12 else 2)
  for node,group in groups.items():
   assert len(group)>=3;stem=next(o for o in group if not o.get('inferred_branch'));bpy.ops.object.select_all(action='DESELECT')
   for o in group:o.select_set(True)
   bpy.context.view_layer.objects.active=stem;bpy.ops.object.join();stem['source_node']=node;stem['asset_group']=ASSET;collection.objects.link(stem);joined.append(stem)
  assert triangles(joined)==before;assert protected=={o.name:_geometry(o,protect_appearance=True) for o in foliage};assert images=={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.name in images}
  bpy.ops.wm.save_as_mainfile(filepath=str(out/'normalized.blend'),compress=True);write_json(out/'normalization.json',dict(status='PASS world triangles/RGBA and protected crown exact; projection preparation still pending',approved_model_sha256=sha(source),derived_model_sha256=sha(out/'normalized.blend'),world_triangles=len(before),source_nodes=[o['source_node'] for o in joined],source_part_join_only=True,protected_crown_exact=True,original_images_exact=True))
  mask=B/f'tree{TREE}-bark-proposal-v1/proposed-bark.png';assert np.count_nonzero(np.array(Image.open(mask)))==(505 if TREE==12 else 358);inventory=json.loads((B.parent/'baseline/masks/manifest.json').read_text())
  for row in inventory['masks']:row['png']=str(B.parent/'baseline/masks'/row['png'])
  inventory['masks'].append(dict(index=131,layer=0,layer_index=131,png=str(mask),box_top_left=[0,0],box_size=list(Image.open(mask).size),authored=True,mask_type=0,obstacle_indices=[int(n.rsplit('-',1)[1]) for n in groups]));write_json(out/'mask-inventory.json',inventory);write_json(out/'source-masks.json',dict(version=1,mask_inventory=str(out/'mask-inventory.json'),projections={'exterior':dict(state='Exactly approved observed bark pixels; mixed leaves and ground excluded',source_sha256=sha(B.parent/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=ASSET,mask_indices=[131])])}))
  workspace=out/'asset';prepare(workspace,asset_id=ASSET,scene_name=scene.name,collection_name=collection.name,source_path=B.parent/'baseline/covered.png',grouping_manifest=B.parent/'catalog.json',inventory_path=B.parent/'inventory/inventory.json',review_path=B.parent/'grouping-review.json',source_mask_manifest=out/'source-masks.json',width=384,height=384,framing_padding=1.25,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05));modified(workspace);assert sha(source)==decision['model_sha256'];assert protected=={o.name:_geometry(o,protect_appearance=True) for o in foliage};write_json(out/'prepared.json',dict(status='Private source-protected wood input; original native/camera inspection required before API',approved_geometry_sha256=sha(source),normalized_model_sha256=sha(out/'normalized.blend'),prepared_model_sha256=sha(workspace/'model.blend'),protected_crown_exact=True,workspace=str(workspace),limits=['Wood texture only; approved leaf materials unchanged.','Preparation cameras show wood only; technical input review and exact native sample check required.','No API request or new user approval implied.']))
 finally:release()
if __name__=='__main__':main()
