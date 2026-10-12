"""Classify actual static/moving canopy samples within both hidden-archer crops."""
import sys,json,hashlib
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE));import qualify as q
from operational_records import read_record
ROOT=q.ROOT;EXTERNAL=ROOT/'.worktrees/native-editor-delivery/work/active/croisement02'
FULL=EXTERNAL/'canopy05-domain-v1/sampler-packet-v2/browser-v1'
STATIC=EXTERNAL/'native-editor-operational-packets-v3/static/browser-v1'
MOVING=EXTERNAL/'native-editor-operational-packets-v3/moving/browser-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def load(folder):
 manifest=json.loads((folder/'manifest.json').read_text());rows=[]
 for f in manifest['rows'][0]['frames']:
  a=[]
  for kind in ['color','depth']:
   p=folder/f[kind]['path'];assert sha(p)==f[kind]['sha256'];a.append(np.array(Image.open(p)))
  rows.append((f,*a))
 assert len(rows)==14;return manifest,rows

def main():
 partial='--static-only' in sys.argv;dest=q.OUT/('actual-static-contribution-v1' if partial else 'actual-contributor-classification-v1');assert not dest.exists();dest.mkdir()
 full,ff=load(FULL);static,sf=load(STATIC);mf=None
 if not partial:moving,mf=load(MOVING)
 assert all(np.array_equal(sf[0][1],x[1]) and np.array_equal(sf[0][2],x[2]) for x in sf),'Static diagnostic changes between poses'
 qualification=read_record(q.OUT/'qualification.json');native=read_record(q.OUT/'profile02-native-loop-composition-v1/report.json')['loop_frames'];assert len(native)==14
 all_labels=[];union_records=[]
 for i,((f,c,d),(s,sc,sd)) in enumerate(zip(ff,sf)):
  assert f['provenance']==s['provenance'];a=c[:,:,3]>0;sa=sc[:,:,3]>0;sm=a&sa&np.all(c==sc,axis=2)&(d==sd)
  labels=np.zeros(a.shape,np.uint8);labels[a]=4;labels[sm]=1
  if mf:
   m,mc,md=mf[i];assert m['provenance']==f['provenance'];ma=mc[:,:,3]>0;mm=a&ma&np.all(c==mc,axis=2)&(d==md)
   assert np.array_equal(a,sa|ma),'Diagnostic alpha union differs from actual whole sample'
   labels[mm]=2;labels[sm&mm]=3
  Image.fromarray(labels).save(dest/f'frame-{i:02}-labels.png');all_labels.append(labels)
  union_records.append(dict(frame=i,counts={str(k):int((labels==k).sum()) for k in range(5)}))
 profiles=[]
 for n,crop in [(1,[79,83,240,206]),(2,[411,0,641,136])]:
  x0,y0,x1,y1=crop;w,h=x1-x0,y1-y0;row=qualification['rows'][n-1];states=row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames'];records=[]
  sheet=Image.new('RGB',(w*4,(h+28)*4),(25,25,25));draw=ImageDraw.Draw(sheet)
  for j,(phase,frame) in enumerate(zip(['initial','transition-0','transition-1','applied'],states)):
   patch=Image.new('RGBA',(w,h));patch.alpha_composite(q.rgba(frame),(int(frame['top_left'][0])-x0,int(frame['top_left'][1])-y0));pm=np.array(patch)[:,:,3]>0;counts=[]
   for i,labels in enumerate(all_labels):
    nl=Image.new('RGBA',(w,h));nf=native[i];nl.alpha_composite(q.rgba(nf),(nf['top_left'][0]-x0,nf['top_left'][1]-y0));nm=np.array(nl)[:,:,3]>0
    local=Image.new('L',(w,h));local.paste(Image.fromarray(labels),(104-x0,-y0));cl=np.array(local);extra=pm&~nm&(cl>0)
    counts.append(dict(frame=i,extra_native_visible_pixels=int(extra.sum()),extra_by_contributor={str(k):int((extra&(cl==k)).sum()) for k in range(1,5)},patch_body_pixels=int(pm.sum())))
    if i in [0,7]:
     rgb=np.array(patch.convert('RGB'));rgb[~pm]=[45,45,45]
     for k,color in [(1,[225,135,35]),(2,[50,180,80]),(3,[70,140,230]),(4,[255,0,255])]:rgb[extra&(cl==k)]=color
     col=0 if i==0 else 2;sheet.paste(Image.fromarray(rgb),(col*w,j*(h+28)+28));draw.text((col*w+2,j*(h+28)+2),f'{phase} f{i} extra cover',fill='white')
     im=Image.fromarray(ff[i][1]);layer=Image.new('RGBA',(w,h));layer.alpha_composite(im,(104-x0,-y0));sheet.paste(Image.alpha_composite(patch,layer).convert('RGB'),((col+1)*w,j*(h+28)+28));draw.text(((col+1)*w+2,j*(h+28)+2),'actual full contributor',fill='white')
   records.append(dict(phase=phase,frames=counts))
  sheet.save(dest/f'profile{n:02}-contribution.png');profiles.append(dict(profile=n,crop=crop,states=records))
 report=dict(status='STATIC_EXACT_MATCH_PARTIAL' if partial else 'ACTUAL_STATIC_MOVING_CLASSIFIED',static_identical_all_14=True,legend={'0':'transparent','1':'static wood exact RGBA/depth match','2':'moving crown exact RGBA/depth match','3':'both match/tie ambiguous','4':'unresolved (including all nonstatic in partial pass)'},whole_frame_counts=union_records,profiles=profiles,manifests={str(p/'manifest.json'):sha(p/'manifest.json') for p in [FULL,STATIC]+([] if partial else [MOVING])},limits=['No source-alpha trimming used.','Static/animated contributor is not automatically native owner role; animated crown includes source-static foliage.','Original background retained only as diagnostic context; no receiver ownership changed.','All hidden-archer01/02 native poses are zero-elevation background elements before canopy05.'])
 (dest/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(output=str(dest),profiles=[dict(profile=p['profile'],counts=[dict(phase=s['phase'],range_by_class={str(k):[min(f['extra_by_contributor'][str(k)] for f in s['frames']),max(f['extra_by_contributor'][str(k)] for f in s['frames'])] for k in range(1,5)}) for s in p['states']]) for p in profiles])))
if __name__=='__main__':main()
