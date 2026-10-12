"""Check proposed ascending branch against the complete retained stone domain."""
import json
import hashlib
import math
from pathlib import Path
import numpy as np
B=Path(__file__).resolve().parents[3]/'level-editor/work/active/croisement03/restart2'


def main():
    folder=B/'tree18-five-track-cpu-v2';gp=folder/'geometry.json';d=json.loads(gp.read_text());part=next(p for p in d['parts']if p['track']=='left_ascending');bp=B/'bank-coupled-left-full-source-v3/projection.json';bank=json.loads(bp.read_text());known={tuple(q)for q in bank['accepted_pixels']}|{tuple(q[:2])for q in bank['protected_firsthits']};assert len(known)==29000
    v=np.array(part['vertices']);S,C=math.sin(math.radians(35)),math.cos(math.radians(35));xy=np.stack((v[:,0],-v[:,1]*S-v[:,2]*C),axis=1);lo,hi=xy.min(0),xy.max(0);candidates=[q for q in known if all(np.array(q)+.5>=lo)and all(np.array(q)+.5<=hi)];conflicts=[]
    for x,y in candidates:
        q=np.array([x+.5,y+.5]);hit=False
        for f in part['faces']:
            t=xy[f];a,b=t[1]-t[0],t[2]-t[0];det=a[0]*b[1]-a[1]*b[0]
            if abs(det)<1e-10:continue
            delta=q-t[0];u=(delta[0]*b[1]-delta[1]*b[0])/det;w=(a[0]*delta[1]-a[1]*delta[0])/det
            if min(u,w,1-u-w)>=-1e-8:hit=True;break
        if hit:conflicts.append([x,y])
    receipt={'status':'PASS_NO_KNOWN_STONE_OVERLAP'if not conflicts else'HOLD_NEW_BRANCH_VS_KNOWN_STONE','pixels':sorted(conflicts),'count':len(conflicts),'pins':{str(p):hashlib.sha256(p.read_bytes()).hexdigest()for p in [gp,bp,Path(__file__)]},'limits':['Independent projected pixel-center test. New source hypothesis cannot overwrite knownstone solely because addedgeometry hits first.','No overlap does not itself prove anatomy; paired source and actual saved views remain mandatory.']};output=folder/'known-stone-source-check.json';assert not output.exists();output.write_text(json.dumps(receipt,indent=2)+'\n');print(receipt['status'],len(conflicts))


if __name__=='__main__':main()
