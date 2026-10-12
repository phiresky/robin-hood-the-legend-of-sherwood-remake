"""Create a versioned saved-model derivative with nearest-hit source ownership."""
from pathlib import Path
import sys,json,hashlib,math,shutil,os,time
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/blender/croisement02')]
import bpy,bmesh
from mathutils import Vector
from PIL import Image
from render_slots import acquire,release
from scenery_geometry import Mesh
from logtrap_first_hit_oct12 import write_atlases,SIN,COS
from log_trap_state_candidate import material
OUT=ROOT/'level-editor/work/croisement03-refinement/mission-logtrap-oct12-v1'
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 destination=OUT/'owned-candidate-v1';destination.mkdir(exist_ok=False)
 receipt={'status':'queued','pid':os.getpid(),'start_ns':time.time_ns(),'recipe_sha256':sha(Path(__file__)),'source_worker':str(OUT/'candidate-v1/worker.blend')}
 def save(): (destination/'process.json').write_text(json.dumps(receipt,indent=2)+'\n')
 save();acquire(slots=2)
 try:
  mem=dict(x.split(':',1) for x in Path('/proc/meminfo').read_text().splitlines());assert int(mem['MemAvailable'].split()[0])*1024>=6*1024**3;assert shutil.disk_usage(ROOT).free>=10*1024**3
  source=OUT/'candidate-v1/worker.blend';receipt['source_worker_sha256']=sha(source);receipt['status']='loading';save();bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;objects={name:[o for o in scene.objects if o.get('state_endpoint')==name] for name in ['initial','terminal']};reports={}
  fitfile=OUT/'initial-mask-fit.json';fit=json.loads(fitfile.read_text());x,y,r,dx,dy=fit['candidate'];axis=Vector((dx,-dy/SIN,0));normal=Vector((axis.y,-axis.x,0)).normalized();base=Vector((x,-(y+r*COS)/SIN,r));survey=json.loads((OUT/'candidate-v1/geometry.json').read_text())['survey']['initial'];support=[]
  for ob in objects['initial']:
   row=survey[int(ob.name.split()[-1])];course=row['course'];column=row['column'];a=base-normal*r*(course+2*column)+Vector((0,0,math.sqrt(3)*r*course));m=Mesh();m.tube(a,a+axis,r,n=24);oldslots=list(ob.data.materials);mesh=bpy.data.meshes.new(ob.name+' mask-fitted');mesh.from_pydata(m.vertices,[],m.faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));assert all(e.is_manifold for e in bm.edges);bm.to_mesh(mesh);bm.free();mesh.update();ob.data=mesh
   for slot in oldslots:mesh.materials.append(slot)
   uv=mesh.uv_layers.new(name='Native target projection')
   for face in mesh.polygons:
    face.material_index=0 if face.normal.dot(Vector((0,-COS,SIN)))>.05 else 1
    for li in face.loop_indices:
     vertex=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((vertex.x-420)/300,1-(-vertex.y*SIN-vertex.z*COS-380)/240)
   support.append({'object':ob.name,'course':course,'column':column,'start':list(a),'end':list(a+axis),'radius':r})
  # Land the scattered endpoint on the inferred common support plane without
  # changing any source-projected vertex. This is not a map terrain fit.
  contact=[]
  for ob in objects['terminal']:
   low=min((ob.matrix_world@v.co).z for v in ob.data.vertices);delta=Vector((0,low*COS/SIN,-low));errors=[]
   for vertex in ob.data.vertices:
    old=vertex.co.copy();vertex.co+=delta;errors.append(abs((-old.y*SIN-old.z*COS)-(-vertex.co.y*SIN-vertex.co.z*COS)))
   ob.data.update();contact.append({'object':ob.name,'old_min_z':low,'new_min_z':min(v.co.z for v in ob.data.vertices),'native_projection_max_error':max(errors),'translation':list(delta)})
  for state,group in objects.items():
   meshes=[]
   for ob in group:
    ob.data.calc_loop_triangles();meshes.append({'name':ob.name,'vertices':[list(ob.matrix_world@v.co) for v in ob.data.vertices],'triangles':[list(t.vertices) for t in ob.data.loop_triangles]})
   report=write_atlases(meshes,OUT/f'{state}-aligned.png',destination/state);reports[state]=report
   for ob,row in zip(group,report['owners']):
    projected=material(Path(row['path']));nodes=projected.node_tree.nodes;links=projected.node_tree.links;mix=next(node for node in nodes if node.type=='MIX_RGB');output=next(node for node in nodes if node.type=='OUTPUT_MATERIAL')
    emission=nodes.new('ShaderNodeEmission');emission.inputs['Strength'].default_value=1;links.new(mix.outputs[0],emission.inputs['Color']);links.new(emission.outputs[0],output.inputs['Surface']);nodes.remove(nodes.get('Principled BSDF'));ob.data.materials[0]=projected
  # Freeze eight actual-camera views before saving, with source direction first.
  pts=[ob.matrix_world@v.co for group in objects.values() for ob in group for v in ob.data.vertices];center=Vector(tuple((min(p[i] for p in pts)+max(p[i] for p in pts))/2 for i in range(3)));scale=max((p-center).length for p in pts)*2.15;views=[]
  for i in range(8):
   a=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(a)*COS,math.sin(a)*COS,SIN));position=center+direction*3000;views.append({'location':list(position),'rotation':list((center-position).to_track_quat('-Z','Y').to_euler()),'ortho_scale':scale,'original_direction':i==0})
  scene.camera.data.ortho_scale=scale;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.cycles.samples=8;bpy.ops.wm.save_as_mainfile(filepath=str(destination/'worker.blend'));modelsha=sha(destination/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(destination/'worker.blend'));scene=bpy.context.scene;objects={name:[o for o in scene.objects if o.get('state_endpoint')==name] for name in objects};gray=bpy.data.materials['Unknown hidden wood — geometry only'];receipt['status']='saved model reopened; rendering';save()
  for state,group in objects.items():
   for other,items in objects.items():
    for ob in items:ob.hide_render=other!=state
   for mode in ['textured','solid']:
    slots=[]
    if mode=='solid':
     for ob in group:
      for slot in ob.material_slots:slots.append((slot,slot.material));slot.material=gray
    for i,view in enumerate(views):
     scene.camera.location=view['location'];scene.camera.rotation_euler=view['rotation'];scene.render.filepath=str(destination/f'{state}-{mode}-{i}.png');bpy.ops.render.render(write_still=True);receipt['last_completed_image']=scene.render.filepath;save()
    for slot,old in slots:slot.material=old
    sheet=Image.new('RGB',(1536,768),'#252525')
    for i in range(8):
     im=Image.open(destination/f'{state}-{mode}-{i}.png');sheet.paste(im,(i%4*384,i//4*384),im)
    sheet.save(destination/f'{state}-{mode}-eight.png')
  assert sha(source)==receipt['source_worker_sha256'];assert sha(destination/'worker.blend')==modelsha
  (destination/'handoff.json').write_text(json.dumps({'status':'saved eight-view review required, unapproved endpoint geometry','model_sha256':modelsha,'source_worker_sha256':receipt['source_worker_sha256'],'views':views,'ownership':reports,'inferred_terminal_support':contact,'initial_mask_fit_sha256':sha(fitfile),'initial_support':support,'scope':['Own-source initial and terminal endpoints only.','No animated correspondence or terrain contact claim.','Initial inferred support and terminal smaller debris require visual review.']},indent=2)+'\n');receipt['status']='rendered, self-review pending';save()
 except BaseException as exc:
  receipt['status']='failed';receipt['error']=repr(exc);save();raise
 finally:release()
if __name__=='__main__':main()
