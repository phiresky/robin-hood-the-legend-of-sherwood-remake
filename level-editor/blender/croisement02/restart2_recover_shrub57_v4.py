"""Recover metadata after serialization failed, without changing saved candidate."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS
from sign_rigid_depth import bend
from render_slots import acquire,release

def main():
    dest=OUT/'restart2-fence/shrub57-sign-bend-v4';candidate=dest/'model.blend';digest=sha(candidate)
    assert not (dest/'report.json').exists()
    source=OUT/'understory-round-9/assets/croisement02-shrub-57/model.blend';details=OUT/'restart2-fence/sign-fragment-bounds-v1/target-7.json';detail=json.loads(details.read_text())
    assert sha(source)==detail['inputs']['shrub-57']['model_sha256']
    bpy.ops.wm.open_mainfile(filepath=str(source));obj=bpy.data.objects['West Rock Foliage 57'];before=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices]);uv=np.array([tuple(x.uv) for x in obj.data.uv_layers.active.data]);deformation=bend(obj,detail);expected=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices])
    bpy.ops.wm.open_mainfile(filepath=str(candidate));obj=bpy.data.objects['West Rock Foliage 57'];actual=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices]);assert np.array_equal(expected,actual);assert np.array_equal(uv,np.array([tuple(x.uv) for x in obj.data.uv_layers.active.data]))
    error=np.max(abs(np.column_stack((actual[:,0],-SIN*actual[:,1]-COS*actual[:,2]))-np.column_stack((before[:,0],-SIN*before[:,1]-COS*before[:,2]))))
    a,b=[np.array(Image.open(dest/n).convert('RGBA')) for n in ['native-before.png','native-after.png']];opaque=(a[:,:,3]>127)|(b[:,:,3]>127)
    write_json(dest/'report.json',dict(status='Private rigid-pair depth candidate; source and physical checks pending',model_sha256=digest,source_model_sha256=sha(source),constraints_sha256=sha(details),deformation=deformation,uv_unchanged=True,source_projection_max_error=float(error),bounds_before=[before.min(0).tolist(),before.max(0).tolist()],bounds_after=[actual.min(0).tolist(),actual.max(0).tolist()],native_raster=dict(alpha_changed_pixels=int(np.count_nonzero(a[:,:,3]!=b[:,:,3])),opaque_rgb_changed_pixels=int(np.count_nonzero(np.any(a[:,:,:3]!=b[:,:,:3],axis=2)&opaque))),recovery='Metadata recomputed from frozen source; expected deformed saved-float vertex positions and UV equal reopened candidate exactly. Candidate model and images were not rewritten.',limitations=['No user approval inheritance or selector change.','Full32sign, sourceRGBA, and reopened bank crossing checks pending.']))
    assert sha(candidate)==digest
    print(dest)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
