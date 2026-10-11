"""Count unfilled atlas samples visible in the final eight saved-model cameras."""
import json,shutil,sys
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector,Matrix
from mathutils.geometry import barycentric_transform
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';W=R/'approved-tree08-bark-fill-v3/croisement01-tree-08';O=W/'baked-v3-native-exact'
if '--' in sys.argv:O=Path(sys.argv[sys.argv.index('--')+1]).resolve()
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 bpy.ops.wm.open_mainfile(filepath=str(O/'worker.blend'));obj=next(o for o in bpy.context.scene.objects if o.type=='MESH');mesh=obj.data
 world=[obj.matrix_world@v.co for v in mesh.vertices];tree=BVHTree.FromPolygons(world,[p.vertices[:] for p in mesh.polygons])
 provenance={'exterior':np.load(json.loads((W/'baked-v1/layer-0.json').read_text())['objects'][0]['texel_provenance']['path'])['ownership'].copy()}
 if O.name in ('baked-v5-bounded-gaps','baked-v6-connected-gaps','baked-v7-connected-gaps','baked-v8-connected-gaps'):
  provenance['exterior / visible-gap-v1']=np.load(json.loads((W/'baked-v5-bounded-gaps/layer-0.json').read_text())['objects'][0]['texel_provenance']['path'])['ownership'].copy()
 maps={};by_image={}
 for slot,material in enumerate(mesh.materials):
  if not material or not material.get('source_ownership_bake'):continue
  label=material['source_ownership_label'];ownership=provenance[label];uv=mesh.uv_layers[next(n.uv_map for n in material.node_tree.nodes if n.type=='UVMAP')];image=next(n.image for n in material.node_tree.nodes if n.type=='TEX_IMAGE');maps[slot]=(uv,ownership);by_image[image.name]=ownership
 samples=O/'native-texel-samples.json'
 if not samples.exists():samples=W/'baked-v3-native-exact/native-texel-samples.json'
 for row in json.loads(samples.read_text()):
  ownership=by_image.get(row['image'])
  if ownership is not None:x,y=row['texel'];ownership[ownership.shape[0]-1-y,x]=1
 if (O/'gap-repair.json').exists():
  for row in json.loads((O/'gap-repair.json').read_text())['repairs']:
   x,y=row['texel'];by_image[row['image']][y,x]=3
 frames=json.loads((W/'experiment/views.json').read_text());width,height=frames['tile_size'];rows=[];folder=O/'visible-fill-audit';folder.mkdir(exist_ok=False)
 for v in frames['views']:
  matrix=Matrix(v['camera_matrix_world']);scale=v['ortho_scale'];horizontal=scale*width/height;direction=matrix.to_3x3()@Vector((0,0,-1));classes=np.zeros((height,width),np.uint8);hits=0;unknown=[]
  for y in range(height):
   for x in range(width):
    origin=matrix@Vector((((x+.5)/width-.5)*horizontal,(.5-(y+.5)/height)*scale,0));hit=tree.ray_cast(origin,direction)
    if hit[2] is None:continue
    hits+=1;face=mesh.polygons[hit[2]]
    if face.material_index not in maps:classes[y,x]=5;continue
    uv,ownership=maps[face.material_index]
    mapped=barycentric_transform(hit[0],*[world[i] for i in face.vertices],*[Vector((*uv.data[i].uv,0)) for i in face.loop_indices]);sx=int(mapped.x*ownership.shape[1]);sy=int(mapped.y*ownership.shape[0]);value=int(ownership[sy,sx]);classes[y,x]=value+1
    if value==0:unknown.append([x,y,face.index,sx,sy])
  rgba=np.zeros((height,width,4),np.uint8);rgba[:,:,3]=255;rgba[classes==1,:3]=[255,30,30];rgba[classes==2,:3]=[30,180,70];rgba[classes==3,:3]=[80,140,210];rgba[classes==4,:3]=[180,80,210];rgba[classes==5,:3]=[220,170,20]
  Image.fromarray(rgba).save(folder/f'view-{v["index"]}-coverage.png');rows.append({'view':v['index'],'surface_pixels':hits,'unfilled_pixels':len(unknown),'unfilled_fraction':len(unknown)/max(1,hits),'unfilled':unknown})
 output={'status':'AUDITED','model_sha256':sha(O/'worker.blend'),'views':rows,'legend':'red=unfilled,green=source,blue=generated,yellow=retained tiny face','limitation':'Pixel-center sample audit in the eight reviewed cameras, not hidden-surface completeness.'};(folder/'report.json').write_text(json.dumps(output,indent=2)+'\n');print([(r['view'],r['unfilled_pixels'],r['surface_pixels']) for r in rows],flush=True)
finally:release()
