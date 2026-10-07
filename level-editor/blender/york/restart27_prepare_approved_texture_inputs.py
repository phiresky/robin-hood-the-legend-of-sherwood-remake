"""Prepare fresh source-owned views for exact approved York geometry scopes."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';key=sys.argv[sys.argv.index('--')+1];OUT=WORK/'restart2/approved-texture-inputs-v1'/key
if OUT.exists():raise FileExistsError(OUT)
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
receipt=ROOT/'level-editor/work/croisement02-refinement/restart3-review-batches/pending-v17-v23-plus-two-hub-v1/user-approval.json';assert sha(receipt)=='ca25ba9362b26dfb8ac1239f7acd7b56929463498b125bed0f42dcd98ec628f4'
config={
 'shed':('york-riverside-storehouse-timber-shed','baseline/masks/000001.png',[1432,1019]),
 'storehouse':('york-riverside-stone-storehouse','baseline/masks/000000.png',[1449,840]),
 'well':('york-market-roofed-stone-well','restart7-market-well-v4/native-domain.png',[202,1138]),
 'stable24':('york-castle-winch-stable-components',None,None)}
asset,mask_path,origin=config[key];user=json.loads(receipt.read_text());member=next(m for card in user['decisions_by_card']for m in card['members']if m['asset_id']==asset);model=Path(member['model']);assert sha(model)==member['model_sha256'];assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire()
import bpy
from PIL import Image
from refinement_review import render_review
from refinement_workspace import _geometry
bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
if key=='stable24':
 freeze=json.loads((model.parent/'component-freeze.json').read_text());scene.frame_set(freeze['poses'][44]['tick']);names=freeze['scope'];render_asset='york-castle-winch'
else:names=[o.name for o in scene.objects if o.type=='MESH'and o.get('asset_group')==asset];render_asset=asset
assert names;bpy.context.view_layer.update();before={o.name:_geometry(o,protect_appearance=True)for o in scene.objects};OUT.mkdir(parents=True);(OUT/'model.blend').symlink_to(model)
source=WORK/'baseline/covered.png';label='approved-native'
if key=='stable24':
 source=model.parent/'source-frame44.png';manifest=model.parent/'source-masks.json';label='winch-stable-components-final'
else:
 mask=Image.open(WORK/mask_path).convert('L');mask.save(OUT/'domain.png');inventory={'version':1,'index_namespace':'Frozen approved artwork domain','masks':[{'index':0,'kind':'authored-artwork-domain','box_top_left':origin,'box_size':list(mask.size),'png':'domain.png'}]};(OUT/'inventory.json').write_text(json.dumps(inventory,indent=2)+'\n');manifest=OUT/'source-masks.json';manifest.write_text(json.dumps({'version':1,'mask_inventory':str((OUT/'inventory.json').resolve()),'projections':{label:{'source_sha256':sha(source),'state':'Frozen native covered artwork','assignments':[{'reviewed':True,'asset_group':asset,'mask_indices':[0],'review_evidence':member['source_evidence']}]}}},indent=2)+'\n')
collection=bpy.data.collections.new('Approved York texture context');scene.collection.children.link(collection)
for o in scene.objects:
 if o.type=='MESH':collection.objects.link(o)
receivers=sorted({scene.objects[n].get('source_node')for n in names});occluders=sorted({o.get('source_node')for o in collection.objects if o.get('source_node')});assert None not in receivers
layer={'source_path':str(source.resolve()),'projection_label':label,'receiver_nodes':receivers,'occluder_nodes':occluders}
render_review(OUT/'modified',scene_name=scene.name,collection_name=collection.name,asset_id=render_asset,source_path=source,width=320,height=384,source_mask_manifest=manifest,projection_layers=[layer],render_object_names=names)
assert before=={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.name in before};assert sha(model)==member['model_sha256'];views=json.loads((OUT/'modified/views.json').read_text());counts=[v['counts']['source']for v in views['views']];assert counts[0]>0
report={'status':'PRIVATE_INPUTS_NEED_VISUAL_REVIEW','approved_scope':member,'receipt_sha256':sha(receipt),'geometry_uv_materials_preserved':True,'render_asset':render_asset,'object_names':names,'source_counts':counts,'views_sha256':sha(OUT/'modified/views.json'),'source_sha256':sha(source),'synthesis':'Not requested','limits':['Explicit existing geometry approval permits dependent fill only; resulting appearance still needs grouped review.','Context receivers excluded by exact render names and declared source receiver ownership.']};(OUT/'input-review.json').write_text(json.dumps(report,indent=2)+'\n');assert sum(p.stat().st_size for p in OUT.rglob('*')if p.is_file()and not p.is_symlink())<32*1024**2;print(json.dumps({'output':str(OUT),'source_counts':counts,'objects':len(names)}))
