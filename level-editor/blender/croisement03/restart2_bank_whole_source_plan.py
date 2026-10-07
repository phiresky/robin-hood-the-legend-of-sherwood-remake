"""CPU whole-bank morphology and source reservation plan; no mesh mutation."""
import hashlib,json,math
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];B=R/'level-editor/work/croisement03-refinement';O=B/'restart2/bank-whole-source-plan-v1'
S=math.sin(math.radians(35));C=math.cos(math.radians(35))
# Short visible rock breaks stop at foliage/trunk occlusions, not mask edges.
SOUTH=[
 dict(id='south-upper-short-ledge',points=[[377,357],[387,360],[400,357],[413,353]],lower=[[377,363],[387,366],[400,363],[413,359]]),
 dict(id='south-middle-short-ledge',points=[[369,380],[384,375],[400,371],[413,366]],lower=[[369,387],[384,382],[400,378],[413,373]]),
 dict(id='south-low-exposed-tip',points=[[352,420],[364,426],[376,430]],lower=[[352,426],[364,433],[376,437]]),
]
EAST=[
 dict(id='east-upper-block',points=[[603,206],[610,211],[613,219]],lower=[[603,215],[610,220],[613,228]]),
 dict(id='east-middle-block',points=[[596,248],[606,243],[613,239]],lower=[[596,260],[606,255],[613,251]]),
 dict(id='east-low-block',points=[[594,274],[603,270],[610,267]],lower=[[594,288],[603,284],[610,281]]),
]
PATCHES={
 'west-top-interior':[[160,279],[174,285],[187,283],[195,280],[191,275],[176,270],[165,270]],
 'west-middle-interior':[[133,311],[146,315],[158,314],[174,310],[185,308],[188,313],[174,316],[158,320],[143,321],[133,318]],
 'west-bottom-interior':[[133,365],[144,365],[157,360],[173,356],[185,352],[188,358],[175,364],[160,369],[143,375],[134,372]],
 'east-middle-interior':[[602,247],[609,244],[609,251],[602,256]],
 'east-low-interior':[[599,276],[605,273],[605,282],[598,286]],
 'south-upper-interior':[[385,357],[394,356],[402,353],[405,355],[394,360],[386,361]],
 'south-middle-interior':[[385,376],[396,372],[404,370],[405,373],[396,377],[386,381]],
}
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 O.mkdir(exist_ok=True);assert not (O/'plan.json').exists()
 source=B/'baseline/covered.png';lp=B/'baseline/Croisement03.rhp.json';level=json.loads(lp.read_text());src=Image.open(source).convert('RGB')
 recipe=json.loads((B/'restart2/bank-morphology-trace-v1/recipe.json').read_text());scope=json.loads((B/'restart2/bank-west-transition-v1/surface-scope.json').read_text())
 traces=[]
 for owner,rows,color in [(54,SOUTH,'#ff66dd'),(52,EAST,'#55ffff')]:
  plane=scope['surfaces'][str(owner)]['height_from_native_xy']
  for row in rows:
   # Anchor height is a hypothesis from the authored support, not source-measured.
   x,y=row['points'][len(row['points'])//2];anchor=plane[0]*x+plane[1]*y+plane[2]
   if owner==52:anchor-=max(0,y-210)*.45
   pairs=[]
   for a,b in zip(row['points'],row['lower']):
    za=anchor;zb=za-(b[1]-a[1]);assert zb>0
    pairs.append(dict(source_upper=a,source_lower=b,upper_world=[a[0],-(a[1]+za)/S,za/C],lower_world=[b[0],-(b[1]+zb)/S,zb/C]))
   traces.append(dict(**row,owner=owner,color=color,inferred_anchor_height=anchor,paired_world=pairs,confidence='Visible short crease; endpoint and height interpretation need next saved native review.'))
 masks={}
 for i,r in enumerate(level['masks']):
  im=Image.new('L',src.size);im.paste(Image.open(B/f'baseline/masks/{i:06}.png'),tuple(r['box_top_left']));masks[i]=np.array(im)>0
 bank=masks[96]|masks[108]|masks[109];foreign=np.zeros(bank.shape,bool)
 for i,m in masks.items():
  if i not in (96,108,109):foreign|=m
 alltrace=np.zeros(bank.shape,bool);records=[]
 for name,poly in PATCHES.items():
  im=Image.new('L',src.size);ImageDraw.Draw(im).polygon(poly,fill=255);trace=np.array(im)>0;alltrace|=trace
  owners={str(i):int((trace&m).sum()) for i,m in masks.items() if i not in (96,108,109) and np.any(trace&m)}
  records.append(dict(id=name,polygon=poly,traced_pixels=int(trace.sum()),unreserved_bank_pixels=int((trace&bank&~foreign).sum()),foreign_reservations=owners,outside_bank_domain=int((trace&~bank).sum())))
 candidate=alltrace&bank&~foreign;held=alltrace&~candidate
 Image.fromarray(candidate.astype('uint8')*255).save(O/'candidate-rock-seeds.png');Image.fromarray(held.astype('uint8')*255).save(O/'held-mixed-ownership.png')
 marked=np.array(src);marked[candidate]=(marked[candidate]*.4+np.array([0,255,255])*.6).astype('uint8');marked[held]=(marked[held]*.5+np.array([255,60,120])*.5).astype('uint8');marked=Image.fromarray(marked);d=ImageDraw.Draw(marked)
 for t in traces:
  for k in ['points','lower']:d.line([tuple(p) for p in t[k]],fill=t['color'],width=1)
 for t in recipe['main_bank_observed_breaks']:d.line([tuple(p) for p in t['points']],fill='white',width=1)
 box=(115,150,640,485);sheet=Image.new('RGB',(1050,1400),'#222222');draw=ImageDraw.Draw(sheet)
 for n,(label,im) in enumerate([('Native source: short stone breaks behind separate vegetation',src),('White: previous breaks; cyan: east; pink: south. Filled pink: reserved ownership; filled cyan: proposal only.',marked)]):
  sheet.paste(im.crop(box).resize((1050,670)),(0,n*700+30));draw.text((5,n*700+8),label,fill='white')
 sheet.save(O/'whole-bank-traces-and-reservations.png')
 pins=[source,lp,B/'restart2/bank-continuous-strata-v1/worker.blend',B/'restart2/tree02-ridge-ray-guard-v4/receipt.json']
 plan=dict(status='CPU whole-bank construction scope; no render lane or geometry approval',input_sha256={str(p):sha(p) for p in pins},new_traces=traces,previous_main_breaks=recipe['main_bank_observed_breaks'],source_proposals=records,additional_unreserved_pixels=int(candidate.sum()),held_pixels=int(held.sum()),
 construction=[
 'Retain bank53 continuous-strata-v1 as working baseline. Preserve all34 exact source trace points; vary inferred shoulder setbacks asymmetrically only if needed at52 interface. No circular regularization.',
 'Bank52: keep every fixed crest point x175..340 exact at85. Split exposed west shoulder at the existing two irregular breaks, blending to a recessed connecting face and the retained53 top. Retain continuous enclosed bulk; no overlay slabs or detached caps.',
 'Bank52 east: use three short block breaks, shoulder recesses and angular lateral offsets. Connect intervening hidden stone behind vegetation into a closed body; do not trace vegetation as the rock silhouette.',
 'Bank52 top: triangulate between fixed rear crest, source-supported shoulder and irregular outer front. Keep limited height variations away from frozen crest; do not turn grass RGB into displacement.',
 'Bank54: keep original graded footprint and lower contact. Insert short sloping ledges from the three observed source segments, join through recessed risers into a closed volume. The large intervening gold crown, trunk and ivy are separate appearance domains.',
 'Resolve52/53 and52/54 interface vertices together; preserve authored gameplay metadata rather than rewriting collision or elevation links. Shared contact surfaces must not be doubled into overlapping thin curtains.',
 'Project the original3446 seeds and any newly reviewed unreserved rock samples by saved-model first hit. All mixed-mask proposals stay reserved pending explicit source/frame ownership review.',
 ],next_checks=['CPU construct closed52/54 surface arrays and joint constraints before requesting render lane.','New rock proposals require source zoom and all-state alpha overlap, then actual face ownership; no automatic assignment from mask IDs.','Saved native first, whole actual/solid8, west/east/south contacts and Tree01/Tree02–07 guards.','Keep known RGBA/UV unchanged; no texture synthesis until wholebank geometry gate.'],limits=['Trace endpoints uncertain approximately1–3 native pixels. Inferred heights/shoulder depths are not measured.','No model, canonical files, frozen cards or galleries changed.','Additional source proposal is deliberately incomplete and does not transfer any foreign mask pixel.'])
 (O/'plan.json').write_text(json.dumps(plan,indent=2)+'\n');print({'additional_unreserved':int(candidate.sum()),'held':int(held.sum()),'trace_segments':len(traces)})
if __name__=='__main__':main()
