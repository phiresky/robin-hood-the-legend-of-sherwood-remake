"""Trace the fourth visible Tree01 stem and propose its hidden lower junction."""
import json,math,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement/restart2';O=B/'tree01-left-branch-source-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
# Ordered from foliage-covered upper continuation to an inferred lower join.
CONTROLS=[(75,44,3,False),(95,45,5,True),(110,47,6,True),(125,50,6,True),(140,54,7,True),(155,59,7,True),(175,65,7,False),(192,73,8,False),(210,79,8,False)]
def main():
 O.mkdir(exist_ok=False);v=[];r=12;depth=303.;rows=[];source=Image.open(B.parent/'baseline/covered.png').convert('RGB');draw=ImageDraw.Draw(source)
 for y,x,width,observed in CONTROLS:
  center=np.array([x,-depth/S,(depth-y)/C]);v.extend([(center+np.array([math.cos(i*2*math.pi/r)*width/2,math.sin(i*2*math.pi/r)*width/2,0])).tolist()for i in range(r)]);rows.append({'native':[x,y],'width':width,'depth':width,'status':'proposed visible bark'if observed else'inferred behind foliage','uncertainty_pixels':2});draw.line((x-width/2,y,x+width/2,y),fill=(0,255,255)if observed else(255,0,255))
 faces=[]
 for row in range(len(CONTROLS)-1):
  for i in range(r):
   a=row*r+i;b=row*r+(i+1)%r;c=b+r;d=a+r;faces.extend([[a,b,c],[a,c,d]])
 for row,rev in [(0,True),(len(CONTROLS)-1,False)]:
  ci=len(v);v.append(np.mean(v[row*r:(row+1)*r],axis=0).tolist())
  for i in range(r):
   f=[ci,row*r+i,row*r+(i+1)%r];faces.append(f[::-1]if rev else f)
 volume=sum(float(np.dot(v[f[0]],np.cross(v[f[1]],v[f[2]])))/6 for f in faces)
 if volume<0:faces=[f[::-1]for f in faces];volume=-volume
 ec=Counter(tuple(sorted((a,b)))for f in faces for a,b in zip(f,f[1:]+f[:1]));assert set(ec.values())=={2}and len(v)-len(ec)+len(faces)==2
 branch={'source_obstacle_context':1003,'vertices':v,'faces':faces,'ring_count':len(CONTROLS),'vertices_per_ring':r,'observed_section_indices':[i for i,c in enumerate(CONTROLS)if c[3]],'proposed_upper_sections':[],'source_contexts':[],'controls':rows,'volume':volume,'inferred_join_owner':3,'source_ownership_assigned':False};(O/'branch.json').write_text(json.dumps(branch,indent=2)+'\n');source.crop((30,55,95,225)).resize((390,1020),Image.Resampling.NEAREST).save(O/'native-branch-controls.png')
 inp=B/'tree01-cpu-upper-v1/geometry.json';data=json.loads(inp.read_text());p=next(p for p in data['parts']if p['source_obstacle_context']==3);offset=len(p['vertices']);p['vertices']+=v;p['faces']+=[[i+offset for i in f]for f in faces];data['left_branch_revision']={'input_sha256':hashlib.sha256(inp.read_bytes()).hexdigest(),'branch_sha256':hashlib.sha256((O/'branch.json').read_bytes()).hexdigest(),'status':'CPU_FIRSTHIT_STUDY_ONLY','limits':['Temporary intersecting components require exact saved Boolean junction before appearance review.','Visible left branch is distinct from the already modeled vertical stem; no pixels transferred from other assets.','No source ownership assigned.']};(O/'geometry.json').write_text(json.dumps(data,indent=2)+'\n');print({'observed_sections':sum(c[3]for c in CONTROLS),'inferred_sections':sum(not c[3]for c in CONTROLS),'closed':True})
if __name__=='__main__':main()
