"""Coverage-aware source-only bake on frozen finite sawn timber geometry."""
import hashlib,json,math,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];WORK=ROOT/'level-editor/work/york-refinement/restart2';BASE=WORK/'loose-planks-candidate-v5';OUT=WORK/'loose-planks-source-v6'
if OUT.exists():raise FileExistsError(OUT)
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
assert hashlib.sha256((BASE/'model.blend').read_bytes()).hexdigest()=='0d877de4b63cbbe805ffe12c638e419e79a60dc9cc2a65efb1668447ddf08986'
assert hashlib.sha256((WORK/'timber-observed-domains-v1/report.json').read_bytes()).hexdigest()=='13b315ac0a225fd37d2ae4899291e10a488dafa573de61acfe15761eba8669c3'
assert hashlib.sha256((WORK/'timber-sawn-source-review-v1/source-classification.json').read_bytes()).hexdigest()=='e8c3d649c54d22e086d7f1fa056ebb47491be4f02e70fe4162f2986019f8d3a6'
assert hashlib.sha256((WORK/'pair-v14/assets/york-southwest-square-west-house/reference/source.png').read_bytes()).hexdigest()=='3d83bfab6b8ff1c27f87c82c0c034dc854df44b4e2322a09e7e083db57fb409e'
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from render_views import render_views
bpy.ops.wm.open_mainfile(filepath=str(BASE/'model.blend'));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;bpy.context.view_layer.update();objects=[o for o in scene.objects if o.type=='MESH']
def shape():
 return {o.name:{'world':[list(v)for v in o.matrix_world],'vertices':[list(v.co)for v in o.data.vertices],'faces':[list(p.vertices)for p in o.data.polygons],'uv':[[list(v.uv)for v in layer.data]for layer in o.data.uv_layers]}for o in objects}
before=shape();vv=[];ff=[];owners=[]
for o in objects:
 offset=len(vv);vv.extend(o.matrix_world@v.co for v in o.data.vertices)
 for f in o.data.polygons:ff.append(tuple(offset+i for i in f.vertices));owners.append((o.data.name,f.index,'top'if f.normal.z>.99 else'bottom'if f.normal.z<-.99 else'side'))
tree=BVHTree.FromPolygons(vv,ff);s=math.sin(math.radians(35));c=math.cos(math.radians(35));back=Vector((0,-c,s));art=Image.open(WORK/'pair-v14/assets/york-southwest-square-west-house/reference/source.png').convert('RGBA');domains=json.loads((WORK/'timber-observed-domains-v1/report.json').read_text());classification=json.loads((WORK/'timber-sawn-source-review-v1/source-classification.json').read_text());excluded={tuple(r['pixel'])for r in classification['accepted_blue_side_review']if r['classification']=='FOREIGN_GROUND_PROPOSED_EXCLUSION'};pending={tuple(r['pixel'])for r in classification['accepted_blue_side_review']if r['classification']=='BOUNDARY_OR_SHADED_WOOD_UNRESOLVED'};corrections={(1269,938),(1270,938),(1271,938)}
images={(p,f):Image.new('RGBA',(80,58),(128,128,128,255))for p,f,role in owners};ledger=[]
for row in domains['pixels']:
 x,y=row['pixel'];key=(x,y);expected=tuple(row['owner']);item={'pixel':[x,y],'previous_owner':expected}
 if key in excluded:item['status']='EXPLICIT_FOREIGN_GROUND';ledger.append(item);continue
 if key in pending:item['status']='UNRESOLVED_BLUE_DARK_BOUNDARY';ledger.append(item);continue
 if key in corrections:expected=('long-crossing','side');item['correction']='Independent brown beam row preceding cream board edge'
 contributions={}
 for j in range(8):
  for i in range(8):
   sx=x+(i+.5)/8;sy=y+(j+.5)/8;origin=Vector((sx,-sy/s,0))+back*1000;loc,normal,index,d=tree.ray_cast(origin,-back)
   if loc is None or normal.dot(back)<.05:continue
   piece,face,role=owners[index]
   if (piece,role)!=expected:continue
   contributions[(piece,face)]=contributions.get((piece,face),0)+1
 for owner in contributions:images[owner].putpixel((x-1240,y-920),art.getpixel((x,y)))
 item.update({'expected_owner':expected,'status':'OBSERVED_SOURCE_FOOTPRINT'if contributions else'UNMAPPED_SOURCE_TARGET','contributions':[{'piece':p,'face':f,'samples':n}for(p,f),n in contributions.items()]});ledger.append(item)
OUT.mkdir();(OUT/'face-source').mkdir()
for o in objects:
 for f in o.data.polygons:
  path=OUT/'face-source'/f'{o.data.name}-{f.index}.png';images[(o.data.name,f.index)].save(path);image=bpy.data.images.load(str(path));image.pack();mat=o.data.materials[f.material_index];nodes=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE'];assert len(nodes)==1;nodes[0].image=image
assert shape()==before
bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True);assert(OUT/'model.blend').stat().st_size<8*1024**2
bpy.ops.wm.open_mainfile(filepath=str(OUT/'model.blend'));scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];assert shape()==before
report=json.loads((BASE/'candidate.json').read_text());counts={status:sum(r['status']==status for r in ledger)for status in sorted(set(r['status']for r in ledger))};report.update({'model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'geometry_uv_world_unchanged':True,'source_diagnostic':{'original_target_count':973,'statuses':counts,'ledger':ledger,'limits':'8x8 first-visible pixel footprint allocation. Original source RGB preserved; no synthesized texels. Six explicit foreign exclusions and14 unresolved boundary texels remain separately accounted.'}});(OUT/'candidate.json').write_text(json.dumps(report,indent=2)+'\n')
views={str(i):f'View{i}'for i in range(8)};render_views(scene.name,views,OUT/'review',modes=('textured','solid'),width=256)
for mode in ('textured','solid'):
 sheet=Image.new('RGBA',(1024,552),(35,40,45,255));draw=ImageDraw.Draw(sheet)
 for i in range(8):
  im=Image.open(OUT/'review'/f'{i}-{mode}.png').convert('RGBA');x=(i%4)*256;y=(i//4)*276;sheet.alpha_composite(im,(x,y+20));draw.text((x+5,y+4),'Original art camera'if i==0 else f'View{i}',fill='white')
 sheet.convert('RGB').save(OUT/f'{mode}8.png')
assert sum(p.stat().st_size for p in OUT.rglob('*')if p.is_file())<32*1024**2
print(json.dumps({'model_sha256':report['model_sha256'],'counts':counts,'geometry_uv_world_unchanged':True}))
