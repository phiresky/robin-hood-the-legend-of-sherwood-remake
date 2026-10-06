"""Recover legacy source ownership for a frozen lower-bark texture candidate.

Reprojection occurs only in memory. The approved model is never saved; exact
original atlas samples are retained separately for later unknown-only filling.
"""
from pathlib import Path
import hashlib,json,sys
import bpy,numpy as np
ROOT=Path.cwd();sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from source_projection_bake import bake
O=ROOT/'level-editor/work/croisement02-refinement';W=O/'root-stem-round-2/assets/croisement02-tree-07';D=O/'restart6-tree07-native-atlas-guard-v1';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest();read=lambda p:json.loads(p.read_text());expected='8bcb2ea9a6c40f920e15590569934f18c3bb3c002c26bb8160a6903605f107df'
def pixels(im):
 a=np.empty(len(im.pixels),np.float32);im.pixels.foreach_get(a);return a.reshape(im.size[1],im.size[0],4)
def state(ob):
 return dict(vertices=[list(v.co)for v in ob.data.vertices],faces=[list(p.vertices)for p in ob.data.polygons],world=[list(v)for v in ob.matrix_world],uv=[list(x.uv)for x in ob.data.uv_layers['Owned source / exterior'].data],slots=[p.material_index for p in ob.data.polygons])
def owned(ob):
 return next((m,next(n.image for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image))for m in ob.data.materials if m and m.get('source_ownership_label')=='exterior')
assert sha(W/'model.blend')==expected;D.mkdir(exist_ok=True);acquire()
try:
 bpy.ops.wm.open_mainfile(filepath=str(W/'model.blend'));cfg=read(W/'workspace.json');bpy.context.window.scene=bpy.data.scenes[cfg['scene_name']];bpy.context.view_layer.update();objects=[o for o in bpy.data.collections[cfg['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==cfg['asset_id'] and o.name.endswith(('wood 058','wood 062'))];assert len(objects)==2;prior={o.name:dict(state=state(o),rgba=pixels(owned(o)[1]),image=owned(o)[1].name)for o in objects}
 report=bake(cfg['map_name'],cfg['source_path'],D/'reprojection.json',receiver_nodes=['building-058','building-062'],projection_label='exterior',elevation_deg=cfg['elevation_degrees'],preserve_authored=False,source_mask_manifest=cfg['source_mask_manifest'],receiver_asset_id=cfg['asset_id'],receiver_object_names=[o.name for o in objects],provenance_directory=D/'provenance')
 records=[]
 for ob in objects:
  old=prior[ob.name];new=pixels(owned(ob)[1]);row=next(r for r in report['objects'] if r['object']==ob.name);pr=row['texel_provenance'];print(pr,flush=True);path=Path(pr.get('path')or pr.get('file'));flags=np.load(path)['ownership'];known=flags==1;unknown=flags==0;geometry_exact=old['state']==state(ob);shape_exact=old['rgba'].shape==new.shape
  rgb_equal=shape_exact and np.array_equal(np.rint(old['rgba'][known,:3]*255).astype(np.uint8),np.rint(new[known,:3]*255).astype(np.uint8));name=ob.name.rsplit(' ',1)[-1];saved=D/f'original-{name}.npz';np.savez_compressed(saved,rgba=old['rgba'],known=known,ownership=flags);records.append(dict(object=ob.name,geometry_uv_slots_exact=geometry_exact,atlas_shape_exact=shape_exact,known_count=int(known.sum()),unknown_or_padding_count=int(unknown.sum()),original_known_rgb8_matches_fresh_source=rgb_equal,original_atlas_path=str(saved),original_atlas_sha256=sha(saved),original_image=old['image'],provenance=pr))
 assert sha(W/'model.blend')==expected
 result=dict(status='PASS' if all(r['geometry_uv_slots_exact'] and r['atlas_shape_exact'] and r['original_known_rgb8_matches_fresh_source'] for r in records)else'HOLD',approved_model=str(W/'model.blend'),model_sha256=expected,source_path=cfg['source_path'],source_sha256=sha(Path(cfg['source_path'])),source_mask_manifest=cfg['source_mask_manifest'],source_mask_sha256=sha(Path(cfg['source_mask_manifest'])),records=records,scope='Only lower058/062 original atlas restoration. Unknown flags include padding; later writes must intersect physical face UV and reviewed generated view support. All alpha and all original known RGBA restored exactly, not regenerated. No saved model/API/render.',recipe_sha256=sha(Path(__file__)));(D/'guard.json').write_text(json.dumps(result,indent=2)+'\n');print(result['status'],flush=True)
finally:release()
