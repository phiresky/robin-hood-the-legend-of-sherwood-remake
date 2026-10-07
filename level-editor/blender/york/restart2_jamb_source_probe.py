"""Inspect visible approved jamb pixels before assigning any native material domain."""
import json,math,hashlib,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';OUT=WORK/'restart2/jamb-source-probe-v1'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
model=WORK/'restart2/gate-geometry-v10/covered/model.blend';digest=hashlib.sha256(model.read_bytes()).hexdigest();bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();verts=[];faces=[];owners=[]
for o in bpy.context.scene.objects:
 if o.type!='MESH' or o.hide_render:continue
 n=len(verts);verts.extend(o.matrix_world@v.co for v in o.data.vertices)
 for p in o.data.polygons:faces.append(tuple(n+i for i in p.vertices));owners.append(o.name)
tree=BVHTree.FromPolygons(verts,faces);s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));mask=Image.new('L',(220,250));pixels=[]
for y in range(250):
 for x in range(220):
  start=Vector((2250+x+.5,-(780+y+.5)/s,0))+back*10000;_,_,face,_=tree.ray_cast(start,-back)
  if face is not None and owners[face]=='building-778-portcullis-jamb-return':mask.putpixel((x,y),255);pixels.append([2250+x,780+y])
OUT.mkdir(parents=True);mask.save(OUT/'visible-jamb-domain.png');source=Image.open(WORK/'restart2/gate-source-study-v2/initial.png').convert('RGBA');overlay=Image.new('RGBA',source.size,(255,40,190,0));overlay.putalpha(mask.point(lambda v:120 if v else 0));marked=Image.alpha_composite(source,overlay);sheet=Image.new('RGB',(880,524),'#303840');sheet.paste(source.resize((440,500),Image.Resampling.NEAREST),(0,24));sheet.paste(marked.resize((440,500),Image.Resampling.NEAREST),(440,24));d=ImageDraw.Draw(sheet);d.text((5,5),'Native source',fill='white');d.text((445,5),'Diagnostic first-hit jamb, NOT material authority',fill='white');sheet.save(OUT/'source-overlay.png');assert hashlib.sha256(model.read_bytes()).hexdigest()==digest;(OUT/'report.json').write_text(json.dumps({'status':'Diagnostic only; semantic material domain not approved','approved_model_sha256':digest,'visible_pixels':len(pixels),'pixels':pixels,'new_component':'building-778-portcullis-jamb-return'},indent=2)+'\n');print('VISIBLE JAMB',len(pixels))
