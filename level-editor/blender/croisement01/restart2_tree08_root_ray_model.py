"""Saved lower-root depth hypothesis with frozen upper source and strict CPU guard."""
import hashlib,json,math,shutil,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'));sys.path.insert(0,str(Path(__file__).parent))
from render_slots import acquire
from restart2_tree08_union import topology
R=ROOT/'level-editor/work/croisement01-refinement/restart2';monotone='--monotone' in sys.argv;bank_hug='--bank-hug' in sys.argv;packet=R/('tree08-bank-hug-cpu-v2' if bank_hug else 'tree08-monotone-back-cpu-v4-stable' if monotone else 'tree08-root-ray-cpu-v4' if '--fit' in sys.argv else 'tree08-root-ray-cpu-v3');out=R/('tree08-wood-prototype-v16-bank-hug' if bank_hug else 'tree08-wood-prototype-v15-monotone-root-support' if monotone else 'tree08-wood-prototype-v14-root-ray' if '--fit' in sys.argv else 'tree08-wood-prototype-v13-root-ray');parent=R/('tree08-wood-prototype-v14-root-ray/model.blend' if monotone or bank_hug else 'tree08-wood-prototype-v12-local-junctions/model.blend');cap=32*1024**2
assert not out.exists()
def guard(reserve=1024**2):
 used=sum(p.stat().st_size for p in out.rglob('*') if p.is_file());assert used+reserve<=cap
 assert shutil.disk_usage(R).free>=10*1024**3+cap-used
 assert next(int(s.split()[1]) for s in Path('/proc/meminfo').read_text().splitlines() if s.startswith('MemAvailable:'))>=6*1024**2
report=json.loads((packet/'report.json').read_text());model_parent_hash='39c329562cdc4e232061ca4f583126a6747de90054f1bd11bba9c799cf0714cf' if monotone or bank_hug else report['model_parent_sha256']
if monotone:
 previous=hashlib.sha256((R/'tree08-root-ray-cpu-v4/candidate.npz').read_bytes()).hexdigest()
 for name in ['tree08-monotone-back-cpu-v1','tree08-monotone-back-cpu-v2','tree08-monotone-back-cpu-v3-stable','tree08-monotone-back-cpu-v4-stable']:
  folder=R/name;stage=json.loads((folder/'report.json').read_text());check=json.loads((folder/'intersection-guard.json').read_text());digest=hashlib.sha256((folder/'candidate.npz').read_bytes()).hexdigest();assert stage['parent_sha256']==previous and stage['candidate_sha256']==check['candidate_sha256']==digest and not check['new_intersections'] and not check['retained_intersections'];previous=digest
 final=json.loads((packet/'full-chain-source-contact-guard.json').read_text());assert final['candidate_sha256']==previous and final['frontmost_root_interval_contacts']==198 and final['source_core_pixels']==6276 and final['source_core_depth_error']==0 and final['all_first_hit_depth_error']<=.0002 and final['silhouette_lost']==final['silhouette_gained']==0
if bank_hug:
 contact=json.loads((packet/'receiver-intervals.json').read_text());assert contact['candidate_sha256']==report['candidate_sha256'];assert len(contact['samples'])==198 and all(row['front_interval_receiver_contact'] for row in contact['samples']);assert report['source_core_depth_error']==0 and report['silhouette_lost']==report['silhouette_gained']==0
proof=json.loads((packet/'intersection-guard.json').read_text());assert proof['status']=='PASS_NO_NEW_INTERSECTIONS' and not proof['new_intersections'] and not proof['retained_intersections'];assert proof['candidate_sha256']==report['candidate_sha256']==hashlib.sha256((packet/'candidate.npz').read_bytes()).hexdigest();assert hashlib.sha256(parent.read_bytes()).hexdigest()==model_parent_hash;guard();acquire();out.mkdir();bpy.ops.wm.open_mainfile(filepath=str(parent));bpy.context.preferences.filepaths.save_version=0;obj=next(o for o in bpy.context.scene.objects if o.type=='MESH');mesh=obj.data;data=np.load(packet/'candidate.npz');before=np.array([obj.matrix_world@v.co for v in mesh.vertices]);reference=np.load(R/'tree08-root-ray-cpu-v4/candidate.npz')['vertices'] if monotone or bank_hug else data['before_vertices'][:len(before)];assert np.max(abs(before-reference))<.0001;reference_faces=[list(f.vertices) for f in mesh.polygons];reference_uv={tuple(f.vertices):[tuple(mesh.uv_layers.active.data[i].uv) for i in f.loop_indices] for f in mesh.polygons} if monotone or bank_hug else {};reference_tree=BVHTree.FromPolygons([Vector(p) for p in before],reference_faces);retained_core_faces=set();old_topology=topology(obj);oldmesh=mesh;materials=list(mesh.materials);mesh=bpy.data.meshes.new('Conforming lower-root source-ray bands');mesh.from_pydata((data['vertices']-np.array(obj.location)).tolist(),[],data['faces'].tolist());obj.data=mesh
for material in materials:mesh.materials.append(material)
for face in mesh.polygons:face.use_smooth=True
mesh.uv_layers.new(name='Native source projection');bpy.data.meshes.remove(oldmesh)
mesh.update();bpy.context.view_layer.update();saved_topology=topology(obj);assert not any(saved_topology[k] for k in ['zero_area_triangles','nonmanifold_edges','inconsistent_edge_winding']);s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s));down=Vector((0,-s,-c));right=Vector((1,0,0));uv=mesh.uv_layers.active
for loop in mesh.loops:
 point=obj.matrix_world@mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=((point.x-331)/446,1-(-point.y*s-point.z*c-11)/461)
if bank_hug:
 for face in mesh.polygons:
  assert tuple(face.vertices) in reference_uv
  for i,value in zip(face.loop_indices,reference_uv[tuple(face.vertices)]):uv.data[i].uv=value
guard(8*1024**2);bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);assert (out/'model.blend').stat().st_size<=8*1024**2;model_hash=hashlib.sha256((out/'model.blend').read_bytes()).hexdigest()
bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'));scene=bpy.context.scene;obj=next(o for o in scene.objects if o.type=='MESH');mesh=obj.data
bvh=BVHTree.FromPolygons([obj.matrix_world@v.co for v in mesh.vertices],[list(p.vertices) for p in mesh.polygons]);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;missing=[]
for yy,xx in np.argwhere(core):
 x,y=int(xx)+331+.5,int(yy)+11+.5
 if bvh.ray_cast(right*x+down*y+ray*2000,-ray,4000)[0] is None:missing.append([int(x-.5),int(y-.5)])
 if monotone or bank_hug:
  oldhit=reference_tree.ray_cast(right*x+down*y+ray*2000,-ray,4000);assert oldhit[2] is not None;retained_core_faces.add(tuple(reference_faces[oldhit[2]]))
assert not missing,'Saved model lost source core after local junction reconstruction'
if monotone or bank_hug:
 saved_faces={tuple(f.vertices):f for f in mesh.polygons};world=np.array([obj.matrix_world@v.co for v in mesh.vertices])
 for indices in retained_core_faces:
  assert indices in saved_faces and np.array_equal(world[list(indices)],before[list(indices)]);face=saved_faces[indices];assert reference_uv[indices]==[tuple(mesh.uv_layers.active.data[i].uv) for i in face.loop_indices]
reopened_topology=topology(obj);assert reopened_topology==saved_topology,'Saved topology changed'
scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.image_settings.file_format='PNG';scene.render.resolution_percentage=100;scene.render.film_transparent=False
scene.world=bpy.data.worlds.new('Neutral review background');scene.world.color=(.1,.1,.1)
camdata=bpy.data.cameras.new('Tree08 review camera');cam=bpy.data.objects.new('Tree08 review camera',camdata);scene.collection.objects.link(cam);camdata.type='ORTHO';camdata.clip_end=10000;scene.camera=cam
points=[obj.matrix_world@v.co for v in mesh.vertices];target=Vector(tuple((min(v[k] for v in points)+max(v[k] for v in points))/2 for k in range(3)));extent=0;directions=[]
for i in range(8):
 a=math.tau*i/8;direction=Vector((c*math.sin(a),-c*math.cos(a),s));side=direction.cross(Vector((0,0,1))).normalized();up=side.cross(direction).normalized();extent=max(extent,max(max(abs((v-target).dot(side)),abs((v-target).dot(up))) for v in points));directions.append(direction)
records=[]
for mode in ['actual','solid']:
 folder=out/mode;folder.mkdir();scene.render.engine='CYCLES' if mode=='actual' else 'BLENDER_WORKBENCH';scene.cycles.samples=4;scene.cycles.device='CPU';scene.cycles.use_denoising=False
 scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.5,.5,.5);scene.display.shading.show_shadows=True;scene.display.shading.show_cavity=True
 scene.render.resolution_x=384;scene.render.resolution_y=384;camdata.ortho_scale=extent*2*1.12
 sheet=Image.new('RGB',(1536,816),'#222222');draw=ImageDraw.Draw(sheet)
 for i,direction in enumerate(directions):
  cam.location=target+direction*1800;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(folder/f'{i}.png');guard(2*1024**2);bpy.ops.render.render(write_still=True);sheet.paste(Image.open(folder/f'{i}.png').convert('RGB'),(i%4*384,i//4*408+24));draw.text((i%4*384+5,i//4*408+5),f'{mode} {i}'+(' native angle' if i==0 else ''),fill='white')
 guard(2*1024**2);sheet.save(folder/'sheet.png');records.append(dict(mode=mode,sheet_sha256=hashlib.sha256((folder/'sheet.png').read_bytes()).hexdigest()))
scene.render.engine='CYCLES';scene.render.resolution_x=446;scene.render.resolution_y=461;camdata.ortho_scale=461;native_target=right*554+down*241.5;cam.location=native_target+ray*1500;cam.rotation_euler=(-ray).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(out/'native.png');guard();bpy.ops.render.render(write_still=True)

assert hashlib.sha256(parent.read_bytes()).hexdigest()==model_parent_hash;guard();(out/'receipt.json').write_text(json.dumps(dict(status='SAVED HYPOTHESIS; SELF REVIEW PENDING',model_sha256=model_hash,parent_model_sha256=model_parent_hash,cpu_report=report,cpu_intersection_guard_sha256=hashlib.sha256((packet/'intersection-guard.json').read_bytes()).hexdigest(),saved_source_core_pixels=int(core.sum()),source_core_face_geometry_and_uv_exact=True if monotone or bank_hug else None,retained_source_core_faces=len(retained_core_faces),all_uvs_preserved=bank_hug,lower_depth_inferred_refit=bank_hug,miss_native_pixels=missing,saved_topology=saved_topology,actual8_and_solid8=records),indent=2)+'\n');print('ROOT RAY MODEL COMPLETE',model_hash,flush=True)
