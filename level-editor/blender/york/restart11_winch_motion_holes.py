"""Name the actual component covering each independently measured source hole."""
import hashlib,json,math,sys
from pathlib import Path
from collections import Counter
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-motion-physical-v2';OUT=BASE/'hole-owner-audit-v2.json'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');motion=json.loads((BASE/'motion.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;rows=[]
for pose,f in zip(motion['rows'],frames):
 scene.frame_set(pose['tick']);bpy.context.view_layer.update();vs=[];fs=[];owners=[]
 for o in scene.objects:
  if o.type!='MESH' or o.hide_render:continue
  off=len(vs);vs.extend(o.matrix_world@v.co for v in o.data.vertices)
  for face in o.data.polygons:fs.append(tuple(off+i for i in face.vertices));owners.append((o.name,o.get('native_patch')=='patch-004'))
 tree=BVHTree.FromPolygons(vs,fs);alpha=Image.open(source/f['image']).getchannel('A');remaining={(x,y) for y in range(alpha.height) for x in range(alpha.width) if alpha.getpixel((x,y))<128};holes=[]
 while remaining:
  seed=min(remaining);remaining.remove(seed);part=[seed]
  for x,y in part:
   for p in ((x-1,y),(x+1,y),(x,y-1),(x,y+1)):
    if p in remaining:remaining.remove(p);part.append(p)
  if len(part)<3 or any(x in (0,alpha.width-1) or y in (0,alpha.height-1) for x,y in part):continue
  cx=sum(p[0] for p in part)/len(part);cy=sum(p[1] for p in part)/len(part);x,y=min(part,key=lambda p:((p[0]-cx)**2+(p[1]-cy)**2,p));px,py=f['bbox'][0]+x,f['bbox'][1]+y;hit=tree.ray_cast(Vector((px+.5,-(py+.5)/s,0))+back*10000,-back);owner,filled=owners[hit[2]] if hit[2] is not None else ('NO_HIT',False);holes.append({'native_pixel':[px,py],'area':len(part),'owner':owner,'filled_by_mechanism':filled})
 rows.append({'frame':pose['source_frame'],'holes':holes})
report={'scope':'All45 independently enclosed source holes with≥3pixels; deterministic lexicographic tie-breaking. All current scene components participate.','model_sha256':hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest(),'frames':rows};OUT.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'filled_owners':dict(Counter(h['owner'] for r in rows for h in r['holes'] if h['filled_by_mechanism'])),'final':rows[-1]},indent=2))
