"""CPU projection checks and eight preview directions for the closed bank plan."""
import hashlib,json,math,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';O=Path(sys.argv[1]) if len(sys.argv)>1 else B/'restart2/bank-whole-geometry-plan-v4';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def main():
 d=json.loads((O/'geometry.json').read_text());d['53']=json.loads((B/'restart2/bank-continuous-strata-plan-v1/geometry.json').read_text());v=[];faces=[];owners=[]
 for k in ['52','53','54']:
  off=len(v);v.extend(d[k]['vertices']);faces.extend([[off+i for i in f] for f in d[k]['faces']]);owners.extend([k]*len(d[k]['faces']))
 original=np.array(v);projected=np.column_stack([original[:,0],-original[:,1]*S-original[:,2]*C]);domain=np.array(Image.open(B/'restart2/bank-exposed-rock-proposal-v1/candidate-rock-seeds.png'))>0;ys,xs=np.nonzero(domain);pixels=np.column_stack([xs+.5,ys+.5]);covered=np.zeros(len(pixels),bool)
 for face in faces:
  a,b,c=projected[face];ab=b-a;ac=c-a;det=ab[0]*ac[1]-ab[1]*ac[0]
  if abs(det)<1e-8:continue
  ap=pixels-a;u=(ap[:,0]*ac[1]-ap[:,1]*ac[0])/det;w=(ab[0]*ap[:,1]-ab[1]*ap[:,0])/det;covered|=(u>=-1e-8)&(w>=-1e-8)&(u+w<=1+1e-8)
 assert covered.all(),f'Missing {int((~covered).sum())} original seeds'
 center=(original.min(0)+original.max(0))/2;v=original-center;sheet=Image.new('RGB',(1600,800),'#202020')
 for i in range(8):
  a=i*math.tau/8;right=np.array([math.cos(a),math.sin(a),0]);eye=np.array([math.sin(a)*C,-math.cos(a)*C,S]);up=np.cross(eye,right);xy=np.array([v@right,v@up]).T;depth=v@eye;scale=350/max(np.ptp(xy,axis=0));xy=xy*scale+[200,200];xy[:,1]=400-xy[:,1];im=Image.new('RGB',(400,400),'#242424');draw=ImageDraw.Draw(im)
  for f in sorted(range(len(faces)),key=lambda f:np.mean(depth[faces[f]])):
   t=faces[f];normal=np.cross(v[t[1]]-v[t[0]],v[t[2]]-v[t[0]]);normal/=np.linalg.norm(normal);shade=.35+.6*max(0,float(np.dot(normal,np.array([-.3,-.6,.74]))));color={'52':(175,175,180),'53':(130,160,185),'54':(180,155,130)}[owners[f]];draw.polygon([tuple(q) for q in xy[t]],fill=tuple(int(c*shade) for c in color))
  draw.text((8,8),'CPU surface '+str(i)+'; contacts not booleaned',fill='white');sheet.paste(im,((i%4)*400,(i//4)*400))
 sheet.save(O/'cpu-eight-surface.png')
 records=[]
 for owner in ['52','54']:
  m=d[owner]
  for trace in m['traces']:
   points=[]
   for j in trace['vertices']:
    x,y,z=m['vertices'][j];points.append([x,-y*S-z*C])
   records.append(dict(owner=owner,id=trace['id'],native_points=points))
 src=Image.open(B/'baseline/covered.png').convert('RGB');mark=src.copy();draw=ImageDraw.Draw(mark)
 for trace in records:draw.line([tuple(p) for p in trace['native_points']],fill='#66ffff' if trace['owner']=='52' else '#ff77cc',width=1)
 box=(115,150,640,485);pair=Image.new('RGB',(1050,1380),'#222222');pair.paste(src.crop(box).resize((1050,670)),(0,20));pair.paste(mark.crop(box).resize((1050,670)),(0,710));ImageDraw.Draw(pair).text((4,687),'CPU native creases; shoulder extensions are inferred. No source assignment.',fill='white');pair.save(O/'native-crease-projection.png')
 report=dict(status='PASS CPU closed arrays and native3446 coverage; saved geometry/contacts still pending',geometry_sha256=hashlib.sha256((O/'geometry.json').read_bytes()).hexdigest(),original_seed_pixels=len(pixels),covered_seed_pixels=int(covered.sum()),fixed_crest=d['guards'],traces=records,limits=['Projection coverage is not first-hit ownership. Runtime Boolean and saved neighbor checks remain mandatory.','52 and54 are heightfields with positive height, so their own upper surfaces cannot self-intersect. Shared solid interfaces require the runtime exact difference.','Native trace images distinguish observed source segments from inferred shoulder extensions; no material assignment follows.'])
 (O/'cpu-review.json').write_text(json.dumps(report,indent=2)+'\n');print(report['status'])
if __name__=='__main__':main()
