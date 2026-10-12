"""Add privately inferred room floor/returns and source-scoped revealed appearance."""
import json,math,hashlib,sys,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/active/york/restart2';OUT=W/'restart61-room-interior-v18';assert not OUT.exists();assert shutil.disk_usage(ROOT).free>10*1024**3;sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
import shutil
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
acquire(slots=2)
import bpy,numpy as np
from PIL import Image
from collections import deque
from mathutils import Vector
bpy.ops.wm.open_mainfile(filepath=str(W/'restart61-room-split-v14/model.blend'));scene=bpy.context.scene;s,c=math.sin(math.radians(35)),math.cos(math.radians(35));OUT.mkdir()
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
# A connected footprint with an oblique rear wall parallel to castle context.
foot=[(2373,-1894),(2434,-1894),(2434,-1847),(2415,-1815),(2382,-1834.8),(2382,-1847),(2373,-1847),(2373,-1863),(2365,-1863),(2365,-1865),(2373,-1865)]
def roof(x):return max(235,min(272,235+(x-2373)*37/15))
def prism(name,poly,low,high):
 n=len(poly);v=[(x,y,(roof(x)+(z-272) if z>=272 else z)) for z in (low,high) for x,y in poly];faces=[tuple(range(n-1,-1,-1)),tuple(range(n,2*n))]+[(i,(i+1)%n,(i+1)%n+n,i+n)for i in range(n)];mesh=bpy.data.meshes.new(name);mesh.from_pydata(v,[],faces);mesh.update();o=bpy.data.objects.new('Room inferred '+name,mesh);scene.collection.objects.link(o);o.data.materials.append(unknown);o['source_node']='scenery-york-room-interior';o['scenery']=True;o['native_patch']='patch-005';o['room_receiver']=True;new.append(o);return o
prism('Floor',foot,107.87095,109.87095)
def clipx(poly,x,less):
 out=[]
 for a,b in zip(poly,poly[1:]+poly[:1]):
  ia=(a[0]<=x) if less else (a[0]>=x);ib=(b[0]<=x)if less else(b[0]>=x)
  if ia:out.append(a)
  if ia!=ib:t=(x-a[0])/(b[0]-a[0]);out.append((x,a[1]+t*(b[1]-a[1])))
 return out
for k,(lo,hi)in enumerate([(2300,2373),(2373,2388),(2388,2500)]):
 poly=clipx(clipx(foot,lo,False),hi,True)
 if len(poly)>2:prism('Ceiling '+str(k),poly,272,274)
fronts=[]
for i,(a,b) in enumerate(zip(foot,foot[1:]+foot[:1])):
 points=[a]+[(x,a[1]+(x-a[0])/(b[0]-a[0])*(b[1]-a[1]))for x in sorted([x for x in [2373,2388]if min(a[0],b[0])<x<max(a[0],b[0])],reverse=b[0]<a[0])]+[b]
 for j,(a,b)in enumerate(zip(points,points[1:])):
  dx,dy=b[0]-a[0],b[1]-a[1];length=math.hypot(dx,dy);nx,ny=2*dy/length,-2*dx/length
  o=prism('Wall '+str(i)+'-'+str(j),[a,(a[0]+nx,a[1]+ny),(b[0]+nx,b[1]+ny),b],109.87095,274)
  if i==0:fronts.append(o)
# The source-facing entrance cuts only the front wall; the rear stays closed.
# Keep all original geometry elsewhere in this candidate.
def split_plain(poly,n,d):
 inside=[];outside=[]
 for a,b in zip(poly,poly[1:]+poly[:1]):
  da=a.dot(n)-d;db=b.dot(n)-d;ia=da<=1e-7;ib=db<=1e-7;(inside if ia else outside).append(a)
  if ia!=ib:p=a.lerp(b,da/(da-db));inside.append(p);outside.append(p)
 return inside,outside
for front in fronts:
 front.data.calc_loop_triangles();polys=[[front.data.vertices[i].co.copy()for i in t.vertices]for t in front.data.loop_triangles]
 for xx in sorted({x for y,x in seen}):
  ys=[y for y,x in seen if x==xx];planes=[(Vector((1,0,0)),2354+xx),(Vector((-1,0,0)),-2353-xx),(Vector((0,-s,-c)),784+max(ys)),(Vector((0,s,c)),-783-min(ys))];remaining=[]
  for poly in polys:
   current=poly
   for n,d in planes:
    current,outside=split_plain(current,n,d)
    if len(outside)>=3:remaining.append(outside)
    if len(current)<3:break
  polys=remaining
 # Close the cut thickness with ruled reveals along the exact pixel boundary.
 aperture_edges=[]
 for y,x in seen:
  for neighbor,a,b in [((y-1,x),(2353+x,783+y),(2354+x,783+y)),((y+1,x),(2354+x,784+y),(2353+x,784+y)),((y,x-1),(2353+x,784+y),(2353+x,783+y)),((y,x+1),(2354+x,783+y),(2354+x,784+y))]:
   if neighbor not in seen:aperture_edges.append((a,b))
 vv=[v.co for v in front.data.vertices];xmin=min(v.x for v in vv);xmax=max(v.x for v in vv);ymin=min(v.y for v in vv);ymax=max(v.y for v in vv);slope=(roof(xmax)-roof(xmin))/(xmax-xmin);intercept=roof(xmin)+2-slope*xmin
 for a,b in aperture_edges:
  poly=[Vector((x,y,(-y*s-py)/c))for (x,py),y in [(a,ymin),(b,ymin),(b,ymax),(a,ymax)]]
  for n,d in [(Vector((-1,0,0)),-xmin),(Vector((1,0,0)),xmax),(Vector((0,0,-1)),-109.87095),(Vector((-slope,0,1)),intercept)]:
   poly,_=split_plain(poly,n,d)
   if len(poly)<3:break
  if len(poly)>=3:polys.append(poly)
 verts=[];faces=[]
 for p in polys:faces.append(tuple(range(len(verts),len(verts)+len(p))));verts.extend(p)
 m=bpy.data.meshes.new('Front entrance shell');m.from_pydata(verts,[],faces);m.materials.append(unknown);m.update();front.data=m
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
