"""Private closed wood volumes from native width sections; hidden endpoints inferred."""
import json,math,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'tree01-cpu-wood-v1';P=B/'tree01-width-depth-constraints-v1/constraints.json'
def main():
 assert not O.exists();O.mkdir();data=json.loads(P.read_text());parts=[]
 for part in data['parts']:
  sections=part['sections'];centers=[np.array(s['center_world'],dtype=float) for s in sections];widths=[s['observed_width'] for s in sections]
  assert len(centers)>1 and all(s['inferred_horizontal_depth']>=s['observed_width'] for s in sections)
  # Continue each unobserved end by one local width along the observed axis.
  # Caps lie beyond source-supported sections; they are not observed silhouettes.
  first=centers[0]-centers[1];last=centers[-1]-centers[-2];first/=np.linalg.norm(first);last/=np.linalg.norm(last)
  centers=[centers[0]+first*widths[0]]+centers+[centers[-1]+last*widths[-1]];widths=[widths[0]]+widths+[widths[-1]];vertices=[];rings=12
  for center,width in zip(centers,widths):
   for i in range(rings):
    theta=2*math.pi*i/rings;vertices.append((center+np.array([math.cos(theta)*width/2,math.sin(theta)*width/2,0.])).tolist())
  faces=[]
  for row in range(len(centers)-1):
   for i in range(rings):
    a=row*rings+i;b=row*rings+(i+1)%rings;c=b+rings;d=a+rings;faces += [[a,b,c],[a,c,d]]
  for row,reverse in [(0,True),(len(centers)-1,False)]:
   center=len(vertices);vertices.append(centers[row].tolist())
   for i in range(rings):
    f=[center,row*rings+i,row*rings+(i+1)%rings];faces.append(f[::-1] if reverse else f)
  volume=sum(float(np.dot(np.array(vertices[f[0]]),np.cross(vertices[f[1]],vertices[f[2]])))/6 for f in faces)
  if volume<0:faces=[f[::-1] for f in faces];volume=-volume
  edgecounts=Counter(tuple(sorted((a,b))) for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(edgecounts.values())=={2};assert len(vertices)-len(edgecounts)+len(faces)==2 and volume>0
  parts.append({'source_obstacle_context':part['source_obstacle_context'],'vertices':vertices,'faces':faces,'volume':volume,'euler':2,'ring_count':len(centers),'vertices_per_ring':rings,'observed_section_indices':list(range(1,len(centers)-1)),'hidden_extension_sections':[0,len(centers)-1],'width_depth_ratio':1.0,'source_ownership_assigned':False})
 report={'status':'HOLD_PRIVATE_CPU_GEOMETRY_NOT_VISUALLY_REVIEWED','input':{'path':str(P),'sha256':hashlib.sha256(P.read_bytes()).hexdigest()},'parts':parts,'constraints':['Each horizontal wood section has equal width and depth.','Four source-supported parts remain independent; no invented branch connections.','Hidden end continuations are inferred, not cut at final visible source sample.','Authored depth references are hypotheses pending native bank/tree contact review.','No source textures or ownership assignments added.'],'next':['Check source bark seeds and bank firsthits in saved native camera.','Inspect actual eight-view/contact shape before geometry approval.','Only the two authorized Leicester trees may inform tree asset construction.']};(O/'geometry.json').write_text(json.dumps(report,indent=2)+'\n');print({'parts':len(parts),'vertices':sum(len(p['vertices'])for p in parts),'all_closed':True,'status':report['status']})
if __name__=='__main__':main()
