"""Verify original packed appearance bytes and newly projected native source after reopen."""
import sys,hashlib
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release

def packed():
 images={n.image for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-south-field-wattle-fence' for m in o.data.materials if m and m.use_nodes for n in m.node_tree.nodes if n.type=='TEX_IMAGE' and n.image}
 return {im.name:hashlib.sha256(im.packed_file.data).hexdigest()for im in images if im.packed_file}
def main():
 base=OUT/'texture-fill-round-1/croisement02-south-field-wattle-fence/experiment/bake-v1/worker.blend';d=OUT/'restart3-initial-fence/geometry-v5';model=d/'model.blend';modelsha=sha(model)
 bpy.ops.wm.open_mainfile(filepath=str(base));old=packed();bpy.ops.wm.open_mainfile(filepath=str(model));new=packed();assert all(new.get(n)==h for n,h in old.items()),str({n:[h,new.get(n)]for n,h in old.items()if new.get(n)!=h})
 native=sha(OUT/'animation-references/composite-frame-0.png');assert native in new.values();assert sha(d/'source-domain.png')in new.values();assert sha(model)==modelsha
 write_json(d/'packed-appearance-guard.json',dict(status='PASS',model_sha256=modelsha,base_model_sha256=sha(base),original_packed_images_exact=old,native_source_png_exact=native,new_source_domain_png_exact=sha(d/'source-domain.png'),no_model_saved=True));print('PASS',len(old),'original packed images and exact native PNG',flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
