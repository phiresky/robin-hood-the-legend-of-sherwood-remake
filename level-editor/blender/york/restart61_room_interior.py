"""Add privately inferred room floor/returns and source-scoped revealed appearance."""
import json,math,hashlib,sys,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';OUT=W/'restart61-room-interior-v12';assert not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3;sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy,numpy as np
from PIL import Image
from collections import deque
from mathutils import Vector
bpy.ops.wm.open_mainfile(filepath=str(W/'restart61-room-split-v6/model.blend'));scene=bpy.context.scene;s,c=math.sin(math.radians(35)),math.cos(math.radians(35));OUT.mkdir()
img=Image.open(W.parent/'source-states-complete/revealed.png').convert('RGBA').crop((2353,783,2467,1080));mask=np.array(Image.open(W.parent/'baseline/masks/000615.png').convert('L'))>0;q=deque([(150,40)]);seen=set(q)
while q:
 y,x=q.popleft()
 for yy,xx in [(y+1,x),(y-1,x),(y,x+1),(y,x-1)]:
  if 0<=yy<297 and 0<=xx<114 and not mask[yy,xx] and (yy,xx)not in seen:seen.add((yy,xx));q.append((yy,xx))
a=np.array(img);a[:,:,3]=0
for y,x in seen:a[y,x,3]=255
Image.fromarray(a).save(OUT/'revealed-source-domain.png');sourceimage=bpy.data.images.load(str(OUT/'revealed-source-domain.png'));sourceimage.pack()
unknown=bpy.data.materials.new('Unknown room interior');unknown.use_nodes=True;unknown.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.3,.3,.3,1)
# Extents leave space around the approved drum and its upper guide.
new=[]
for name,lo,hi in [('Left inner return',(2365,-1865,109.87095),(2381,-1863,235)),('Floor',(2373,-1896,107.87095),(2434,-1820,109.87095)),('Rear left wall',(2365,-1842,109.87095),(2381,-1840,200)),('Rear corner return',(2381,-1842,109.87095),(2383,-1818,200)),('Rear right wall',(2381,-1820,109.87095),(2434,-1818,200)),('Left wall',(2371,-1880,109.87095),(2373,-1842,200)),('Right wall',(2434,-1880,109.87095),(2436,-1818,200))]:
 bpy.ops.mesh.primitive_cube_add(size=1);o=bpy.context.object;o.name='Room inferred '+name;o.location=tuple((a+b)/2 for a,b in zip(lo,hi));o.dimensions=tuple(b-a for a,b in zip(lo,hi));bpy.ops.object.transform_apply(location=False,rotation=False,scale=True);o.data.materials.append(unknown);o['source_node']='scenery-york-room-interior';o['scenery']=True;o['native_patch']='patch-005';o['room_receiver']=True;new.append(o)
receivers=new+[o for o in scene.objects if o.type=='MESH' and o.get('source_node')in {'building-764','building-765','building-776','building-779'}]
for o in receivers:
 uv=o.data.uv_layers.new(name='Room revealed projection')
 for face in o.data.polygons:
  for li in face.loop_indices:
   p=o.matrix_world@o.data.vertices[o.data.loops[li].vertex_index].co;uv.data[li].uv=((p.x-2353)/114,1-(-p.y*s-p.z*c-783)/297)
 originals=list(o.data.materials)
 for idx,original in enumerate(originals):
  m=original.copy();m.name=original.name+' / room-domain';m.use_nodes=True;nt=m.node_tree;output=next(n for n in nt.nodes if n.type=='OUTPUT_MATERIAL');surface=output.inputs['Surface'];old=surface.links[0].from_socket if surface.links else None
  assert old is not None
  mix=nt.nodes.new('ShaderNodeMixShader');em=nt.nodes.new('ShaderNodeEmission');tex=nt.nodes.new('ShaderNodeTexImage');tex.image=sourceimage;tex.extension='CLIP';tex.interpolation='Closest';coords=nt.nodes.new('ShaderNodeUVMap');coords.uv_map=uv.name;nt.links.new(coords.outputs['UV'],tex.inputs['Vector']);nt.links.new(tex.outputs['Color'],em.inputs['Color']);nt.links.new(tex.outputs['Alpha'],mix.inputs[0]);nt.links.new(old,mix.inputs[1]);nt.links.new(em.outputs[0],mix.inputs[2]);nt.links.new(mix.outputs[0],surface);o.data.materials[idx]=m
 for original in originals:o.data.materials.append(original)
 for face in o.data.polygons:
  normal=(o.matrix_world.to_3x3().inverted().transposed()@face.normal).normalized()
  if normal.dot(Vector((0,-c,s)))<=.05:face.material_index+=len(originals)
bpy.context.view_layer.update();bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True);(OUT/'report.json').write_text(json.dumps({'status':'PRIVATE_CAVITY_CANDIDATE_UNAPPROVED','model_sha256':hashlib.sha256((OUT/'model.blend').read_bytes()).hexdigest(),'source_domain_pixels':len(seen),'source_path':str(W.parent/'source-states-complete/revealed.png'),'source_sha256':hashlib.sha256((W.parent/'source-states-complete/revealed.png').read_bytes()).hexdigest(),'inference':'Room floor at90game elevation; seven solid receivers. Rear wall folds around the approved gate horizontal envelope at X2381/Y-1840; exact bounds are recorded in the contact audit. Wall depth and returns are inferred, not source-measured.','source_appearance':'Original revealed RGB scoped to mask615 aperture only. No image synthesis. Hidden surfaces unknown gray.','approval':'Geometry pending; no texture-fill approval requested.'},indent=2)+'\n')
