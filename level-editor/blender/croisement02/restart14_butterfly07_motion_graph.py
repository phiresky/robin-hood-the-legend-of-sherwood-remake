"""Source-pose alternatives joined only through sampled contact-free motion."""
import copy,json
import numpy as np
from scipy.spatial.transform import Rotation
import restart14_butterfly07_anatomy_contacts as audit
B=audit.B;OUT=B/'butterfly07-body-motion-graph-v1'

def main():
    fitp=B/'butterfly07-body-axis-fit-v1/fit.json';contactp=B/'butterfly07-body-axis-contacts-v1/report.json';fit=json.loads(fitp.read_text());contacts=json.loads(contactp.read_text());base={r['phase']:r for r in fit['rows']};candidates={}
    for r in contacts['rows']:
        if r['contacts']:continue
        p=r['phase'];i=r['alternative'];c=copy.deepcopy(fit['candidate_banks'][str(p)][i]);c.update(phase=p,source=base[p]['source'],fixed_path_anchor_zup=base[p]['fixed_path_anchor_zup'],alternative=i);candidates.setdefault(p,[]).append(c)
    edges={p:[(a['alternative'],b['alternative'],a,b)for a in candidates[p]for b in candidates[p+1]]for p in [18,19,20,21,91,92]}
    audit.OUT=OUT;audit.main(B/'butterfly07-body-axis-selected-v1/fit.json',transition_candidates=edges)
    report=json.loads((OUT/'report.json').read_text());valid={}
    for phase,pairs in edges.items():
        valid[phase]=[]
        for ia,ib,a,b in pairs:
            passes=all(not report['results'][f'selected:edge:{phase}:{ia}-{ib}:{t}']['contact_counts']for t in [.25,.5,.75])
            if passes:valid[phase].append((ia,ib))
    def motion(a,b):
        angle=(Rotation.from_quat(a['quaternion']).inv()*Rotation.from_quat(b['quaternion'])).magnitude();h=np.deg2rad(np.array(a['parameters'][3:5])-b['parameters'][3:5]);return .3*(angle**2+.1*float(h@h))
    selected={};holds=[]
    for phases in [[18,19,20,21,22],[91,92,93]]:
        cost={c['alternative']:c['loss']for c in candidates[phases[0]]};back=[]
        for prev,current in zip(phases,phases[1:]):
            nextcost={};parent={}
            for b in candidates[current]:
                options=[(cost[a['alternative']]+motion(a,b)+b['loss'],a['alternative'])for a in candidates[prev]if a['alternative']in cost and (a['alternative'],b['alternative'])in valid[prev]]
                if options:nextcost[b['alternative']],parent[b['alternative']]=min(options)
            cost=nextcost;back.append(parent)
        if not cost:holds.append({'phases':phases,'reason':'No candidate chain clears all three sampled intermediate poses at every edge.'});continue
        last=min(cost,key=cost.get);ids=[last]
        for parent in reversed(back):ids.append(parent[ids[-1]])
        for phase,i in zip(phases,reversed(ids)):selected[phase]=next(c for c in candidates[phase]if c['alternative']==i)
    result={'status':'PARTIAL_SAMPLED_MOTION_CHAIN'if holds else'SAMPLED_MOTION_CHAIN_NOT_CONTINUOUS_CLEARANCE','fit_sha256':audit.reader.sha(fitp),'endpoint_contacts_sha256':audit.reader.sha(contactp),'edge_report_sha256':audit.reader.sha(OUT/'report.json'),'valid_edges':valid,'holds':holds,'selected':selected,'limits':['Quarter/mid/three-quarter samples do not prove swept clearance.','Only own-source eight phases, not full99.','Fixed native anchors/heights, anatomy dimensions and materials.','Body landmarks uncertain; missing source pixels retained.']}
    (OUT/'selection.json').write_text(json.dumps(result,indent=2)+'\n');print('VALID',valid,'HOLDS',holds,flush=True)

if __name__=='__main__':main()
