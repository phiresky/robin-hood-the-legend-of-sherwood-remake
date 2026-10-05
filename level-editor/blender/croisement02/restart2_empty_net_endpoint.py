"""Build the empty net endpoint on the separately reviewed attachment assembly."""
import sys,json,math
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw,ImageFilter
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import RAY,SIN,COS
from scenery_geometry import Mesh
from log_trap_state_candidate import material
from render_slots import acquire,release
DEST=OUT/'restart2-state/net-empty01-v7'
def volume(obj):
 bm=bmesh.new();bm.from_mesh(obj.data);closed=all(e.is_manifold for e in bm.edges);value=bm.calc_volume(signed=True);bm.free()
 if not closed or value<=0:raise ValueError('Nonclosed or nonpositive '+obj.name)
 return value
def main():
 if DEST.exists():raise FileExistsError(DEST)
 base=OUT/'restart2-state/net-attached-v1';meta=json.loads((base/'manifest.json').read_text())
 if sha(base/'worker.blend')!=meta['model_sha256']:raise ValueError('Reviewed base changed')
 fit=json.loads((OUT/'net-endpoint-volume-fit-v2/manifest.json').read_text());row=next(r for r in fit['records']if r['family']=='piege01'and r['variant']=='e');source=Path(row['source'])
 if sha(source)!=row['source_sha256']:raise ValueError('Source changed')
 acquire()
 try:
  DEST.mkdir();bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene
  cord=bpy.data.objects['Bag hanging cord'];highest=max(v.co.z for v in cord.data.vertices);cap=[v.co.copy() for v in cord.data.vertices if v.co.z>highest-.8];upper=sum(cap,Vector())/len(cap)
  for name in ['Occupied bag','Bag hanging cord']:bpy.data.objects.remove(bpy.data.objects[name],do_unlink=True)
  wood=bpy.data.objects['Wooden piece'];center=sum((v.co for v in wood.data.vertices),Vector())/len(wood.data.vertices)
  for v in wood.data.vertices:v.co=center+(v.co-center)*1.15
  x,y,w,h=row['bbox'];cx,bottom,height,tilt,*radii=row['parameters'];lift=(1107-y-bottom)/COS;vertices=[];faces=[];n=48
  for t,r in zip(row['ring_fractions'],[.15,*[r*1.06 for r in radii],.15]):
   for j in range(n):
    a=j*2*math.pi/n;vertices.append(Vector((x+cx+tilt*(t-.5)+r*math.cos(a),-(y+bottom+lift*COS)/SIN+row['depth_to_horizontal_radius']*r*math.sin(a),lift+t*height))+RAY*54)
  for k in range(len(row['ring_fractions'])-1):
   for j in range(n):a=k*n+j;b=k*n+(j+1)%n;faces.append((a,b,b+n,a+n))
  faces.extend([tuple(range(n-1,-1,-1)),tuple((len(row['ring_fractions'])-1)*n+j for j in range(n))])
  def make(name,vs,fs):
   mesh=bpy.data.meshes.new(name);mesh.from_pydata(vs,[],fs);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();o=bpy.data.objects.new(name,mesh);scene.collection.objects.link(o);return o
  bag=make('Empty bag',vertices,faces);before=volume(bag);bpy.context.view_layer.objects.active=bag;mod=bag.modifiers.new('Counterweight contact','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';cutter=bpy.data.objects['Wooden piece'].copy();cutter.data=cutter.data.copy();scene.collection.objects.link(cutter);c=sum((v.co for v in cutter.data.vertices),Vector())/len(cutter.data.vertices)
  for v in cutter.data.vertices:v.co=c+(v.co-c)*1.001
  mod.object=cutter;bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(cutter,do_unlink=True);after=volume(bag)
  if after<before*.9:raise ValueError('Excess cloth carving')
  raw=np.array(Image.open(source).convert('RGBA'));mask=Image.new('L',(w,h));ImageDraw.Draw(mask).polygon(row['outline_crop_coordinates'],fill=255);mask=mask.filter(ImageFilter.MaxFilter(5));raw[:,:,3]=np.minimum(raw[:,:,3],np.array(mask));image=DEST/'bag-observed.png';Image.fromarray(raw).save(image);bag.data.materials.clear();bag.data.materials.append(material(image));gray=bpy.data.materials['Unobserved net surfaces'];bag.data.materials.append(gray);uv=bag.data.uv_layers.get('Native target projection') or bag.data.uv_layers.new(name='Native target projection')
  for face in bag.data.polygons:
   face.material_index=0 if face.normal.dot(RAY)>.05 else 1
   for loop in face.loop_indices:
    p=bag.data.vertices[bag.data.loops[loop].vertex_index].co;uv.data[loop].uv=((p.x-x)/w,1-(-p.y*SIN-p.z*COS-y)/h)
  # Keep the solid timber, but bind its observed color to this exact empty phase.
  for name in ['Wooden piece','Wood hanging cord']:
   obj=bpy.data.objects[name];oldmat=obj.data.materials[0];oldimage=next(n.image for n in oldmat.node_tree.nodes if n.type=='TEX_IMAGE');oldpixels=np.array(oldimage.pixels[:]).reshape(oldimage.size[1],oldimage.size[0],4)[::-1];pixels=np.array(Image.open(source).convert('RGBA'));known=np.zeros((h,w),bool);hh=min(h,oldpixels.shape[0]);ww=min(w,oldpixels.shape[1]);known[:hh,:ww]=oldpixels[:hh,:ww,3]>.5;pixels[:,:,3]=np.where(known,pixels[:,:,3],0);path=DEST/(name.replace(' ','-')+'-observed.png');Image.fromarray(pixels).save(path);obj.data.materials[0]=material(path)
   uv=obj.data.uv_layers.get('Native target projection')
   if uv is None:raise ValueError('Missing source UV for '+name)
   for loop in obj.data.loops:
    p=obj.matrix_world@obj.data.vertices[loop.vertex_index].co;uv.data[loop.index].uv=((p.x-x)/w,1-(-p.y*SIN-p.z*COS-y)/h)
  top=sum(vertices[-n:],Vector())/n;mesh=Mesh();mesh.tube(top-Vector((0,0,.15)),upper+Vector((0,0,.15)),.85,n=8);cord=make('Empty bag hanging cord',mesh.vertices,mesh.faces);cord.data.materials.append(gray)
  objects=[o for o in scene.objects if o.type=='MESH'];audit=[{'name':o.name,'closed_volume':volume(o)}for o in objects];points=[o.matrix_world@v.co for o in objects for v in o.data.vertices];lo=Vector(tuple(min(p[i]for p in points)for i in range(3)));hi=Vector(tuple(max(p[i]for p in points)for i in range(3)));center=(lo+hi)/2;scene.camera.data.ortho_scale=(hi-lo).length*1.2
  scene.camera.location=center+RAY*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler();bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'model.blend'));renders=[]
  for view in range(0 if '--no-render' in sys.argv else 8):
   a=-math.pi/2+view*math.pi/4;direction=Vector((math.cos(a)*COS,math.sin(a)*COS,SIN));scene.camera.location=center+direction*3000;scene.camera.rotation_euler=(center-scene.camera.location).to_track_quat('-Z','Y').to_euler()
   for mode in ['actual','solid']:
    scene.view_layers[0].material_override=gray if mode=='solid' else None;path=DEST/f'{view:02}-{mode}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);renders.append({'view':view,'mode':mode,'path':path.name,'sha256':sha(path)})
  write_json(DEST/'report.json',{'status':'Private empty endpoint; source and joint review pending','model_sha256':sha(DEST/'model.blend'),'base_sha256':meta['model_sha256'],'source_sha256':row['source_sha256'],'objects':audit,'contact_removed_fraction':(before-after)/before,'bag_top':list(top),'cord_upper':list(upper),'renders':renders,'limits':['Gray unknown back and hanging cord appearance unfilled.','Only empty piege01 final phase0.','No inferred actor geometry or temporal body identity.','Remaining joint/source first-hit review required.']})
 finally:release()
if __name__=='__main__':main()
