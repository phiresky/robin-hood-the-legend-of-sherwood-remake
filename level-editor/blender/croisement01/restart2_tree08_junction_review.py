"""CPU diagnosis of retained closed-section junctions and all native misses."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from PIL import Image,ImageDraw
from restart2_tree08_hierarchy import build_hierarchy_sections
R=Path(__file__).resolve().parents[3]/'level-editor/work/croisement01-refinement/restart2';packet=R/'tree08-wood-prototype-v11';out=packet/'cpu-junction-review';out.mkdir(exist_ok=False)
plan=R/'tree08-v11-curvature-cpu-v7';report=json.loads((plan/'report.json').read_text());data=np.load(plan/'mesh.npz');receipt=json.loads((packet/'receipt.json').read_text());old_coverage=json.loads((R/'tree08-wood-prototype-v10/coverage.json').read_text())
traces=json.loads((R/'tree08-source-trace-v2/trace.json').read_text())['polylines'];selected=[p['trace_id'] for p in json.loads((R/'tree08-wood-prototype-v8/construction.json').read_text())['sections'] if isinstance(p['trace_id'],int)];root=json.loads((R/'tree08-topology-plan-v1/plan.json').read_text())['root_native'];oldmiss=json.loads((R/'tree08-wood-prototype-v8/coverage.json').read_text())['miss_native_pixels'];core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;support=[[int(x)+331,int(y)+11] for y,x in np.argwhere(core)]
old,_,_=build_hierarchy_sections(traces,selected,root,oldmiss,support,True,True,False);old={s['trace_id']:s for s in old}
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));sections=[];endpoints=[]
for q in report['mesh_sections']:
 i=q['index'];v=data[f'vertices_{i}'];f=data[f'faces_{i}'];rings=v.reshape(-1,16,3);centers=rings.mean(1);oldrings=np.asarray(old[q['trace_id']]['vertices']).reshape(-1,16,3);oldcenters=oldrings.mean(1)
 section=dict(trace_id=q['trace_id'],vertices=v,triangles=v[f],lo=v.min(0),hi=v.max(0));sections.append(section)
 for end in [0,-1]:
  center=centers[end];previous=oldcenters[end];native=[float(center[0]),float(-center[1]*s-center[2]*c)];radial=rings[end]-center;oldradial=oldrings[end]-previous
  endpoints.append(dict(trace_id=q['trace_id'],end='origin' if end==0 else 'tip',center=center,native=native,rim=rings[end],radius=float(np.linalg.norm(radial[0])),previous_radius=float(np.linalg.norm(oldradial[0])),center_displacement_from_v10=float(np.linalg.norm(center-previous))))

def winding(points,triangles):
 results=[]
 for point in points:
  a,b,c=(triangles[:,j]-point for j in range(3));la,lb,lc=(np.linalg.norm(x,axis=1) for x in [a,b,c]);den=la*lb*lc+np.einsum('ij,ij->i',a,b)*lc+np.einsum('ij,ij->i',b,c)*la+np.einsum('ij,ij->i',c,a)*lb;num=np.einsum('ij,ij->i',a,np.cross(b,c));results.append(abs(float(np.arctan2(num,den).sum()/(2*math.pi))))
 return np.asarray(results)

records=[]
for endpoint in endpoints:
 points=np.vstack((endpoint['center'],endpoint['rim']));overlaps=[]
 for section in sections:
  if endpoint['trace_id']==section['trace_id']:continue
  eligible=np.all((points>=section['lo']-1e-8)&(points<=section['hi']+1e-8),axis=1)
  if not eligible.any():continue
  numbers=np.zeros(len(points));numbers[eligible]=winding(points[eligible],section['triangles']);inside=numbers>.95;boundary=(numbers>=.05)&(numbers<=.95)
  if inside.any() or boundary.any():overlaps.append(dict(other_trace_id=section['trace_id'],center_inside=bool(inside[0]),rim_inside=int(inside[1:].sum()),boundary_samples=int(boundary.sum())))
 record={k:v for k,v in endpoint.items() if k not in ['center','rim']};record['overlaps']=overlaps;record['same_native_endpoint']=[dict(trace_id=e['trace_id'],end=e['end'],radius=e['radius']) for e in endpoints if e['trace_id']!=endpoint['trace_id'] and np.linalg.norm(np.array(e['native'])-endpoint['native'])<1e-5];records.append(record)
# Every missed source pixel remains an explicit routing obligation.
allxy=[];owners=[]
for trace_id,path in enumerate(traces):
 for sample,p in enumerate(path):allxy.append(p[:2]);owners.append((trace_id,sample))
kdtree=cKDTree(allxy);routes=[]
for native in receipt['miss_native_pixels']:
 distance,index=kdtree.query(np.array(native)+.5);trace_id,sample=owners[index];active=trace_id in selected
 routes.append(dict(native=native,nearest_source_trace=trace_id,nearest_trace_sample=sample,distance_to_trace=float(distance),trace_selected=active,route='Retained wood source support gap; repair receiving branch volume and reaudit' if active else 'Unselected native wood trace; restore its owned limb instead of discarding the pixel'))
oldset=set(map(tuple,old_coverage['miss_native_pixels']));newset=set(map(tuple,receipt['miss_native_pixels']))
result=dict(status='HOLD: visible root and crown collars remain',model_sha256=receipt['model_sha256'],cpu_mesh_sha256=report['mesh_sha256'],closed_section_count=len(sections),capped_end_count=len(endpoints),new_extra_junction_meshes=0,removed_section='inferred-basal-continuation',retained_endpoint_max_displacement=max(e['center_displacement_from_v10'] for e in records),caps_with_overlap=sum(bool(e['overlaps']) for e in records),caps_with_shared_source_endpoint=sum(bool(e['same_native_endpoint']) for e in records),endpoints=records,native_misses=routes,source_coverage=dict(v10_misses=len(oldset),v11_misses=len(newset),new_misses=sorted(newset-oldset),recovered_count=len(oldset-newset)),limitations=['Solid-angle samples diagnose overlap at capped section ends; they are not a complete triangle-intersection or welded-topology proof.','Retained source-graph crossings still need anatomical ownership.','No terrain/contact receiver was included in this isolated model review.'])
(out/'report.json').write_text(json.dumps(result,indent=2)+'\n')
# Original native RGB remains unmodified; markers are in a separate diagnostic.
rgba=Image.open(R/'tree08-semantic-source-v1/bark-core-proposal-rgba.png').convert('RGBA');image=Image.new('RGB',rgba.size,'#333333');image.paste(rgba,mask=rgba.getchannel('A'));image=image.resize((892,922),Image.Resampling.NEAREST);draw=ImageDraw.Draw(image)
for index,item in enumerate(routes):
 x,y=item['native'];x=(x-331)*2+1;y=(y-11)*2+1;draw.ellipse((x-3,y-3,x+3,y+3),outline='#ff00dd',width=1)
image.save(out/'all-24-misses.png')
print(json.dumps({k:v for k,v in result.items() if k not in ['endpoints','native_misses']},indent=2));print('Miss routing',[(r['native'],r['nearest_source_trace'],r['trace_selected']) for r in routes])
