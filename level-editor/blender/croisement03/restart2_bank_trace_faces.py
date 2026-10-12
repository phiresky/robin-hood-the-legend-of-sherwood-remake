"""Route native shoulder traces to visible closed-bank faces before geometry edits."""
import json,math,hashlib
from pathlib import Path
from collections import Counter
import numpy as np
from PIL import Image,ImageDraw
from restart2_bank_boundary_ray import boundary_hits
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03/restart2';O=B/'bank-shoulder-face-routing-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35));ray=np.array([0.,-C,S])
def main():
 O.mkdir(exist_ok=False);path=B/'bank-whole-geometry-plan-v4/geometry.json';data=json.loads(path.read_text());west=json.loads((B/'bank-continuous-strata-plan-v1/geometry.json').read_text());data['53']=west;meshes={k:(v['vertices'],v['faces'])for k,v in data.items()if k in ['52','53','54']};source=json.loads((B/'bank-shoulder-source-v1/traces.json').read_text());rows=[];counts=Counter();im=Image.open(B.parent/'baseline/covered.png').convert('RGB');draw=ImageDraw.Draw(im)
 for trace in source['traces']:
  points=[list(p)for p in trace['points']]
  if trace['id']=='south-upper-short-shoulder':points[4][0]-=.5
  for segment,(a,b) in enumerate(zip(points,points[1:])):
   a=np.array(a);b=np.array(b);n=max(2,math.ceil(np.linalg.norm(b-a)*2))
   for i in range(n):
    x,y=a+(b-a)*i/(n-1);origin=np.array([x,-y/S,0.])+ray*5000;hits=boundary_hits(origin,-ray,meshes)
    if not hits:kind='miss';row={'trace':trace['id'],'source':[float(x),float(y)],'surface':kind}
    else:
     hit=hits[0];owner=hit['owner'];f=data[owner]['faces'][hit['triangle']];limit=data[owner].get('source_vertex_count');kind=('top'if all(v<limit for v in f)else'bottom'if all(v>=limit for v in f)else'side')if limit else'retained-west';point=origin-ray*hit['distance'];row={'trace':trace['id'],'source':[float(x),float(y)],'intended_context':trace['owner'],'firsthit_owner':owner,'surface':kind,'triangle':hit['triangle'],'world':point.tolist(),'barycentric':hit['barycentric']}
    rows.append(row);counts[(trace['id'],kind)]+=1;draw.point((round(x),round(y)),fill={'top':'cyan','side':'yellow','bottom':'magenta','retained-west':'orange','miss':'red'}[kind])
 report={'status':'CPU_FACE_ROUTING_NOT_MATERIAL_OWNERSHIP','geometry_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'counts':[{'trace':a,'surface':b,'samples':n}for(a,b),n in counts.items()],'rows':rows,'next':'Split constraints on each actual sidewall or top plane; do not force sidewall samples into the top heightfield. Preserve closed bulk and fixed crest.','source_pixels_assigned':False};(O/'routing.json').write_text(json.dumps(report,indent=2)+'\n');im.crop((270,270,450,490)).resize((720,880),Image.Resampling.NEAREST).save(O/'south-face-routing.png');print(report['counts'])
if __name__=='__main__':main()
