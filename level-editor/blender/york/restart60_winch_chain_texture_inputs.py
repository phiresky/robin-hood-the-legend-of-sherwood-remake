"""Freeze approved winch chain and hardware as source-owned texture receivers."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';O=W/'restart60-winch-chain-texture-inputs-v1'
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
source=W/'restart59-approved-winch-assembly-v1/model.blend'
assert sha(source)=='dd3177aa0639bebadcf99a954f92a3908db2d69a681e2ae48b5592beddbed33f'
receipt=ROOT/'level-editor/work/active/croisement02/restart3-review-batches/pending-v17-v23-plus-two-hub-v1/user-approval.json'
assert sha(receipt)=='ca25ba9362b26dfb8ac1239f7acd7b56929463498b125bed0f42dcd98ec628f4'
assert not O.exists();assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(l.split()[1] for l in Path('/proc/meminfo').read_text().splitlines() if l.startswith('MemAvailable:')))*1024>6*1024**3
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
import bpy
from PIL import Image
from source_projection_bake import bake
from refinement_workspace import _geometry
from refinement_review import render_review
bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.frame_set(88);bpy.context.view_layer.update()
stable=set(json.loads((W/'approved-texture-inputs-v1/stable24/input-review.json').read_text())['object_names'])
receivers=[o for o in scene.objects if o.type=='MESH' and o.get('native_patch')=='patch-004' and o.name not in stable];assert len(receivers)==85
names=[o.name for o in receivers];nodes=sorted({o.get('source_node') for o in receivers});assert None not in nodes
outside={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o not in receivers};before={o.name:_geometry(o) for o in scene.objects}
O.mkdir();print('RECEIVERS',names,flush=True)
native=ROOT/'level-editor/work/active/york/geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((native/'manifest.json').read_text())['records'] if r['id']=='patch-004');f=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition')[44]
im=Image.open(native/f['image']).convert('RGBA');x,y,w,h=f['bbox'];assert im.size==(w,h)
canvas=Image.new('RGBA',(2500,1000));canvas.paste(im,(x,y));canvas.save(O/'source-frame44.png');im.getchannel('A').save(O/'native-alpha.png')
(O/'inventory.json').write_text(json.dumps({'masks':[{'index':0,'box_top_left':[x,y],'box_size':[w,h],'png':'native-alpha.png'}]},indent=2)+'\n')
label='approved-winch-chain-frame44';asset=receivers[0]['asset_group'];assert all(o['asset_group']==asset for o in receivers)
member=next(m for card in json.loads(receipt.read_text())['decisions_by_card'] for m in card['members'] if m.get('model_sha256')=='ebe41b23e0b4f3695eace56ed8fdaf57b4f03eb193932a737b66cee18e2f2422')
(O/'source-masks.json').write_text(json.dumps({'version':1,'mask_inventory':str(O/'inventory.json'),'projections':{label:{'source_sha256':sha(O/'source-frame44.png'),'state':'Patch004 exact transition frame44; alpha plus full first-hit geometry ownership','assignments':[{'reviewed':True,'source_node':n,'mask_indices':[0],'review_evidence':str(receipt)} for n in nodes]}}},indent=2)+'\n')
collection=bpy.data.collections.new('Approved winch chain texture context');scene.collection.children.link(collection)
for o in scene.objects:
 if o.type=='MESH':collection.objects.link(o)
bake('york',O/'source-frame44.png',O/'projection.json',receiver_nodes=nodes,receiver_object_names=names,projection_label=label,source_mask_manifest=O/'source-masks.json',texels_per_unit=2,preserve_authored=False,collection_name=collection.name)
assert before=={o.name:_geometry(o) for o in scene.objects};assert outside=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in outside}
bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(O/'model.blend'),compress=True)
render_review(O/'modified',scene_name=scene.name,collection_name=collection.name,asset_id=asset,source_path=O/'source-frame44.png',width=320,height=384,source_mask_manifest=O/'source-masks.json',projection_layers=[{'source_path':str(O/'source-frame44.png'),'projection_label':label,'receiver_nodes':nodes,'occluder_nodes':sorted({o.get('source_node') for o in collection.objects if o.get('source_node')})}],render_object_names=names)
v=json.loads((O/'modified/views.json').read_text());report={'status':'PRIVATE_INPUT_REQUIRES_VISUAL_REVIEW','approved_scope':member,'receipt_sha256':sha(receipt),'source_model_sha256':sha(source),'derived_model_sha256':sha(O/'model.blend'),'render_asset':asset,'object_names':names,'source_counts':[r['counts']['source'] for r in v['views']],'source_frame_sha256':sha(native/f['image']),'geometry_unchanged':True,'outside_appearance_unchanged':len(outside),'stable24_protected':sorted(stable),'synthesis':'Not requested','policy':'Exact frame44 alpha and first-hit visibility; all unknown chain backs and inferred support surfaces require supplementary material examples. Stable24 and room context excluded from synthesis.'};(O/'input-review.json').write_text(json.dumps(report,indent=2)+'\n');assert sha(source)==report['source_model_sha256'];print(O,flush=True)
