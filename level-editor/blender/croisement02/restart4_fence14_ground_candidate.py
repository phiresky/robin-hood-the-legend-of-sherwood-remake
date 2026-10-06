"""Private conservative contact-shade fallback preserving fourteen exact source pixels."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
HERE=Path(__file__).resolve().parent;sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json
from restore_ground75_source import geometry
from restart3_fence_receiver import atlas
D=OUT/'restart4-fence14-ground-candidate-v1';BASE=OUT/'restart4-final-floor-bake-v1/model.blend'
def main():
 assert not D.exists()and sha(BASE)=='206c7c562b9aa9fc204ce42c0b0e9b1b78a22bee79ac00e6dab7fbbc59c64630'
 proof=OUT/'restart4-fence14-role-audit-v1/report.json';r=json.loads(proof.read_text());assert r['counts']==dict(native_ground_first_hits=14,applied_ground_first_hits=14,native_preserved=0,regenerated_underlay=14,inside_terminal_patch=0)
 coords=[v['pixel']for v in r['rows']if v['state']=='initial'];sourcepath=OUT/'source-states/covered.png';source=np.array(Image.open(sourcepath).convert('RGBA'));assert sha(sourcepath)=='df7939d17d06ef4a9a930aeefafb8c9571b9bfc5dd982c2b39f4c04b0e2b58d2'
 bpy.ops.wm.open_mainfile(filepath=str(BASE));bpy.context.preferences.filepaths.save_version=0;bpy.context.view_layer.update();o=bpy.data.objects['Croisement02 Terrain'];sig=geometry(o);node,original=atlas(o);filled=original.copy();domain=np.zeros(original.shape[:2],bool)
 for x,y in coords:domain[y,x]=True;filled[y,x]=source[y,x]
 known=np.array(Image.open(BASE.parent/'known-native-domain.png').convert('L'))>0;assert domain.sum()==14 and not(domain&known).any()and np.array_equal(filled[~domain],original[~domain])and np.array_equal(filled[:,:,3],original[:,:,3])
 D.mkdir();Image.fromarray(filled).save(D/'composite.png');Image.fromarray(domain.astype('uint8')*255).save(D/'source-context14-domain.png');image=bpy.data.images.load(str(D/'composite.png'),check_existing=False);image.pack();node.image=image;bpy.ops.wm.save_as_mainfile(filepath=str(D/'model.blend'),compress=True);bpy.ops.wm.open_mainfile(filepath=str(D/'model.blend'));bpy.context.view_layer.update();o=bpy.data.objects['Croisement02 Terrain'];assert geometry(o)==sig and np.array_equal(atlas(o)[1],filled)
 write_json(D/'validation.json',dict(status='PASS private source-context appearance candidate; user approval pending',model_sha256=sha(D/'model.blend'),base_model_sha256=sha(BASE),atlas_sha256=sha(D/'composite.png'),source_sha256=sha(sourcepath),role_audit_sha256=sha(proof),exact_source_context_pixels=14,all_outside_rgba_exact=int((~domain).sum()),prior_known_pixels_preserved=int(known.sum()),alpha_exact=True,geometry_uv_signature=sig,geometry_uv_unchanged=True,packed_rgba_exact=True,terminal_patch_overlap=0,initial_and_applied_same_static_context=True,role='Conservative2D source-preserving contact shade fallback; uncertain wood/shadow boundary, not proved physical wood or automatic ground ownership',native_ground_domain_reassignment=False,new_geometry=False,api_calls=0,user_approval=None))
 print('CANDIDATE',sha(D/'model.blend'),flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
