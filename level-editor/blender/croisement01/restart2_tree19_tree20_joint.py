"""Check the source branch against the exact published neighboring tree."""
import argparse,json,math,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw,ImageChops
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree18 import ROOT,OUT,SIN,COS
from evidence_io import sha
from render_slots import acquire
p=argparse.ArgumentParser();p.add_argument('revision');a=p.parse_args(sys.argv[sys.argv.index('--')+1:])
w=OUT/f'restart2/tree19-v{a.revision}/assets/croisement01-tree-19';out=w/'inspection/tree20-joint';out.mkdir(exist_ok=False)
live=ROOT/'level-editor/library/3d-assets/croisement01/croisement01-tree-20';expected='2909f07178f8e00405362133fcb35ee6a1b99df3f0f048c4d1b6bce58299fd2c';assert sha(live/'model.glb')==expected
acquire();bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));cfg=json.loads((w/'workspace.json').read_text());scene=bpy.data.scenes[cfg['scene_name']];bpy.context.window.scene=scene
own=[o for o in bpy.data.collections[cfg['collection_name']].all_objects if o.type=='MESH' and o.get('asset_group')==cfg['asset_id']];wood=[o for o in own if o.get('source_node','').startswith('building-')]
before=set(bpy.data.objects);bpy.ops.import_scene.gltf(filepath=str(live/'model.glb'));added=set(bpy.data.objects)-before
pivot=Vector(json.loads((live/'asset.json').read_text())['source_origin_scene'])
for obj in added:
 if obj.parent not in added:obj.location+=pivot
bpy.context.view_layer.update()
neighbor=[o for o in added if o.type=='MESH'];stem=[o for o in neighbor if o.parent and o.parent.name=='building-082'];assert len(stem)==1
for obj in bpy.data.objects:
 if obj.type=='MESH':obj.hide_render=obj not in own+neighbor

def bvh(objects):
 verts=[];faces=[]
 for obj in objects:
  offset=len(verts);verts.extend(obj.matrix_world@v.co for v in obj.data.vertices);faces.extend(tuple(offset+i for i in f.vertices) for f in obj.data.polygons)
 return BVHTree.FromPolygons(verts,faces)
first=bvh(wood);second=bvh(stem);toward=Vector((0,-COS,SIN))
manifest=json.loads((OUT/'baseline/masks/manifest.json').read_text());m19=next(m for m in manifest['masks'] if m['index']==19);m20=next(m for m in manifest['masks'] if m['index']==20)
x0,y0=m20['box_top_left'];width,height=m20['box_size'];domain19=Image.open(w.parents[1]/'wood-domain.png').convert('L');mask19=Image.open(OUT/'baseline/masks'/m19['png']).convert('L');mask20=Image.open(OUT/'baseline/masks'/m20['png']).convert('L')
canvas=Image.new('L',(width,height));canvas.paste(mask19,(m19['box_top_left'][0]-x0,m19['box_top_left'][1]-y0));overlap=ImageChops.multiply(canvas,mask20)
authored=Image.new('L',(width,height));authored.paste(domain19,(m19['box_top_left'][0]-x0,m19['box_top_left'][1]-y0));owned=np.asarray(ImageChops.multiply(overlap,authored))>0
counts=dict(native_overlap=int(np.count_nonzero(np.asarray(overlap))),authored_branch_samples=int(owned.sum()),foreground=0,behind=0,missing_branch=0,missing_neighbor=0);depths=[];diagnostic=Image.new('RGB',(width,height),'#333333')
for row,col in zip(*np.where(owned)):
 origin=Vector((x0+col+.5,-(y0+row+.5)/SIN,0))+toward*5000
 h1=first.ray_cast(origin,-toward,10000);h2=second.ray_cast(origin,-toward,10000)
 if h1[0] is None:counts['missing_branch']+=1;color=(40,80,255)
 elif h2[0] is None:counts['missing_neighbor']+=1;color=(255,200,0)
 elif h1[3]<h2[3]-.01:counts['foreground']+=1;color=(40,230,80);depths.append(h2[3]-h1[3])
 else:counts['behind']+=1;color=(255,40,40)
 diagnostic.putpixel((int(col),int(row)),color)
diagnostic.resize((width*6,height*6),Image.Resampling.NEAREST).save(out/'source-order.png')
# The actual native camera also checks source colors and the visible joint.
left,top,right,bottom=1120,0,1265,305;target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0))
data=bpy.data.cameras.new('Original game joint camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=right-left;data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+toward*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256;scene.render.resolution_x=right-left;scene.render.resolution_y=bottom-top;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.filepath=str(out/'actual-native.png');scene.view_settings.view_transform='Standard';scene.view_settings.look='None';bpy.ops.render.render(write_still=True,scene=scene.name)
source=Image.open(OUT/'baseline/covered.png').convert('RGBA').crop((left,top,right,bottom));actual=Image.open(out/'actual-native.png').convert('RGBA');board=Image.new('RGB',((right-left)*3*3,(bottom-top)*3+28),'#444444');draw=ImageDraw.Draw(board)
for i,(label,image) in enumerate([('Original native source',source),('Saved tree19 + published tree20',actual),('Native overlay',Image.alpha_composite(source,actual))]):
 draw.text((i*(right-left)*3+4,5),label,fill='white');scaled=image.resize(((right-left)*3,(bottom-top)*3),Image.Resampling.NEAREST);board.paste(scaled,(i*(right-left)*3,28),scaled)
board.save(out/'native-comparison.png')
report=dict(status='Diagnostic; saved image and physical source-order judgment required',tree19_model_sha256=sha(w/'model.blend'),published_tree20_glb_sha256=expected,published_tree20_descriptor_sha256=sha(live/'asset.json'),counts=counts,minimum_foreground_depth_margin=min(depths) if depths else None,native_comparison_sha256=sha(out/'native-comparison.png'),source_order_image_sha256=sha(out/'source-order.png'),scope='Only exact native19/20 overlap samples within authored tree19 wood domain. No appearance or unrelated foliage completion claim; live assets unchanged.')
(out/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
