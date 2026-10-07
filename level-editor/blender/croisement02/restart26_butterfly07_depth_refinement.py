"""Private source-ray depth samples for the pinned revised butterfly07 anatomy."""
import json
from pathlib import Path
import numpy as np
import restart14_butterfly07_anatomy_contacts as audit
B=audit.B
OUT=B/'butterfly07-v3-depth-samples-v1'
FIT=B/'butterfly07-v3-baseline-contacts-v1/joined-fit.json'
FIT_SHA='1e2fe4e6d01c746eaea3a57f393fb074f2a6bdb3f2a109b3b3a28ea5e877a7d6'
HELPER=Path(__file__).with_name('restart21_butterfly07_geometry_v2.py')
HELPER_SHA='9eaf1308871b35dd2dde30126dc63d9a9bcc002690346abc1fb78494772f280e'
WINDOWS=[(5,10),(15,26),(27,34),(82,94)]
OFFSETS=np.arange(-6,6.01,.5)

def main():
    assert audit.reader.sha(FIT)==FIT_SHA
    assert audit.reader.sha(HELPER)==HELPER_SHA
    assert not OUT.exists()
    OUT.mkdir()
    fit=json.loads(FIT.read_text())
    fit['rows']=[r for r in fit['rows'] if any(a<=r['phase']<=b for a,b in WINDOWS)]
    fit['depth_probe_parent_sha256']=FIT_SHA
    path=OUT/'window-fit.json';path.write_text(json.dumps(fit,indent=2)+'\n')
    trials={f'offset{float(v)}':np.full(99,v).tolist() for v in OFFSETS}
    audit.OUT=OUT/'contacts'
    audit.main(path,depth_trials=trials,sweep_subdivisions=0,geometry_helper=HELPER,
        geometry_helper_sha256=HELPER_SHA,receiver_asset_ids=None,first_witness_only=True)
    select()

def select():
    report=json.loads((OUT/'contacts/report.json').read_text())
    rows={r['phase']:r for r in json.loads(FIT.read_text())['rows']}
    zero=int(np.argmin(abs(OFFSETS)));windows=[]
    for start,end in WINDOWS:
        times=np.arange(start,end+.01,.25)
        baseline=np.interp(times,list(rows),[r['fixed_path_anchor_zup'][2] for r in rows.values()])
        allowed=[]
        for t in times:
            phase=int(t);frac=round(t-phase,2)
            suffix=f'pose:{phase}' if frac==0 else f'edge:{phase}:{frac}'
            allowed.append([i for i,v in enumerate(OFFSETS) if not report['results'][f'depth-trial:offset{float(v)}:{suffix}']['contact_counts']])
        allowed[0]=[zero] if zero in allowed[0] else []
        allowed[-1]=[zero] if zero in allowed[-1] else []
        states={(i,i):(float(OFFSETS[i]**2),[i]) for i in allowed[0]}
        for k in range(1,len(times)):
            nxt={}
            for (older,previous),(cost,path) in states.items():
                for current in allowed[k]:
                    now=baseline[k]+OFFSETS[current];before=baseline[k-1]+OFFSETS[previous]
                    if abs(now-before)>1.+1e-9:continue
                    bend=0. if k==1 else now-2*before+baseline[k-2]+OFFSETS[older]
                    if abs(bend)>.6+1e-9:continue
                    score=cost+OFFSETS[current]**2+2*bend*bend;key=(previous,current)
                    if key not in nxt or score<nxt[key][0]:nxt[key]=(float(score),path+[current])
            states=nxt
        chosen=None
        if states:
            cost,ids=min(states.values(),key=lambda item:item[0])
            chosen=dict(objective=cost,delta=OFFSETS[ids].tolist(),height=(baseline+OFFSETS[ids]).tolist())
        windows.append(dict(phases=[start,end],times=times.tolist(),admissible_depth_offsets=[[float(OFFSETS[i]) for i in ids] for ids in allowed],selected=chosen))
    result=dict(status='SAMPLED_CORRIDOR_ONLY_NOT_CONTINUOUS_CLEARANCE',parent_fit_sha256=FIT_SHA,
        contacts_sha256=audit.reader.sha(OUT/'contacts/report.json'),windows=windows,
        constraints=dict(trust_region_z=[-6,6],sample_spacing_z=.5,quarter_phase_max_height_step=1.,quarter_phase_max_second_difference=.6,window_boundary_depth_delta=0,objective='sum(deltaZ²)+2sum(secondDifference(height)²)'),
        limits=['Fixed global anatomy, local source registrations, native screen anchors and timing unchanged. Only inferred camera-ray depth varied.',
        'Static receiver geometry and material alpha checked; dynamic receiver motion remains separate.',
        'The frozen source fit still misses344 pixels, adds528, and misses70 bright pixels.',
        'Admissible samples do not prove a continuous path; fit smooth periodic motion then audit actual and bounded sweeps across all99 intervals.'])
    (OUT/'corridor.json').write_text(json.dumps(result,indent=2)+'\n')
    print([(w['phases'],w['selected'] is not None) for w in windows],flush=True)

if __name__=='__main__':main()
