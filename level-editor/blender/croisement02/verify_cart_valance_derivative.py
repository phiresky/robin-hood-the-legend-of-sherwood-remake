"""Verify canopy additions preserve saved cart structure, UVs, and native source."""
import hashlib,json,sys
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def snapshot(path):
    bpy.ops.wm.open_mainfile(filepath=str(path));result={}
    for o in bpy.context.scene.objects:
        if o.type!='MESH':continue
        result[o.name]={'vertices':[list(v.co)for v in o.data.vertices],'faces':[list(f.vertices)for f in o.data.polygons],'uv':[list(v.uv)for v in o.data.uv_layers['Native target projection'].data],'matrix':[list(r)for r in o.matrix_world]}
    return result

def main():
    old=OUT/'north-cart-initial-candidate-v3';new=OUT/'north-cart-initial-candidate-v5'
    hashes=[sha(p/'worker.blend')for p in[old,new]]
    a=snapshot(old/'worker.blend');b=snapshot(new/'worker.blend')
    assert set(a)<=set(b)
    for name,row in a.items():assert row==b[name],name
    for name in ['cart-owned-source.png','cart-source-domain.png']:assert sha(old/name)==sha(new/name),name
    assert hashes==[sha(p/'worker.blend')for p in[old,new]]
    report=dict(status='PASS saved/reopened exact preservation',base_model_sha256=hashes[0],model_sha256=hashes[1],unchanged_objects=list(a),added_objects=sorted(set(b)-set(a)),preserved=['Existing component vertices, faces, UVs and world transforms exact','Native owned RGB image and source domain bytes exact'],limitations=['Native material face assignment is recomputed because new cloth can occlude prior source-camera surfaces.','New geometry remains unapproved; this does not prove source proportions or ownership.'])
    (new/'derivative-preservation.json').write_text(json.dumps(report,indent=2)+'\n');print(report['status'])
if __name__=='__main__':main()
