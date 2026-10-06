"""Extend this wagon's own straw atlas into unfilled hidden canopy texels."""
import sys,json,hashlib,shutil
from pathlib import Path
from array import array
import bpy,numpy as np
from scipy.ndimage import distance_transform_edt
HERE=Path(__file__).resolve().parent;ROOT=HERE.parents[2]
sys.path[:0]=[str(HERE),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from bake_texture_candidate import snapshot,pixels,array_hash
from render_multiview_asset import render
from refinement_review import _tile
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
E=OUT/'restart4-south-cart-texture/approved-fill-v1/experiment';P=E/'native-retained-v1';D=E/'underedge-fill-v1';meta=json.loads((E/'views.json').read_text());names=set(meta['object_names'])
assert shutil.disk_usage(OUT).free>25*1024**3
acquire()
try:
 assert not D.exists();bpy.ops.wm.open_mainfile(filepath=str(P/'worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;before=snapshot(scene,names)
 obj=scene.objects['Straw canopy shell'];mat=next(m for m in obj.data.materials if m and m.get('source_ownership_bake'));im=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');rgba=pixels(im);before_rgba=rgba.copy()
 report=json.loads((E/'bake-v1/report.json').read_text());row=next(r for r in report['layers'][0]['objects'] if r['object']==obj.name);proof=row['texel_provenance'];assert sha(Path(proof['path']))==proof['sha256'];flags=np.load(proof['path'])['ownership'];assert flags.shape==rgba.shape[:2]
 # Atlas padding and unseen faces share zero provenance. Only their RGB changes;
 # physical alpha and all observed/generated source texels are fixed.
 unknown=flags==0;donors=flags>0;_,nearest=distance_transform_edt(~donors,return_indices=True);rgba[unknown,:3]=before_rgba[nearest[0][unknown],nearest[1][unknown],:3]
 assert np.array_equal(rgba[~unknown],before_rgba[~unknown]);assert np.array_equal(rgba[:,:,3],before_rgba[:,:,3]);im.pixels.foreach_set(rgba.ravel());im.update();im.pack();expected=array_hash(pixels(im));assert snapshot(scene,names)==before
 D.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(D/'worker.blend'),compress=True);digest=sha(D/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(D/'worker.blend'));scene=bpy.data.scenes[meta['scene_name']];bpy.context.window.scene=scene;assert snapshot(scene,names)==before;obj=scene.objects['Straw canopy shell'];mat=next(m for m in obj.data.materials if m and m.get('source_ownership_bake'));im=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');assert array_hash(pixels(im))==expected
 scene.render.engine='CYCLES';scene.cycles.samples=8;render(E/'views.json',D/'actual',width=384);buffers=[]
 for i in range(8):
  im=bpy.data.images.load(str(D/'actual'/f'view-{i}-textured.png'),check_existing=False);a=array('f',[0])*len(im.pixels);im.pixels.foreach_get(a);buffers.append(a);bpy.data.images.remove(im)
 _tile(buffers,384,384,D/'actual/textured.png')
 (D/'underedge-preservation.json').write_text(json.dumps(dict(status='PASS',model_sha256=digest,parent_model_sha256=sha(P/'worker.blend'),parent_native_guard_sha256=sha(P/'native-preservation.json'),provenance=proof,known_and_generated_texels_exact=True,physical_alpha_exact=True,geometry_unchanged=True,modified_image=mat.name,zero_provenance_texels=int(unknown.sum()),method='Nearest existing same-canopy native/generated atlas texel RGB into zero-provenance unseen/padding texels only; no new external references or API',scope='Inferred hidden roof underside material extrapolation; original source and all generated evidence retained; actual review pending'),indent=2)+'\n')
finally:release()
