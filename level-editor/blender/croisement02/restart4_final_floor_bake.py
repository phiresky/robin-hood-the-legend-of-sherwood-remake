"""Bake the two explicitly approved disjoint floor-reuse scopes onto saved ground."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from restore_ground75_source import geometry
from restart3_fence_receiver import atlas
D=OUT/'restart4-final-floor-bake-v1'
def rgba(p):return np.array(Image.open(p).convert('RGBA'))
def mask(p):return np.array(Image.open(p).convert('L'))>0
def main():
 assert shutil.disk_usage(OUT).free>23*2**30 and not D.exists()
 receipt=OUT/'restart3-review-batches/batch-v11/user-approval.json';assert sha(receipt)=='373f735cc92928173ad113d04ca5ee93976946ffbbf95129aa51dd76b09eef9f';approval=json.loads(receipt.read_text());assert approval['exact_user_text']=='All four approved'
 members={m['asset_id']:m for c in approval['cards'] for m in c['members']}
 small=OUT/'restart4-floor-closure-input-v1';bank=OUT/'restart4-bank-underlay-input-v1'
 for p,asset,h in [(small,'croisement02-final-nonrelief-floor-input','40bdfaec7930ae02a60127e5de8e1bf6a64ba783be9a698db28f5c66315c93b8'),(bank,'croisement02-bank-underlay-floor-input','b639342dfa2b2f8ba8750f80c2595e35385dd4d50fbb9519ceceb323915708fe')]:
  assert sha(p/'proposal.json')==h;ev=json.loads((p/'gallery/evidence.json').read_text())['items'][0];assert members[asset]['review_revision']==ev['review_revision'];pr=json.loads((p/'proposal.json').read_text())
  for n,expected in pr['files'].items():assert sha(p/n)==expected
  write_json(p/'user-input-approval.json',dict(status='user-approved',scope=ev['review_scope'],proposal_sha256=h,receipt_sha256=sha(receipt),receipt=str(receipt),answer=approval['exact_user_text'],review_revision=ev['review_revision']))
 base=OUT/'restart4-remaining-floor-bake-v1/model.blend';assert sha(base)=='76bbaeb029104448ccf06a257de9151bba254a9a563a538189be16506495c3f3'
 original=rgba(base.parent/'composite.png');known=mask(base.parent/'known-native-domain.png');a=mask(small/'domain.png');b=mask(bank/'inferred-domain.png');native=mask(small/'native-return-domain.png');domain=a|b;assert a.sum()==5930 and b.sum()==569880 and not(a&b).any() and not(domain&known).any()
 filled=original.copy();filled[a]=rgba(small/'proposed-appearance.png')[a];filled[b]=rgba(bank/'proposed-appearance.png')[b];assert np.array_equal(filled[~domain],original[~domain]) and np.array_equal(filled[:,:,3],original[:,:,3])
 assert not np.all(filled[:,:,:3]==127,axis=2).any()
 bpy.ops.wm.open_mainfile(filepath=str(base));bpy.context.preferences.filepaths.save_version=0;bpy.context.view_layer.update();obj=bpy.data.objects['Croisement02 Terrain'];sig=geometry(obj);node,actual=atlas(obj);assert np.array_equal(actual,original)
 D.mkdir();Image.fromarray(filled).save(D/'composite.png');Image.fromarray((known|native).astype('uint8')*255).save(D/'known-native-domain.png');Image.fromarray(domain.astype('uint8')*255).save(D/'combined-domain.png');im=bpy.data.images.load(str(D/'composite.png'),check_existing=False);im.pack();node.image=im
 bpy.ops.wm.save_as_mainfile(filepath=str(D/'model.blend'),compress=True);bpy.ops.wm.open_mainfile(filepath=str(D/'model.blend'));bpy.context.view_layer.update();obj=bpy.data.objects['Croisement02 Terrain'];assert geometry(obj)==sig and np.array_equal(atlas(obj)[1],filled)
 write_json(D/'validation.json',dict(status='PASS saved/reopened; saved appearance pending',model_sha256=sha(D/'model.blend'),atlas_sha256=sha(D/'composite.png'),base_model_sha256=sha(base),approval_receipt_sha256=sha(receipt),small_proposal_sha256=sha(small/'proposal.json'),bank_proposal_sha256=sha(bank/'proposal.json'),editable_pixels=int(domain.sum()),inferred_pixels=int((domain&~native).sum()),native_returns=int(native.sum()),known_rgba_preserved=int(known.sum()),new_known_pixels=int((known|native).sum()),all_outside_rgba_exact=int((~domain).sum()),prior_ground_changes_preserved=True,alpha_exact=True,geometry_uv_signature=sig,geometry_uv_unchanged=True,packed_rgba_exact=True,neutral_gray_atlas_pixels=0,bank_material_geometry_untouched=True,state_overlays_unchanged=True,source_ownership_transfer=False,foreground_geometry_completion=False,api_calls=0,user_saved_appearance_approval=None))
 print('SAVED',sha(D/'model.blend'),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
