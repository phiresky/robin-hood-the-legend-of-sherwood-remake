"""Separate touching leaf shells and taper cut boundaries along native rays."""
from pathlib import Path
import sys,json,hashlib,math
from collections import defaultdict
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from refinement_review import _tree
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def split_fans(obj):
 mesh=obj.data;edges=defaultdict(list);incident=defaultdict(list)
 for p in mesh.polygons:
  vs=list(p.vertices)
  for v in vs:incident[v].append(p.index)
  for a,b in zip(vs,vs[1:]+vs[:1]):edges[tuple(sorted((a,b)))].append(p.index)
 fan={};counter=0
 for vertex,faces in incident.items():
  neighbors={f:set()for f in faces}
  for edge,shared in edges.items():
   if vertex in edge and len(shared)==2:
    a,b=shared;neighbors[a].add(b);neighbors[b].add(a)
  todo=set(faces)
  while todo:
   pending=[todo.pop()]
   while pending:
    f=pending.pop();fan[(vertex,f)]=counter
    for n in neighbors[f]&todo:todo.remove(n);pending.append(n)
   counter+=1
 verts=[];polys=[];lookup={};material=[]
 for p in mesh.polygons:
  poly=[]
  for v in p.vertices:
   key=(v,fan[(v,p.index)])
   if key not in lookup:lookup[key]=len(verts);verts.append(tuple(mesh.vertices[v].co))
   poly.append(lookup[key])
  polys.append(poly);material.append(p.material_index)
 new=bpy.data.meshes.new(mesh.name+' separated shells');new.from_pydata(verts,[],polys);new.update()
 for mat in mesh.materials:new.materials.append(mat)
 for p,i in zip(new.polygons,material):p.material_index=i
 obj.data=new;return len(verts)-len(mesh.vertices)
def main():
 parent=OUT/'restart11-hiding-mound/closed-small-clumps-v4';out=OUT/'restart11-hiding-mound/closed-small-clumps-v6';out.mkdir(exist_ok=False);r=json.loads((parent/'validation.json').read_text());assert sha(parent/'model.blend')==r['model_sha256'];source=json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());rgba=np.array(Image.open(source['source']).convert('RGBA'));h,w=rgba.shape[:2];mask=rgba[:,:,3]>0;bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];records=[]
 for obj in objects:
  duplicates=split_fans(obj);mesh=obj.data;tri=bmesh.new();tri.from_mesh(mesh);bmesh.ops.triangulate(tri,faces=list(tri.faces),quad_method='FIXED',ngon_method='EAR_CLIP');tri.to_mesh(mesh);tri.free();mesh.update();boundary=set(v for p in mesh.polygons if abs(p.normal.dot(RAY))<.0001 for v in p.vertices);groups=defaultdict(list)
  for index in boundary:
   p=mesh.vertices[index].co;groups[(round(p.x,4),round(-p.y*SIN-p.z*COS,4))].append(index)
  maximum=0.
  for ids in groups.values():
   depths=[mesh.vertices[i].co.dot(RAY)for i in ids];lo=min(depths);hi=max(depths)
   if hi-lo<.1:continue
   center=(lo+hi)*.5
   for index,d in zip(ids,depths):
    delta=(center+(d-center)*.18)-d;mesh.vertices[index].co+=RAY*delta;maximum=max(maximum,abs(delta))
  mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));volume=bm.calc_volume(signed=True);nonmanifold=sum(not e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free();uv=mesh.uv_layers.new(name='Native source projection')
  for p in mesh.polygons:
   p.material_index=0 if p.normal.dot(RAY)>.00001 else 1;p.use_smooth=p.material_index==0
   for li in p.loop_indices:q=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((q.x+w/2)/w,1-(-q.y*SIN-q.z*COS+h/2)/h)
  records.append(dict(object=obj.name,volume=volume,nonmanifold_edges=nonmanifold,duplicated_touch_vertices=duplicates,maximum_boundary_ray_shift=maximum,minimum_z=min(v.co.z for v in mesh.vertices)))
 bpy.context.view_layer.update();tree,owners,_=_tree(objects);assigned={o:np.zeros((h,w),bool)for o in objects};missing=[];foreign=[]
 for y in range(h):
  for x in range(w):
   p,_,_,_=tree.ray_cast(Vector((x+.5-w/2,-(y+.5-h/2)/SIN,0))+RAY*500,-RAY)
   if mask[y,x]and p is None:missing.append([x,y])
   if not mask[y,x]and p is not None:foreign.append([x,y])
 for y,x in np.argwhere(mask):
  for dx,dy in [(a,b)for a in [.01,.25,.5,.75,.99]for b in [.01,.25,.5,.75,.99]]:
   p,_,ti,_=tree.ray_cast(Vector((x+dx-w/2,-(y+dy-h/2)/SIN,0))+RAY*500,-RAY)
   if p is not None:assigned[owners[ti]][y,x]=True
 for obj in objects:
  pixels=rgba.copy();pixels[:,:,:3]=137;pixels[assigned[obj]]=rgba[assigned[obj]];file=out/(obj.name.replace(' ','-')+'-observed.png');Image.fromarray(pixels).save(file);image=bpy.data.images.load(str(file));image.pack()
  for mat in obj.data.materials:
   for node in mat.node_tree.nodes:
    if node.type=='TEX_IMAGE':node.image=image
 model=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));solid=bpy.data.materials.new('Lit closed-solid diagnostic');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.65,.65,.65,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.85;light=bpy.data.objects.new('Review light',bpy.data.lights.new('Review light','AREA'));scene.collection.objects.link(light);light.location=(30,-55,85);light.rotation_euler=(-light.location).to_track_quat('-Z','Y').to_euler();light.data.energy=6000;light.data.size=35
 for mode in ['actual','solid']:
  scene.view_layers[0].material_override=solid if mode=='solid'else None;paths=[]
  for i in range(8):
   angle=i*math.pi/4;frame(scene,objects,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)),384,1.15);file=out/f'{mode}-{i:02}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);paths.append(file)
  sheet(paths,out/f'{mode}-eight.png')
 (out/'validation.json').write_text(json.dumps(dict(status='PRIVATE_CLOSED_BOUNDARY_TEST',model_sha256=sha(model),parent_sha256=sha(parent/'model.blend'),records=records,missing_native_centers=missing,foreign_native_centers=foreign,scope='Explicit silhouette-cut surfaces close formerly alpha-clipped envelope ends. Gray solid equals actual physical coverage, with no alpha discard. Exact source first-hit and topology must be reviewed before any support variant.'),indent=2)+'\n')
 print(json.dumps(dict(nonmanifold=sum(x['nonmanifold_edges']for x in records),missing=missing,foreign=foreign)))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
