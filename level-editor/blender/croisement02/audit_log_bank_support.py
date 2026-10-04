"""Read-only terrain support audit for source-positioned full log hypotheses."""
import hashlib,json,sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from tree_geometry import RAY
from log_trap_state_candidate import point

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    bank=OUT/'terrain-bank-candidate/assets/croisement02-north-woodland-bank';audit=json.loads((bank/'inspection/saved-model-audit.json').read_text());assert sha(bank/'model.blend')==audit['model_sha256'];fit=OUT/'state-target-evidence/log-trap/applied-cylinder-fit.json';survey=json.loads(fit.read_text())['survey'];bpy.ops.wm.read_factory_settings(use_empty=True)
    names=[r['object']for r in audit['objects']if r['source_node']in [f'building-{i:03d}'for i in range(5)]]
    with bpy.data.libraries.load(str(bank/'model.blend'),link=False)as(src,dst):dst.objects=names
    objects=[o for o in dst.objects if o];[bpy.context.scene.collection.objects.link(o)for o in objects];bpy.context.view_layer.update();vertices=[];faces=[]
    for o in objects:
        base=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(base+j for j in p.vertices)for p in o.data.polygons)
    bvh=BVHTree.FromPolygons(vertices,faces);highest=max(p.z for p in vertices)
    def ground(p):
        hit=bvh.ray_cast(Vector((p.x,p.y,500)),Vector((0,0,-1)),1000);return max(0,float(hit[0].z))if hit[0]is not None else 0
    records=[]
    for i,(ax,ay,bx,by,r,z)in enumerate(survey):
        samples=[]
        for t in (0,.25,.5,.75,1):
            sx=390+ax+(bx-ax)*t;sy=441+ay+(by-ay)*t;start=point(sx,sy,0)+RAY*5000;hit=bvh.ray_cast(start,-RAY);source_first_z=float(hit[0].z)if hit[0]is not None else 0;roots=[]
            for seed in (0,highest,source_first_z):
                current=seed
                for _ in range(12):
                    p=point(sx,sy,current+r+1);new=ground(p)
                    if abs(new-current)<.01:roots.append(new);break
                    current=new
            support=max(roots)if roots else None;original=point(sx,sy,z);actual=ground(original);samples.append(dict(t=t,source=[sx,sy],current_center_z=z,current_vertical_terrain_z=actual,current_bottom_clearance=z-r-actual,source_ray_first_bank_z=source_first_z,support_solution=support,inferred_center_z=None if support is None else support+r+1))
        records.append(dict(survey_index=i,samples=samples,requires_raised_support=any(s['support_solution']is not None and s['support_solution']>1 for s in samples)))
    report=dict(status='Read-only support diagnostic, no geometry relocation',bank_model_sha256=audit['model_sha256'],survey_sha256=sha(fit),records=records,limitations=['Five axial samples bound a cylinder support hypothesis; full underside contacts still require solid-geometry review.','Highest consistent vertical support branch is selected after source-ray seeding; no nonexistent terrain is fabricated.','If support changes along a log, a rigid tilted or bridging body is required; independently raising endpoint fragments is not a solution.']);dest=OUT/'log-trap-state-candidate-v7/bank-support.json';dest.write_text(json.dumps(report,indent=2)+'\n');print([(r['survey_index'],[round(s['support_solution'],2)if s['support_solution']is not None else None for s in r['samples']])for r in records])
if __name__=='__main__':main()
