"""Reopen both endpoint workers and prove the covered-only source-ray correction."""
import json,sys
from pathlib import Path
import bpy
import numpy as np
sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from tree_geometry import SIN,COS
from log_trap_state_candidate import sha

def snapshot(path):
    bpy.ops.wm.open_mainfile(filepath=str(path));result={}
    for obj in bpy.context.scene.objects:
        if not obj.get('state_endpoint'):continue
        positions=np.array([tuple(obj.matrix_world@v.co)for v in obj.data.vertices]);uv=np.array([tuple(v.uv)for v in obj.data.uv_layers['Native target projection'].data]);result[obj.name]=dict(positions=positions,uv=uv,faces=[tuple(p.vertices)for p in obj.data.polygons],materials=[p.material_index for p in obj.data.polygons],state=obj['state_endpoint'])
    return result

def main():
    before=OUT/'log-trap-state-candidate-v14/worker.blend';after=OUT/'log-trap-state-candidate-v15/worker.blend';first=snapshot(before);second=snapshot(after);assert first.keys()==second.keys();records=[]
    for name,a in first.items():
        b=second[name];assert a['faces']==b['faces'];assert a['materials']==b['materials'];uv=float(np.max(np.abs(a['uv']-b['uv'])));assert uv<1e-6
        delta=b['positions']-a['positions'];projected=np.column_stack((delta[:,0],-delta[:,1]*SIN-delta[:,2]*COS));error=float(np.max(np.abs(projected)));assert error<.001
        if a['state']=='applied':assert not np.any(delta);assert not np.any(a['uv']-b['uv'])
        else:assert float(np.max(np.abs(delta-delta[0])))<.001
        records.append(dict(object=name,state=a['state'],maximum_world_change=float(np.max(np.abs(delta))),maximum_source_projection_change=error,maximum_uv_change=uv,rigid_translation=delta[0].tolist()))
    result=dict(status='PASS saved/reopened covered-only rigid source-ray corrections; applied geometry and UV exactly unchanged',before_sha256=sha(before),after_sha256=sha(after),objects=records);(after.parent/'derivative-preservation.json').write_text(json.dumps(result,indent=2)+'\n');print(result['status'])
if __name__=='__main__':main()
