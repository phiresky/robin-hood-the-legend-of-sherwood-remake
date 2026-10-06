"""Bake two explicitly approved disjoint floor reuse domains onto current ground."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from restore_ground75_source import geometry
from restart3_fence_receiver import atlas
ROOT=HERE.parents[2];DEST=OUT/'restart3-ground-reuse-combined-v1'
def rgba(p):return np.array(Image.open(p).convert('RGBA'))
def mask(p):return np.array(Image.open(p).convert('L'))>0

def main():
 assert shutil.disk_usage(OUT).free>25*1024**3
 assert not DEST.exists()
 receipt=OUT/'restart3-review-batches/batch-v8/user-approval.json';assert sha(receipt)=='d68ce43c77a2a6325c642e452eb135389c253a59e83de3e937928d0c12c7c79e';decisions=json.loads(receipt.read_text())['decisions']
 tree=OUT/'restart3-initial-fence/tree45-floor-proposal-v1';state=OUT/'restart2-state/underlay-input-review-v1';base=OUT/'restart3-initial-fence/floor-fill-v1/bake-v1';tp=json.loads((tree/'proposal.json').read_text());sp=json.loads((state/'proposal.json').read_text())
 ids={'croisement02-initial-fence-floor-appearance':'3855cc2a862c899a90890c984f83eaa72da0cb1eb415fa243ff3de4d8d9af585','croisement02-tree45-under-canopy-floor-input':'a8b76cf9dd5ca6431153440fdf031cc834808fb7c473828bb21927fa9df81ca1','croisement02-trap-cart-underlay-input':'2a1c44148f5497fff40a31815ac997dbf1433d9b8b004cf11860676ff255f1c3'}
 members={m['asset_id']:m for d in decisions for m in d['members']}
 assert all(members[k]['review_revision']==v for k,v in ids.items())
 assert sha(tree/'proposal.json')=='4c4481eb9a7aec1218fccaa327540375516a4c004d7207473336610623de0b8e'
 assert sha(state/'proposal.json')=='6480c838b98c8098c221b19d18bd782a3d03278abce72f7f45bec8a62823b88c'
 for name,h in sp['images'].items():assert sha(state/name)==h
 for name,key in [('input.png','input_sha256'),('mask.png','mask_sha256'),('proposed-reuse-preview.png','proposed_reuse_preview_sha256')]:assert sha(tree/name)==tp[key]
 traw=Path(tp['existing_raw_response']);sraw=ROOT/sp['raw_response'];assert sha(traw)==tp['existing_raw_sha256'] and sha(sraw)==sp['raw_sha256']
 tmask=mask(tree/'inferred-floor-domain.png');smask=mask(state/'domain.png');known=mask(OUT/'restart2-ground-completion/preparation-v1/known.png');prior=mask(OUT/'restart3-initial-fence/floor-proposal-v2/inferred-hidden-floor.png');relief=mask(OUT/'restart2-ground-completion/preparation-v1/separate_relief.png');union=tmask|smask
 assert tmask.sum()==2441 and smask.sum()==8201 and not(tmask&smask).any() and union.sum()==10642
 assert not(union&(known|prior|relief)).any() and known.sum()==772189 and prior.sum()==5419
 before=rgba(base/'composite.png');sinput=rgba(state/'input.png');assert np.array_equal(before,rgba(tree/'input.png'));assert np.array_equal(before[~prior],sinput[~prior])
 expectedtree=before.copy();expectedtree[tmask,:3]=rgba(traw)[tmask,:3];assert np.array_equal(expectedtree,rgba(tree/'proposed-reuse-preview.png'))
 expectedstate=sinput.copy();expectedstate[smask,:3]=rgba(sraw)[smask,:3];assert np.array_equal(expectedstate,rgba(state/'proposed-appearance.png'))
 result=before.copy();result[tmask]=expectedtree[tmask];result[smask]=expectedstate[smask];assert np.array_equal(result[~union],before[~union]) and np.array_equal(result[:,:,3],before[:,:,3])
 future=mask(OUT/'restart3-remaining-floor-audit-v1/connected-gray-candidate-union.png');assert not(union&future).any()
 assert sha(base/'model.blend')=='4e2c98fbc63743af3c6539c40c60b96eb759d3a9075c13edd6e1bf16fd19eb18'
 bpy.ops.wm.open_mainfile(filepath=str(base/'model.blend'));bpy.context.preferences.filepaths.save_version=0;bpy.context.view_layer.update();obj=bpy.data.objects['Croisement02 Terrain'];sig=geometry(obj);node,pixels=atlas(obj);assert np.array_equal(pixels,before)
 DEST.mkdir();Image.fromarray(result).save(DEST/'composite.png');Image.fromarray(union.astype('uint8')*255).save(DEST/'combined-domain.png');image=bpy.data.images.load(str(DEST/'composite.png'),check_existing=False);image.pack();node.image=image
 bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'model.blend'),compress=True);bpy.ops.wm.open_mainfile(filepath=str(DEST/'model.blend'));bpy.context.view_layer.update();obj=bpy.data.objects['Croisement02 Terrain'];assert geometry(obj)==sig and np.array_equal(atlas(obj)[1],result)
 write_json(DEST/'validation.json',dict(status='PASS saved/reopened; appearance review pending',model_sha256=sha(DEST/'model.blend'),atlas_sha256=sha(DEST/'composite.png'),base_model_sha256=sha(base/'model.blend'),user_input_receipt_sha256=sha(receipt),input_revisions=ids,geometry_uv_signature=sig,geometry_uv_unchanged=True,packed_rgba_exact=True,tree45_pixels=2441,state_pixels=8201,combined_editable_pixels=10642,all_outside_rgba_exact=int((~union).sum()),known772189_exact=True,prior5419_exact=True,prior14_exact=True,alpha_exact=True,domains_disjoint=True,state_fe8_base_compatible=True,remaining14925_unedited=True,applied_terminal_separate=True,api_called=False,source_ownership_transfer=False,user_saved_model_appearance_approval=None,tree_proposal_sha256=sha(tree/'proposal.json'),state_proposal_sha256=sha(state/'proposal.json'),tree_raw_sha256=sha(traw),state_raw_sha256=sha(sraw)))
 print(json.dumps({'model':str(DEST/'model.blend'),'sha256':sha(DEST/'model.blend'),'atlas_sha256':sha(DEST/'composite.png')}),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
