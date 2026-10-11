"""Extend unfinished Tree01 stems to their native upper bark/canopy contacts."""
import json,math,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'tree01-cpu-upper-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
# Hand-reviewed native bark boundaries; topmost foliage-obscured section remains
# inferred. These controls do not assign native pixels or authorize a texture.
CONTROLS={3:[(70,81,87),(92,79,84),(105,78,83),(118,77,82),(135,77,82)],4:[(85,95,99),(96,97,101),(108,98,104),(121,100,106),(133,102,108)],5:[(75,151,156),(91,148,154),(105,146,152),(119,144,151),(127,143,150)]}
def main():
 inp=B/'tree01-cpu-roots-v1/geometry.json';data=json.loads(inp.read_text());O.mkdir(exist_ok=False);rows=[];source=Image.open(B.parent/'baseline/covered.png').convert('RGB');draw=ImageDraw.Draw(source)
 for p in data['parts']:
  id=p['source_obstacle_context'];old=np.array(p['vertices']);r=p['vertices_per_ring'];n=p['ring_count'];rings=[old[i*r:(i+1)*r].tolist()for i in range(n)];depth=-float(np.mean(rings[0],axis=0)[1])*S;extra=[]
  for index,(y,left,right)in enumerate(CONTROLS[id]):
   width=right-left;c=np.array([(left+right)/2,-depth/S,(depth-y)/C]);extra.append([(c+np.array([math.cos(j*2*math.pi/r)*width/2,math.sin(j*2*math.pi/r)*width/2,0])).tolist()for j in range(r)]);draw.line((left,y,right,y),fill=(0,255,255)if index else(255,0,255));rows.append({'owner':id,'source_y':y,'left_x':left,'right_x':right,'width_depth':width,'status':'inferred foliage-obscured contact'if index==0 else'proposed native bark boundary','uncertainty_pixels':2.})
  newrings=extra+rings;vertices=np.array(newrings).reshape((-1,3)).tolist();faces=[]
  for row in range(len(newrings)-1):
   for i in range(r):
    a=row*r+i;b=row*r+(i+1)%r;c=b+r;dd=a+r;faces.extend([[a,b,c],[a,c,dd]])
  for row,rev in [(0,True),(len(newrings)-1,False)]:
   ci=len(vertices);vertices.append(np.mean(newrings[row],axis=0).tolist())
   for i in range(r):
    f=[ci,row*r+i,row*r+(i+1)%r];faces.append(f[::-1]if rev else f)
  volume=sum(float(np.dot(vertices[f[0]],np.cross(vertices[f[1]],vertices[f[2]])))/6 for f in faces)
  if volume<0:faces=[f[::-1]for f in faces];volume=-volume
  ec=Counter(tuple(sorted((a,b)))for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(ec.values())=={2}and len(vertices)-len(ec)+len(faces)==2 and volume>0
  offset=len(extra);assert np.array_equal(np.array(vertices[offset*r:(offset+n)*r]),old[:n*r]);p.update(vertices=vertices,faces=faces,volume=volume,ring_count=len(newrings),observed_section_indices=[i+offset for i in p['observed_section_indices']],hidden_extension_sections=[i+offset for i in p['hidden_extension_sections']]+[0],proposed_upper_sections=list(range(1,offset)))
  if 'hidden_connection_ring_pair'in p:p['hidden_connection_ring_pair']=[i+offset for i in p['hidden_connection_ring_pair']]
 data['upper_revision']={'input_sha256':hashlib.sha256(inp.read_bytes()).hexdigest(),'controls':rows,'limits':['Original measured bark rings remain exact; new upper boundaries have two-pixel uncertainty.','Root junction and fourth left stem remain unresolved; no full tree completion claim.','Arbre08 native foliage remains a separate observed owner.','No source pixel ownership assigned.']};(O/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');source.crop((35,45,165,165)).resize((780,720),Image.Resampling.NEAREST).save(O/'source-upper-controls.png');print({'parts':len(data['parts']),'upper_controls':len(rows),'known_old_rings':'exact','status':'PRIVATE_CANDIDATE'})
if __name__=='__main__':main()
