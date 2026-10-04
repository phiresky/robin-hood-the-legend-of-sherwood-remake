"""Lift the covered stack along fixed source rays and bound its inter-log contacts."""
import json,math
import numpy as np
from scipy.optimize import minimize_scalar
from catalog import OUT
from native_log_foreground_reference import sha
SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35))

def point(x,y,z):return np.array((x,-(y+z*COS)/SIN,z))
def endpoints(row):return point(row[0]+505,row[1]+453,row[5]),point(row[2]+505,row[3]+453,row[5])
def distance(a,b,c,d):
    # Minimize one segment parameter; the other has a closed clipped projection.
    u=b-a;v=d-c
    def f(t):
        p=a+t*u;s=np.clip((p-c)@v/(v@v),0,1);return np.linalg.norm(p-(c+s*v))
    result=minimize_scalar(f,bounds=(0,1),method='bounded',options={'xatol':1e-10});options=[(f(0),0),(f(1),1),(result.fun,result.x)];value,t=min(options);s=float(np.clip((a+t*u-c)@v/(v@v),0,1));return float(value),float(t),s

def main():
    base=OUT/'log-trap-state-candidate-v14';manifest=json.loads((base/'manifest.json').read_text());rows=np.array(manifest['surveys']['initial']);dest=OUT/'state-target-evidence/log-trap/covered-bank-stack-v1';dest.mkdir(exist_ok=False);report=[];original=rows.copy();plateau=43.9491081237793
    rows[:,5]+=plateau+rows[0,4]+.1-rows[0,5]
    for i in range(1,len(rows)):
        initial=rows[i,5]
        def gaps(z):
            row=rows[i].copy();row[5]=z;a,b=endpoints(row);return [distance(a,b,*endpoints(rows[j]))[0]-row[4]-rows[j,4]for j in range(i)]
        # Preserve source ordering while separating whole cylindrical support volumes.
        low=initial;high=initial+30
        if min(gaps(low))<.03:
            assert min(gaps(high))>.03
            for _ in range(45):
                middle=(low+high)/2
                if min(gaps(middle))>=.03:high=middle
                else:low=middle
            rows[i,5]=high
        else:
            # A gap is not silently called contact. Retain it for audit/next refinement.
            rows[i,5]=initial
        a,b=endpoints(rows[i]);contacts=[]
        for j in range(i):
            value,t,s=distance(a,b,*endpoints(rows[j]));contacts.append(dict(lower_log=j,circular_surface_clearance=value-rows[i,4]-rows[j,4],upper_segment_parameter=t,lower_segment_parameter=s))
        report.append(dict(log=i,source_ray_lift=float(rows[i,5]-original[i,5]),contacts=contacts))
    result=dict(status='private covered support hypothesis; actual mesh-bank and inter-log audit required',base_model_sha256=sha(base/'worker.blend'),covered_audit_sha256=sha(base/'covered-contact-audit.json'),survey=rows.tolist(),applied_survey=manifest['surveys']['applied'],contacts=report,limitations=['Circular cylinder support proxy is conservative; exact sixteen-sided solids must be checked.','Source endpoints/radii and projection remain unchanged.','No stable balance, per-log temporal identity or geometry approval claim.'])
    (dest/'survey.json').write_text(json.dumps(result,indent=2)+'\n');print([(r['log'],r['source_ray_lift'],min(c['circular_surface_clearance']for c in r['contacts']))for r in report])
if __name__=='__main__':main()
