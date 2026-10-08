"""Compare bounded depth corridors while retaining fixed source registrations."""
from pathlib import Path
import sys,json,numpy as np
sys.path.insert(0,'level-editor/blender/croisement02')
from scipy.interpolate import CubicSpline
from restart26_butterfly07_depth_curve import derivative_bounds

def main():
    b=Path('level-editor/work/croisement02-refinement/restart14-butterflies');r=json.loads((b/'butterfly07-v3-depth-outer-samples-v1/combined/contacts/report.json').read_text());f=json.loads((b/'butterfly07-v3-baseline-contacts-v1/joined-fit.json').read_text());h=np.array([x['fixed_path_anchor_zup'][2]for x in f['rows']]);offs=np.sort(np.r_[np.arange(-6,6.01,.5),[-10,-9,-8,-7,-6.5,6.5,7,8,9,10]]);times=np.arange(15,34.01,.25);base=np.interp(times,np.arange(99),h);allowed=[]
    for t in times:
     ph=int(t);fr=round(t-ph,2);suffix=f'pose:{ph}'if fr==0 else f'edge:{ph}:{fr}';allowed.append([i for i,v in enumerate(offs) if not r['results'][f'depth-trial:offset{float(v)}:{suffix}']['contact_counts']])
    allresults=[]
    for penalty in [2,20,100,500]:
     states={(i,i):(offs[i]**2,[i])for i in allowed[0]}
     for k in range(1,len(times)):
      nxt={}
      for (old,prev),(cost,path)in states.items():
       for cur in allowed[k]:
        d=base[k]+offs[cur]-base[k-1]-offs[prev];bend=0 if k==1 else base[k]+offs[cur]-2*(base[k-1]+offs[prev])+base[k-2]+offs[old]
        if abs(d)>1.5+1e-9 or abs(bend)>1+1e-9:continue
        score=cost+offs[cur]**2+penalty*bend*bend;key=(prev,cur)
        if key not in nxt or score<nxt[key][0]:nxt[key]=(float(score),path+[cur])
      states=nxt
     assert states
     cost,path=min(states.values(),key=lambda z:z[0]);d=offs[path];tall=np.arange(0,99.01,.25);height=np.interp(tall,np.arange(100),np.r_[h,h[0]])
     original=json.loads((b/'butterfly07-v3-depth-samples-v1/corridor.json').read_text())
     for w in original['windows']:
      if w['selected']:height[np.rint(np.array(w['times'])*4).astype(int)]=w['selected']['height']
     height[np.rint(times*4).astype(int)]=base+d
     for edge,delta,end in [(15,d[0],13),(34,d[-1],36)]:
      lo,hi=sorted([edge,end]);m=(tall>=lo)&(tall<=hi);baseline=np.interp(tall[m],np.arange(100),np.r_[h,h[0]]);height[m]=baseline+np.interp(tall[m],[lo,hi],[0,delta]if end<edge else[delta,0])
     curve=CubicSpline(tall,height,bc_type='periodic');bounds=derivative_bounds(curve)
     print(penalty,'bounds',bounds,'delta',min(d),max(d),'ends',d[0],d[-1])
     allresults.append(dict(penalty=penalty,cost=cost,times=times.tolist(),delta=d.tolist(),height=(base+d).tolist(),curve=dict(times=tall.tolist(),heights=height.tolist()),analytic_bounds=bounds))
    dest=b/'butterfly07-v3-depth-corridor-comparison-v1';dest.mkdir(exist_ok=False);(dest/'candidates.json').write_text(json.dumps(allresults,indent=2)+'\n')

if __name__=='__main__':main()
