"""Plot bounded contour and collar hypotheses without opening a model."""
import argparse,json
from pathlib import Path
import numpy as np
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from prepare_wood_field_integration import STUDY,source,CONFIG
from fit_wood_collars_cpu import loop_data,sample

def main():
 p=argparse.ArgumentParser();p.add_argument('--packets',type=Path,nargs='+',required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
 retained=json.loads((STUDY/'retained-collars-v1.json').read_text());fig,axes=plt.subplots(2,2,figsize=(13,10));reports=[]
 for folder in a.packets:
  packet=json.loads((folder/'recipe.json').read_text());arrays=np.load(folder/'field-collars.npz')
  for record in packet['records']:
   tree=record['tree'];row=0 if tree==32 else 1
   old=next(r for r in retained['records'] if r['tree']==tree);cut=next(c for c in old['cuts'] if c['height']==record['upper_cut_z'])
   for c in record['collars']:
    rows=arrays[c['array_prefix']+'_rows'];points=rows[-1]
    ids=cut['ordered_loops'][c['upper_extraction_loop']]
    positions=np.array([cut['vertices'][str(i)]['position'] for i in ids]);normals=np.array([cut['vertices'][str(i)]['geometric_normal'] for i in ids])
    positions,normals,arc=loop_data(positions,normals)
    target=[]
    for pt in points:
     edge=np.roll(positions,-1,axis=0)-positions;t=np.clip(np.sum((pt-positions)*edge,axis=1)/np.sum(edge*edge,axis=1),0,1);i=np.argmin(np.linalg.norm(positions+t[:,None]*edge-pt,axis=1));target.append(normals[i]*(1-t[i])+normals[(i+1)%len(normals)]*t[i])
    target=np.array(target);target/=np.linalg.norm(target,axis=1)[:,None]
    around=np.roll(points,-1,axis=0)-np.roll(points,1,axis=0);around/=np.linalg.norm(around,axis=1)[:,None]
    up=rows[-1]-rows[-2];actual=np.cross(around,up);actual/=np.linalg.norm(actual,axis=1)[:,None]
    angle=np.degrees(np.arccos(np.clip(np.abs(np.sum(actual*target,axis=1)),0,1)))
    incompatible=np.degrees(np.arcsin(np.clip(np.abs(np.sum(around*target,axis=1)),0,1)))
    label=f'{folder.name} / {c["array_prefix"]}'
    axes[row,1].plot(np.arange(len(angle))/len(angle),angle,label=label,linewidth=.7)
    axes[row,0].plot(points[:,0],points[:,1],label=label,linewidth=.8)
    reports.append(dict(packet=str(folder),tree=tree,collar=c['array_prefix'],upper_normal_mismatch_max=float(angle.max()),upper_normal_mismatch_p95=float(np.percentile(angle,95)),retained_average_normal_boundary_incompatibility_p95=float(np.percentile(incompatible,95)),eligible=c['eligible_for_bounded_integration'],quality=c['quality']))
 for row,tree in enumerate([32,38]):
  axes[row,0].set_title(f'Tree {tree}: exact retained cut loops (world XY)');axes[row,0].axis('equal');axes[row,1].set_title(f'Tree {tree}: upper collar normal mismatch (degrees)');axes[row,1].set_xlabel('Normalized correspondence index');axes[row,1].legend(fontsize=5)
 fig.suptitle('CPU collar diagnostic only; no model or appearance approval');fig.tight_layout();a.output.mkdir(parents=True,exist_ok=False);fig.savefig(a.output/'collar-preview.png',dpi=150);(a.output/'report.json').write_text(json.dumps(reports,indent=2)+'\n');print(json.dumps(reports,indent=2))
if __name__=='__main__':main()
