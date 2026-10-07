"""Measure separate chain phase hypotheses without clipping or saving chain geometry."""
import hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-motion-physical-v2';OUT=WORK/'restart2/winch-chain-motion-fit-v1'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageOps,ImageFilter
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');motion=json.loads((BASE/'motion.json').read_text());hole_audit=json.loads((BASE/'hole-owner-audit-v2.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;bpy.context.view_layer.update()
def tree(objects):
 vs=[];fs=[];owners=[]
 for o in objects:
  off=len(vs);vs.extend(o.matrix_world@v.co for v in o.data.vertices)
  for f in o.data.polygons:fs.append(tuple(off+i for i in f.vertices));owners.append(o.get('native_patch')=='patch-004')
 return BVHTree.FromPolygons(vs,fs),owners
chains=[o for o in scene.objects if o.name.startswith('Suspended chain')];groups={'left':[o for o in chains if o.location.x<2406],'right':[o for o in chains if o.location.x>=2406]};chain_trees={key:tree(obs)[0] for key,obs in groups.items()};rows=[]
for pose,f,ha in zip(motion['rows'],frames,hole_audit['frames']):
 scene.frame_set(pose['tick']);bpy.context.view_layer.update();alpha=Image.open(source/f['image']).getchannel('A');core=ImageOps.expand(alpha,border=1,fill=0).filter(ImageFilter.MinFilter(3)).crop((1,1,alpha.width+1,alpha.height+1));holes={tuple(h['native_pixel']) for h in ha['holes']};sides={}
 for side,objects in groups.items():
  fixed,owners=tree([o for o in scene.objects if o.type=='MESH' and not o.hide_render and o not in objects]);mesh=chain_trees[side];points=[];x0,x1=(2395,2405) if side=='left' else (2407,2415)
  for y in range(882,958):
   for x in range(x0,x1):
    origin=Vector((x+.5,-(y+.5)/s,0))+back*10000;hit=fixed.ray_cast(origin,-back);depth=hit[3] if hit[0] is not None else 1e20;fixed_winch=hit[2] is not None and owners[hit[2]];xx,yy=x-f['bbox'][0],y-f['bbox'][1];inside=0<=xx<alpha.width and 0<=yy<alpha.height
    points.append((origin,depth,fixed_winch,inside and alpha.getpixel((xx,yy))>=128,inside and core.getpixel((xx,yy))>=128,(x,y) in holes))
  candidates=[]
  for dx in (-.5,0,.5):
   for k in range(-7,8):
    dy=k*.5;offset=Vector((dx,0,-dy/c));counts={'opaque_missed':0,'core_missed':0,'empty_filled':0,'hole_centers_filled':0,'holes_filled_by_chain':0}
    for origin,depth,fixed_winch,opaque,core_pixel,hole in points:
     hit=mesh.ray_cast(origin-offset,-back);chain_first=hit[0] is not None and hit[3]<depth-1e-5;shown=chain_first or fixed_winch
     if opaque and not shown:counts['opaque_missed']+=1;counts['core_missed']+=int(core_pixel)
     if not opaque and shown:counts['empty_filled']+=1;counts['hole_centers_filled']+=int(hole);counts['holes_filled_by_chain']+=int(hole and chain_first)
    candidates.append({'dx_world':dx,'dy_source':dy,**counts})
  candidates.sort(key=lambda r:(r['hole_centers_filled'],r['opaque_missed']+r['empty_filled']+3*r['core_missed'],abs(r['dx_world'])+abs(r['dy_source'])))
  sides[side]={'best':candidates[0],'candidates':candidates}
 rows.append({'frame':pose['source_frame'],'sides':sides})
OUT.mkdir();(OUT/'fit.json').write_text(json.dumps({'status':'Private phase diagnostics only, no geometry changes','model_sha256':motion['model_sha256'],'scope':'Independent left/right chain native bands, all opaque/empty pixels and independently enclosed hole centers; source coordinate vertical offsets limited±3.5 pixels, horizontal±0.5.','limitations':['Period-wrap solutions do not yet define a complete physical chain return/attachment or continuous link identity.','No screen-plane clipping or alpha cutouts were used.','A phase fit is not source-color/material approval.'],'frames':rows},indent=2)+'\n');print(json.dumps({'best_left_holes':[r['sides']['left']['best']['hole_centers_filled'] for r in rows],'best_right_holes':[r['sides']['right']['best']['hole_centers_filled'] for r in rows]}))
