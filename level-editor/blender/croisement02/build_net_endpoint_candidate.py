"""Private complete occupied-net endpoint with separate wood and hanging cords."""
import sys,json,math
from pathlib import Path
import bpy,bmesh
import numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw,ImageFilter
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT
from tree_geometry import RAY,SIN,COS
from scenery_geometry import Mesh
from log_trap_state_candidate import material,point,sha
from render_slots import acquire,release


def main():
 separate='--separate-depth' in sys.argv
 fit=OUT/'net-endpoint-volume-fit-v2/manifest.json';report=json.loads(fit.read_text());row=next(r for r in report['records']if r['family']=='piege01'and r['variant']=='i');dest=OUT/('net-endpoint-candidate-v3'if separate else 'net-endpoint-candidate-v2');dest.mkdir(exist_ok=True);assert not(dest/'worker.blend').exists();source=Path(row['source']);assert sha(source)==row['source_sha256'];raw=np.array(Image.open(source).convert('RGBA'));x,y,w,h=row['bbox'];cx,bottom,height,tilt,*radii=row['parameters'];ground_screen_y=1107.;lift=(ground_screen_y-y-bottom)/COS;assert lift>0
 acquire()
 try:
  bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.name='Private net endpoint';scene.render.engine='CYCLES';scene.cycles.samples=16;scene.render.resolution_x=scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.15,.15,.15,1)
  light=bpy.data.objects.new('Shared review sun',bpy.data.lights.new('Shared review sun','SUN'));scene.collection.objects.link(light);light.data.energy=2;light.rotation_euler=(-Vector((-.45,-.55,.70))).to_track_quat('-Z','Y').to_euler();gray=bpy.data.materials.new('Unobserved net surfaces');gray.use_nodes=True;gray.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.17,.17,.17,1);objects=[];audits=[]
  def make(name,vertices,faces,domain):
   mask=Image.new('L',(w,h));ImageDraw.Draw(mask).polygon(domain,fill=255);mask=mask.filter(ImageFilter.MaxFilter(5)) if name=='Occupied bag' else mask;pixels=raw.copy();pixels[:,:,3]=np.minimum(pixels[:,:,3],np.array(mask));path=dest/(name.replace(' ','-')+'-observed.png');Image.fromarray(pixels).save(path);mat=material(path);mesh=bpy.data.meshes.new(name);mesh.from_pydata(vertices,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume(signed=True);assert volume>0;bm.to_mesh(mesh);bm.free();mesh.update();obj=bpy.data.objects.new(name,mesh);scene.collection.objects.link(obj);mesh.materials.append(mat);mesh.materials.append(gray);uv=mesh.uv_layers.new(name='Native target projection')
   for face in mesh.polygons:
    face.material_index=0 if face.normal.dot(RAY)>.05 else 1
    for loop in face.loop_indices:
     p=mesh.vertices[mesh.loops[loop].vertex_index].co;uv.data[loop].uv=((p.x-x)/w,1-(-p.y*SIN-p.z*COS-y)/h)
   obj['asset_group']='croisement02-net-piege01';obj['state_variant']='occupied i / final phase0';obj['source_patch']=row['patch_id'];obj['geometry_status']='private hypothesis';objects.append(obj);audits.append(dict(object=name,closed=True,volume=volume,observed_image_sha256=sha(path),observed_pixels=int((pixels[:,:,3]>0).sum()),source_domain=domain,source_domain_dilation_pixels=2 if name=='Occupied bag' else 0));return obj
  vertices=[];faces=[];fractions=row['ring_fractions'];n=48
  for t,r in zip(fractions,[.15,*radii,.15]):
   for j in range(n):
    angle=j*2*math.pi/n;vertices.append((x+cx+tilt*(t-.5)+r*math.cos(angle),-(y+bottom+lift*COS)/SIN+row['depth_to_horizontal_radius']*r*math.sin(angle),lift+t*height))
  for k in range(len(fractions)-1):
   for j in range(n):a=k*n+j;b=k*n+(j+1)%n;faces.append((a,b,b+n,a+n))
  faces.extend([tuple(range(n-1,-1,-1)),tuple((len(fractions)-1)*n+j for j in range(n))]);make('Occupied bag',vertices,faces,row['outline_crop_coordinates'])
  zwood=lift+height*.43;wood=Mesh();wood.tube(point(x+2,y+39,zwood),point(x+21,y+35,zwood),7,n=16);make('Wooden piece',wood.vertices,wood.faces,[(0,29),(17,29),(20,34),(17,46),(9,49),(0,44)])
  top=Vector(vertices[-n]);top.x=x+cx+tilt*.5;top.y=-(y+bottom+lift*COS)/SIN;top.z=lift+height;top_screen=-top.y*SIN-top.z*COS
  cord=Mesh();cord.tube(top,point(x+33.5,y+.5,top.z+(top_screen-y-.5)/COS),.55,n=8);make('Bag hanging cord',cord.vertices,cord.faces,[(33,0),(37,0),(38,9),(33,9)])
  bottom_cord=point(x+20.5,y+32,zwood+3/COS);cord=Mesh();cord.tube(bottom_cord,bottom_cord+Vector((0,0,24/COS)),.55,n=8);make('Wood hanging cord',cord.vertices,cord.faces,[(18,7),(22,7),(22,33),(18,33)])
  depth=None
  if separate:
   from settle_initial_rock_pile import collision_interval
   bag=bpy.data.objects['Occupied bag'];wood_obj=bpy.data.objects['Wooden piece'];bpy.context.view_layer.update();interval=collision_interval(bag,wood_obj);assert interval and interval['first']<0<interval['last'];shift=interval['first']-.02;error=0.
   for obj in [wood_obj,bpy.data.objects['Wood hanging cord']]:
    for vertex in obj.data.vertices:
     old=vertex.co.copy();vertex.co+=RAY*shift;delta=vertex.co-old;error=max(error,abs(delta.x),abs(delta.y*SIN+delta.z*COS))
    obj.data.update()
   assert error<1e-4 and min(v.co.z for v in wood_obj.data.vertices)>0
   depth=dict(wood_and_cord_camera_ray_shift=shift,source_projection_max_error=error,reason='Infer wood behind bag to remove solid overlap and preserve the native projected positions.',remaining_interval=collision_interval(bag,wood_obj));assert depth['remaining_interval']['first']>0
  cam=bpy.data.objects.new('Review camera',bpy.data.cameras.new('Review camera'));scene.collection.objects.link(cam);scene.camera=cam;cam.data.type='ORTHO';cam.data.clip_end=20000;coords=[v.co for o in objects for v in o.data.vertices];lo=Vector(tuple(min(v[i]for v in coords)for i in range(3)));hi=Vector(tuple(max(v[i]for v in coords)for i in range(3)));center=(lo+hi)/2;cam.data.ortho_scale=(hi-lo).length*1.2
  cam.location=center+RAY*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler()
  bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'));renders=[]
  for view in range(8):
   angle=math.pi/2+view*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN));cam.location=center+direction*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler()
   for mode in ['actual','solid']:
    scene.view_layers[0].material_override=gray if mode=='solid'else None;image=dest/f'view-{view:02}-{mode}.png';scene.render.filepath=str(image);bpy.ops.render.render(write_still=True);renders.append(dict(view=view,mode=mode,image=image.name,sha256=sha(image)))
  result=dict(status='Private endpoint hypothesis; self-review and source ownership audit pending',model_sha256=sha(dest/'worker.blend'),source_sha256=sha(source),fit_manifest_sha256=sha(fit),source_patch=row['patch_id'],objects=audits,renders=renders,depth_inference=depth,ground_reference=dict(source_y=ground_screen_y,bag_bottom_z=lift,interpretation='Inferred ground under initial native leaf pile; terrain integration not yet validated.'),limitations=['Only occupied piege01 final phase0 is modeled.','Source traces have2px manual uncertainty; hidden bag volume and wood depth are inferred.','Cord attachment above the source patch and tree contact remain unverified.','Wood and bag source regions are conservative manual partitions; actual first-hit ownership requires audit.','No captured actor is modeled as permanent scenery.','Initial rigging, empty variant, all moving/final phases, leaf effect and mission export remain incomplete.']);(dest/'manifest.json').write_text(json.dumps(result,indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
