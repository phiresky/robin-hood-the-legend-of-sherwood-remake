"""Compare chain wire thickness against source opacity and empty pixels."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';OUT=WORK/'restart2/winch-chain-wire-v1'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageFilter,ImageOps
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s))
def tree(objects):
 vs=[];fs=[]
 for o in objects:
  off=len(vs);vs.extend(o.matrix_world@v.co for v in o.data.vertices)
  fs.extend(tuple(off+i for i in f.vertices) for f in o.data.polygons)
 return BVHTree.FromPolygons(vs,fs)
source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004')
audit=json.loads((WORK/'restart2/winch-room-physical-v9/native-first-hit-audit.json').read_text());results=[]
for state,index in [('transition-00',0),('transition-44',44)]:
 path=WORK/'restart2/winch-room-physical-v9'/state/'model.blend';bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.view_layer.update()
 parts=[o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render and o.get('native_patch')=='patch-004'];chains=[o for o in parts if o.name.startswith('Suspended chain')];original={o.name:[v.co.copy() for v in o.data.vertices] for o in chains}
 context=tree([o for o in bpy.context.scene.objects if o.type=='MESH' and not o.hide_render and o not in parts])
 frame=record['rows'][1]['frames'][index];alpha=Image.open(source/frame['image']).getchannel('A');core=ImageOps.expand(alpha,border=1,fill=0).filter(ImageFilter.MinFilter(3)).crop((1,1,alpha.width+1,alpha.height+1));points=[]
 centers={tuple(h['native_pixel']) for h in next(r for r in audit['states'] if r['state']==state)['source_hole_centers']}
 for y in range(-3,alpha.height+3):
  for x in range(-3,alpha.width+3):
   px,py=frame['bbox'][0]+x,frame['bbox'][1]+y;origin=Vector((px+.5,-(py+.5)/s,0))+back*10000;ch=context.ray_cast(origin,-back)
   inside=0<=x<alpha.width and 0<=y<alpha.height
   points.append((origin,bool(inside and alpha.getpixel((x,y))>=128),bool(inside and core.getpixel((x,y))>=128),(px,py) in centers,ch[3] if ch[0] is not None else 1e20))
 candidates=[]
 for left in (1,1.133333333,1.266666667,1.4):
  for right in (0,):
   for o in chains:
    
    for v,old in zip(o.data.vertices,original[o.name]):
     r=math.hypot(old.x,old.y);scale=(1.5+(r-1.5)*left)/r;v.co=Vector((old.x*scale,old.y*scale,old.z*left))
    o.data.update()
   bpy.context.view_layer.update();mesh=tree(parts);counts={'opaque_missed':0,'core_missed':0,'source_empty_filled':0,'hole_centers_filled':0}
   for origin,opaque,core_pixel,hole,depth in points:
    hit=mesh.ray_cast(origin,-back);shown=hit[0] is not None and hit[3]<depth-1e-5
    if opaque and not shown:counts['opaque_missed']+=1;counts['core_missed']+=int(core_pixel)
    if not opaque and shown:counts['source_empty_filled']+=1;counts['hole_centers_filled']+=int(hole)
   candidates.append({'wire_radius':left*.75,**counts})
 results.append({'state':state,'model_sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'candidates':candidates})
OUT.mkdir();(OUT/'sweep.json').write_text(json.dumps({'status':'Private diagnostic, no model edited or automatic score acceptance','guard':'All source empty pixels in frame plus 3-pixel margin contribute; independently detected enclosed hole centers must stay clear.','states':results},indent=2)+'\n')
print(json.dumps([{'state':r['state'],'best_core':sorted(r['candidates'],key=lambda x:(x['hole_centers_filled'],x['core_missed'],x['source_empty_filled']))[:4]} for r in results],indent=2))
