"""Compact, read-only diagnostics for the private cyclic butterfly fit."""
from pathlib import Path
import json,hashlib,sys,math
import numpy as np
from PIL import Image,ImageDraw
from scipy.spatial.transform import Rotation
from scipy.spatial import ConvexHull
ROOT=Path(__file__).resolve().parents[3];OUT=ROOT/'level-editor/work/croisement02-refinement/restart14-butterflies'/(sys.argv[1] if len(sys.argv)>1 else 'joint99-cpu-v1')
def main():
 report=json.loads((OUT/'joint-fit-proposal.json').read_text());selected=report['selected'];rows=selected['rows'];im=Image.new('RGB',(1200,850),'#202020');d=ImageDraw.Draw(im)
 def chart(top,title,series,limit):
  d.text((20,top),title,fill='white');x0,y0=60,top+32;ww,hh=1080,150;d.rectangle((x0,y0,x0+ww,y0+hh),outline='#999999')
  for value in [0,limit/2,limit]:
   yy=y0+hh-value/limit*hh;d.line((x0,yy,x0+ww,yy),fill='#444444');d.text((15,yy-4),f'{value:.1f}',fill='white')
  for i in range(0,99,10):xx=x0+i/98*ww;d.text((xx,y0+hh+4),str(i),fill='white')
  for k,(name,values,color) in enumerate(series):
   d.line([(x0+i/98*ww,y0+hh-v/limit*hh) for i,v in enumerate(values)],fill=color,width=2);d.text((x0+k*270,y0+hh+22),name,fill=color)
 chart(15,'Native source-center coverage fraction, every phase',[('Previous full-v2',[r['baseline']['covered']/r['source_centers'] for r in rows],'#aaaaaa'),('Joint candidate',[r['covered']/r['source_centers'] for r in rows],'#00dddd')],1.)
 colors=['#ff6666','#ffaa00','#00dddd','#bb88ff'];chart(260,'Quaternion body step to next phase (degrees), including loop',[(f"continuity weight{s['continuity_weight']}",s['per_phase_body_step_degrees'],c) for s,c in zip(report['tradeoffs'],colors)],180.)
 chart(505,'Anatomical landmark penalty (zero where no manual anchor)',[('Previous full-v2',[r['baseline']['anchor_loss'] for r in rows],'#aaaaaa'),('Joint candidate',[r['anchor_loss'] for r in rows],'#00dddd')],max(1.,max(r['baseline']['anchor_loss'] for r in rows)))
 d.text((20,760),'CPU analytical projections only; fixed anatomy/pattern; no saved model or rendered-parity claim.',fill='white');d.text((20,782),'Sparse manual landmarks are hypotheses. Phase30 remains uncertain; inspect each regression before acceptance.',fill='white');im.save(OUT/'coverage-anchor-continuity.png')
 hinges=np.array([r['parameters'][3:5] for r in rows]);steps=np.abs(np.roll(hinges,-1,axis=0)-hinges);
 changed=[{'phase':r['phase'],'delta':r['coverage_delta'],'covered':r['covered'],'total':r['source_centers'],'anchor_change':r['anchor_loss']-r['baseline']['anchor_loss']} for r in rows]
 summary={'proposal_sha256':hashlib.sha256((OUT/'joint-fit-proposal.json').read_bytes()).hexdigest(),'regressions':sorted([r for r in changed if r['delta']<0],key=lambda r:r['delta']),'improvements':sorted([r for r in changed if r['delta']>0],key=lambda r:-r['delta']),'unchanged_count':sum(r['delta']==0 for r in changed),'source_centers':sum(r['source_centers'] for r in rows),'selected_continuity_weight':selected['continuity_weight'],'selected_loop_pose_exact':selected['loop_pose_exact'],'hinge_range_degrees':[hinges.min(axis=0).tolist(),hinges.max(axis=0).tolist()],'max_hinge_step_degrees':float(steps.max()),'hinge_steps_above90':np.where(np.max(steps,axis=1)>90)[0].tolist(),'tradeoffs':[{k:v for k,v in s.items() if k!='per_phase_body_step_degrees'} for s in report['tradeoffs']]}
 assert abs(selected['objective']-(sum(r['unary'] for r in rows)+selected['continuity_weight']*selected['continuity_cost']))<1e-7;assert len(rows)==99 and selected['loop_pose_exact'];assert sum(r['covered']+r['missing'] for r in rows)==summary['source_centers'];(OUT/'diagnostic-summary.json').write_text(json.dumps(summary,indent=2)+'\n');print(json.dumps(summary['tradeoffs'],indent=2))
def focus():
 report=json.loads((OUT/'joint-fit-proposal.json').read_text());fit=json.loads((OUT.parent/'rig-full-v2/fit.json').read_text());wing=np.array(fit['wing_outline']);body=np.array([[.36*math.sin(a)*math.cos(t),4.5*math.cos(a),.55*math.sin(a)*math.sin(t)] for a in np.linspace(0,math.pi,13) for t in np.linspace(0,math.tau,25)[:-1]]);phases=[18,23,27,28,29,30,47];sheet=Image.new('RGB',(960,240*len(phases)),'#252525');draw=ImageDraw.Draw(sheet)
 for index,phase in enumerate(phases):
  row=report['selected']['rows'][phase];source=fit['poses'][phase];image=Image.open(source['source']['source']).convert('RGBA');center=np.array(source['source_center'])
  for column,entry in enumerate([None,row['baseline'],row]):
   x=column*320+45;y=index*240+38;scale=12;enlarged=image.resize((image.width*scale,image.height*scale),Image.Resampling.NEAREST);sheet.paste(enlarged,(x,y),enlarged);draw.text((column*320+8,index*240+8),f"Phase{phase}: {['source','full-v2','joint CPU v2'][column]}",fill='white')
   if entry:
    params=np.array(entry['parameters']);rotation=Rotation.from_euler('xyz',params[:3],degrees=True).as_matrix();shift=center+params[5:];point=lambda v:(x+v[0]*scale,y+v[1]*scale)
    for sign,angle,color in [(-1,params[3],'#00dddd'),(1,params[4],'#ff66ff')]:
     vertices=wing.copy();vertices[:,0]*=sign;vertices=Rotation.from_euler('y',-sign*angle,degrees=True).apply(vertices);vertices[:,0]+=sign*.3;vertices=(vertices@rotation.T)[:,:2]+shift;draw.line([point(v) for v in np.vstack((vertices,vertices[0]))],fill=color,width=2)
    vertices=(body@rotation.T)[:,:2]+shift;vertices=vertices[ConvexHull(vertices).vertices];draw.line([point(v) for v in np.vstack((vertices,vertices[0]))],fill='#ffaa00',width=2);draw.text((column*320+8,index*240+207),f"covered{entry['covered']}/{entry['source_centers']} extra{entry['extra']}",fill='white')
 sheet.save(OUT/'focused-source-before-proposed.png')
if __name__=='__main__':main();focus()
