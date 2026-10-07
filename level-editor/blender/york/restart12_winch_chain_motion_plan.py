"""Select source-supported link phases while penalizing abrupt physical travel."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';version=sys.argv[1] if len(sys.argv)>1 else 'v5';BASE=WORK/f'winch-closed-loop-fit-{version}';OUT=BASE/'motion-plan.json'
if OUT.exists():raise FileExistsError(OUT)
fit=json.loads((BASE/'fit.json').read_text());motion=json.loads((WORK/'winch-motion-physical-v2/motion.json').read_text());contacts=json.loads((WORK/f'winch-chain-loop-prototype-{version}/phase-contacts.json').read_text());assert all(r['conservative_link_surface_clearance']>0 and r['idler_penetrating_vertices']==0 for r in contacts['rows'])
phases=[i*.5 for i in range(14)];costs=[];data=[]
for row in fit['frames']:
 candidates={r['phase_game']:r for r in row['candidates']};data.append(candidates);costs.append([100000*candidates[p]['hole_centers_filled']+candidates[p]['opaque_missed']+candidates[p]['empty_filled']+3*candidates[p]['core_missed'] for p in phases])
def displacement(a,b,expected):
 delta=b-a;return delta+7*round((expected-delta)/7)
solutions=[]
for sign in (-1,1):
 expected=[0]+[sign*(motion['rows'][i]['screen_center_y']-motion['rows'][i-1]['screen_center_y']) for i in range(1,45)];states={};back=[]
 for a in range(14):
  for b in range(14):
   delta=displacement(phases[a],phases[b],expected[1]);states[a,b]=(costs[0][a]+costs[1][b]+2*(delta-expected[1])**2,delta)
 for i in range(2,45):
  nxt={};parents={}
  for (a,b),(value,previous_delta) in states.items():
   for c in range(14):
    delta=displacement(phases[b],phases[c],expected[i]);cost=value+costs[i][c]+2*(delta-expected[i])**2+(delta-previous_delta)**2
    if (b,c) not in nxt or cost<nxt[b,c][0]:nxt[b,c]=(cost,delta);parents[b,c]=(a,b)
  states=nxt;back.append(parents)
 pair=min(states,key=lambda k:states[k][0]);score=states[pair][0];indices=[pair[1],pair[0]]
 for parents in reversed(back):pair=parents[pair];indices.append(pair[0])
 indices=list(reversed(indices));assert len(indices)==45;unwrapped=phases[indices[0]];rows=[]
 for i,index in enumerate(indices):
  delta=0 if i==0 else displacement(phases[indices[i-1]],phases[index],expected[i]);unwrapped+=delta;rows.append({'source_frame':i,'tick':motion['rows'][i]['tick'],'phase_game':phases[index],'unwrapped_phase_game':unwrapped,'phase_delta_game':delta,'source_fit':data[i][phases[index]]})
 solutions.append({'direction_hypothesis':sign,'weighted_score':score,'rows':rows})
best=min(solutions,key=lambda s:s['weighted_score']);OUT.write_text(json.dumps({'status':'Private motion hypothesis; discrete source timing, whole links follow complete path; not approved','model_sha256':fit['model_sha256'],'fit_sha256':hashlib.sha256((BASE/'fit.json').read_bytes()).hexdigest(),'contacts_sha256':hashlib.sha256((WORK/f'winch-chain-loop-prototype-{version}/phase-contacts.json').read_bytes()).hexdigest(),'chosen':best,'alternative_score':max(s['weighted_score'] for s in solutions),'limitations':['Source-hole penalties dominate; displayed source-space descent supplies a smoothness prior, not a proven gear ratio.','The source does not prove the hidden chain return or the moving part attachment.','Every actual saved pose must be checked before review.']},indent=2)+'\n');print(json.dumps({'version':version,'direction':best['direction_hypothesis'],'score':best['weighted_score'],'holes':sum(r['source_fit']['hole_centers_filled'] for r in best['rows']),'phase_deltas':[r['phase_delta_game'] for r in best['rows']]}))
