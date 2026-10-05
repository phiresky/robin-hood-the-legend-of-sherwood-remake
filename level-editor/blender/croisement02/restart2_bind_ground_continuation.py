"""Save the fourteen-pixel inferred underlay as a private receiver derivative."""
import sys,json
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
BASE=OUT/'restart2-state/trap-ground-continuation-v1';DEST=OUT/'restart2-state/trap-ground-continuation-model-v1'
def main():
 if DEST.exists():raise FileExistsError(DEST)
 report=json.loads((BASE/'report.json').read_text());model=next(r for r in report['receiver_models']if r['receiver']=='ground');path=Path(model['path']);assert sha(path)==model['sha256'];acquire()
 try:
  DEST.mkdir();bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.preferences.filepaths.save_version=0;bpy.context.view_layer.update();obj=bpy.data.objects['Croisement02 Terrain'];signature=geometry(obj);node,original=atlas(obj);assert np.array_equal(original,np.array(Image.open(report['base_atlas']).convert('RGBA')))
  expected=np.array(Image.open(BASE/'proposed-atlas.png').convert('RGBA'));mask=np.array(Image.open(BASE/'editable.png'))>0;assert mask.sum()==14;assert np.array_equal(original[~mask],expected[~mask]);assert np.array_equal(original[:,:,3],expected[:,:,3]);Image.fromarray(expected).save(DEST/'atlas.png');image=bpy.data.images.load(str(DEST/'atlas.png'),check_existing=False);image.pack();node.image=image;bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'model.blend'),compress=True)
  bpy.ops.wm.open_mainfile(filepath=str(DEST/'model.blend'));bpy.context.view_layer.update();obj=bpy.data.objects['Croisement02 Terrain'];assert geometry(obj)==signature;assert np.array_equal(atlas(obj)[1],expected);assert sha(path)==model['sha256']
  write_json(DEST/'validation.json',{'status':'Saved private inferred receiver appearance; user approval pending','model_sha256':sha(DEST/'model.blend'),'base_model_sha256':model['sha256'],'proposal_sha256':sha(BASE/'report.json'),'geometry_uv_signature':signature,'geometry_uv_unchanged':True,'packed_rgba_exact':True,'changed_pixels':14,'outside_exact':True,'alpha_exact':True,'user_approval':None,'publication':False})
 finally:release()
if __name__=='__main__':main()
