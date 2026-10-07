"""Read-only source projection overlap for nearby baseline wood proxies."""
import json, math, sys
from pathlib import Path
import bpy
import numpy as np
from PIL import Image, ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import hashlib
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
OUT=ROOT/'level-editor/work/croisement01-refinement'
DEST=OUT/'restart2/tree04-ownership-v1';DEST.mkdir(exist_ok=False)
acquire();source=OUT/'croisement01-grouped.blend';bpy.ops.wm.open_mainfile(filepath=str(source))
S,C=math.sin(math.radians(35)),math.cos(math.radians(35))
manifest=json.loads((OUT/'baseline/masks/manifest.json').read_text());domains={}
for i in [2,3,4]:
 row=next(r for r in manifest['masks'] if r['index']==i);a=Image.new('L',(1408,960));a.paste(Image.open(OUT/'baseline/masks'/row['png']).convert('L'),tuple(row['box_top_left']));domains[str(i)]=np.asarray(a)>0
domains['4-reviewed-wood']=domains['4']
rows=[];base=Image.open(OUT/'baseline/covered.png').convert('RGB')
for obj in bpy.data.collections['Croisement01 Working'].all_objects:
 if obj.type!='MESH' or not str(obj.get('source_node','')).startswith('building-'):continue
 pts=[obj.matrix_world@v.co for v in obj.data.vertices];xy=[(v.x,-v.y*S-v.z*C) for v in pts]
 pixels=np.zeros((960,1408),dtype=bool);obj.data.calc_loop_triangles()
 for face in obj.data.loop_triangles:
  tri=np.array([xy[i] for i in face.vertices]);lo=np.maximum(np.floor(tri.min(axis=0)).astype(int),[0,0]);hi=np.minimum(np.ceil(tri.max(axis=0)).astype(int),[1408,960])
  if np.any(hi<=lo):continue
  yy,xx=np.mgrid[lo[1]:hi[1],lo[0]:hi[0]];p=np.stack((xx+.5,yy+.5),axis=-1);edges=[]
  for j in range(3):
   a,b=tri[j],tri[(j+1)%3];edges.append((p[...,0]-a[0])*(b[1]-a[1])-(p[...,1]-a[1])*(b[0]-a[0]))
  edges=np.stack(edges);inside=np.all(edges>=-1e-8,axis=0)|np.all(edges<=1e-8,axis=0);pixels[lo[1]:hi[1],lo[0]:hi[0]]|=inside
 mask=Image.fromarray((pixels*255).astype('uint8'));counts={key:int(np.count_nonzero(pixels&domain)) for key,domain in domains.items()}
 if counts['4-reviewed-wood']==0:continue
 Image.fromarray(((pixels&domains['4-reviewed-wood'])*255).astype('uint8')).save(DEST/(obj['source_node']+'-tree04-overlap.png'))
 pic=base.copy();overlay=Image.new('RGB',pic.size,(245,65,30));pic=Image.composite(Image.blend(pic,overlay,.5),pic,mask);pic.crop((60,0,330,650)).resize((324,780)).save(DEST/(obj['source_node']+'.png'))
 rows.append(dict(node=obj['source_node'],bounds_source=[[min(p[i] for p in xy),max(p[i] for p in xy)] for i in range(2)],overlap_pixels=counts,polygons=len(obj.data.polygons),vertices=len(pts)))
report=dict(status='Read-only conservative projected polygon coverage; does not assert rendered visibility or gameplay association',source_sha256=sha(source),rows=rows)
(DEST/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
