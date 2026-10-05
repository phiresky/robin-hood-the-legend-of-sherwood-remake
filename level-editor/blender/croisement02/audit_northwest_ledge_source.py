"""Audit the complete native rock footprint and slanted contact-edge coverage."""
import json,math
from pathlib import Path
import sys
import bpy
import numpy as np
from PIL import Image
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from correct_bank_foot import surface
from trial_northwest_ledge26 import hit
from tree_geometry import SIN,COS


def main():
    folder=OUT/'restart2-bank321/northwest-ledge26-v2'
    report=folder/'full-source-audit.json'
    if report.exists():raise FileExistsError(report)
    bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update()
        bank_tree=surface([o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank'])
        bpy.ops.wm.open_mainfile(filepath=str(folder/'worker.blend'));bpy.context.view_layer.update()
        rocks=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-northwest-rock-outcrop'];tree=surface(rocks)
        def visible(x,y):
            p,_,_,d=hit(tree,x,y);b,_,_,bd=hit(bank_tree,x,y)
            return p is not None and(b is None or d<bd)
        vertices=np.array([o.matrix_world@v.co for o in rocks for v in o.data.vertices]);screen=np.column_stack((vertices[:,0],-vertices[:,1]*SIN-vertices[:,2]*COS))
        width=max(1,int(math.ceil(screen[:,0].max()))+2);height=max(1,int(math.ceil(screen[:,1].max()))+2)
        before=np.array(Image.open(OUT/'restart2-bank321/northwest-edge-ramp-v1/after-visible.png').convert('L'))>0
        domain=np.array(Image.open(OUT/'northwest-rock-source-revision/domain-380.png').convert('L'))>0
        after=np.zeros_like(before)
        for y in range(min(height,1152)):
            for x in range(min(width,1792)):after[y,x]=visible(x+.5,y+.5)
        gained=after&~before;regressed=before&~after;foreign=gained&~domain;remaining=domain&~after
        for name,mask in [('native-first-hit',after&domain),('remaining',remaining),('gained',gained),('foreign',foreign),('regressed',regressed)]:Image.fromarray(mask.astype('uint8')*255).save(folder/f'full-source-{name}.png')
        rows=[]
        for x,y in [(108,109),(109,112),(110,115)]:
            points=[]
            for a in range(64):
                for b in range(64):
                    point=np.array([x+(a+.5)/64,y+(b+.5)/64])
                    if visible(*point):points.append(point)
            center=np.array([x+.5,y+.5]);distances=[float(np.linalg.norm(p-center)) for p in points]
            if not points:raise ValueError('No subpixel rock coverage')
            closest=points[int(np.argmin(distances))];lo,hi=0.,1.
            for _ in range(20):
                mid=(lo+hi)/2;point=center+(closest-center)*mid
                if visible(*point):hi=mid
                else:lo=mid
            bound=float(np.linalg.norm(closest-center)*hi)
            rows.append(dict(pixel=[x,y],visible_subpixels=len(points),total_subpixels=4096,visible_fraction=len(points)/4096,nearest_sample_distance=min(distances),distance_to_visible_edge_upper_bound=bound,classification='partially covered terrain-contact edge' if bound<.5 else 'unresolved'))
        write_json(report,dict(model_sha256=sha(folder/'worker.blend'),bank_sha256=sha(bank),baseline_coverage_sha256=sha(OUT/'restart2-bank321/northwest-edge-ramp-v1/after-visible.png'),native_domain_sha256=sha(OUT/'northwest-rock-source-revision/domain-380.png'),scope='Complete in-map rock geometric footprint versus bank, using center rays; unchanged other scenery is not re-audited.',checked_rectangle=[0,0,width,height],native_domain_pixels=int(domain.sum()),native_visible_before=int((before&domain).sum()),native_visible_after=int((after&domain).sum()),new_native_pixels=int((gained&domain).sum()),new_foreign_pixels=int(foreign.sum()),regressed_pixels=int(regressed.sum()),remaining_native_center_misses=int(remaining.sum()),edge_method='64x64 subpixel rays; nearest covered sample and binary search toward pixel center give a 2D upper bound, including slanted boundaries.',edge_rows=rows,geometry_changed=False,source_ownership_changed=False))
        print(json.dumps(dict(new_native=int((gained&domain).sum()),foreign=int(foreign.sum()),regressions=int(regressed.sum()),remaining=int(remaining.sum()),edges=rows)))
    finally:release()


if __name__=='__main__':main()
