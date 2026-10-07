"""Check inferred travelling-disc thickness against observed silhouette and apertures."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-motion-physical-v2';OUT=WORK/'restart2/winch-travelling-disc-fit-v1'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageOps,ImageFilter
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');motion=json.loads((BASE/'motion.json').read_text());holes=json.loads((BASE/'hole-owner-audit-v2.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;disc=scene.objects['Travelling round part solid drum'];original=disc.scale.copy();rows=[]
def tree(objects):
 vs=[];fs=[];owners=[]
 for o in objects:
  off=len(vs);vs.extend(o.matrix_world@v.co for v in o.data.vertices)
  for face in o.data.polygons:fs.append(tuple(off+i for i in face.vertices));owners.append(o.get('native_patch')=='patch-004')
 return BVHTree.FromPolygons(vs,fs),owners
for m,f,ha in zip(motion['rows'][22:],frames[22:],holes['frames'][22:]):
 scene.frame_set(m['tick']);bpy.context.view_layer.update();fixed,owners=tree([o for o in scene.objects if o.type=='MESH' and not o.hide_render and o!=disc]);alpha=Image.open(source/f['image']).getchannel('A');core=ImageOps.expand(alpha,border=1,fill=0).filter(ImageFilter.MinFilter(3)).crop((1,1,alpha.width+1,alpha.height+1));centers={tuple(h['native_pixel']) for h in ha['holes']};points=[]
 for y in range(878,945):
  for x in range(2386,2409):
   origin=Vector((x+.5,-(y+.5)/s,0))+back*10000;hit=fixed.ray_cast(origin,-back);xx,yy=x-f['bbox'][0],y-f['bbox'][1];inside=0<=xx<alpha.width and 0<=yy<alpha.height;points.append((origin,hit[3] if hit[0] is not None else 1e20,hit[2] is not None and owners[hit[2]],inside and alpha.getpixel((xx,yy))>=128,inside and core.getpixel((xx,yy))>=128,(x,y) in centers))
 candidates=[]
 for radius in (7.2,7.4):
  for thickness in (2,3,4):
   disc.scale=Vector((original.x*radius/7.4,original.y*radius/7.4,original.z*thickness/4));bpy.context.view_layer.update();mesh=tree([disc])[0];counts={'opaque_missed':0,'core_missed':0,'empty_filled':0,'holes_filled_by_disc':0}
   for origin,depth,other,opaque,core_pixel,hole in points:
    hit=mesh.ray_cast(origin,-back);disc_first=hit[0] is not None and hit[3]<depth-1e-5;shown=other or disc_first
    if opaque and not shown:counts['opaque_missed']+=1;counts['core_missed']+=int(core_pixel)
    if not opaque and shown:counts['empty_filled']+=1;counts['holes_filled_by_disc']+=int(hole and disc_first)
   candidates.append({'radius':radius,'thickness':thickness,**counts})
 rows.append({'frame':m['source_frame'],'candidates':candidates})
sums=[]
for k in range(6):
 rr=[r['candidates'][k] for r in rows];sums.append({key:(rr[0][key] if key in ('radius','thickness') else sum(x[key] for x in rr)) for key in rr[0]})
OUT.mkdir();(OUT/'fit.json').write_text(json.dumps({'status':'Diagnostic only; rim and crossing spokes unchanged, no geometry saved','model_sha256':motion['model_sha256'],'crop':[2386,878,2409,945],'frames':rows,'summed_candidates':sums},indent=2)+'\n');print(json.dumps(sums,indent=2))
