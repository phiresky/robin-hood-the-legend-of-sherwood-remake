"""Review all saved physical winch poses, native first hits and four complete orbits."""
import hashlib,json,math,sys
from pathlib import Path
from collections import deque
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement';BASE=WORK/'restart2/winch-motion-physical-v2';OUT=BASE/'review'
if OUT.exists():raise FileExistsError(OUT)
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw,ImageOps,ImageFilter
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
sys.path.insert(0,str(Path(__file__).parent))
from restart2_camera_audit import audit_manifest,labeled_copy
source=WORK/'geometry-pass-01/native-state-source-v1';record=next(r for r in json.loads((source/'manifest.json').read_text())['records'] if r['id']=='patch-004');frames=next(r['frames'] for r in record['rows'] if r['action']=='PatchTransition');motion=json.loads((BASE/'motion.json').read_text());assert hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest()==motion['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));camdata=bpy.data.cameras.new('Native animated mechanism');cam=bpy.data.objects.new(camdata.name,camdata);scene.collection.objects.link(cam);camdata.type='ORTHO';camdata.ortho_scale=110;camdata.clip_end=20000;cam.location=Vector((2410,-930/s,0))+back*10000;cam.rotation_euler=(-back).to_track_quat('-Z','Y').to_euler();OUT.mkdir();sheets=[Image.new('RGB',(1800,1065),'#303840') for _ in range(3)];audits=[]
def holes(alpha):
 left={(x,y) for y in range(alpha.height) for x in range(alpha.width) if alpha.getpixel((x,y))<128};out=[]
 while left:
  seed=left.pop();todo=[seed];part=[seed]
  for x,y in todo:
   for p in ((x-1,y),(x+1,y),(x,y-1),(x,y+1)):
    if p in left:left.remove(p);todo.append(p);part.append(p)
  if len(part)<3 or any(x in (0,alpha.width-1) or y in (0,alpha.height-1) for x,y in part):continue
  cx=sum(x for x,y in part)/len(part);cy=sum(y for x,y in part)/len(part);out.append(min(part,key=lambda p:((p[0]-cx)**2+(p[1]-cy)**2,p)))
 return out
for pose,f in zip(motion['rows'],frames):
 i=pose['source_frame'];scene.frame_set(pose['tick']);bpy.context.view_layer.update();vs=[];fs=[];owners=[]
 for o in scene.objects:
  if o.type!='MESH' or o.hide_render:continue
  off=len(vs);vs.extend(o.matrix_world@v.co for v in o.data.vertices)
  for face in o.data.polygons:fs.append(tuple(off+j for j in face.vertices));owners.append(o.get('native_patch')=='patch-004')
 tree=BVHTree.FromPolygons(vs,fs);alpha=Image.open(source/f['image']).getchannel('A');core=ImageOps.expand(alpha,border=1,fill=0).filter(ImageFilter.MinFilter(3)).crop((1,1,alpha.width+1,alpha.height+1));counts={'opaque':0,'opaque_missed':0,'core_missed':0,'hole_centers_filled':0,'hole_centers_total':0}
 for y in range(alpha.height):
  for x in range(alpha.width):
   if alpha.getpixel((x,y))<128:continue
   px,py=f['bbox'][0]+x+.5,f['bbox'][1]+y+.5;hit=tree.ray_cast(Vector((px,-py/s,0))+back*10000,-back);known=hit[2] is not None and owners[hit[2]];counts['opaque']+=1
   if not known:counts['opaque_missed']+=1;counts['core_missed']+=int(core.getpixel((x,y))>=128)
 for x,y in holes(alpha):
  px,py=f['bbox'][0]+x+.5,f['bbox'][1]+y+.5;hit=tree.ray_cast(Vector((px,-py/s,0))+back*10000,-back);counts['hole_centers_total']+=1;counts['hole_centers_filled']+=int(hit[2] is not None and owners[hit[2]])
 audits.append({'frame':i,'tick':pose['tick'],**counts});scene.render.resolution_x=180;scene.render.resolution_y=330;out=OUT/f'frame-{i:02d}';render_views(scene.name,{'native':cam.name},out,modes=('textured',),width=180)
 tile=Image.new('RGBA',(60,110),'#303840');tile.alpha_composite(Image.open(source/f['image']).convert('RGBA'),(f['bbox'][0]-2380,f['bbox'][1]-875));tile=tile.resize((180,330),Image.Resampling.NEAREST);candidate=Image.open(out/'native-textured.png');sheet=sheets[i//15];x=(i%5)*360;y=((i%15)//5)*355;sheet.paste(tile.convert('RGB'),(x,y+25));sheet.paste(candidate,(x+180,y+25),candidate);ImageDraw.Draw(sheet).text((x+3,y+6),f'{i}: source / saved physical pose',fill='white')
for i,sheet in enumerate(sheets):sheet.save(OUT/f'native-frames-{15*i:02d}-{15*i+14:02d}.png')
for index in (0,22,36,44):
 scene.frame_set(motion['rows'][index]['tick']);bpy.context.view_layer.update()
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('native_patch')!='patch-004'
 out=OUT/f'isolated-{index:02d}';out.mkdir();names={f'view-{i}':f'Winch{i}' for i in range(8)};rows=[]
 for i in range(8):
  cc=scene.objects[f'Winch{i}'];rows.append({'index':i,'azimuth_degrees':i*45,'camera_matrix_world':[list(r) for r in cc.matrix_world],'ortho_scale':cc.data.ortho_scale})
 (out/'views.json').write_text(json.dumps({'layout':{'columns':4,'rows':2},'views':rows}));audit_manifest(out/'views.json');scene.render.resolution_x=320;scene.render.resolution_y=384;render_views(scene.name,names,out/'renders',modes=('textured',),width=320);sheet=Image.new('RGBA',(1280,768))
 for i in range(8):sheet.paste(Image.open(out/f'renders/view-{i}-textured.png'),((i%4)*320,(i//4)*384))
 sheet.save(out/'solid8.png');labeled_copy(out/'solid8.png',out/'solid8-native-labeled.png')
(OUT/'first-hit-audit.json').write_text(json.dumps({'status':'Diagnostic only; full native opaque and independently enclosed hole centres','model_sha256':motion['model_sha256'],'frames':audits},indent=2)+'\n');print(json.dumps({'core_misses':[r['core_missed'] for r in audits],'filled_hole_centers':[r['hole_centers_filled'] for r in audits]}))
