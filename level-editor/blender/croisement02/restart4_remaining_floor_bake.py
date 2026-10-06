"""Apply approved residual ground reuse and exact native background returns."""
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
D=OUT/'restart4-remaining-floor-bake-v1'
def rgba(p):return np.array(Image.open(p).convert('RGBA'))
def mask(p):return np.array(Image.open(p).convert('L'))>0

def main():
 assert shutil.disk_usage(OUT).free>25*1024**3 and not D.exists()
 p=OUT/'restart3-remaining-floor-input-v1';proposal=json.loads((p/'proposal.json').read_text());assert sha(p/'proposal.json')=='efecc0b0a97a899ff2b6689afbdcd3ff27b2dca108263667e6d4ebbefcdd6e5a'
 receipt=OUT/'restart3-review-batches/batch-v9/user-approval.json';assert sha(receipt)=='f2fc3aea62a55c61a44d282d43e9543f9d53f3bd1fa7d57e9afe189c3a4d586c';decisions=json.loads(receipt.read_text())['decisions'];members={m['asset_id']:m for d in decisions for m in d['members']}
 assert members['croisement02-cumulative-ground-reuse-appearance']['review_revision']=='604a2aaccf30e5451658145ed01da441f4cb74a2b378a575fc4ca88e73c61395'
 assert members['croisement02-remaining-floor-reuse-input']['review_revision']=='f1f5ca637b8c8a5c0499ddc8ba0f9031b42afb065251a8557ba68e2443ac9e06'
 for n,h in proposal['images'].items():assert sha(p/n)==h
 base=Path(proposal['base_model']);assert sha(base)==proposal['base_model_sha256']=='868cf916e4b8d7e808262974396f825080e053aac9173923fa513a1c12012929'
 original=rgba(p/'input.png');filled=rgba(p/'proposed-appearance.png');inferred=mask(p/'inferred-domain.png');native=mask(p/'native-return-domain.png');domain=inferred|native
 assert inferred.sum()==14876 and native.sum()==49 and not(inferred&native).any() and domain.sum()==14925
 source=OUT/'animation-references/composite-frame-0.png';raw=Path(proposal['raw_response']);assert sha(raw)==proposal['raw_sha256'] and sha(source)==proposal['native_source_sha256']
 expected=original.copy();expected[inferred,:3]=rgba(raw)[inferred,:3];expected[native]=rgba(source)[native];assert np.array_equal(filled,expected) and np.array_equal(filled[:,:,3],original[:,:,3])
 known=mask(OUT/'restart2-ground-completion/preparation-v1/known.png');relief=mask(OUT/'restart2-ground-completion/preparation-v1/separate_relief.png');prior=mask(base.parent/'combined-domain.png')|mask(OUT/'restart3-initial-fence/floor-proposal-v2/inferred-hidden-floor.png');assert not(domain&(known|relief|prior)).any() and np.array_equal(filled[~domain],original[~domain])
 bpy.ops.wm.open_mainfile(filepath=str(base));bpy.context.preferences.filepaths.save_version=0;bpy.context.view_layer.update();obj=bpy.data.objects['Croisement02 Terrain'];sig=geometry(obj);node,pixels=atlas(obj);assert np.array_equal(pixels,original)
 D.mkdir();Image.fromarray(filled).save(D/'composite.png');Image.fromarray((known|native).astype('uint8')*255).save(D/'known-native-domain.png');im=bpy.data.images.load(str(D/'composite.png'),check_existing=False);im.pack();node.image=im
 bpy.ops.wm.save_as_mainfile(filepath=str(D/'model.blend'),compress=True);bpy.ops.wm.open_mainfile(filepath=str(D/'model.blend'));bpy.context.view_layer.update();obj=bpy.data.objects['Croisement02 Terrain'];assert geometry(obj)==sig and np.array_equal(atlas(obj)[1],filled)
 write_json(D/'validation.json',dict(status='PASS saved/reopened; appearance pending',model_sha256=sha(D/'model.blend'),atlas_sha256=sha(D/'composite.png'),base_model_sha256=sha(base),proposal_sha256=sha(p/'proposal.json'),approval_receipt_sha256=sha(receipt),inferred_pixels=14876,native_return_pixels=49,native_returns_exact=True,combined_editable_pixels=14925,all_outside_rgba_exact=2049459,prior_known772189_exact=True,new_known_native_pixels=int((known|native).sum()),known_native_domain_sha256=sha(D/'known-native-domain.png'),prior5419_2441_8201_14_exact=True,relief_exact=True,alpha_exact=True,geometry_uv_signature=sig,geometry_uv_unchanged=True,packed_rgba_exact=True,transitioning_overlay_preserved_separately=True,api_called=False,user_saved_appearance_approval=None))
 print(json.dumps({'model':str(D/'model.blend'),'sha256':sha(D/'model.blend'),'atlas_sha256':sha(D/'composite.png')}),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
