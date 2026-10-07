"""Exact sampled depth freedom and a bounded minimum-deviation motion corridor."""
import json
import numpy as np
import restart14_butterfly07_anatomy_contacts as audit
B=audit.B;OUT=B/'butterfly07-depth-corridor-v1'

def main(reuse=False, free_boundaries=False):
    offsets=np.arange(-6,6.01,.5);trials={f'offset{float(v)}':np.full(99,v).tolist()for v in offsets};audit.OUT=OUT
    if not reuse:audit.main(B/'butterfly07-body-axis-selected-v1/fit.json',depth_trials=trials)
    report=json.loads((OUT/'report.json').read_text());fit=json.loads((B/'butterfly07-body-axis-selected-v1/fit.json').read_text());rows={r['phase']:r for r in fit['rows']};windows=[]
    for start,end in [(18,22),(91,93)]:
        times=np.arange(start,end+.01,.25);base=np.interp(times,list(rows),[r['fixed_path_anchor_zup'][2]for r in rows.values()]);allowed=[]
        for t in times:
            phase=int(t);frac=round(t-phase,2);suffix=f'pose:{phase}'if frac==0 else f'edge:{phase}:{frac}'
            allowed.append([i for i,v in enumerate(offsets)if not report['results'][f'depth-trial:offset{float(v)}:{suffix}']['contact_counts']])
        zero=int(np.argmin(abs(offsets)))
        if not free_boundaries:allowed[0]=[zero]if zero in allowed[0]else[];allowed[-1]=[zero]if zero in allowed[-1]else[]
        # State retains two heights to bound speed and second difference.
        states={(i,i):(float(offsets[i]**2),[i])for i in allowed[0]}
        for k in range(1,len(times)):
            nxt={}
            for (prevprev,prev),(cost,path)in states.items():
                for current in allowed[k]:
                    now=base[k]+offsets[current];before=base[k-1]+offsets[prev]
                    if abs(now-before)>.75+1e-9:continue
                    curvature=0. if k==1 else now-2*before+base[k-2]+offsets[prevprev]
                    if abs(curvature)>.55+1e-9:continue
                    score=cost+offsets[current]**2+2*curvature**2;key=(prev,current)
                    if key not in nxt or score<nxt[key][0]:nxt[key]=(float(score),path+[current])
            states=nxt
        selected=None
        if states:
            score,indices=min(states.values(),key=lambda x:x[0]);selected={'score':score,'delta':[float(offsets[i])for i in indices],'height':(base+offsets[indices]).tolist(),'maximum_abs_delta':float(abs(offsets[indices]).max())}
        windows.append({'phases':[start,end],'times':times.tolist(),'admissible_delta_samples':[[float(offsets[i])for i in ids]for ids in allowed],'selected':selected})
    result={'status':'SAMPLED_DEPTH_CORRIDOR_NOT_SWEPT_CLEARANCE','contacts_sha256':audit.reader.sha(OUT/'report.json'),'windows':windows,'constraints':{'native_screen_anchors_and_heights_metadata':'Unchanged; inferred body depth only.','end_corrections':'Free; full99smooth continuation required' if free_boundaries else 0,'maximum_height_change_per_quarter_phase':.75,'maximum_second_difference_per_quarter_phase':.55,'objective':'sum(deltaZ²)+2sum(secondDifference(height)²)','sample_spacing_Z':.5,'trust_region_Z':[-6,6]},'limits':['Only pinnedTree01/02 checked in this probe; fullscene/noncanopy and99phase footprints must follow.','Actual8source-supported poses only;91missing anatomical poses and sourcefit omissions remain.','Selected sampled corridor must be fitted continuously and swept-audited before acceptance.']};(OUT/('corridor-free-boundaries.json' if free_boundaries else 'corridor.json')).write_text(json.dumps(result,indent=2)+'\n');print([(w['phases'],w['selected'])for w in windows],flush=True)

if __name__=='__main__':main()
