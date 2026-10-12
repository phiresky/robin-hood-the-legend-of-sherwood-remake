"""Independent saved Tree14 source first-hit audit and alpha-aware solid review."""
import io
import json
import math
from pathlib import Path
import shutil
import sys

import bpy
import numpy as np
from PIL import Image
from scipy.ndimage import binary_erosion
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform

ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(ROOT/'level-editor/blender/leicester')]
from render_slots import acquire
from evidence_io import sha
from foliage_trees import geometry_hash
from render_multiview_asset import render

R=ROOT/'level-editor/work/active/croisement01/restart2';OUT=R/'tree14-whole-tree-v1'
if '--' in sys.argv:
 args=sys.argv[sys.argv.index('--')+1:]
 assert len(args)==1
 OUT=Path(args[0]).resolve();assert OUT.parent==R.resolve()
acquire(slots=2)
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024>6*1024**3
packet=json.loads((OUT/'construction.json').read_text());model=OUT/'model.blend';model_sha=sha(model);assert model_sha==packet['model_sha256']
dest=OUT/'joint-audit-v1';assert not dest.exists();dest.mkdir()
bpy.ops.wm.open_mainfile(filepath=str(model))
scene=bpy.data.scenes['Croisement01 Refinement'];bpy.context.window.scene=scene
wood=next(o for o in scene.objects if o.get('source_node')=='scenery-tree14-wood')
assert geometry_hash(wood)==packet['wood_geometry_sha256'] and [list(row) for row in wood.matrix_world]==packet['wood_matrix']
wood_tree=BVHTree.FromPolygons([wood.matrix_world@v.co for v in wood.data.vertices],[list(p.vertices) for p in wood.data.polygons])
crowns=[o for o in scene.objects if o.get('source_node') in ('foliage-tree14-crown94','foliage-tree14-crown94-interior')]
assert len(crowns)==2
layers=[]
for obj in crowns:
 assert geometry_hash(obj)==packet['crown_geometry_hashes'][obj.name]
 assert all(len(p.vertices)==3 for p in obj.data.polygons)
 tree=BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(p.vertices) for p in obj.data.polygons],all_triangles=True)
 textures=[]
 for mat in obj.data.materials:
  image=next(n.image for n in mat.node_tree.nodes if n.type=='TEX_IMAGE');assert image.packed_file
  textures.append(np.asarray(Image.open(io.BytesIO(bytes(image.packed_file.data))).convert('RGBA')))
 layers.append((obj,tree,textures))
sine,cosine=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-cosine,sine))
wood_domain=np.asarray(Image.open(OUT/'wood-role.png').convert('L'))>0
leaf_domain=np.asarray(Image.open(OUT/'crown-static-role.png').convert('L'))>0
interior=binary_erosion(wood_domain,structure=np.ones((3,3)))
source=np.asarray(Image.open(R.parent/'baseline/covered.png').convert('RGBA'))
records=[];counts={};picture=np.zeros_like(source)
for y,x in zip(*np.nonzero(wood_domain|leaf_domain)):
 origin=Vector((float(x)+.5,-(float(y)+.5)/sine,0))+ray*5000
 hit,_,_,wd=wood_tree.ray_cast(origin,-ray,10000);wd=wd if hit is not None else float('inf')
 nearest=None
 for obj,tree,textures in layers:
  uv=obj.data.uv_layers['Foliage UV'].data;start=origin.copy();travel=0.
  for attempt in range(128):
   point,normal,face,distance=tree.ray_cast(start,-ray,10000-travel)
   if point is None:break
   poly=obj.data.polygons[face];mat=obj.data.materials[poly.material_index]
   coords=barycentric_transform(point,*[obj.matrix_world@obj.data.vertices[i].co for i in poly.vertices],*[Vector((*uv[i].uv,0.)) for i in poly.loop_indices])
   image=textures[poly.material_index];h,w=image.shape[:2]
   tx=min(w-1,max(0,int(coords.x*w)));ty=min(h-1,max(0,int((1-coords.y)*h)))
   front=normal.dot(ray)>0 or mat.get('foliage_card_sides')=='double-sided'
   if front and image[ty,tx,3]>127:
    candidate=(travel+distance,bool(mat.get('foliage_observed')),image[ty,tx],obj.name,poly.material_index)
    if nearest is None or candidate[0]<nearest[0]:nearest=candidate
    break
   travel+=distance+.001;start=origin-ray*travel
  else:raise RuntimeError('Too many transparent source intersections')
 ld=nearest[0] if nearest else float('inf')
 if wood_domain[y,x]:
  if hit is None:state='wood_interior_missing' if interior[y,x] else 'wood_boundary_missing'
  elif ld<wd:state='wood_occluded_by_known_leaf' if nearest[1] else 'wood_occluded_by_unknown'
  else:state='wood_first_hit'
 else:
  if wd<ld:state='leaf_occluded_by_wood'
  elif nearest is None:state='leaf_missing'
  elif not nearest[1]:state='unknown_front_of_leaf'
  elif not np.array_equal(nearest[2],source[y,x]):state='leaf_rgba_mismatch'
  else:state='leaf_exact_rgba'
 counts[state]=counts.get(state,0)+1
 bad=state not in ('wood_first_hit','leaf_exact_rgba','wood_boundary_missing')
 color=(50,205,100,255) if state in ('wood_first_hit','leaf_exact_rgba') else (245,180,35,255) if state=='wood_boundary_missing' else (240,50,100,255)
 picture[y,x]=color
 if bad or state=='wood_boundary_missing':records.append(dict(pixel=[int(x),int(y)],state=state,wood_distance=wd if math.isfinite(wd) else None,leaf_distance=ld if math.isfinite(ld) else None))
report=dict(status='MEASURED_PENDING_REVIEW',model_sha256=model_sha,counts=counts,exceptions=records,
 method='Reopened saved model. All wood and crown/interior BVHs; packed material RGBA sampled through actual barycentric UVs, with side and physical alpha semantics.',
 limits=['Wood raw source colors remain in the prior protected atlas; this pass checks its first-hit receiver and mask reduction.','Boundary center-ray misses are reported separately from full3x3 interior misses.','Neighbor source context is excluded from owned-asset coverage; its native RGBA conservation is separately measured.','Native animation integration and the required known-source atlas rebake remain pending.'])
errors=sum(value for key,value in counts.items() if key not in ('wood_first_hit','leaf_exact_rgba','wood_boundary_missing'))
report['unexpected_errors']=errors;report['status']='OWNED_SOURCE_INTERIOR_PASS_WITH_BOUNDARY_DISCLOSURE' if not errors else 'SOURCE_GEOMETRY_HOLD'
(dest/'report.json').write_text(json.dumps(report,indent=2)+'\n')
Image.fromarray(picture).crop((645,39,985,442)).resize((1020,1209),Image.Resampling.NEAREST).save(dest/'source-first-hit.png')
print('Joint first-hit:',counts,flush=True)
# Keep physical alpha and culling while replacing color only. Workbench solids
# would turn textured foliage cutouts into opaque rectangular sheets.
for obj in [wood]+crowns:
 for slot in obj.material_slots:
  mat=slot.material.copy();slot.material=mat
  if obj is wood:
   nodes=mat.node_tree.nodes;nodes.clear();shader=nodes.new('ShaderNodeBsdfPrincipled');output=nodes.new('ShaderNodeOutputMaterial');mat.node_tree.links.new(shader.outputs[0],output.inputs[0])
   shader.inputs['Base Color'].default_value=(.42,.42,.42,1);shader.inputs['Roughness'].default_value=1
  else:
   for node in mat.node_tree.nodes:
    if node.type=='BSDF_PRINCIPLED':
     for name in ('Base Color','Emission Color'):
      if name in node.inputs:
       for link in list(node.inputs[name].links):mat.node_tree.links.remove(link)
       node.inputs[name].default_value=(.42,.42,.42,1)
     if 'Emission Strength' in node.inputs:node.inputs['Emission Strength'].default_value=0
light=bpy.data.lights.new('Neutral source-role geometry sun','SUN');light.energy=2
sun=bpy.data.objects.new(light.name,light);scene.collection.objects.link(sun);sun.rotation_euler=Vector((.6,.4,-.7)).to_track_quat('-Z','Y').to_euler()
scene.cycles.samples=8
render(OUT/'views.json',dest/'physical-solids',modes=('textured',),width=256)
images=[]
for i in range(8):
 path=dest/f'physical-solids/view-{i}-textured.png';rgba=Image.open(path).convert('RGBA');a=np.asarray(rgba);assert a[:,:,:3].max()>20 and (a[:,:,3]>0).any();images.append(rgba.convert('RGB'))
w,h=images[0].size;sheet=Image.new('RGB',(w*4,h*2))
for i,img in enumerate(images):sheet.paste(img,((i%4)*w,(i//4)*h))
sheet.save(dest/'physical-solid-eight.png')
assert sha(model)==model_sha
print('Saved physical alpha-aware solids; errors',errors,flush=True)
