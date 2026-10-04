"""Derive only the supported late translations of the two exposed upper logs."""
import json
import numpy as np
from catalog import OUT
from native_log_foreground_reference import sha


def main():
    source=OUT/'log-motion-correspondence-v1/manifest.json';survey=OUT/'state-target-evidence/log-trap/residual-fit-v1/supported-survey.json';m=json.loads(source.read_text());axes=np.array(json.loads(survey.read_text())['survey']);dest=OUT/'log-late-settling-proof-v1';dest.mkdir(exist_ok=False);tracks=[]
    for index in [0,1]:
        row=axes[index];d=row[2:4]-row[:2];length=np.linalg.norm(d);offset=np.zeros(2);keys=[dict(tick=87,offset=[0.,0.])];evidence=[]
        for pair in reversed([p for p in m['pairs']if p['first_tick']>=63]):
            accepted=[]
            for match in pair['matches']:
                q=np.array(match['target'])-(row[:2]+offset);along=q@d/length;cross=abs(d[0]*q[1]-d[1]*q[0])/length
                if 4<along<length-4 and cross<row[4]-1:accepted.append(match)
            assert len(accepted)>=3,(index,pair['first_tick'],len(accepted))
            deltas=np.array([np.array(v['target'])-v['source']for v in accepted]);delta=np.median(deltas,axis=0);residual=np.linalg.norm(deltas-delta,axis=1);assert np.mean(residual<=1)>=.8
            offset-=delta;keys.append(dict(tick=pair['first_tick'],offset=offset.tolist()));evidence.append(dict(first_tick=pair['first_tick'],next_tick=pair['next_tick'],matched_points=len(accepted),median_translation=delta.tolist(),maximum_residual=float(residual.max()),matches=accepted))
        tracks.append(dict(object=f'applied log {index:02d}',keys=sorted(keys,key=lambda x:x['tick']),evidence=evidence))
    report=dict(status='bounded source-supported translation hypothesis for two bodies only; full collapse and roll unresolved',correspondence_sha256=sha(source),survey_sha256=sha(survey),tick_rate=25,start_tick=63,end_tick=87,tracks=tracks,limitations=['Repeated bark ambiguity remains; supported late translations do not establish identities beforetick63.','Depth remains the fixed approved-direction endpoint hypothesis; bank contact must be verified for the translated poses.','Rotation and rolling bark appearance are not inferred from translation matches.','Other eight endpoint bodies have no motion claim.'])
    (dest/'tracks.json').write_text(json.dumps(report,indent=2)+'\n');print([(t['object'],t['keys'])for t in tracks])
if __name__=='__main__':main()
