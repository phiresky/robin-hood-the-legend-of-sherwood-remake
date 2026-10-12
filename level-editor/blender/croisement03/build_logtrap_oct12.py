"""Own-art solid endpoint hypothesis; save and reopen before fixed-view review."""
from pathlib import Path
import sys,math,json,hashlib,shutil
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/blender/croisement02')]
import bpy,bmesh
from mathutils import Vector
from PIL import Image
from render_slots import acquire,release
from scenery_geometry import Mesh
from log_trap_state_candidate import point,material,SIN,COS,RAY
OUT=ROOT/'level-editor/work/croisement03-refinement/mission-logtrap-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def resources():
 values=dict(line.split(':',1) for line in Path('/proc/meminfo').read_text().splitlines());assert int(values['MemAvailable'].split()[0])*1024>=6*1024**3,'RAM guard';assert shutil.disk_usage(ROOT).free>=10*1024**3,'disk guard'
def main():
 acquire(slots=2)
 try:
  resources();dest=OUT/'candidate-v1';dest.mkdir(exist_ok=False);bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.view_settings.view_transform='Standard';scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.world=bpy.data.worlds.new('World');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.3,.3,.3,1)
  ld=bpy.data.lights.new('Review sun','SUN');ld.energy=2;lo=bpy.data.objects.new('Review sun',ld);scene.collection.objects.link(lo);lo.rotation_euler=(.5,-.4,-.5)
  gray=bpy.data.materials.new('Unknown hidden wood — geometry only');gray.use_nodes=True;gray.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.25,.25,.25,1);gray.node_tree.nodes.get('Principled BSDF').inputs['Roughness'].default_value=1
  states={};surveys={};audits=[]
  # Four own-source visible front courses, with complete triangular support behind.
  r=8.7;axis=Vector((62,28/SIN,0));normal=Vector((axis.y,-axis.x,0)).normalized();base=point(522,505,r);initial=[]
  for course in range(4):
   for column in range(4-course):
    a=base-normal*r*(course+2*column)+Vector((0,0,math.sqrt(3)*r*course));initial.append((a,a+axis,r,{'course':course,'column':column,'inferred_support':column>0}))
  # Surveyed centers follow terminal source silhouettes; no per-log temporal identity claimed.
  terminal=[]
  rows=[(496,480,551,455,6,17,17),(508,492,570,455,7,16,16),(517,505,579,475,7.5,15,15),(520,520,602,487,7.5,7.5,7.5),(551,541,597,509,5,5,5),(576,534,600,518,4,4,4),(610,511,632,499,3.2,3.2,3.2),(625,525,647,515,4,4,4),(628,532,662,522,4,4,4)]
  for row in rows:
   ax,ay,bx,by,rr,az,bz=row;terminal.append((point(ax,ay,az),point(bx,by,bz),rr,{'source_axis':row,'height_inferred':True}))
  for state,entries in [('initial',initial),('terminal',terminal)]:
   mat=material(OUT/f'{state}-aligned.png');objects=[];surveys[state]=[]
   for i,(a,b,radius,meta) in enumerate(entries):
    m=Mesh();m.tube(a,b,radius,n=24);mesh=bpy.data.meshes.new(f'{state} log {i:02}');mesh.from_pydata(m.vertices,[],m.faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);volume=bm.calc_volume();assert volume>0;bm.to_mesh(mesh);bm.free();mesh.update();obj=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(obj);mesh.materials.append(mat);mesh.materials.append(gray);uv=mesh.uv_layers.new(name='Native target projection')
    for face in mesh.polygons:
     face.material_index=0 if face.normal.dot(RAY)>.05 else 1
     for li in face.loop_indices:
      p=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((p.x-420)/300,1-(-p.y*SIN-p.z*COS-380)/240)
    obj['state_endpoint']=state;obj['status']='unapproved own-source geometry';objects.append(obj);surveys[state].append({'start':list(a),'end':list(b),'radius':radius,**meta});audits.append({'object':obj.name,'closed':True,'positive_volume':volume})
   states[state]=objects
  cd=bpy.data.cameras.new('Original first fixed review');cd.type='ORTHO';cd.clip_end=10000;camera=bpy.data.objects.new(cd.name,cd);scene.collection.objects.link(camera);scene.camera=camera;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100
  points=[v.co for objects in states.values() for ob in objects for v in ob.data.vertices];center=Vector(tuple((min(p[i] for p in points)+max(p[i] for p in points))/2 for i in range(3)));cd.ortho_scale=max((p-center).length for p in points)*2.15
  views=[]
  for i in range(8):
   angle=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN));location=center+direction*3000;rotation=(center-location).to_track_quat('-Z','Y').to_euler();views.append({'location':list(location),'rotation':list(rotation),'ortho_scale':cd.ortho_scale,'source_camera':i==0})
  camera.location=views[0]['location'];camera.rotation_euler=views[0]['rotation'];bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'));saved=sha(dest/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(dest/'worker.blend'));scene=bpy.context.scene;camera=scene.camera;states={k:[ob for ob in scene.objects if ob.get('state_endpoint')==k] for k in states};gray=bpy.data.materials['Unknown hidden wood — geometry only']
  for state,objects in states.items():
   for key,other in states.items():
    for ob in other:ob.hide_render=key!=state
   for mode in ['textured','solid']:
    replacements=[]
    if mode=='solid':
     for ob in objects:
      for slot in ob.material_slots:replacements.append((slot,slot.material));slot.material=gray
    for i,view in enumerate(views):
     camera.location=view['location'];camera.rotation_euler=view['rotation'];scene.render.filepath=str(dest/f'{state}-{mode}-{i}.png');bpy.ops.render.render(write_still=True)
    for slot,old in replacements:slot.material=old
    sheet=Image.new('RGB',(1536,768),'#252525')
    for i in range(8):
     im=Image.open(dest/f'{state}-{mode}-{i}.png');sheet.paste(im,(i%4*384,i//4*384),im)
    sheet.save(dest/f'{state}-{mode}-eight.png')
  assert sha(dest/'worker.blend')==saved
  (dest/'geometry.json').write_text(json.dumps({'status':'self-review pending, not approved','model_sha256':saved,'source_qualification_sha256':sha(OUT/'source-qualification.json'),'views':views,'survey':surveys,'checks':audits,'limitations':['Four-course ten-log initial pile is own-source inference; supporting rear logs unobserved.','Terminal nine axes are independent endpoint hypotheses, not a physical log identity mapping.','Small loose debris and receiver shadow remain source-only, not meshed.','Heights and flat ground support inferred; terrain contact not yet verified.','No transient b2 geometry or temporal correspondence claim.','Gray backs need later approved texture fill.']},indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
