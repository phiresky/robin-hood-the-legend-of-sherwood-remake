"""Audit actual saved texture texels at the 6,276 protected native bark ray hits."""
import io,json,sys,shutil
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.geometry import barycentric_transform
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release
from review_evidence import sha
R=ROOT/'level-editor/work/active/croisement01/restart2';W=R/'approved-tree08-bark-fill-v3/croisement01-tree-08'
model=W/'baked-v1/worker.blend'
if '--' in sys.argv:model=Path(sys.argv[sys.argv.index('--')+1]).resolve()
acquire(slots=2)
try:
 assert shutil.disk_usage(R).free>=10*1024**3
 assert next(int(x.split()[1]) for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))>=6*1024**2
 bpy.ops.wm.open_mainfile(filepath=str(model));o=next(o for o in bpy.context.scene.objects if o.type=='MESH');mesh=o.data
 world=[o.matrix_world@v.co for v in mesh.vertices];bvh=BVHTree.FromPolygons(world,[p.vertices[:] for p in mesh.polygons])
 core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0
 expected=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal-rgba.png').convert('RGBA'))
 cache={};textures={}
 for slot,mat in enumerate(mesh.materials):
  if not mat:continue
  nodes=[n for n in mat.node_tree.nodes if n.type=='TEX_IMAGE' and n.image]
  if len(nodes)!=1:continue
  node=nodes[0];im=node.image
  if im.name not in cache:cache[im.name]=np.asarray(Image.open(io.BytesIO(im.packed_file.data)).convert('RGBA'))
  links=node.inputs['Vector'].links
  uv=mesh.uv_layers[links[0].from_node.uv_map] if links and links[0].from_node.type=='UVMAP' else mesh.uv_layers.active
  textures[slot]=(node,uv,cache[im.name])
 s,c=np.sin(np.radians(35)),np.cos(np.radians(35));ray,down=Vector((0,-c,s)),Vector((0,-s,-c))
 misses=[];mismatches=[];samples=[]
 for y,x in np.argwhere(core):
  hit=bvh.ray_cast(Vector((x+331.5,0,0))+down*(y+11.5)+ray*2000,-ray,4000)
  if hit[2] is None:misses.append([int(x),int(y)]);continue
  face=mesh.polygons[hit[2]];node,uv,pixels=textures[face.material_index]
  mapped=barycentric_transform(hit[0],*[world[i] for i in face.vertices],*[Vector((*uv.data[i].uv,0)) for i in face.loop_indices])
  sx=int(np.floor(mapped.x*pixels.shape[1]));sy=int(np.floor((1-mapped.y)*pixels.shape[0]))
  actual=pixels[sy,sx];row={'native':[int(x),int(y)],'face':face.index,'image':node.image.name,'texel':[sx,sy],'actual':actual.tolist(),'expected':expected[y,x].tolist(),'interpolation':node.interpolation}
  samples.append(row)
  if not np.array_equal(actual,expected[y,x]):mismatches.append(row)
 report={'status':'PASS_NATIVE_TEXEL_RGBA' if not misses and not mismatches else 'HOLD_NATIVE_TEXEL_RGBA','model_sha256':sha(model),'source_pixels':int(core.sum()),'misses':misses,'mismatches':mismatches,'sample_count':len(samples),'interpolation':sorted({x['interpolation'] for x in samples}),'scope':'Native first-hit UV nearest atlas texel audit; Linear materials additionally interpolate neighbors and are not certified exact by this audit.'}
 (model.parent/'native-texel-guard.json').write_text(json.dumps(report,indent=2)+'\n')
 (model.parent/'native-texel-samples.json').write_text(json.dumps(samples,separators=(',',':'))+'\n')
 print(report['status'],len(misses),len(mismatches),report['interpolation'],flush=True)
finally:release()
