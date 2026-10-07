"""CPU-only diagnosis of explicit source sweep frames; writes no model/render."""
import hashlib,json,math,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree08_hierarchy import build_hierarchy_sections
R=ROOT/'level-editor/work/croisement01-refinement/restart2';out=R/'tree08-v10-cpu-frame-audit';out.mkdir(exist_ok=False)
tr=json.loads((R/'tree08-source-trace-v2/trace.json').read_text())['polylines'];old=json.loads((R/'tree08-wood-prototype-v8/construction.json').read_text());selected=[p['trace_id'] for p in old['sections'] if isinstance(p['trace_id'],int)];root=json.loads((R/'tree08-topology-plan-v1/plan.json').read_text())['root_native'];miss=json.loads((R/'tree08-wood-prototype-v8/coverage.json').read_text())['miss_native_pixels'];cores=[[int(x)+331,int(y)+11] for y,x in np.argwhere(np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0)]
sections,obligations,hierarchy=build_hierarchy_sections(tr,selected,root,miss,cores,continuous_nodes=True,continuous_trunk=True)
records=[];profiles={}
for section in sections:
 n=24 if isinstance(section['trace_id'],str) else 16;v=np.asarray(section['vertices']).reshape(-1,n,3);centers=v.mean(axis=1);radial=v-centers[:,None,:];radius=np.linalg.norm(radial[:,0,:],axis=1);normals=radial[:,0,:]/radius[:,None];angles=np.degrees(np.arccos(np.clip((normals[:-1]*normals[1:]).sum(1),-1,1)));ds=np.linalg.norm(np.diff(centers,axis=0),axis=1);dr=np.abs(np.diff(radius))/np.maximum(ds,1e-9);inward=[];degenerate=[];mindot=1.0
 for j in range(len(v)-1):
  for k in range(n):
   nxt=(k+1)%n;a,b,c=v[j,k],v[j,nxt],v[j+1,nxt];normal=np.cross(b-a,c-a);length=np.linalg.norm(normal);expected=(radial[j,k]+radial[j,nxt]+radial[j+1,nxt]+radial[j+1,k])/4;denom=length*np.linalg.norm(expected)
   if denom<1e-10:degenerate.append([j,k]);continue
   dot=float(normal@expected/denom);mindot=min(mindot,dot)
   if dot<=0:inward.append([j,k,dot])
 record=dict(trace_id=section['trace_id'],rings=len(v),held_crossing=section['held_crossing'],max_adjacent_frame_angle=float(angles.max(initial=0)),max_radius_slope=float(dr.max(initial=0)),inward_side_triangles=len(inward),degenerate_side_triangles=len(degenerate),min_outward_dot=mindot,inward_examples=inward[:20],radius_range=[float(radius.min()),float(radius.max())]);records.append(record);profiles[str(section['trace_id'])]=dict(radius=radius.tolist(),frame_angle=angles.tolist(),center_step=ds.tolist(),radius_slope=dr.tolist())
report=dict(status='CPU reconstruction diagnosis only; no Blender/model/render',source_model_sha256=hashlib.sha256((R/'tree08-wood-prototype-v10/model.blend').read_bytes()).hexdigest(),recipe_sha256=hashlib.sha256(Path(__file__).with_name('restart2_tree08_hierarchy.py').read_bytes()).hexdigest(),total_inward_side_triangles=sum(x['inward_side_triangles'] for x in records),total_degenerate=sum(x['degenerate_side_triangles'] for x in records),sections=records,limitations=['Computed first triangle of each swept quad against radial outward direction; detects folded strips, not a full self-intersection test.','Reconstructs exact v10 recipe inputs; no changes to saved model or source.','Separate basal volume overlap is intentional duplicate construction and must be removed, not globally remeshed.'])
total=0;minority=0
for record in records:
 n=24 if isinstance(record['trace_id'],str) else 16;count=(record['rings']-1)*n;total+=count;record['tested_side_triangles']=count;record['minority_orientation_count']=min(record['inward_side_triangles'],count-record['inward_side_triangles']);minority+=record['minority_orientation_count'];record['majority_winding']='inward' if record['inward_side_triangles']>count/2 else 'outward'
report.update(total_tested_side_triangles=total,minority_orientation_total=minority,interpretation='Predominant inward winding is a separate construction error from local minority-direction folded strips. Reversing face order alone cannot repair local folds.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');(out/'profiles.json').write_text(json.dumps(profiles,indent=2)+'\n')
sheet=Image.new('RGB',(1000,610),'#222222');d=ImageDraw.Draw(sheet);data=profiles['10000']
for row,(key,label) in enumerate([('radius','Continuous trunk radius'),('frame_angle','Adjacent screen-frame turn degrees'),('radius_slope','Absolute radius change / center step')]):
 values=data[key];top=30+row*195;maximum=max(values) or 1;d.text((8,top),f'{label}; max {maximum:.3f}',fill='white');points=[(20+950*i/max(1,len(values)-1),top+160-130*v/maximum) for i,v in enumerate(values)];d.line(points,fill=(60,200,230),width=2)
sheet.save(out/'trunk-profiles.png');print(json.dumps(dict(inward=report['total_inward_side_triangles'],degenerate=report['total_degenerate'],main=next(x for x in records if x['trace_id']==10000)),indent=2))
