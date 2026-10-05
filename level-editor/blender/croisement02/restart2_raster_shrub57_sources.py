"""Compare source-first-hit RGBA using a source-space triangle depth buffer."""
import sys,json
from pathlib import Path
import bpy,numpy as np
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import write_json,sha
from sign_source_raster import raster
from render_slots import acquire,release

def main(versions=(5,),name='shrub57-source-raster-v1'):
    dest=OUT/'restart2-fence'/name;dest.mkdir(exist_ok=False);box=[-42,200,116,390];source=OUT/'understory-round-9/assets/croisement02-shrub-57/model.blend'
    bpy.ops.wm.open_mainfile(filepath=str(source));old,roles=raster(bpy.data.objects['West Rock Foliage 57'],box);results=[]
    for version in versions:
        candidate=OUT/f'restart2-fence/shrub57-sign-bend-v{version}/model.blend';bpy.ops.wm.open_mainfile(filepath=str(candidate));new,newroles=raster(bpy.data.objects['West Rock Foliage 57'],box);diff=np.any(abs(old-new)>1e-5,axis=2);known=roles==1
        rows=[dict(pixel=[int(x+box[0]),int(y+box[1])],old_rgba=old[y,x].tolist(),new_rgba=new[y,x].tolist(),old_role=int(roles[y,x]),new_role=int(newroles[y,x])) for y,x in zip(*np.nonzero(diff))]
        results.append(dict(version=version,model_sha256=sha(candidate),source_model_sha256=sha(source),alpha_coverage_changed=int(np.count_nonzero((old[:,:,3]>.5)!=(new[:,:,3]>.5))),rgba_changed=int(diff.sum()),observed_rgba_changed=int((diff&known).sum()),differences=rows))
    write_json(dest/'report.json',dict(status='Native source-space double-precision first-hit comparison',crop=box,method='Rasterizes actual saved triangles in native source XY and chooses nearest alpha-valid surface in double precision. No transparent ray-step epsilon or material lighting.',results=results));print(dest)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
