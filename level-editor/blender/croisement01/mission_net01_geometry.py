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
  packet=json.loads((OUT/'source.json').read_text());dest=OUT/'candidate-v9';dest.mkdir(exist_ok=False)
  wood_only='--wood-only' in sys.argv;cores=json.loads((OUT/'wood-cores-continuous-v2.json').read_text())
  def wood_surface(source,x,y):
   rows=np.asarray(cores[source]['profile']);cy=float(np.interp(x,rows[:,0],rows[:,1]));radius=float(np.interp(x,rows[:,0],rows[:,2]));delta=(cy-y)/COS
   return -delta*SIN+math.sqrt(max(0,radius*radius-(cy-y)**2))
  context_pin=json.loads((OUT/'tree04-context.json').read_text());context_path=ROOT/context_pin['path'];assert sha(context_path)==context_pin['sha256']
  bpy.ops.wm.open_mainfile(filepath=str(context_path));context_scene=bpy.data.scenes[context_pin['scene']];bpy.context.window.scene=context_scene;bpy.context.view_layer.update()
  context_ob=bpy.data.objects.get('Tree04 decorative wood');assert context_ob is not None,'Missing exact Tree04 decorative wood'
  evaluated=context_ob.evaluated_get(bpy.context.evaluated_depsgraph_get());context_matrix=evaluated.matrix_world.copy();evaluated_mesh=evaluated.to_mesh()
  context_vertices=[tuple(context_matrix@v.co) for v in evaluated_mesh.vertices];context_faces=[tuple(f.vertices) for f in evaluated_mesh.polygons];evaluated.to_mesh_clear()
  context_geometry_sha=hashlib.sha256(json.dumps([context_vertices,context_faces],separators=(',',':')).encode()).hexdigest()
  from mathutils.bvhtree import BVHTree
  support_tree=BVHTree.FromPolygons([Vector(v) for v in context_vertices],context_faces);support_bindings={}
  for label,x,y in [('right',120,309),('initial',105,290)]:
   base=point(x,y,(561-y)/COS);hit=support_tree.ray_cast(base+RAY*1000,-RAY,2000);assert hit[0] is not None,(label,'Missing source-supported branch ray')
   shift=(hit[0]-base).dot(RAY)-.25;support_bindings[label]={'source':[x,y],'base_world':list(base),'branch_surface_world':list(hit[0]),'ray_shift':shift,'endpoint_world':list(base+RAY*shift)}
  original=packet['sources']['initial'];initial_x=105-original['position'][0]-original['offset'][0];initial_y=290-original['position'][1]-original['offset'][1];surface_offset=wood_surface('initial',initial_x,initial_y)+.15
  support_bindings['initial']['anchor_geometry_front_offset']=surface_offset;support_bindings['initial']['ray_shift']-=surface_offset
  bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=384;scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.world=bpy.data.worlds.new('Neutral review world');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.25,.25,.25,1)
  gray=bpy.data.materials.new('Unknown hidden surfaces');gray.diffuse_color=(.32,.32,.32,1);gray.use_nodes=True;gray.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.32,.32,.32,1)
  mats={};sources=packet['sources']
  for state,s in sources.items():
   assert sha(s['path'])==s['sha256'];m=bpy.data.materials.new(state+' native source');m.use_nodes=True;n=m.node_tree.nodes;n.clear();o=n.new('ShaderNodeOutputMaterial');im=n.new('ShaderNodeTexImage');im.image=bpy.data.images.load(s['path']);im.image.pack();im.interpolation='Closest';im.extension='CLIP';em=n.new('ShaderNodeEmission');unknown=n.new('ShaderNodeBsdfPrincipled');unknown.inputs['Base Color'].default_value=(.32,.32,.32,1);mix=n.new('ShaderNodeMixShader');m.node_tree.links.new(im.outputs['Color'],em.inputs[0]);m.node_tree.links.new(im.outputs['Alpha'],mix.inputs[0]);m.node_tree.links.new(unknown.outputs[0],mix.inputs[1]);m.node_tree.links.new(em.outputs[0],mix.inputs[2]);m.node_tree.links.new(mix.outputs[0],o.inputs[0]);mats[state]=m
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
  domains=json.loads((OUT/'domains-rope-separated-rounded-v1.json').read_text())
  def domain(name,key,state,ground,depth):
   row=domains[key];source=row['source'];src=sources[source];left=src['position'][0]+src['offset'][0];top=src['position'][1]+src['offset'][1]
   if not ground:
    profile=cores[source]['profile'];vertices=[];faces=[];count=48
    for x,cy,radius in profile:
     center=point(left+x,top+cy,(561-top-cy)/COS)
     for j in range(count):angle=j*math.tau/count;vertices.append(center+Vector((0,radius*math.sin(angle),radius*math.cos(angle))))
    for k in range(len(profile)-1):
     for j in range(count):a=k*count+j;b=k*count+(j+1)%count;faces.append((a,b,b+count,a+count))
    faces.extend([tuple(reversed(range(count))),tuple((len(profile)-1)*count+j for j in range(count))]);core=mesh(name+' circular wood core',vertices,faces,state,source);core['inference']='Round wood cross-sections fitted inside source body; source fibers are thin separate shell.'
    for poly in core.data.polygons:poly.use_smooth=True
   for part_index,part in enumerate(row['parts']):
    points=[];lookup={};faces=[]
    def vertex(xy,back):
     token=(round(float(xy[0]),9),round(float(xy[1]),9),back)
     if token not in lookup:
      x,y=left+xy[0],top+xy[1]
      if ground:v=point(x,y,.2-depth if back else .2)
      else:
       surface=wood_surface(source,xy[0],xy[1]);v=point(x,y,(561-y)/COS)+RAY*(surface+(-.15 if back else .15))
      lookup[token]=len(points);points.append(v)
     return lookup[token]
    for tri in part['triangles']:
     faces.append(tuple(vertex(x,False) for x in tri));faces.append(tuple(vertex(x,True) for x in reversed(tri)))
    for ring in [part['exterior'],*part['holes']]:
     for a,b in zip(ring,ring[1:]+ring[:1]):faces.append((vertex(a,False),vertex(b,False),vertex(b,True),vertex(a,True)))
    ob=mesh(name+' '+str(part_index),points,faces,state,source);ob['receiver_only']=ground;ob['depth_inferred']=depth
    if not ground:
     ob['depth_profile']='Thin native source shell over round wood core; source fibers remain thin'
     for poly in ob.data.polygons:poly.use_smooth=True
  for suffix,state in [('e','empty'),('i','occupied')]:
   s=sources[suffix];left=s['position'][0]+s['offset'][0];top=s['position'][1]+s['offset'][1];fit=packet['fits'][suffix];cx,bottom,height,tilt,*radii=fit['parameters'];lift=(561-top-bottom)/COS;vs=[];fs=[];n=48
   for t,r in zip(fit['fractions'],[.15,*radii,.15]):
    for j in range(n):a=j*math.tau/n;vs.append((left+cx+tilt*(t-.5)+r*math.cos(a),-561/SIN+fit['depth_ratio']*r*math.sin(a),lift+t*height))
   for k in range(len(fit['fractions'])-1):
    for j in range(n):a=k*n+j;b=k*n+(j+1)%n;fs.append((a,b,b+n,a+n))
   fs.extend([tuple(reversed(range(n))),tuple((len(fit['fractions'])-1)*n+j for j in range(n))]);mesh(state+' fitted bag',vs,fs,state,suffix)
   def native(x,y):return point(left+x,top+y,(561-top-y)/COS)
   domain(state+' counterweight wood',suffix+'-weight',state,False,18)
   tube(state+' left suspension cord',[native(26,17),native(25,45),native(25,82),native(25,102)],.65,state,suffix)
   tube(state+' right suspension cord',[native(37,0),native(39,45),native(40,88),native(40,113),native(42,136)],.65,state,suffix)
   for poly in bpy.data.objects[state+' fitted bag'].data.polygons:poly.use_smooth=True
   topbag=Vector((left+cx+tilt*.5,-561/SIN,lift+height));tube(state+' bag neck join',[native(42,134),topbag],.8,state,suffix)
  # Initial support has its own source and shape; it is not the applied log position.
  s=sources['initial'];left=s['position'][0]+s['offset'][0];top=s['position'][1]+s['offset'][1]
  def native0(x,y):return point(left+x,top+y,max(0,(561-top-y)/COS))
  domain('initial irregular counterweight','initial-weight','initial',False,22)
  alpha=np.asarray(Image.open(s['path']).convert('RGBA'))[:,:,3];rope_pixels=[]
  for y in [48,51,54,57,60,*range(65,266,5),270]:
   xs=np.where(alpha[y]>0)[0]
   if y<60:
    xs=xs[xs>=58];xs=max(np.split(xs,np.where(np.diff(xs)>1)[0]+1),key=len)
   if y>=240:xs=xs[xs>=110]
   assert len(xs),(y,'Missing source rope');rope_pixels.append((float(np.median(xs)),y))
  tube('initial routed suspension cord',[native0(x,y) for x,y in rope_pixels],.65,'initial','initial')

  for state,key in [('initial','initial-ground'),('empty','applied-ground'),('occupied','applied-ground')]:domain(state+' ground receiver',key,state,True,.7)
  # Original-camera projection is invariant under every displacement along RAY.
  for state in ['empty','occupied']:
   shift=support_bindings['right']['ray_shift']
   for ob in objects[state]:
    if not ob.get('receiver_only'):
     for v in ob.data.vertices:v.co+=RAY*shift
     ob.data.update()
   tip=point(109,326,(561-326)/COS)+RAY*shift;near=support_tree.find_nearest(tip);assert near[0] is not None
   finish=near[0]-near[1]*.25;connector=tube(state+' inferred upper left support',[tip,finish],.65,state,'e' if state=='empty' else 'i')
   for poly in connector.data.polygons:poly.material_index=1
   connector['inference']='Short source-unobserved route from observed cord end to nearest branch; requires explicit geometry review.'
   support_bindings[state+'-left-inference']={'observed_tip_world':list(tip),'branch_join_world':list(finish),'length':(finish-tip).length,'branch_join_source':[finish.x,-finish.y*SIN-finish.z*COS]}
  for ob in objects['initial']:
   if 'counterweight' in ob.name:
    for v in ob.data.vertices:v.co+=RAY*support_bindings['initial']['ray_shift']
    ob.data.update()
   elif 'suspension cord' in ob.name:
    ground_shift=(.2-(561-(top+270))/COS)/SIN
    for v in ob.data.vertices:
     source_y=-v.co.y*SIN-v.co.z*COS;factor=max(0,min(1,(top+270-source_y)/(270-rope_pixels[0][1])));v.co+=RAY*(factor*(support_bindings['initial']['ray_shift']+wood_surface('initial',*rope_pixels[0]))+(1-factor)*ground_shift)
    ob.data.update()
    support_bindings['initial-ground-end']={'source_y':top+270,'center_z':.2,'receiver_top_z':.2,'endpoint_ray_shift':ground_shift}
  clearance=[]
  for state in ['empty','occupied']:
   bag=next(o for o in objects[state] if 'fitted bag' in o.name);vertices=[v.co.copy() for v in bag.data.vertices];faces=[tuple(f.vertices) for f in bag.data.polygons];bvh=BVHTree.FromPolygons(vertices,faces);overlap=support_tree.overlap(bvh);zmin=min(v.z for v in vertices);assert zmin>0 and not overlap,(state,zmin,len(overlap));clearance.append({'state':state,'bag_min_z':zmin,'bag_branch_surface_intersection_pairs':len(overlap),'scope':'Endpoint surface intersection and flat-ground clearance; not transition sweep or containment certification.'})
  rope_contacts=[]
  for state,owned in objects.items():
   for ob in owned:
    if not any(term in ob.name for term in ['cord','bag neck','upper left support']):continue
    vertices=[v.co.copy() for v in ob.data.vertices];faces=[tuple(f.vertices) for f in ob.data.polygons];pairs=support_tree.overlap(BVHTree.FromPolygons(vertices,faces));ys=[-vertices[i].y*SIN-vertices[i].z*COS for _,face_index in pairs for i in faces[face_index]]
    if state=='initial':assert not ys or max(ys)<360,('Initial rope intersects tree below the support region',max(ys))
    else:assert not ys or max(ys)<346,('Applied rope intersects tree below its support region',max(ys))
    rope_contacts.append({'state':state,'object':ob.name,'intersection_pairs':len(pairs),'intersection_native_y_bounds':[min(ys),max(ys)] if ys else None,'scope':'Contacts near branch attachment are allowed and disclosed; no intersections farther down the suspended cords.'})
  (dest/'support-bindings.json').write_text(json.dumps({'tree':context_pin,'bindings':support_bindings,'bag_clearance':clearance,'rope_tree_contacts':rope_contacts,'scope':'Right anchor and initial weight use exact native source rays. Upper left connector routing and initial rope depth are inferred; ground source domains remain unchanged.'},indent=2)+'\n')
  ld=bpy.data.lights.new('Diagnostic fixed light','SUN');ld.energy=2;lo=bpy.data.objects.new('Diagnostic fixed light',ld);scene.collection.objects.link(lo);lo.rotation_euler=(.65,-.5,-.4)
  cd=bpy.data.cameras.new('Review camera');cd.type='ORTHO';cd.clip_end=10000;co=bpy.data.objects.new('Review camera',cd);scene.collection.objects.link(co);scene.camera=co
  allpts=[o.matrix_world@v.co for os in objects.values() for o in os for v in o.data.vertices];low=Vector(tuple(min(p[i] for p in allpts) for i in range(3)));high=Vector(tuple(max(p[i] for p in allpts) for i in range(3)));center=(low+high)/2;cd.ortho_scale=(high-low).length*1.2
  for os in objects.values():
   for o in os:o.hide_render=o['state']!='empty'
  co.location=center+RAY*2000;co.rotation_euler=(center-co.location).to_track_quat('-Z','Y').to_euler();bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'));modelsha=sha(dest/'model.blend');bpy.ops.wm.open_mainfile(filepath=str(dest/'model.blend'));scene=bpy.context.scene;co=scene.camera;gray=bpy.data.materials['Unknown hidden surfaces'];renders=[];cameras=[]
  for state in ([] if wood_only else objects):
   for o in scene.objects:
    if o.type=='MESH':o.hide_render=o.get('state')!=state
   for view in range(8):
    a=-math.pi/2+view*math.pi/4;direction=Vector((math.cos(a)*COS,math.sin(a)*COS,SIN));assert view!=0 or (direction-RAY).length<1e-12;co.location=center+direction*2000;co.rotation_euler=(center-co.location).to_track_quat('-Z','Y').to_euler();bpy.context.view_layer.update();cameras.append({'state':state,'view':view,'direction':list(direction),'matrix_world':[list(x) for x in co.matrix_world],'ortho_scale':co.data.ortho_scale})
    for mode in ['actual','solid']:
     scene.view_layers[0].material_override=gray if mode=='solid' else None;path=dest/f'{state}-{view:02}-{mode}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);renders.append({'state':state,'view':view,'mode':mode,'path':path.name,'sha256':sha(path)})
  report={'status':'Private geometry candidate; contact/source completeness self-review pending','model_sha256':modelsha,'source_packet_sha256':sha(OUT/'source.json'),'objects':audits,'renders':renders,'cameras':cameras,'lighting':'Explicit diagnostic fixed light, not texture-generation lighting approval','limitations':packet['inference']+['Thin ground receivers cross the pinned flat Z=0 terrain; canonical partition/integration remains pending.','Counterweight hidden depth and rope depth are inferred; support attachment remains diagnostic.','No user geometry approval, synthesis or canonical integration.']};(dest/'report.json').write_text(json.dumps(report,indent=2)+'\n')
  from mathutils.bvhtree import BVHTree
  verts=[Vector(p) for p in context_vertices];polys=context_faces;matrices=[{'object':'Tree04 decorative wood','matrix_world':[list(row) for row in context_matrix],'evaluated_world_geometry_sha256':context_geometry_sha}]
  context_mesh=bpy.data.meshes.new('Exact evaluated Tree04 context');context_mesh.from_pydata(context_vertices,[],context_faces);context_mesh.update();context_ob=bpy.data.objects.new('Private Tree04 diagnostic context',context_mesh);scene.collection.objects.link(context_ob)
  context_material=bpy.data.materials.new('Diagnostic brown context, not approved appearance');context_material.diffuse_color=(.20,.105,.035,1);context_material.use_nodes=True;context_material.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.20,.105,.035,1);context_mesh.materials.append(context_material)
  tree=BVHTree.FromPolygons(verts,polys);contacts=[]
  for label,x,y in [('left observed tip',109,326),('right observed tip',120,309),('initial weight',105,290)]:
   p=Vector(support_bindings['initial']['endpoint_world']) if label=='initial weight' else point(x,y,(561-y)/COS)+RAY*support_bindings['right']['ray_shift'];nearest=tree.find_nearest(p);hit=tree.ray_cast(p-RAY*1000,RAY,2000);contacts.append({'label':label,'screen':[x,y],'world':list(p),'nearest_world':list(nearest[0]) if nearest[0] is not None else None,'nearest_distance':nearest[3],'same_source_ray_hit':list(hit[0]) if hit[0] is not None else None})
  context_renders=[];scene.view_layers[0].material_override=None
  for state in ([] if wood_only else ['initial','empty','occupied']):
   for ob in scene.objects:
    if ob.type=='MESH' and ob!=context_ob:ob.hide_render=ob.get('state')!=state
   for label,direction in [('original',RAY),('reverse',Vector((0,COS,SIN)))]:
    co.location=center+direction*2000;co.rotation_euler=(center-co.location).to_track_quat('-Z','Y').to_euler();path=dest/f'{state}-tree04-context-{label}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);context_renders.append({'path':path.name,'sha256':sha(path)})
  (dest/'tree04-contact.json').write_text(json.dumps({'scope':'Private pending-approval Tree04 context only; candidate tree geometry untouched; brown context material is diagnostic only. Nearest distances are diagnostics, not certified rope attachment.','source':context_pin,'evaluated_matrices':matrices,'contacts':contacts,'renders':context_renders},indent=2)+'\n')
  closeups=[];context_ob.hide_render=True
  for state in ['initial','empty','occupied']:
   selected=[ob for ob in scene.objects if ob.type=='MESH' and ob.get('state')==state and 'counterweight' in ob.name]
   assert selected
   for ob in scene.objects:
    if ob.type=='MESH':ob.hide_render=ob not in selected
   coordinates=[ob.matrix_world@v.co for ob in selected for v in ob.data.vertices];lo=Vector(tuple(min(p[i] for p in coordinates) for i in range(3)));hi=Vector(tuple(max(p[i] for p in coordinates) for i in range(3)));focus=(lo+hi)/2;co.data.ortho_scale=(hi-lo).length*1.4
   for view in range(4):
    angle=-math.pi/2+view*math.pi/2;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN));co.location=focus+direction*2000;co.rotation_euler=(focus-co.location).to_track_quat('-Z','Y').to_euler()
    for mode in ['actual','solid']:
     scene.view_layers[0].material_override=gray if mode=='solid' else None;path=dest/f'{state}-wood-closeup-{view:02}-{mode}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);closeups.append({'state':state,'view':view,'mode':mode,'path':path.name,'sha256':sha(path)})
  diagnostics=[]
  def role_material(name,color):
   material=bpy.data.materials.new(name);material.use_nodes=True;material.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(*color,1);return material
  core_role=role_material('Diagnostic round core cyan',(.02,.55,.7));shell_role=role_material('Diagnostic source shell orange',(.8,.18,.02))
  for state in ['initial','empty','occupied']:
   selected=[ob for ob in scene.objects if ob.type=='MESH' and ob.get('state')==state and 'counterweight' in ob.name]
   for ob in scene.objects:
    if ob.type=='MESH':ob.hide_render=ob not in selected
   coordinates=[ob.matrix_world@v.co for ob in selected for v in ob.data.vertices];lo=Vector(tuple(min(p[i] for p in coordinates) for i in range(3)));hi=Vector(tuple(max(p[i] for p in coordinates) for i in range(3)));focus=(lo+hi)/2;co.data.ortho_scale=(hi-lo).length*1.4
   direction=Vector((COS/math.sqrt(2),-COS/math.sqrt(2),SIN));co.location=focus+direction*2000;co.rotation_euler=(focus-co.location).to_track_quat('-Z','Y').to_euler()
   original={ob.name:([m for m in ob.data.materials],[f.material_index for f in ob.data.polygons]) for ob in selected}
   for mode in ['actual','solid','roles','core-only-solid','cutoff-0.3']:
    for ob in selected:
     materials,indices=original[ob.name];ob.data.materials.clear()
     for material in materials:ob.data.materials.append(material)
     ob.hide_render=mode=='core-only-solid' and 'circular wood core' not in ob.name
     for face,index in zip(ob.data.polygons,indices):face.material_index=index
     if mode=='roles':
      ob.data.materials.clear();ob.data.materials.append(core_role if 'circular wood core' in ob.name else shell_role)
      for face in ob.data.polygons:face.material_index=0
     elif mode=='cutoff-0.3':
      for face in ob.data.polygons:face.material_index=0 if face.normal.dot(RAY)>.3 else 1
    scene.view_layers[0].material_override=gray if mode in ['solid','core-only-solid'] else None;path=dest/f'{state}-wood-oblique-{mode}.png';scene.render.filepath=str(path);bpy.ops.render.render(write_still=True);diagnostics.append({'state':state,'mode':mode,'direction':list(direction),'path':path.name,'sha256':sha(path)})
   for ob in selected:
    materials,indices=original[ob.name];ob.data.materials.clear()
    for material in materials:ob.data.materials.append(material)
    for face,index in zip(ob.data.polygons,indices):face.material_index=index
  assert sha(dest/'model.blend')==modelsha
  (dest/'wood-oblique-diagnostics.json').write_text(json.dumps({'scope':'Diagnostic material/visibility alternatives only; saved candidate untouched. Actual uses original cosine cutoff 0.05. Cutoff-0.3 suppresses grazing projection only, not an approved replacement. Cyan=round core; orange=thin source silhouette shell, including source fibers. Core-only isolates geometric profile ridges.','model_sha256':modelsha,'renders':diagnostics},indent=2)+'\n')
  (dest/'counterweight-closeups.json').write_text(json.dumps({'scope':'Supplementary wood-only closeups; original direction first; same saved candidate geometry.','renders':closeups},indent=2)+'\n')
  print('NET01 CANDIDATE COMPLETE',flush=True)
 finally:release()
if __name__=='__main__':main()
