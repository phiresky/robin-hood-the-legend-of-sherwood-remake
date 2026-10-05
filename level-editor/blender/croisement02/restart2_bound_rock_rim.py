"""Measure a bounded smooth upper-outline correction without sacrificing known source samples."""
import json,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent)]
from catalog import OUT
from log_trap_state_candidate import sha,point
from tree_geometry import RAY,SIN,COS


def main():
    base=OUT/'rock-trap-state-candidate-v14';dest=OUT/'restart2-state/rock-rim-candidate-v1';dest.mkdir(exist_ok=False)
    m=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==m['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'))
    objects=[o for o in bpy.context.scene.objects if o.get('state_endpoint')=='covered'];original={o.name:[v.co.copy()for v in o.data.vertices]for o in objects}
    centers={f'covered inferred complete boulder {r["index"]:02}':r['source_center'][1]for r in m['geometry']if r['state']=='covered'}
    source=OUT/'state-target-evidence/rock-trap';s=json.loads((source/'manifest.json').read_text());left,top,right,bottom=s['bbox'];w,h=right-left,bottom-top;scale=max(w,h)*1.2;alpha=np.asarray(Image.open(source/'tick--01.png'))[:,:,3]>0
    context=json.loads((OUT/'restart2-state/rock-full-context-v1/manifest.json').read_text());samples=[r['render_pixel']for r in context['records']if not r['occluded_by_context']]
    known_origins=[point(left+float(x)+.5,top+float(y)+.5,0)+RAY*5000 for y,x in np.argwhere(alpha)]
    rim_origins=[point(left+w/2+(x+.5-256)*scale/512,top+h/2+(y+.5-256)*scale/512,0)+RAY*5000 for x,y in samples]
    trials=[]
    for amount in [0,.25,.5,.75,1.0]:
        volumes=[];max_change=0
        for obj in objects:
            cy=centers[obj.name];coords=original[obj.name];min_z=min(v.z for v in coords);top_y=min(-v.y*SIN-v.z*COS for v in coords);extent=cy-top_y
            for vertex,p in zip(obj.data.vertices,coords):
                projected_y=-p.y*SIN-p.z*COS;weight=max(0,min(1,(cy-projected_y)/extent));support_weight=max(0,min(1,(p.z-min_z-2)/5));shift=amount*weight*support_weight;vertex.co=p+Vector((0,-shift*SIN,-shift*COS));max_change=max(max_change,shift)
            obj.data.update();bm=bmesh.new();bm.from_mesh(obj.data);assert all(e.is_manifold for e in bm.edges);volumes.append(bm.calc_volume(signed=True));bm.free()
        trees=[BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(p.vertices)for p in obj.data.polygons])for obj in objects]
        def hits(origins):return sum(any(tree.ray_cast(origin,-RAY)[0]is not None for tree in trees)for origin in origins)
        trials.append(dict(amount=amount,max_screen_displacement=max_change,known_native_received=hits(known_origins),remaining_exposed_rim_samples=hits(rim_origins),volumes=volumes))
    baseline=trials[0];eligible=[r for r in trials if r['known_native_received']>=baseline['known_native_received'] and all(v>=old*.95 for v,old in zip(r['volumes'],baseline['volumes']))];best=min(eligible,key=lambda r:(r['remaining_exposed_rim_samples'],r['amount']))
    # Save only if the bounded experiment demonstrates a real improvement.
    result=dict(status='Bounded source-preserving contour diagnostic',base_model_sha256=m['model_sha256'],trials=trials,selected=best,known_sample_count=len(known_origins),rim_sample_count=len(rim_origins),limitations=['Camera depth is preserved; lower support vertices remain unchanged.','A selected derivative still requires saved contact/intersection and visual review.'])
    (dest/'experiment.json').write_text(json.dumps(result,indent=2)+'\n');print(result)


if __name__=='__main__':main()
