"""Render ownership-aware bark transport from the exact approved Tree08 geometry."""
import hashlib,json,os,shutil,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from refinement_review import render_review
R=ROOT/'level-editor/work/active/croisement01/restart2'
MODEL=R/'tree08-wood-prototype-v23-smooth-join/model.blend'
OUT=R/'approved-tree08-bark-fill-v3/croisement01-tree-08'
APPROVAL=ROOT/'level-editor/work/active/croisement02/restart3-review-batches/three-tree-geometry-oct11-v1/user-approval.json'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def write(p,x):p.write_text(json.dumps(x,indent=2)+'\n')
assert sha(MODEL)=='b0c061321400d466b28d41cd90d45a30b86c11a25aaa904bc54f3b7d97e50242'
assert sha(APPROVAL)=='a30e6aa6559e71347d7e1ee4b37a51defa2d4d880504ba7eeb334feafcef7d1e'
assert not OUT.exists()
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 OUT.mkdir(parents=True);os.link(MODEL,OUT/'model.blend')
 bpy.ops.wm.open_mainfile(filepath=str(MODEL));scene=bpy.context.scene
 scene.render.threads_mode='FIXED';scene.render.threads=2
 objects=[o for o in scene.objects if o.type=='MESH'];assert len(objects)==1
 obj=objects[0];obj['asset_group']='croisement01-tree-08'
 collection=bpy.data.collections.new('Tree08 approved wood transport');scene.collection.children.link(collection);collection.objects.link(obj)
 bpy.context.preferences.filepaths.save_version=0
 bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'transport-model.blend'),compress=True)

 cfg=json.loads((R/'tree01-soil-joint-v10/assets/croisement01-tree-01/workspace.json').read_text())
 source=Path(cfg['source_path']);assert sha(source)=='e6c41651511d5e593c2df079cccbfd4c5c272d79ae662943b1bea74cd6f97ec6'
 mask=R/'tree08-semantic-source-v1/bark-core-proposal.png';shutil.copyfile(mask,OUT/'bark-core.png')
 write(OUT/'mask-inventory.json',{'version':1,'masks':[{'index':0,'box_top_left':[331,11],'box_size':[446,461],'png':'bark-core.png'}],'note':'Derived semantic ownership bitmap, not a game mask ID. Exact approved 6276-pixel bark core.'})
 write(OUT/'source-mask-manifest.json',{'version':1,'mask_inventory':'mask-inventory.json','projections':{'exterior':{'source_sha256':sha(source),'state':'Approved Tree08 wood: exact 6276 source-owned bark pixels; foliage and ambiguous pixels outside this receiver scope.','assignments':[{'reviewed':True,'asset_group':'croisement01-tree-08','mask_indices':[0]}]}},'approval':str(APPROVAL),'approval_sha256':sha(APPROVAL)})
 result=render_review(OUT/'modified',scene_name=scene.name,collection_name=collection.name,asset_id='croisement01-tree-08',source_path=source,width=384,height=384,elevation_degrees=35,context_padding=24,framing_padding=1.12,lighting=cfg['lighting'],source_mask_manifest=OUT/'source-mask-manifest.json')
 write(OUT/'workspace.json',{'asset_id':'croisement01-tree-08','scene_name':scene.name,'collection_name':collection.name,'source_path':str(source),'source_mask_manifest':str(OUT/'source-mask-manifest.json'),'width':384,'height':384,'elevation_degrees':35,'context_padding':24,'framing_padding':1.12,'lighting':cfg['lighting']})
 write(OUT/'derivation-proof.json',{'status':'PASS_EXACT_APPROVED_GEOMETRY_TRANSPORT','model_sha256':sha(MODEL),'approval_sha256':sha(APPROVAL),'ownership_mask_sha256':sha(mask),'views_sha256':sha(OUT/'modified/views.json'),'transport_model_sha256':sha(OUT/'transport-model.blend'),'transport_derivation':'Only asset_group and collection membership metadata assigned for shared renderer/baker; original geometry and packed source unchanged.','source_rgba_policy':'Only approved bark core is known. All other wood surface is inferred; no foliage or neighboring texture projected. Final native first-hit RGBA guard remains required after baking.','object':obj.name,'source_node':obj.get('source_node')})
 assert sha(MODEL)=='b0c061321400d466b28d41cd90d45a30b86c11a25aaa904bc54f3b7d97e50242'
 print('TREE08_PACKET_READY',OUT,flush=True)
finally:release()
