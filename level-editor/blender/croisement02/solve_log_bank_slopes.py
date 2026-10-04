"""Infer rigid endpoint slopes against exact bank geometry without cutting logs."""
import hashlib,json,sys,math
from pathlib import Path
import bpy
import numpy as np
from PIL import Image
from scipy.optimize import minimize
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(Path(__file__).parent))
from catalog import OUT
from tree_geometry import RAY
from log_trap_state_candidate import point

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
    bank=OUT/'terrain-bank-candidate/assets/croisement02-north-woodland-bank';audit=json.loads((bank/'inspection/saved-model-audit.json').read_text());assert sha(bank/'model.blend')==audit['model_sha256'];source=OUT/'state-target-evidence/log-trap';coherent='--coherent' in sys.argv;dense='--dense' in sys.argv;fit=source/('applied-coherent-log-fit-v2.json' if coherent else 'applied-cylinder-fit.json');survey=json.loads(fit.read_text())['survey'];alpha=np.array(Image.open(source/'tick-089.png'))[:,:,3]>0;bpy.ops.wm.read_factory_settings(use_empty=True)
    names=[r['object']for r in audit['objects']if r['source_node']in [f'building-{i:03d}'for i in range(5)]]
    with bpy.data.libraries.load(str(bank/'model.blend'),link=False)as(src,dst):dst.objects=names
    objects=[o for o in dst.objects if o];[bpy.context.scene.collection.objects.link(o)for o in objects];bpy.context.view_layer.update();vertices=[];faces=[]
    for o in objects:
        offset=len(vertices);vertices.extend(o.matrix_world@v.co for v in o.data.vertices);faces.extend(tuple(offset+j for j in p.vertices)for p in o.data.polygons)
    bvh=BVHTree.FromPolygons(vertices,faces);highest=max(p.z for p in vertices);records=[];result_rows=[]
    def terrain(p):
        hit=bvh.ray_cast(Vector((p.x,p.y,500)),Vector((0,0,-1)),1000);return max(0,float(hit[0].z))if hit[0]is not None else 0
    for index,row in enumerate(survey):
        ax,ay,bx,by,r,z=row;native_constraints=[]
        for t in np.linspace(0,1,13):
            sx=ax+(bx-ax)*t;sy=ay+(by-ay)*t;x=int(round(sx));y=int(round(sy))
            if not(0<=x<alpha.shape[1]and 0<=y<alpha.shape[0]and alpha[y,x]):continue
            hit=bvh.ray_cast(point(sx+390,sy+441,0)+RAY*5000,-RAY)
            if hit[0]is not None:native_constraints.append((float(t),float(hit[0].z)))
        def inspect(values):
            a=point(ax+390,ay+441,values[0]);b=point(bx+390,by+441,values[1]);axis=(b-a).normalized();u=axis.cross(Vector((0,0,1))).normalized();v=axis.cross(u).normalized();clearance=[]
            for t in np.linspace(0,1,41 if dense else 9):
                c=a.lerp(b,float(t))
                for angle in np.arange(64 if dense else 12)*math.tau/(64 if dense else 12):
                    p=c+r*(u*math.cos(angle)+v*math.sin(angle));clearance.append(float(p.z)-terrain(p))
            if dense:
                for c in [a,b]:
                    for radial in np.linspace(0,1,9):
                        for angle in np.arange(64)*math.tau/64:
                            p=c+float(radial)*r*(u*math.cos(angle)+v*math.sin(angle));clearance.append(float(p.z)-terrain(p))
            violation=max([bank_z-(values[0]+(values[1]-values[0])*t+r*float(RAY.z))for t,bank_z in native_constraints]+[0])
            return np.array(clearance),max(0,violation)
        def objective(values):
            clearance,visibility=inspect(values);penetration=np.maximum(0,-clearance);return float(penetration.max()**2*100+np.mean(penetration**2)*20+visibility**2*100+.02*sum(values)+.04*abs(clearance.min()))
        if index in ((2,3,4) if dense else ((0,1,2,3,4,5,6) if coherent else (0,1,2,3,4,5,14))):
            attempts=[]
            for seed in [(r+1,r+1),(highest+r+1,highest+r+1),(highest+r+1,r+1),(r+1,highest+r+1)]:
                result=minimize(objective,seed,method='Powell',bounds=[(r,highest+r+5)]*2,options=dict(maxiter=12,maxfev=700,xtol=.02,ftol=1e-5));attempts.append(result)
            best=min(attempts,key=lambda x:x.fun);values=list(map(float,best.x))
        else:values=json.loads((source/'applied-coherent-bank-slopes-v2.json').read_text())['survey'][index][-2:] if dense else [z,z]
        clearance,visibility=inspect(values);result_rows.append([ax,ay,bx,by,r,*values]);records.append(dict(index=index,heights=values,minimum_sampled_clearance=float(clearance.min()),maximum_native_centerline_bank_occlusion=visibility,source_constraints=len(native_constraints),status='candidate only; exact solid contact and source silhouette review required'));print(records[-1],flush=True)
    report=dict(status='Unapproved rigid-body support hypothesis',dense_caps_and_side_sampling=dense,bank_model_sha256=audit['model_sha256'],source_survey_sha256=sha(fit),survey=result_rows,records=records,limitations=['Two heights define each intact rigid cylinder; no independently raised disconnected alpha pieces.','Perimeter samples approximate contact; saved solid geometry and actual source-camera occlusion must validate each hypothesis.','Target source centerline constraints prevent choosing a lower support branch hidden behind the bank where native wood is visible.','No per-log animation identity or completed motion is claimed.']);(source/('applied-coherent-bank-slopes-v3.json' if dense else ('applied-coherent-bank-slopes-v2.json' if coherent else 'applied-bank-slopes.json'))).write_text(json.dumps(report,indent=2)+'\n')
if __name__=='__main__':main()
