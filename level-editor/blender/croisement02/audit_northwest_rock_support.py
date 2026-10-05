"""Compare a scoped rock-edge underside with the approved physical bank."""
import json
from pathlib import Path
import sys
import bpy
import numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from correct_bank_foot import surface


def main():
    folder=OUT/'restart2-bank321/northwest-edge-ramp-v1'
    output=folder/'underside-support.json'
    if output.exists():raise FileExistsError(output)
    source=OUT/'restart2-northwest-rock/experiment-sloping-cap-v1/bake-single-v2/worker.blend'
    bank=OUT/'restart2-bank321/packaged-v1/assets/croisement02-north-woodland-bank/model.blend'
    acquire()
    try:
        bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update()
        obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='building-035')
        before=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);bottom=before[:,2]<=before[:,2].min()+.001
        obj.data.calc_loop_triangles();triangles=[tuple(t.vertices) for t in obj.data.loop_triangles if all(bottom[i] for i in t.vertices)]
        bpy.ops.wm.open_mainfile(filepath=str(folder/'worker.blend'));bpy.context.view_layer.update()
        obj=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('source_node')=='building-035')
        after=np.array([obj.matrix_world@v.co for v in obj.data.vertices])
        moved=np.max(abs(after-before),axis=1)>.0001
        samples=[dict(kind='moved-bottom-vertex',vertex=int(i),world=after[i].tolist()) for i in np.flatnonzero(bottom&moved)]
        for triangle in triangles:
            if not any(moved[i] for i in triangle):continue
            for a in range(9):
                for b in range(9-a):
                    weights=np.array([a,b,8-a-b])/8
                    point=weights@after[list(triangle)]
                    samples.append(dict(kind='bottom-face-sample',triangle=list(triangle),world=point.tolist()))
        if not samples:raise ValueError('No moved underside samples found')
        bpy.ops.wm.open_mainfile(filepath=str(bank));bpy.context.view_layer.update()
        tree=surface([o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank'])
        for sample in samples:
            x,y,z=sample['world'];hit,_,_,_=tree.ray_cast(Vector((x,y,2000)),Vector((0,0,-1)))
            sample['receiver_z']=hit.z if hit is not None else None
            sample['gap']=z-hit.z if hit is not None else None
        gaps=[s['gap'] for s in samples if s['gap'] is not None]
        write_json(output,dict(status='physical underside sampling; visual review remains required',model_sha256=sha(folder/'worker.blend'),bank_sha256=sha(bank),source_sha256=sha(source),moved_bottom_vertices=int((bottom&moved).sum()),bottom_triangles=len(triangles),samples=len(samples),samples_without_bank=sum(s['gap'] is None for s in samples),positive_gap_samples=sum(g>.001 for g in gaps),maximum_gap=max(gaps) if gaps else None,minimum_gap=min(gaps) if gaps else None,rows=samples))
        print(json.dumps(dict(samples=len(samples),positive_gaps=sum(g>.001 for g in gaps),max_gap=max(gaps) if gaps else None)))
    finally:release()


if __name__=='__main__':main()
