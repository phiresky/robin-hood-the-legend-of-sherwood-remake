"""Measure timber penetration into terminal wheel rims without editing the worker."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from evidence_io import sha,write_json
from render_slots import acquire,release


def main():
    worker=Path(sys.argv[sys.argv.index('--')+1]);model=worker/'worker.blend';digest=sha(model)
    bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene
    objects=[o for o in scene.objects if o.type=='MESH'];wheels=[o for o in objects if o.name.endswith('wheel rim')]
    bounds={o.name:(np.array([o.matrix_world@v.co for v in o.data.vertices]).min(axis=0),np.array([o.matrix_world@v.co for v in o.data.vertices]).max(axis=0)) for o in objects}
    rows=[]
    for wheel in wheels:
        for obj in objects:
            if obj==wheel or 'wheel' in obj.name.lower() or obj.name=='Detached finite box':continue
            lo,hi=bounds[obj.name];wl,wh=bounds[wheel.name]
            if any(hi<wl) or any(wh<lo):continue
            copy=obj.copy();copy.data=obj.data.copy();scene.collection.objects.link(copy);bpy.context.view_layer.objects.active=copy
            mod=copy.modifiers.new('Exact diagnostic overlap','BOOLEAN');mod.operation='INTERSECT';mod.solver='EXACT';mod.object=wheel;bpy.ops.object.modifier_apply(modifier=mod.name)
            bm=bmesh.new();bm.from_mesh(copy.data);volume=abs(bm.calc_volume(signed=True));bm.free();rows.append(dict(timber=obj.name,wheel=wheel.name,volume=volume))
            bpy.data.objects.remove(copy,do_unlink=True)
    assert sha(model)==digest
    write_json(worker/'rim-penetration.json',dict(status='PASS' if all(r['volume']<.001 for r in rows) else 'HOLD',model_sha256=digest,pairs=rows,maximum_volume=max([r['volume'] for r in rows]or[0]),method='Exact boolean intersections of all bounding-box-overlapping timber against both wheel rims; joint hub/axle overlap separate',worker_unchanged=True))
    print(rows)

if __name__=='__main__':
    acquire()
    try:main()
    finally:release()
