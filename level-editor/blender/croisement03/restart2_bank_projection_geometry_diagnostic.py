"""Read-only comparison of a held source projection against its frozen geometry input."""
import json
import sys
from pathlib import Path
import bpy
import numpy as np
sys.path.insert(0,str(Path(__file__).resolve().parent))
import restart2_tree01_canopy_saved as guard
B=guard.B


def capture(path,update):
    bpy.ops.wm.open_mainfile(filepath=str(path))
    if update:
        bpy.context.view_layer.update()
    return {o.name:{'local':np.array([list(v.co)for v in o.data.vertices]),'world':np.array([list(o.matrix_world@v.co)for v in o.data.vertices]),'basis':np.array(o.matrix_basis),'matrix_world':np.array(o.matrix_world),'faces':[list(p.vertices)for p in o.data.polygons]}for o in bpy.context.scene.objects if o.type=='MESH'}


def main():
    out=B/'bank-coupled-left-source-v1/geometry-failure-diagnostic-v1.json'
    assert not out.exists()
    paths=[B/'bank-coupled-left-saved-v1/worker.blend',B/'bank-coupled-left-source-v1/worker.blend']
    pins={str(p):guard.sha(p)for p in paths}
    guard.resources();guard.acquire(slots=2)
    try:
        guard.resources();rounds=[]
        for update in [False,True]:
            a,b=[capture(p,update)for p in paths];assert a.keys()==b.keys();rows=[]
            for name in a:
                row={'object':name,'same_faces':a[name]['faces']==b[name]['faces']}
                for field in ['local','world','basis','matrix_world']:
                    x,y=a[name][field],b[name][field];row[field]={'same_shape':x.shape==y.shape,'exact':np.array_equal(x,y),'maximum_difference':float(np.abs(x-y).max())if x.shape==y.shape and x.size else None}
                rows.append(row)
            rounds.append({'view_layer_updated':update,'objects':rows})
        assert all(guard.sha(Path(p))==h for p,h in pins.items())
        report={'status':'READ_ONLY_DIAGNOSTIC_NOT_ACCEPTANCE','pins':pins,'rounds':rounds,'limits':['No save or geometry/material edits. Exact geometry failure remains HOLD until independently resolved.']}
        out.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
    finally:
        guard.release()


if __name__=='__main__':
    main()
