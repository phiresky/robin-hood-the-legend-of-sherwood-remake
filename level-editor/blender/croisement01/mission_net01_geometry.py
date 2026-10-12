"""Build independently fitted C1 net endpoints with closed ropes and source projection."""
import sys,json,math,hashlib,time,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement')]
from render_slots import acquire,release
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image
OUT=ROOT/'level-editor/work/croisement01-refinement/mission-net01-oct12-v1';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN));UP=Vector((0,SIN,COS))
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def point(x,y,z):return Vector((x,-(y+z*COS)/SIN,z))
def guard():
 mem={a:int(b.split()[0])*1024 for a,b in (line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines())}
 return mem['MemAvailable']>=6*1024**3 and shutil.disk_usage(OUT).free>=10*1024**3

def main():
 acquire(slots=2)
 try:
  while not guard():print('Resource guard: awaiting 6 GiB available RAM and 10 GiB disk',flush=True);time.sleep(30)
  packet=json.loads((OUT/'source.json').read_text());dest=OUT/'candidate-v1';dest.mkdir(exist_ok=False)
  bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=384;scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.world=bpy.data.worlds.new('Neutral review world');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.25,.25,.25,1)
  gray=bpy.data.materials.new('Unknown hidden surfaces');gray.diffuse_color=(.32,.32,.32,1);gray.use_nodes=True;gray.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.32,.32,.32,1)
  mats={};sources=packet['sources']
  for state,s in sources.items():
   assert sha(s['path'])==s['sha256'];m=bpy.data.materials.new(state+' native source');m.use_nodes=True;n=m.node_tree.nodes;n.clear();o=n.new('ShaderNodeOutputMaterial');im=n.new('ShaderNodeTexImage');im.image=bpy.data.images.load(s['path']);im.image.pack();im.interpolation='Closest';em=n.new('ShaderNodeEmission');unknown=n.new('ShaderNodeBsdfPrincipled');unknown.inputs['Base Color'].default_value=(.32,.32,.32,1);mix=n.new('ShaderNodeMixShader');m.node_tree.links.new(im.outputs['Color'],em.inputs[0]);m.node_tree.links.new(im.outputs['Alpha'],mix.inputs[0]);m.node_tree.links.new(unknown.outputs[0],mix.inputs[1]);m.node_tree.links.new(em.outputs[0],mix.inputs[2]);m.node_tree.links.new(mix.outputs[0],o.inputs[0]);mats[state]=m
  objects={k:[] for k in ['initial','empty','occupied']};audits=[]
  def mesh(name,vs,fs,state,source):
   me=bpy.data.meshes.new(name);me.from_pydata(vs,[],fs);me.update();bm=bmesh.new();bm.from_mesh(me);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));volume=bm.calc_volume(signed=True);closed=all(e.is_manifold for e in bm.edges);assert closed and abs(volume)>1e-6,(name,closed,volume);bm.to_mesh(me);bm.free();me.update();ob=bpy.data.objects.new(name,me);scene.collection.objects.link(ob);ob['state']=state;ob['source_profile']=source;ob['source_node']='mission-croisement01-net01';ob['projection_min_cosine']=.05;me.materials.append(mats[source]);me.materials.append(gray);uv=me.uv_layers.new(name='Exact native projection');s=sources[source];left=s['position'][0]+s['offset'][0];top=s['position'][1]+s['offset'][1];w,h=s['size']
   for f in me.polygons:
    f.material_index=0 if f.normal.dot(RAY)>.05 else 1
    for li in f.loop_indices:
     v=me.vertices[me.loops[li].vertex_index].co;uv.data[li].uv=((v.x-left)/w,1-(-v.y*SIN-v.z*COS-top)/h)
   objects[state].append(ob);audits.append({'name':name,'state':state,'vertices':len(vs),'faces':len(fs),'closed':closed,'volume':abs(volume)});return ob
  def tube(name,pts,radius,state,source):
   pts=[Vector(x) for x in pts];vs=[];fs=[];n=12
   for i,p in enumerate(pts):
    direction=(pts[min(i+1,len(pts)-1)]-pts[max(i-1,0)]).normalized();axis=direction.cross(Vector((1,0,0)))
    if axis.length<.01:axis=direction.cross(Vector((0,1,0)))
    axis.normalize();other=direction.cross(axis).normalized()
    for j in range(n):vs.append(p+radius*(axis*math.cos(j*math.tau/n)+other*math.sin(j*math.tau/n)))
   for k in range(len(pts)-1):
    for j in range(n):a=k*n+j;b=k*n+(j+1)%n;fs.append((a,b,b+n,a+n))
   fs.extend([tuple(reversed(range(n))),tuple((len(pts)-1)*n+j for j in range(n))]);return mesh(name,vs,fs,state,source)
  for suffix,state in [('e','empty'),('i','occupied')]:
   s=sources[suffix];left=s['position'][0]+s['offset'][0];top=s['position'][1]+s['offset'][1];fit=packet['fits'][suffix];cx,bottom,height,tilt,*radii=fit['parameters'];lift=(561-top-bottom)/COS;vs=[];fs=[];n=48
   for t,r in zip(fit['fractions'],[.15,*radii,.15]):
    for j in range(n):a=j*math.tau/n;vs.append((left+cx+tilt*(t-.5)+r*math.cos(a),-561/SIN+fit['depth_ratio']*r*math.sin(a),lift+t*height))
   for k in range(len(fit['fractions'])-1):
    for j in range(n):a=k*n+j;b=k*n+(j+1)%n;fs.append((a,b,b+n,a+n))
   fs.extend([tuple(reversed(range(n))),tuple((len(fit['fractions'])-1)*n+j for j in range(n))]);mesh(state+' fitted bag',vs,fs,state,suffix)
   def native(x,y):return point(left+x,top+y,(561-top-y)/COS)
   tube(state+' counterweight log',[native(5,107),native(39,97)],8.0,state,suffix)
   tube(state+' left suspension cord',[native(26,17),native(25,45),native(25,82),native(25,93)],.65,state,suffix)
   tube(state+' right suspension cord',[native(37,0),native(39,45),native(40,88),native(40,113),native(42,136)],.65,state,suffix)
   topbag=Vector((left+cx+tilt*.5,-561/SIN,lift+height));tube(state+' bag neck join',[native(42,134),topbag],.8,state,suffix)
  # Initial support has its own source and shape; it is not the applied log position.
  s=sources['initial'];left=s['position'][0]+s['offset'][0];top=s['position'][1]+s['offset'][1]
  def native0(x,y):return point(left+x,top+y,max(0,(561-top-y)/COS))
  tube('initial counterweight log',[native0(27,18),native0(65,11)],12,'initial','initial')
  tube('initial routed suspension cord',[native0(55,26),native0(62,45),native0(86,65),native0(100,104),native0(106,169),native0(112,237),native0(119,282)],.65,'initial','initial')
  # Thin closed receiver domains are independently labelled; source artwork, not invented dirt bodies.
  for state,key,bounds in [('initial','initial',(0,270,124,325)),('empty','g',(0,0,172,105)),('occupied','g',(0,0,172,105))]:
   s=sources[key];l=s['position'][0]+s['offset'][0];t=s['position'][1]+s['offset'][1];x0,y0,x1,y1=bounds;vs=[point(l+x,t+y,z) for z in [-.5,.2] for x,y in [(x0,y0),(x1,y0),(x1,y1),(x0,y1)]];ob=mesh(state+' ground source receiver',vs,[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)],state,key);ob['receiver_only']=True;ob['terrain_contact_status']='Pending alignment with canonical ground; source plane z0 only'
  ld=bpy.data.lights.new('Diagnostic fixed light','SUN');ld.energy=2;lo=bpy.data.objects.new('Diagnostic fixed light',ld);scene.collection.objects.link(lo);lo.rotation_euler=(.65,-.5,-.4)
  cd=bpy.data.cameras.new('Review camera');cd.type='ORTHO';cd.clip_end=10000;co=bpy.data.objects.new('Review camera',cd);scene.collection.objects.link(co);scene.camera=co
  allpts=[o.matrix_world@v.co for os in objects.values() for o in os for v in o.data.vertices];low=Vector(tuple(min(p[i] for p in allpts) for i in range(3)));high=Vector(tuple(max(p[i] for p in allpts) for i in range(3)));center=(low+high)/2;cd.ortho_scale=(high-low).length*1.2
  for os in objects.values():
   for o in os:o.hide_render=o['state']!='empty'
  co.location=center+RAY*2000;co.rotation_euler=(center-co.location).to_track_quat('-Z','Y').to_euler();bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'));modelsha=sha(dest/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(dest/'model.blend'));scene=bpy.context.scene;co=scene.camera;gray=bpy.data.materials['Unknown hidden surfaces'];renders=[];cameras=[]
  for state in objects:
   for o in scene.objects:
    if o.type=='MESH':o.hide_render=o.get('state')!=state
   for view in range(8):
    a=-math.pi/2+view*math.pi/4;direction=Vector((math.cos(a)*COS,math.sin(a)*COS,SIN));assert view!=0 or (direction-RAY).length<1e-12;co.location=center+direction*2000;co.rotation_euler=(center-co.location).to_track_quat('-Z','Y').to_euler();bpy.context.view_layer.update();cameras.append({'state':state,'view':view,'direction':list(direction),'matrix_world':[list(x) for x in co.matrix_world],'ortho_scale':co.data.ortho_scale})
    for mode in ['actual','solid']:
     scene.view_layers[0].material_override=gray if mode=='solid' else None;path=dest/f'{state}-{view:02}-{mode}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);renders.append({'state':state,'view':view,'mode':mode,'path':path.name,'sha256':sha(path)})
  report={'status':'Private geometry candidate; contact/source completeness self-review pending','model_sha256':modelsha,'source_packet_sha256':sha(OUT/'source.json'),'objects':audits,'renders':renders,'cameras':cameras,'lighting':'Explicit diagnostic fixed light, not texture-generation lighting approval','limitations':packet['inference']+['Ground receiver plane still requires actual canonical terrain contact alignment.','Initial log/rope manual fit needs source-visible domain validation.','No user geometry approval, synthesis or canonical integration.']};(dest/'report.json').write_text(json.dumps(report,indent=2)+'\n');print('NET01 CANDIDATE COMPLETE',flush=True)
 finally:release()
if __name__=='__main__':main()
