"""Small symmetric ray-depth deviations; native screen anchors unchanged."""
import json,math
import numpy as np
import restart14_butterfly07_anatomy_contacts as audit
B=audit.B;OUT=B/'butterfly07-ray-depth-trials-v1'

def main():
    source=B/'footprint-path-proposal-v2/proposal.json';baseline=next(r for r in json.loads(source.read_text())['rows']if r['sequence']==14);h=np.array(baseline['world_zup_knots'])[:,2];ledger=json.loads((B/'butterfly07-depth-provenance-v1/report.json').read_text());lower=np.array([r['old_inferred_lower_bound']for r in ledger['rows']]);trials={};rejected=[]
    for center in [20,92]:
        distance=np.minimum(abs(np.arange(99)-center),99-abs(np.arange(99)-center));weight=np.where(distance<6,.5*(1+np.cos(np.pi*np.minimum(distance,6)/6)),0.)
        for amplitude in [0,-.25,.25,-.5,.5,-1,1,-2,2,-3,3,-4,4,-6,6]:
            delta=amplitude*weight;candidate=h+delta;label=f'center{center}-delta{amplitude}'
            constraints={'minimum_old_noncanopy_slack':float((candidate-lower).min()),'max_height_step':float(abs(np.roll(candidate,-1)-candidate).max()),'max_second_difference':float(abs(np.roll(candidate,-1)-2*candidate+np.roll(candidate,1)).max())}
            if constraints['minimum_old_noncanopy_slack'] < -1e-6 or constraints['max_height_step']>3+1e-6 or constraints['max_second_difference']>2:
                rejected.append({'label':label,**constraints});continue
            trials[label]=delta.tolist()
    audit.OUT=OUT;audit.main(B/'butterfly07-body-axis-selected-v1/fit.json',depth_trials=trials)
    r=json.loads((OUT/'report.json').read_text());summary=[]
    for label,delta in trials.items():
        center=20 if label.startswith('center20-')else 92;phases=[18,19,20,21,22]if center==20 else[91,92,93];edges=phases[:-1];keys=[f'depth-trial:{label}:pose:{p}'for p in phases]+[f'depth-trial:{label}:edge:{p}:{t}'for p in edges for t in [.25,.5,.75]];hits={k:r['results'][k]['contact_counts']for k in keys if r['results'][k]['contact_counts']};summary.append({'label':label,'maximum_delta':float(max(abs(v)for v in delta)),'hits':hits,'sampled_window_clear':not hits})
    (OUT/'summary.json').write_text(json.dumps({'status':'PRIVATE_DEPTH_TRIALS_NOT_SWEPT_OR_FULL99_CLEARANCE','source_path_sha256':audit.reader.sha(source),'constraint_rejections':rejected,'trials':summary,'limits':['Depth unknowns only; unchanged8pose anatomy/sourcefit errors.','99depth knots checked for old inferred receiver bound,3Zstep and2Zcurvature; source2D exact by ray construction.','Actual mesh contacts tested only available8poses and quarter samples; full99footprint and continuous sweep must follow.','Native serialized elevation100 and display composition unchanged.']},indent=2)+'\n');print(summary,flush=True)

if __name__=='__main__':main()
