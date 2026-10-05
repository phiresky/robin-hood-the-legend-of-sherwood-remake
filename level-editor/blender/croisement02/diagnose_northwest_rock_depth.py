"""Compare per-piece rigid depth options without modifying or saving geometry."""
import json
from pathlib import Path
import sys
import bpy
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,RAY
from correct_bank_foot import surface


def depths(tree):
    data=np.full((220,160),np.inf)
    for y in range(220):
        for x in range(160):
            p,_,_,d=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+Vector(RAY)*10000,-Vector(RAY))
            if p is not None:data[y,x]=d
    return data


def main():
    output=OUT/'restart2-bank321/northwest-piece-depth-v1'
    if output.exists():raise FileExistsError(output)
    output.mkdir();bank=OUT/'restart2-bank321/foot-candidate-v2/worker.blend';rock=OUT/'restart2-northwest-rock/experiment-sloping-cap-v1/bake-single-v2/worker.blend'
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update();bankdepth=depths(surface([o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank']))
        bpy.ops.wm.open_mainfile(filepath=str(rock));bpy.context.view_layer.update();objects=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-northwest-rock-outcrop'];rockdepth=np.stack([depths(surface([o])) for o in objects]);names=[o.get('source_node') for o in objects]
        domain=np.array(Image.open(OUT/'northwest-rock-source-revision/domain-380.png').convert('L'))[:220,:160]>0;bankdomain=np.array(Image.open(OUT/'terrain-bank-candidate/bank-source-domain.png').convert('L'))[:220,:160]>0
        baseline=rockdepth.min(axis=0)<bankdepth;rows=[]
        for index,name in enumerate(names):
            for shift in [10,20,30,40,51.5]:
                variant=rockdepth.copy();variant[index]-=shift;visible=variant.min(axis=0)<bankdepth;gained=visible&~baseline
                rows.append(dict(part=name,source_ray_shift=shift,new_native_rock=int((gained&domain).sum()),new_native_bank=int((gained&bankdomain).sum()),new_other=int((gained&~domain&~bankdomain).sum()),native_rock_visible=int((visible&domain).sum())))
        target=json.loads((OUT/'restart2-bank321/northwest-rock-contact-v1/report.json').read_text())['rows'];targets=[]
        for r in target:
            x,y=r['pixel'];i=int(rockdepth[:,y,x].argmin());d=rockdepth[i,y,x];targets.append(dict(pixel=[x,y],part=names[i] if np.isfinite(d) else None,depth_gap=float(d-bankdepth[y,x]) if np.isfinite(d) else None))
        np.savez_compressed(output/'depths.npz',bank=bankdepth,rock=rockdepth,names=names)
        write_json(output/'report.json',dict(status='Read-only per-piece diagnosis',bank_sha256=sha(bank),rock_sha256=sha(rock),baseline_native_visible=int((baseline&domain).sum()),options=rows,targets=targets,geometry_changed=False))
        print(json.dumps(rows))
    finally:release()


if __name__=='__main__':main()
