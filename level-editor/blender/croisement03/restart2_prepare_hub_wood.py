"""Prepare one approved hub tree's exact woody surfaces; protected crown unchanged."""
import sys,json,hashlib,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_workspace import prepare,modified,_geometry
from evidence_io import sha,write_json
B=R/'level-editor/work/croisement03-refinement/restart2'
def triangle_signature(objects):
 rows=[]
 for o in objects:
  m=o.data;m.calc_loop_triangles();layers=sorted(m.uv_layers,key=lambda x:x.name)
  for tri in m.loop_triangles:
   corners=[]
   for vi,li in zip(tri.vertices,tri.loops):corners.append((tuple(round(float(x),5) for x in o.matrix_world@m.vertices[vi].co),tuple((uv.name,tuple(round(float(x),7) for x in uv.data[li].uv)) for uv in layers)))
   rows.append((tuple(sorted(corners)),m.materials[tri.material_index].name))
 return sorted(rows)
def main(tree):
 assert tree in range(2,12);asset=f'croisement03-tree-{tree:02}';archive=B/'approved-hub-v17-v23-plus-two-v1';decision=json.loads((archive/'verified-scope.json').read_text());member=decision['effective_assets'][asset];source=Path(member['model']);assert sha(source)==member['model_sha256']
 masks=[Path(p) for p,h in decision['verified_files'].items() if f'tree{tree:02}-bark-proposal-' in p and p.endswith('/proposed-bark.png')]
 assert len(masks)==1,'Need explicitly bound bark domain or packed-material evidence before preparation';mask=masks[0];assert sha(mask)==decision['verified_files'][str(mask)]
 assert shutil.disk_usage(R).free>10*1024**3;available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024;assert available>=6*1024**3
 out=B/'approved-hub-textures-v1'/asset/'wood-input-v1';out.mkdir(parents=True,exist_ok=False);acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.preferences.filepaths.save_version=0;scene=bpy.context.scene;scene.name='Croisement03 Refinement';scene.render.threads_mode='FIXED';scene.render.threads=2;collection=bpy.data.collections.new('Croisement03 Working');scene.collection.children.link(collection)
  wood=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==asset];assert wood;protected_objects=[o for o in scene.objects if o.type=='MESH' and o not in wood];assert protected_objects
  protected={o.name:_geometry(o,protect_appearance=True) for o in protected_objects};images={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};before=triangle_signature(wood)
  groups={node:[o for o in wood if o.get('source_node')==node] for node in sorted({o['source_node'] for o in wood})};joined=[]
  for node,group in groups.items():
   assert node.startswith('building-');bpy.ops.object.select_all(action='DESELECT')
   for o in group:o.select_set(True)
   stem=group[0];bpy.context.view_layer.objects.active=stem
   if len(group)>1:bpy.ops.object.join()
   stem['source_node']=node;stem['asset_group']=asset;collection.objects.link(stem);joined.append(stem)
  assert triangle_signature(joined)==before,'World geometry, UV or material changed while joining';assert protected=={o.name:_geometry(o,protect_appearance=True) for o in protected_objects};assert images=={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.name in images}
  bpy.ops.wm.save_as_mainfile(filepath=str(out/'normalized.blend'),compress=True);write_json(out/'normalization.json',dict(status='PASS exact world triangles/UV/materials and protected appearance',approved_model=str(source),approved_model_sha256=sha(source),normalized_model_sha256=sha(out/'normalized.blend'),source_nodes=list(groups),triangles=len(before),protected_objects=list(protected),all_images_rgba_exact=True,geometry_scope=member,receipt_sha256=decision['receipt_sha256']))
  inventory=json.loads((B.parent/'baseline/masks/manifest.json').read_text())
  for row in inventory['masks']:row['png']=str(B.parent/'baseline/masks'/row['png'])
  index=max(r['index'] for r in inventory['masks'])+1;count=int(np.count_nonzero(np.array(Image.open(mask))));inventory['masks'].append(dict(index=index,layer=0,layer_index=index,png=str(mask),box_top_left=[0,0],box_size=list(Image.open(mask).size),authored=True,mask_type=0,obstacle_indices=[int(n.rsplit('-',1)[1]) for n in groups]));write_json(out/'mask-inventory.json',inventory);write_json(out/'source-masks.json',dict(version=1,mask_inventory=str(out/'mask-inventory.json'),projections={'exterior':dict(state='Exact bound observed bark domain; all other artwork excluded',source_sha256=sha(B.parent/'baseline/covered.png'),assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[index])])}))
  workspace=out/'asset';prepare(workspace,asset_id=asset,scene_name=scene.name,collection_name=collection.name,source_path=B.parent/'baseline/covered.png',grouping_manifest=B.parent/'catalog.json',inventory_path=B.parent/'inventory/inventory.json',review_path=B.parent/'grouping-review.json',source_mask_manifest=out/'source-masks.json',width=384,height=384,framing_padding=1.25,lighting=dict(toward_sun=[-.45,-.55,.70],ambient=.22,diffuse=.78,shadow_epsilon=.05));modified(workspace)
  assert sha(source)==member['model_sha256'];assert protected=={o.name:_geometry(o,protect_appearance=True) for o in protected_objects};write_json(out/'prepared.json',dict(status='Private input prepared; source first-hit and actual sheet review required before API',asset_id=asset,approved_model_sha256=sha(source),workspace=str(workspace),bark_domain=str(mask),bark_pixels=count,protected_crown_exact=True,approval_receipt_sha256=decision['receipt_sha256'],limits=['No new geometry or source ownership.','Only the two permitted Leicester bark examples may supplement actual images.','Do not generate until native source and packet checks pass.']))
 finally:release()
if __name__=='__main__':
 args=sys.argv[sys.argv.index('--')+1:];assert len(args)==1;main(int(args[0]))
