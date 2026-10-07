"""Private overlapping leaf-clump volumes with exact native-alpha appearance."""
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
 out=OUT/'restart11-hiding-mound/small-clumps-v1';out.mkdir(parents=True,exist_ok=False);src=json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());source=Path(src['source']);assert sha(source)==src['source_sha256'];rgba=np.array(Image.open(source).convert('RGBA'));h,w=rgba.shape[:2];bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene
 image=bpy.data.images.load(str(source));image.pack();mats=[]
 for native in [True,False]:
  mat=bpy.data.materials.new('Native observed front'if native else'Unobserved alpha-bounded back');mat.use_nodes=True;mat['foliage_physical_opacity']=True;mat['opacity_semantics']='physical-coverage';n=mat.node_tree.nodes;n.clear();tex=n.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';shader=n.new('ShaderNodeBsdfPrincipled');shader.inputs['Base Color'].default_value=(0,0,0,1);shader.inputs['Roughness'].default_value=1;shader.inputs['Specular IOR Level'].default_value=0;shader.inputs['Emission Color'].default_value=(.28,.28,.28,1);shader.inputs['Emission Strength'].default_value=1;output=n.new('ShaderNodeOutputMaterial');links=mat.node_tree.links
  if native:links.new(tex.outputs['Color'],shader.inputs['Emission Color'])
  links.new(tex.outputs['Alpha'],shader.inputs['Alpha']);links.new(shader.outputs[0],output.inputs['Surface']);mats.append(mat)
 clumps=[]
 for gy in range(0,h,4):
  for gx in range(0,w,4):
   yy,xx=np.nonzero(rgba[gy:gy+4,gx:gx+4,3]>0)
   if not len(xx):continue
   cx=float(gx+xx.mean()+.5);cy=float(gy+yy.mean()+.5);central=max(0,1-((cx-w*.5)/(w*.58))**2-((cy-h*.5)/(h*.65))**2);seed=len(clumps);height=2.2+3.4*central+.32*math.sin(seed*1.71);clumps.append((cx,cy,3.9+.35*math.sin(seed*1.31),3.9+.35*math.cos(seed*.79),height,.3*math.sin(seed)))

 objects=[];records=[]
 for index,(cx,cy,rx,ry,height,rotation)in enumerate(clumps):
  vertices=[];faces=[];rings=6;segments=20
  for ring in range(rings):
   theta=(ring/rings)*math.pi/2;r=math.cos(theta);z=height*math.sin(theta)
   for i in range(segments):
    angle=2*math.pi*i/segments;organic=1+.08*math.sin(angle*3+index*.91)+.05*math.cos(angle*5-index*.37);u=rx*r*math.cos(angle)*organic;v=ry*r*math.sin(angle)*organic;x=cx-w/2+u*math.cos(rotation)-v*math.sin(rotation);sy=cy-h/2+height*COS*.38+u*math.sin(rotation)+v*math.cos(rotation);vertices.append((x,-sy/SIN,z))
  tip=len(vertices);vertices.append((cx-w/2,-(cy-h/2+height*COS*.38)/SIN,height));bottom=len(vertices);vertices.append((cx-w/2,-(cy-h/2+height*COS*.38)/SIN,0))
  for ring in range(rings-1):
   for i in range(segments):j=(i+1)%segments;faces.append((ring*segments+i,ring*segments+j,(ring+1)*segments+j,(ring+1)*segments+i))
  for i in range(segments):j=(i+1)%segments;faces.append(((rings-1)*segments+i,(rings-1)*segments+j,tip));faces.append((j,i,bottom))
  mesh=bpy.data.meshes.new('Organic closed leaf clump');mesh.from_pydata(vertices,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));volume=bm.calc_volume(signed=True);closed=all(e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free();assert closed and volume>0
  for mat in mats:mesh.materials.append(mat)
  uv=mesh.uv_layers.new(name='Native source projection')
  for poly in mesh.polygons:
   poly.material_index=0 if poly.normal.dot(RAY)>.00001 else 1;poly.use_smooth=poly.center.z>0
   for li in poly.loop_indices:p=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((p.x+w/2)/w,1-(-p.y*SIN-p.z*COS+h/2)/h)
  obj=bpy.data.objects.new(f'Leaf clump {index:02}',mesh);scene.collection.objects.link(obj);objects.append(obj);records.append(dict(object=obj.name,closed=closed,volume=volume,height=height))
 bpy.context.view_layer.update();tree,owners,_=_tree(objects);missing=[];foreign=[]
 for y in range(h):
  for x in range(w):
   p,n,ti,d=tree.ray_cast(Vector((x+.5-w/2,-(y+.5-h/2)/SIN,0))+RAY*500,-RAY)
   if rgba[y,x,3]>0 and p is None:missing.append([x,y])
   if rgba[y,x,3]==0 and p is not None:foreign.append([x,y])
 assert not missing and not foreign,(missing,foreign)
 assigned={o:np.zeros((h,w),bool)for o in objects}
 for y,x in np.argwhere(rgba[:,:,3]>0):
  p,n,ti,d=tree.ray_cast(Vector((x+.5-w/2,-(y+.5-h/2)/SIN,0))+RAY*500,-RAY);assigned[owners[ti]][y,x]=True
 for obj in objects:
  pixels=rgba.copy();pixels[:,:,:3]=137;pixels[assigned[obj]]=rgba[assigned[obj]];file=out/(obj.name.replace(' ','-')+'-source.png');Image.fromarray(pixels).save(file);observed=bpy.data.images.load(str(file));observed.pack()
  for i,material in enumerate(list(obj.data.materials)):
   mat=material.copy()
   for node in mat.node_tree.nodes:
    if node.type=='TEX_IMAGE':node.image=observed
   obj.data.materials[i]=mat
  obj['observed_pixel_count']=int(assigned[obj].sum())
 # Coverage remains independent of the first-hit color assignment.
 assert np.array_equal(sum(assigned.values()),rgba[:,:,3]>0)
 scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';world=bpy.data.worlds.new('Neutral');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.8;scene.world=world
 model=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));solid=bpy.data.materials.new('Lit solid');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.65,.65,.65,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.85;light=bpy.data.objects.new('Review light',bpy.data.lights.new('Review light','AREA'));scene.collection.objects.link(light);light.location=(30,-55,85);light.rotation_euler=(-light.location).to_track_quat('-Z','Y').to_euler();light.data.energy=6000;light.data.size=35
 for mode in ['actual','solid','opacity-gray']:
  scene.view_layers[0].material_override=solid if mode=='solid'else None;paths=[]
  if mode=='opacity-gray':
   for obj in objects:
    for i,material in enumerate(list(obj.data.materials)):
     mat=material.copy();shader=next(n for n in mat.node_tree.nodes if n.type=='BSDF_PRINCIPLED')
     for socket in ['Base Color','Emission Color']:
      for link in list(shader.inputs[socket].links):mat.node_tree.links.remove(link)
     shader.inputs['Base Color'].default_value=(.65,.65,.65,1);shader.inputs['Emission Strength'].default_value=0;obj.data.materials[i]=mat
  for i in range(8):
   angle=i*math.pi/4;frame(scene,objects,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)),384,1.15);file=out/f'{mode}-{i:02}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);paths.append(file)
  sheet(paths,out/f'{mode}-eight.png')
 (out/'validation.json').write_text(json.dumps(dict(status='PRIVATE_CLUMP_PROTOTYPE',model_sha256=sha(model),source_sha256=sha(source),records=records,missing_native_centers=missing,foreign_native_centers=foreign,scope='Small irregular overlapping grounded clumps. Actual silhouette coverage is independent from first-hit texture ownership: hidden surfaces remain gray instead of disappearing. Native camera first. Flat prototype only, no propagation or approval.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
