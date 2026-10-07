"""Review explicit static canopy traces separately from animated fragment samples."""
from pathlib import Path
import hashlib,json
import numpy as np
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'trio-tree-integration-v1/static-leaf-source-proposal-v2'
TRACES={12:[[(935,22),(945,15),(958,21),(973,12),(988,17),(1002,30),(1011,48),(1005,70),(1025,76),(1033,91),(1047,96),(1042,113),(1029,123),(1013,114),(1002,101),(986,107),(974,96),(960,104),(948,92),(937,83),(932,65),(937,47)]],14:[[(1080,12),(1094,2),(1114,4),(1127,15),(1139,19),(1144,32),(1157,39),(1169,50),(1161,67),(1149,77),(1136,70),(1128,89),(1114,91),(1105,103),(1092,94),(1096,78),(1083,63),(1087,44),(1077,28)]]}
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 OUT.mkdir(exist_ok=False);source=B.parent/'baseline/covered.png';native=Image.open(source).convert('RGB');rgb=np.array(native).astype(int);reserved=np.zeros(rgb.shape[:2],bool);guards={str(source):sha(source)};level=json.loads((B.parent/'baseline/Croisement03.rhp.json').read_text());exclusions=[]
 for tree in (10,11,12,13,14):
  p=B/f'tree{tree}-bark-proposal-v{2 if tree==11 else 1}/proposed-bark.png'
  if p.exists():reserved|=np.array(Image.open(p))>0;guards[str(p)]=sha(p);exclusions.append(str(p))
 for i in (35,76,107):
  p=B.parent/f'baseline/masks/{i:06}.png';m=level['masks'][i];x,y=m['box_top_left'];a=np.array(Image.open(p))>0;reserved[y:y+a.shape[0],x:x+a.shape[1]]|=a;guards[str(p)]=sha(p);exclusions.append(f'Reserved context mask{i}')
 p=B/'tree13-canopy-context-v1/provisional75-excluding-known-bark.png';a=np.array(Image.open(p))[:,:,3]>0;reserved[:57,1006:1132]|=a;guards[str(p)]=sha(p);exclusions.append('Approved Tree13 static75 source, independently protected from Tree12/14 attribution')
 gold=(rgb[:,:,0]-rgb[:,:,2]>=25)&(rgb[:,:,1]-rgb[:,:,2]>=15)&(rgb[:,:,0]>=rgb[:,:,1]*.8);rows=[];allnew=np.zeros(reserved.shape,bool);allunion=np.zeros(reserved.shape,bool)
 for tree,polys in TRACES.items():
  trace=Image.new('L',native.size);draw=ImageDraw.Draw(trace)
  for poly in polys:draw.polygon(poly,fill=255)
  traced=np.array(trace)>0;case=B/f'tree{tree}-canopy-fragment-source-v1';scope=json.loads((case/'scope.json').read_text());x,y,u,v=scope['absolute_interval'];phasepaths=sorted(case.glob('???.png'));union=np.zeros(reserved.shape,bool);union[y:v,x:u]=np.any([np.array(Image.open(p))[:,:,3]>0 for p in phasepaths],axis=0);allunion|=union
  candidate=traced&gold&~reserved&~union;assert not np.any(allnew&candidate);allnew|=candidate;Image.fromarray(candidate.astype('uint8')*255).save(OUT/f'tree{tree}-candidate-static-foliage.png');Image.fromarray(traced.astype('uint8')*255).save(OUT/f'tree{tree}-trace-envelope.png');rgba=np.dstack((rgb.astype('uint8'),candidate.astype('uint8')*255));Image.fromarray(rgba).save(OUT/f'tree{tree}-proposed-source-rgba.png');outside=np.zeros_like(candidate);outside[y:v,x:u]=True
  rows.append(dict(tree=tree,traces=polys,candidate_pixels=int(candidate.sum()),candidate_outside_prior_animation_interval=int((candidate&~outside).sum()),dynamic_union_pixels=int(union.sum()),excluded_reserved_pixels=int((traced&gold&reserved).sum()),unassigned_dark_or_green_in_trace=int((traced&~gold).sum()),source_role='Proposed static gold/brown leaf and tiny twig clusters; material/membership review still required'))
 box=(910,0,1210,220);marked=rgb.astype('uint8').copy();marked[allunion]=[255,0,180];marked[allnew]=[0,220,255];marked[reserved]=[250,130,30];overlay=Image.fromarray(marked);draw=ImageDraw.Draw(overlay)
 for tree,polys in TRACES.items():
  for poly in polys:draw.line(poly+[poly[0]],fill='white',width=1)
 native.crop(box).resize((1200,880),Image.Resampling.NEAREST).save(OUT/'native-wide.png');overlay.crop(box).resize((1200,880),Image.Resampling.NEAREST).save(OUT/'trace-and-neighbor-context.png');receipt=dict(status='PRIVATE SOURCE TRACE PROPOSAL; no ownership acceptance, geometry or fill',source_guards=guards,rows=rows,neighbor_exclusions=exclusions,legend={'magenta':'Existing14frame dynamic source union, separate role','cyan':'Proposed static foliage samples','orange':'Protected bark, Tree13static75 and reserved fern/root domains','white':'Authored visual cluster envelopes extending beyond prior animation intervals'},limits=['Color is only a positive filter inside visual cluster traces, not proof of semantic ownership. Dark/green interiors stay unassigned.','Full context and neighboring masks must be reviewed; do not infer source ownership from convenient sprite interval or alpha overlap.','No source pixels transferred, no geometry/material changed, no API. Altered physical crown will require fresh geometry review.','Ground cleanup will be bound only to accepted physical source ownership and separately reviewed input.']);(OUT/'scope.json').write_text(json.dumps(receipt,indent=2)+'\n');print(rows)
if __name__=='__main__':main()
