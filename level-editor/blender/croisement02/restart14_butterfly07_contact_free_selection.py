"""Choose only contact-free endpoint hypotheses; audit motion independently."""
import copy,json,math
import numpy as np
from scipy.spatial.transform import Rotation
import restart14_butterfly07_anatomy_contacts as audit
from restart14_butterfly07_pose_fit import geometry
B=audit.B;OUT=B/'butterfly07-contact-free-selection-v1'

def main():
    assert not OUT.exists()
    source=B/'butterfly07-pose-fit-v1/fit.json';evidence=B/'butterfly07-fit-alternatives-v1/report.json'
    fit=json.loads(source.read_text());report=json.loads(evidence.read_text());banks={}
    for r in report['rows']:
        if not r['contacts']:banks.setdefault(r['phase'],[]).append(r)
    def transition(a,b):
        ra=Rotation.from_euler('xyz',a['parameters'][:3],degrees=True);rb=Rotation.from_euler('xyz',b['parameters'][:3],degrees=True)
        h=np.deg2rad(np.array(a['parameters'][3:5])-b['parameters'][3:5])
        return .12*((ra.inv()*rb).magnitude()**2+.1*float(h@h))
    def unary(r):return fit['candidate_banks'][str(r['phase'])][r['alternative']]['loss']
    selected={}
    for phases in ([18,19,20,21,22],[91,92,93]):
        cost=np.array([unary(c)for c in banks[phases[0]]]);back=[]
        for prev,current in zip(phases,phases[1:]):
            costs=cost[:,None]+np.array([[transition(a,b)for b in banks[current]]for a in banks[prev]])
            parent=costs.argmin(0);cost=costs[parent,np.arange(len(parent))]+[unary(c)for c in banks[current]];back.append(parent)
        ids=[int(cost.argmin())]
        for p in reversed(back):ids.append(int(p[ids[-1]]))
        for phase,idx in zip(phases,reversed(ids)):selected[phase]=banks[phase][idx]
    rows=[]
    for old in fit['rows']:
        chosen=selected[old['phase']];c=copy.deepcopy(fit['candidate_banks'][str(old['phase'])][chosen['alternative']]);p=np.array(c['parameters']);anchor=np.array(old['fixed_path_anchor_zup'])
        def world(v):
            x=v[:,0]+p[5];y=v[:,1]+p[6];d=v[:,2]
            return anchor+np.c_[x,-audit.reader.SIN*y-audit.reader.COS*d,-audit.reader.COS*y+audit.reader.SIN*d]
        body,wings=geometry(p);vertices=np.vstack([world(body),*[world(w)for w in wings]])
        c.update(phase=old['phase'],alternative=chosen['alternative'],source=old['source'],fixed_path_anchor_zup=anchor.tolist(),registration_pixels=p[5:7].tolist(),body_vertices_zup=world(body).tolist(),wing_vertices_zup=[world(w).tolist()for w in wings],height_range=[float(vertices[:,2].min()),float(vertices[:,2].max())])
        rows.append(c)
    fit.update(status='CONTACT_FREE_ENDPOINTS_NOT_SWEPT_CLEARANCE',rows=rows,parent_fit_sha256=audit.reader.sha(source),endpoint_evidence_sha256=audit.reader.sha(evidence))
    OUT.mkdir();(OUT/'fit.json').write_text(json.dumps(fit,indent=2)+'\n')
    audit.OUT=B/'butterfly07-contact-free-selection-motion-v1';audit.main(OUT/'fit.json')

if __name__=='__main__':main()
