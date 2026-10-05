"""Prepare three exact approved cart geometries for source-protected texture filling."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json,record_recipe
from render_slots import acquire,release
from refinement_review import render_review
from prepare_private_texture_inputs import prepare
from restart2_prepare_endpoint_inputs import signature


def prepare_one(key):
    decisions_path=OUT/'restart2-state/batch-v4-state-approvals-v1/decisions.json'
    decisions=json.loads(decisions_path.read_text())
    ids={'north-initial':'croisement02-north-cart-initial-physical','south-cargo':'croisement02-south-cart-terminal-cask','south-loosewood':'croisement02-south-cart-terminal-loose-wood'}
    asset=ids[key];decision=next(r for r in decisions if r['asset_id']==asset)
    assert decision['decision']=='approved' and decision['scope']=='geometry'
    model=Path(decision['model']);base=model.parent;expected=decision['model_sha256'];assert sha(model)==expected
    meta=json.loads((base/'manifest.json').read_text());source=base/'source.png';placement=meta['source_box'][:2]
    dest=OUT/'restart3-approved-cart-texture-inputs-v1'/key;dest.mkdir(parents=True,exist_ok=False)
    approval=OUT/'restart3-review-batches/batch-v4/user-approval.json';assert sha(approval)==decision['source_user_receipt_sha256']
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();objects=sorted([o for o in scene.objects if o.type=='MESH'],key=lambda o:o.name);before=signature(objects)
        collection=bpy.data.collections.new(key+' approved cart Working');scene.collection.children.link(collection)
        for index,obj in enumerate(objects):
            collection.objects.link(obj);obj['asset_group']=asset;obj['source_node']=f'{asset}-part-{index:03}';obj.hide_render=False
        canvas=Image.new('RGBA',Image.open(OUT/'baseline/covered.png').size);canvas.alpha_composite(Image.open(source).convert('RGBA'),tuple(placement));canvas.save(dest/'source.png');alpha=np.asarray(canvas)[:,:,3]>0;Image.fromarray(alpha.astype(np.uint8)*255).save(dest/'source-alpha.png')
        write_json(dest/'mask-inventory.json',dict(masks=[dict(index=0,box_top_left=[0,0],box_size=list(canvas.size),png='source-alpha.png')]))
        write_json(dest/'source-masks.json',dict(version=1,mask_inventory='mask-inventory.json',projections=dict(exterior=dict(source_sha256=sha(dest/'source.png'),state=decision['scope_description'],assignments=[dict(reviewed=True,asset_group=asset,mask_indices=[0])]))))
        assert before==signature(objects);bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'),compress=True)
        render_review(dest/'modified',scene_name=scene.name,collection_name=collection.name,asset_id=asset,source_path=dest/'source.png',source_mask_manifest=dest/'source-masks.json',width=384,height=384,framing_padding=1.15)
        assert before==signature(objects);assert sha(model)==expected
        write_json(dest/'derivation.json',dict(status='Exact geometry preparation; synthesis approval bridge owned by texture lane',source_model=str(model),source_model_sha256=expected,prepared_model_sha256=sha(dest/'model.blend'),geometry_uv_material_signature=before,geometry_uv_materials_unchanged=True,source_image=str(source),source_sha256=sha(source),source_placement=placement,source_alpha_pixels=int(alpha.sum()),object_names=[o.name for o in objects],approval_evidence=str(approval),approval_evidence_sha256=sha(approval),exact_decision=decision,decision_archive_sha256=sha(decisions_path),generation_authorized=False,recipe=record_recipe(dest,Path(__file__))))
        prepare(dest,sha(dest/'model.blend'),dest/'private-inputs')
        frames=json.loads((dest/'modified/views.json').read_text());rows=[]
        for frame in frames['views']:
            i=frame['index'];known=np.asarray(Image.open(dest/f'modified/views/view-{i}-known.png'))[:,:,0]>127;solid=np.asarray(Image.open(dest/f'modified/views/view-{i}-solid.png'))[:,:,3]>0;editable=np.asarray(Image.open(dest/f'private-inputs/views/view-{i}-mask.png'))[:,:,3]==0
            assert np.array_equal(editable,solid&~known);assert not any([solid[0].any(),solid[-1].any(),solid[:,0].any(),solid[:,-1].any()]);rows.append(dict(index=i,known_pixels=int(known.sum()),editable_pixels=int(editable.sum()),native_and_background_protected=True))
        matrix=np.asarray(frames['views'][0]['camera_matrix_world']);direction=matrix[:3,:3]@np.array([0,0,1]);assert np.dot(direction,np.array([0,-.819152044,.573576436]))>.99999
        write_json(dest/'private-input-audit.json',dict(status='PASS unchanged approved geometry, native-first complete framing and exact editable-only masks',source_model_sha256=expected,prepared_model_sha256=sha(dest/'model.blend'),derivation_sha256=sha(dest/'derivation.json'),inputs_sha256=sha(dest/'private-inputs/private-inputs.json'),canvas=[1536,768],views=rows,generation_authorized=False))
    finally:release()

if __name__=='__main__':
    keys=sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else ['north-initial','south-cargo','south-loosewood']
    for key in keys:prepare_one(key)
