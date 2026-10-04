"""Compare native north-fringe ordering with selected adjacent crown scaffolds."""
import json,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(Path(__file__).parent));sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from masks_to_depth import character_threshold
from render_slots import acquire,release

def main():
    destination=OUT/'understory-candidates/north-fringe22-audit';destination.mkdir(exist_ok=False)
    rows={r['index']:r for r in json.loads((OUT/'baseline/masks/manifest.json').read_text())['masks']}
    def canvas(index):
        r=rows[index];x,y=r['box_top_left'];w,h=r['box_size'];a=np.zeros((1152,1792),bool);a[y:y+h,x:x+w]=np.asarray(Image.open(OUT/'baseline/masks'/r['png']).convert('L'))>127;return a
    observed=canvas(22)&~canvas(134)&~canvas(135);Image.fromarray(observed.astype('uint8')*255).save(destination/'domain-480.png')
    yy,xx=np.nonzero(observed);points=np.column_stack((xx+.5,yy+.5));samples=points[::max(1,len(points)//80)];results=[]
    for index in (14,15):
        worker=tree_workspace(index);digest=sha(worker/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update()
        objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')=='crown']
        if not objects:raise ValueError('No selected neighbouring crown')
        trees=[BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(p.vertices) for p in o.data.polygons]) for o in objects];hits=[]
        for x,y in samples:
            origin=Vector((float(x),float(-y*SIN),float(-y*COS)))+RAY*2000.;choices=[]
            for tree in trees:
                hit,normal,face,distance=tree.ray_cast(origin,-RAY,4000.)
                if hit is not None:choices.append((distance,hit))
            if choices:
                distance,hit=min(choices,key=lambda row:row[0]);hits.append(dict(source=[float(x),float(y)],world=list(hit),ray_parameter=2000.-distance))
        if sha(worker/'model.blend')!=digest:raise ValueError('Selected neighbour changed during audit')
        results.append(dict(tree=index,worker=str(worker),model_sha256=digest,ray_samples=len(samples),raw_crown_hits=hits,limitation='Raw triangle intersections estimate existing crown scaffold only; texture alpha and material visibility are not recovered geometry evidence.'))
    records=[]
    for index in (22,14,15,134,135):
        r=rows[index];entry=dict(native_mask=index,layer=r['layer'],mask_type=r['mask_type'],bbox=[*r['box_top_left'],*r['box_size']],character_polyline=r.get('character_polyline'),projectile_polyline=r.get('projectile_polyline'))
        if r.get('character_polyline'):
            thresholds=character_threshold(r['character_polyline'],points[:,0]);valid=thresholds>0;entry['character_threshold_over_unique22']=[float(thresholds[valid].min()),float(np.median(thresholds[valid])),float(thresholds[valid].max())] if valid.any() else None
        records.append(entry)
    write_json(destination/'evidence.json',dict(source_sha256=sha(OUT/'animation-references/composite-frame-0.png'),native_masks=records,unique_leaf_pixels=int(observed.sum()),domain_sha256=sha(destination/'domain-480.png'),selected_crowns=results,status='Read-only placement evidence; no geometry or source assignment',conclusions=['Native22 has a shallow character threshold and no observed trunk. It is not evidence for a grounded shrub.','Animated134/135 have projectile polylines only, so a character depthfield cannot directly compare their physical crown heights.','Any continuation should be a small elevated north-edge foliage component near selected crown scaffolds; association remains inferred.']))
    print(destination,flush=True)
if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
