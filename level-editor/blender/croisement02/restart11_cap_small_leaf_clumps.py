"""Test closed native-silhouette cut boundaries on the irregular flat prototype."""
from pathlib import Path
import sys,json,hashlib,math
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from refinement_review import _tree
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 parent=OUT/'restart11-hiding-mound/small-clumps-v2';out=OUT/'restart11-hiding-mound/closed-small-clumps-v1';out.mkdir(exist_ok=False);r=json.loads((parent/'validation.json').read_text());assert sha(parent/'model.blend')==r['model_sha256'];source=json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());rgba=np.array(Image.open(source['source']).convert('RGBA'));h,w=rgba.shape[:2];mask=rgba[:,:,3]>0;bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];verts=[];faces=[];indices={}
 def vertex(x,y,side):
  key=(x,y,side)
  if key not in indices:
   indices[key]=len(verts);p=Vector((x-w/2,-(y-h/2)/SIN,0))+RAY*(100 if side else-100);verts.append(tuple(p))
  return indices[key]
 for y,x in np.argwhere(mask):
  x=int(x);y=int(y);corners=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)];faces.append(tuple(vertex(a,b,1)for a,b in corners));faces.append(tuple(vertex(a,b,0)for a,b in reversed(corners)))
  for i,(nx,ny)in enumerate([(x,y-1),(x+1,y),(x,y+1),(x-1,y)]):
   if 0<=nx<w and 0<=ny<h and mask[ny,nx]:continue
   a,b=corners[i],corners[(i+1)%4];faces.append((vertex(*a,0),vertex(*b,0),vertex(*b,1),vertex(*a,1)))
 mesh=bpy.data.meshes.new('Native silhouette cutting volume');mesh.from_pydata(verts,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();cutter=bpy.data.objects.new('Private silhouette cutter',mesh);scene.collection.objects.link(cutter);records=[]
 for obj in objects:
  bpy.context.view_layer.objects.active=obj;obj.select_set(True);modifier=obj.modifiers.new('Closed silhouette boundary','BOOLEAN');modifier.operation='INTERSECT';modifier.solver='EXACT';modifier.object=cutter;modifier.use_self=True;modifier.use_hole_tolerant=True;bpy.ops.object.modifier_apply(modifier=modifier.name);obj.select_set(False);mesh=obj.data;bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));volume=bm.calc_volume(signed=True);nonmanifold=sum(not e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free()
  for mat in mesh.materials:
   shader=next(n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED')
   for link in list(shader.inputs['Alpha'].links):mat.node_tree.links.remove(link)
   shader.inputs['Alpha'].default_value=1;mat['opacity_semantics']='closed-solid'
  uv=mesh.uv_layers.active
  for poly in mesh.polygons:
   poly.material_index=0 if poly.normal.dot(RAY)>.05 else 1
   for li in poly.loop_indices:p=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((p.x+w/2)/w,1-(-p.y*SIN-p.z*COS+h/2)/h)
  records.append(dict(object=obj.name,vertices=len(mesh.vertices),faces=len(mesh.polygons),volume=volume,nonmanifold_edges=nonmanifold))
 bpy.data.objects.remove(cutter,do_unlink=True);bpy.context.view_layer.update();tree,owners,_=_tree(objects);missing=[];foreign=[]
 for y in range(h):
  for x in range(w):
   p,_,_,_=tree.ray_cast(Vector((x+.5-w/2,-(y+.5-h/2)/SIN,0))+RAY*500,-RAY)
   if mask[y,x]and p is None:missing.append([x,y])
   if not mask[y,x]and p is not None:foreign.append([x,y])
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
