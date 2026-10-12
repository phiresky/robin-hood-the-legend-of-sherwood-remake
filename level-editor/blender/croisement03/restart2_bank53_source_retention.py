"""Audit all retained stone pixels against revised CPU bank receivers only."""
import argparse,hashlib,json,math
from pathlib import Path
from collections import Counter
import numpy as np
from PIL import Image,ImageDraw
from scipy.ndimage import binary_opening
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/active/croisement03';W=B/'restart2';S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def firstheight(mesh,q):
 v=np.array(mesh['vertices']);screen=np.column_stack((v[:,0],-v[:,1]*S-v[:,2]*C));near=np.full(len(q),-np.inf);triangles=np.full(len(q),-1)
 for i,face in enumerate(mesh['faces']):
  tri=screen[face];ids=np.flatnonzero(np.all(q>=tri.min(0)-1e-8,axis=1)&np.all(q<=tri.max(0)+1e-8,axis=1))
  if not len(ids):continue
  aa=tri[1]-tri[0];bb=tri[2]-tri[0];det=aa[0]*bb[1]-aa[1]*bb[0]
  if abs(det)<1e-10:continue
  k=q[ids]-tri[0];u=(k[:,0]*bb[1]-k[:,1]*bb[0])/det;w=(aa[0]*k[:,1]-aa[1]*k[:,0])/det;inside=(u>=-1e-8)&(w>=-1e-8)&(u+w<=1+1e-8);ids,u,w=ids[inside],u[inside],w[inside];z=v[face[0],2]+u*(v[face[1],2]-v[face[0],2])+w*(v[face[2],2]-v[face[0],2]);win=z>near[ids];near[ids[win]]=z[win];triangles[ids[win]]=i
 return near,triangles

def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);a=p.parse_args();assert not a.output.exists();pins={}
 def load(p):pins[str(p)]=sha(p);return json.loads(p.read_text())
 def im(p):pins[str(p)]=sha(p);return np.array(Image.open(p))>0
 projection=load(W/'bank-main-masses-source-v1/projection.json');faces=load(W/'bank-visible-face-traces-v1/faces.json')['faces'];extra=load(W/'bank-left-exposed-stone-proposal-v1/proposal.json');proposal=im(W/'bank-face-source-proposal-v1/proposed-native-stone.png');protected=im(W/'bank-exposed-rock-proposal-v1/candidate-rock-seeds.png');broad=binary_opening(proposal,structure=np.ones((3,3),bool));expected={};sourcepath=B/'baseline/covered.png';pins[str(sourcepath)]=sha(sourcepath);source=Image.open(sourcepath).convert('RGB');assert sha(sourcepath)==projection['source_sha256']
 for face in faces:
  mask=Image.new('L',source.size);ImageDraw.Draw(mask).polygon([tuple(q)for q in face['native_outline']],fill=255);domain=(np.array(mask)>0)&proposal&broad
  for y,x in np.argwhere(domain):expected[(int(x),int(y))]=(str(face['owner']),face['id'])
 for region in extra['regions']:
  mask=im(W/'bank-left-exposed-stone-proposal-v1'/f"{region['id']}.png");assert int(mask.sum())==region['pixels']
  for y,x in np.argwhere(mask&~protected&~broad):expected[(int(x),int(y))]=(str(region['owner']),region['id'])
  broad|=mask
 rows=[]
 for x,y,name,triangle in projection['protected_firsthits']:rows.append({'source_xy':[x,y],'expected_owner':name.split()[-1],'region':'protected3446','prior_triangle':triangle})
 for q in projection['accepted_pixels']:
  owner,region=expected[tuple(q)];rows.append({'source_xy':q,'expected_owner':owner,'region':region})
 assert len(rows)==3446+7811
 for face in projection['faces']:assert sum(row['region']==face['id']for row in rows)==face['accepted'],face['id']
 q=np.array([row['source_xy']for row in rows],float)+.5;combined=load(W/'bank-coupled-left-cpu-v3/geometry.json');east=load(W/'bank-convex-east-cpu-v1/geometry.json');west=load(W/'bank53-mass-bulk-v2/geometry.json');meshes={'52-left':combined['52'],'52-right':east,'53':west,'54':combined['54']};nearest=np.full(len(q),-np.inf);owner=np.full(len(q),'',dtype=object);triangle=np.full(len(q),-1)
 for name,mesh in meshes.items():
  height,tri=firstheight(mesh,q)
  if name=='52-left':height[q[:,0]>east['x_cut']]=-np.inf
  take=height>nearest;nearest[take]=height[take];owner[take]=name;triangle[take]=tri[take]
 for i,row in enumerate(rows):
  row['new_owner']=owner[i].split('-')[0]if owner[i]else None;row['new_component']=owner[i]or None;row['new_triangle']=int(triangle[i]);row['result']='same_owner'if row['new_owner']==row['expected_owner']else'bank_receiver_gap'if row['new_owner']is None else'different_bank_owner'
 groups={}
 for row in rows:groups.setdefault(row['region'],Counter())[row['result']]+=1
 failures=[row for row in rows if row['result']!='same_owner'];left=[r for r in failures if r['region'].startswith('left-')];colors={'bank_receiver_gap':'#00aaff','different_bank_owner':'#ff44cc'};marked=source.copy();draw=ImageDraw.Draw(marked)
 for row in left:draw.point(tuple(row['source_xy']),fill=colors[row['result']])
 crop=(100,245,225,355);sheet=Image.new('RGB',(1000,470),'#303030');sheet.paste(source.crop(crop).resize((500,440),Image.Resampling.NEAREST),(0,30));sheet.paste(marked.crop(crop).resize((500,440),Image.Resampling.NEAREST),(500,30));d=ImageDraw.Draw(sheet);d.text((4,8),'Unchanged original source',fill='white');d.text((504,8),'Blue: no bank hit; magenta: different bank owner',fill='white')
 a.output.mkdir();sheet.save(a.output/'left-source-failures-4x.png');report={'status':'HOLD_RETAINED_SOURCE_RECEIVER_FAILURES'if failures else'PASS_BANK_ONLY_RETAINED_RECEIVERS','pins':pins,'scope':'Exact revised CPU bank52-left/52-right/53/54 only. Unchanged94–97 saved meshes and native sprites are absent, not approximated.','sampling':'Original projection sample centres x+0.5,y+0.5; nearest depth on native ray via projected barycentric triangle intersection.','samples':len(rows),'protected_samples':3446,'previous_extra_accepted_samples':7811,'groups':{k:dict(v)for k,v in groups.items()},'failures':failures,'left_failure_count':len(left),'limits':['Private expectedowner hypotheses reconstructed from exact original proposal precedence, not assigned by color.','Different bank owner is a regression against prior private acceptance, not independent proof of physical asset ownership.','A bank-only miss cannot prove a full-scene hole because unchanged neighboring context meshes are not available as exact CPU arrays.','No source pixels, acceptance lists, protected3446 requirement, geometry or ownership changed.']};(a.output/'receipt.json').write_text(json.dumps(report,indent=2)+'\n');(a.output/'all-samples.json').write_text(json.dumps(rows,indent=2)+'\n');print(json.dumps({'status':report['status'],'groups':report['groups'],'left_failure_count':len(left)}))
if __name__=='__main__':main()
