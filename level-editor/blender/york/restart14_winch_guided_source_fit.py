"""Score all source frames for the guided chain without changing its saved model."""
import ast,hashlib,json,math,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-guided-entry-candidate-v1';OUT=WORK/'restart2/winch-guided-entry-source-fit-v1'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy,numpy as np
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageOps,ImageFilter
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));up=Vector((0,0,1));screen_side=Vector((1,0,0));spacing=3.5/c
for name in ('restart13_winch_return_study.py','restart14_winch_guided_entry.py'):
 recipe=Path(__file__).with_name(name);exec(compile(ast.Module(body=[n for n in ast.parse(recipe.read_text()).body if isinstance(n,ast.FunctionDef)],type_ignores=[]),str(recipe),'exec'))
center=world(2410,1064,104);axis=(center-world(2402,1050,104)).normalized();radial=Vector((-axis.y,axis.x,0));pose,params,path=guided_route();proposal=json.loads((BASE/'proposal.json').read_text())
def tree(objects):
 vs=[];fs=[];owners=[]
 for o in objects:
  off=len(vs);vs.extend(o.matrix_world@v.co for v in o.data.vertices)
  for face in o.data.polygons:fs.append(tuple(off+i for i in face.vertices));owners.append(o.get('native_patch')=='patch-004')
 return BVHTree.FromPolygons(vs,fs),owners
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;links=sorted((o for o in scene.objects if o.name.startswith('Guided chain link')),key=lambda o:o.name);assert len(links)==proposal['path']['count'];meshes=[]
for phase in range(14):
 for i,o in enumerate(links):p,r=pose(i*spacing+(phase*.5)/c,i);o.location=p;o.rotation_euler=r.to_euler()
 bpy.context.view_layer.update();meshes.append(tree(links)[0])
source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');motion=json.loads((WORK/'restart2/winch-motion-physical-v2/motion.json').read_text());holes=json.loads((WORK/'restart2/winch-motion-physical-v2/hole-owner-audit-v2.json').read_text());rows=[]
for sample,f,ha in zip(motion['rows'],frames,holes['frames']):
 scene.frame_set(sample['tick']);bpy.context.view_layer.update();fixed,owners=tree([o for o in scene.objects if o.type=='MESH' and not o.hide_render and o not in links]);alpha=Image.open(source/f['image']).getchannel('A');core=ImageOps.expand(alpha,border=1,fill=0).filter(ImageFilter.MinFilter(3)).crop((1,1,alpha.width+1,alpha.height+1));centers={tuple(h['native_pixel']) for h in ha['holes']};points=[]
 for y in range(880,979):
  for x in range(2388,2434):
   xx,yy=x-f['bbox'][0],y-f['bbox'][1];inside=0<=xx<alpha.width and 0<=yy<alpha.height;origin=Vector((x+.5,-(y+.5)/s,0))+back*10000;hit=fixed.ray_cast(origin,-back);points.append((origin,hit[3] if hit[0] is not None else 1e20,hit[2] is not None and owners[hit[2]],inside and alpha.getpixel((xx,yy))>=128,inside and core.getpixel((xx,yy))>=128,(x,y) in centers))
 candidates=[]
 for phase,mesh in enumerate(meshes):
  counts={'opaque_missed':0,'core_missed':0,'empty_filled':0,'hole_centers_filled':0}
  for origin,depth,other,opaque,core_pixel,hole in points:
   hit=mesh.ray_cast(origin,-back);shown=other or(hit[0] is not None and hit[3]<depth-1e-5)
   if opaque and not shown:counts['opaque_missed']+=1;counts['core_missed']+=int(core_pixel)
   if not opaque and shown:counts['empty_filled']+=1;counts['hole_centers_filled']+=int(hole)
  candidates.append({'phase_game':phase*.5,**counts})
 candidates.sort(key=lambda r:(r['hole_centers_filled'],r['opaque_missed']+r['empty_filled']+3*r['core_missed']));rows.append({'frame':sample['source_frame'],'best':candidates[0],'candidates':candidates})
OUT.mkdir();(OUT/'fit.json').write_text(json.dumps({'status':'Diagnostic only; complete return and link identity retained but hidden support/contact not ready','model_sha256':proposal['model_sha256'],'frames':rows},indent=2)+'\n');print(json.dumps({'best_holes':[r['best']['hole_centers_filled'] for r in rows],'best_core':[r['best']['core_missed'] for r in rows]}))
