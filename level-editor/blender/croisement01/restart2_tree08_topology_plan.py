"""Source-only connected structural scaffold; no physical depth or ownership claims."""
import hashlib, heapq, json, math
from pathlib import Path
from PIL import Image, ImageDraw
import numpy as np
ROOT=Path(__file__).resolve().parents[3]
R=ROOT/'level-editor/work/croisement01-refinement/restart2'
out=R/'tree08-topology-plan-v1';out.mkdir(exist_ok=False)
trace_path=R/'tree08-source-trace-v2/trace.json';trace=json.loads(trace_path.read_text())
semantic=R/'tree08-semantic-source-v1';core=np.asarray(Image.open(semantic/'bark-core-proposal.png'))>0
leaf=np.asarray(Image.open(semantic/'gold-leaf-cluster-proposal.png'))>0
adj={};edge_ids={};radius={}
for i,path in enumerate(trace['polylines']):
 for x,y,r in path:radius[x,y]=r;adj.setdefault((x,y),{})
 for a,b in zip(path,path[1:]):
  u,v=tuple(a[:2]),tuple(b[:2]);dist=math.dist(u,v)
  # Projected width/core support only rank the source paths; never infer depth.
  support=core[u[1]-11,u[0]-331] or core[v[1]-11,v[0]-331]
  cost=dist*(.85 if support else 1)/(1+.03*min(radius[u],radius[v]))
  adj[u][v]=adj[v][u]=cost;edge_ids[frozenset((u,v))]=i
root=min(adj,key=lambda p:math.dist(p,(557,369)))
dist={root:0};prev={};q=[(0,root)]
while q:
 d,u=heapq.heappop(q)
 if d!=dist[u]:continue
 for v,c in adj[u].items():
  nd=d+c
  if nd<dist.get(v,float('inf')):dist[v]=nd;prev[v]=u;heapq.heappush(q,(nd,v))
# Manually read structural endpoints from native source, excluding hanging vines.
targets=[('left upper bough',(338,110)),('left lower bough',(379,204)),('upper-left fork',(485,35)),('central upper fork',(510,92)),('upper crown fork',(588,15)),('upper-right fork',(645,52)),('right upper bough',(768,183)),('right middle bough',(766,236)),('low right bough',(730,314)),('left basal root',(505,406)),('descending root',(578,469))]
selected=set();routes=[]
for name,target in targets:
 end=min(dist,key=lambda p:math.dist(p,target));path=[end]
 while path[-1]!=root:path.append(prev[path[-1]])
 path.reverse();ids=sorted({edge_ids[frozenset((a,b))] for a,b in zip(path,path[1:])});selected.update(ids)
 routes.append(dict(name=name,requested_native=target,snapped_native=end,snap_distance=math.dist(end,target),trace_ids=ids,source_path=path))
classifications=[]
for i,path in enumerate(trace['polylines']):
 pts=[(int(p[0]),int(p[1])) for p in path];n=len(pts)
 classifications.append(dict(trace_id=i,structural_scaffold=i in selected,points=n,core_samples=sum(bool(core[y-11,x-331]) for x,y in pts),leaf_samples=sum(bool(leaf[y-11,x-331]) for x,y in pts),reason='Connected source scaffold to manually inspected major endpoint' if i in selected else 'Deferred side twig, vine, digital junction spur or isolated tip; preserved, not excised'))
anchors=[dict(name='A upper flare',native=[557,369],constraint='Visible bark widens into mossy basal junction; preserve source boundary.'),dict(name='B left surface root',native=[505,406],constraint='Lateral source root follows soil surface; infer buried underside, not a separate leg.'),dict(name='C descending root shoulder',native=[567,406],constraint='Continuous right basal wood; soil slope must support its exposed upper boundary.'),dict(name='D descending root',native=[581,439],constraint='Retain long sloping wood; native92 overlap is ambiguous bark/edge shadow, not automatic subtraction.'),dict(name='E terminal root',native=[578,469],constraint='Retain source tip and infer embedding into continuous downhill terrain; depth remains unresolved.')]
notes=[dict(region=[701,206,752,296],label='X1',meaning='Native8/14 crossing: connected projection does not prove wood junction. Keep overlapping branch/leaf surfaces separate until depth/owner proof.'),dict(region=[490,190,550,245],label='X2',meaning='Hanging narrow strands beneath central branch are deferred vines/twigs; do not thicken into load-bearing trunks.'),dict(region=[470,125,610,245],label='X3',meaning='Major fork cores remain continuous through gold-leaf clusters. Infer hidden wood continuation under foreground leaves; do not paint leaf pixels onto wood automatically.'),dict(region=[649,43,777,300],label='X4',meaning='Crown93/94 and wood8/14 ownership stays unresolved locally, independently of bark/leaf material class.')]
record=dict(status='Private source-topology construction plan; not geometry/user approval',native_camera=True,source_trace_sha256=hashlib.sha256(trace_path.read_bytes()).hexdigest(),semantic_sha256=hashlib.sha256((semantic/'classification.json').read_bytes()).hexdigest(),root_native=root,total_trace_paths=len(trace['polylines']),retained_scaffold_trace_paths=len(selected),deferred_trace_paths=len(trace['polylines'])-len(selected),root_component_pixels=len(dist),all_paths_preserved=True,structural_routes=routes,path_classification=classifications,root_terrain_anchors=anchors,crossings_and_hidden_continuations=notes,limitations=['2D graph junctions are projected hypotheses, not proof of 3D branch attachment.','Source radius is projected width only. No ground elevation is derived from character thresholds.','Gold leaves can occlude a connected branch: inferred supports preserve uncertain edge obligations.','No geometry, source alpha, terrain, canonical asset or gameplay data changed.'])
(out/'plan.json').write_text(json.dumps(record,indent=2)+'\n')
source=Image.open(ROOT/'level-editor/work/croisement01-refinement/baseline/covered.png').convert('RGB');box=(325,0,795,490);orig=source.crop(box);mark=orig.copy();draw=ImageDraw.Draw(mark)
xy=lambda p:(p[0]-box[0],p[1]-box[1])
for i,path in enumerate(trace['polylines']):
 if len(path)>1:draw.line([xy(p) for p in path],fill=(40,230,240) if i in selected else (150,100,150),width=2 if i in selected else 1)
for a in anchors:
 x,y=xy(a['native']);draw.ellipse((x-3,y-3,x+3,y+3),fill='yellow');draw.text((x+5,y-9),a['name'].split()[0],fill='yellow')
for note in notes[:3]:
 x,y,x2,y2=note['region'];draw.rectangle((x-box[0],y,x2-box[0],y2),outline=(255,150,50));draw.text((x-box[0],y),note['label'],fill='white')
sheet=Image.new('RGB',(940,545),'#222222');sheet.paste(orig,(0,55));sheet.paste(mark,(470,55));d=ImageDraw.Draw(sheet);d.text((5,5),'Native game camera: untouched source | connected construction plan',fill='white');d.text((5,23),'Cyan: structural scaffold / purple: retained deferred traces / yellow: root-soil anchors',fill='white');d.text((5,39),'Projected crossings are hypotheses. No physical depth or final ownership approval.',fill='white');sheet.save(out/'skeleton-overlay.png')
root_sheet=Image.new('RGB',(780,448),'#222222');crop=(485,340,615,478)
for k,im in enumerate([source,source.copy()]):
 part=im.crop(crop).resize((390,414),Image.Resampling.NEAREST)
 if k:
  d=ImageDraw.Draw(part)
  for a in anchors:
   x,y=a['native'];x=(x-crop[0])*3;y=(y-crop[1])*3;d.ellipse((x-5,y-5,x+5,y+5),outline='yellow',width=2);d.text((x+7,y-9),a['name'].split()[0],fill='yellow')
 root_sheet.paste(part,(390*k,34))
ImageDraw.Draw(root_sheet).text((5,8),'Native root/soil source | A-E interface anchors (depth unresolved)',fill='white');root_sheet.save(out/'root-interface-anchors.png')
review=dict(scope='Material-classification direction only; no geometry, user or final ownership approval',reviewer='root',decision='PASS direction',evidence={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in semantic.glob('*.png') if p.name in ['full.png','central-forks.png','left-bough.png','upper-forks.png','right-crossing.png']},notes='Continuous gray/brown bough cores retained; gold clusters plausible; pale twig and antialias edges uncertain. Ownership8/14 and93/94 separate.')
(out/'root-material-review.json').write_text(json.dumps(review,indent=2)+'\n')
print(json.dumps({k:record[k] for k in ['total_trace_paths','retained_scaffold_trace_paths','deferred_trace_paths','root_component_pixels']}));print([(r['name'],round(r['snap_distance'],2)) for r in routes])
