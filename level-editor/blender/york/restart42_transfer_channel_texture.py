"""Transfer inferred texture only to eight approved channel faces, preserving old appearance."""
import hashlib,json,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';P=W/'restart41-channel-texture-inputs-v1';B=W/'restart42-channel-bake-v1';O=W/'restart42-channel-assembly-v1';assert not O.exists();sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest();authority=json.loads((P/'authority.json').read_text());source=W/'gatehouse-channel-candidate-v3/model.blend';assert sha(source)==authority['approved_model_sha256'];assert sha(B/'model.blend')==json.loads((B/'validation.json').read_text())['baked_model_sha256']
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire
acquire(slots=2)
import bpy
from refinement_workspace import _geometry
from workspace_components import appearance_state
bpy.ops.wm.open_mainfile(filepath=str(source));scene=bpy.context.scene;scene.render.threads_mode='FIXED';scene.render.threads=2;bpy.context.view_layer.update();targets={r['original_object']for r in authority['partition']};geometry={o.name:_geometry(o)for o in scene.objects};outside={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.name not in targets};appearance={n:appearance_state(scene.objects[n])for n in targets};names=tuple(r['derived_object']for r in authority['partition'])
with bpy.data.libraries.load(str(B/'model.blend'),link=False)as(a,b):b.objects=list(names)
for ob in b.objects:scene.collection.objects.link(ob)
bpy.context.view_layer.update();records=[]
for r in authority['partition']:
 old=scene.objects[r['original_object']];new=scene.objects[r['derived_object']];assert len(new.data.polygons)==4;mapping=[]
 for face_id,record in zip(r['original_face_indices'],r['face_records']):
  face=old.data.polygons[face_id];assert [list(old.matrix_world@old.data.vertices[i].co)for i in face.vertices]==record['xyz'];candidates=[]
  for donor_face in new.data.polygons:
   points=[list(new.matrix_world@new.data.vertices[i].co)for i in donor_face.vertices]
   if len(points)==len(record['xyz']):candidates.append((max(abs(a-b)for p,q in zip(points,record['xyz'])for a,b in zip(p,q)),donor_face.index))
  candidates.sort();assert candidates and candidates[0][0]<.0005,(old.name,face_id,candidates)
  assert len(candidates)==1 or candidates[1][0]>.01,(old.name,face_id,candidates)
  mapping.append((face,new.data.polygons[candidates[0][1]]))
  records.append({'original_face':face_id,'object':old.name,'donor_world_max_abs_roundoff':candidates[0][0]})
 material_map={}
 for face,donor in mapping:
  material=new.data.materials[donor.material_index];uvnodes=[n for n in material.node_tree.nodes if n.bl_idname=='ShaderNodeUVMap'];assert len(uvnodes)==1;source_layer=new.data.uv_layers[uvnodes[0].uv_map]
  if material.name not in material_map:
   copy=material.copy();copy.name='Inferred approved channel '+r['source_node'];uv=old.data.uv_layers.new(name='GuideInteriorTexture');copy.node_tree.nodes[uvnodes[0].name].uv_map=uv.name;slot=len(old.data.materials);old.data.materials.append(copy);material_map[material.name]=(slot,uv)
  slot,uv=material_map[material.name];face.material_index=slot
  for a,bidx in zip(face.loop_indices,donor.loop_indices):uv.data[a].uv=source_layer.data[bidx].uv
 old.data.uv_layers.active_index=appearance[old.name]['active_uv'];after=appearance_state(old);before=appearance[old.name];assert after['materials'][:len(before['materials'])]==before['materials'];assert after['uv_layers'][:len(before['uv_layers'])]==before['uv_layers'];assert after['active_uv']==before['active_uv'];assert all(after['face_materials'][i]==v for i,v in enumerate(before['face_materials'])if i not in r['original_face_indices']);records.append({'object':old.name,'changed_faces':r['original_face_indices'],'old_materials_exact':True,'all_original_uv_layers_exact':True,'non_channel_face_bindings_exact':True})
for ob in list(bpy.data.objects):
 if ob.name in names:bpy.data.objects.remove(ob,do_unlink=True)
bpy.context.view_layer.update();after_geometry={o.name:_geometry(o)for o in scene.objects};assert geometry==after_geometry,{'missing':sorted(set(geometry)-set(after_geometry)),'added':sorted(set(after_geometry)-set(geometry)),'changed':[n for n in geometry.keys()&after_geometry.keys()if geometry[n]!=after_geometry[n]],'face_mapping':records};assert outside=={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.name in outside};O.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(O/'model.blend'),compress=True);(O/'validation.json').write_text(json.dumps({'status':'PRIVATE_EXACT_FACE_TRANSFER_SAVED_REVIEW_PENDING','approved_geometry_sha256':sha(source),'baked_receiver_sha256':sha(B/'model.blend'),'model_sha256':sha(O/'model.blend'),'changed_face_count':sum(len(r.get('changed_faces',[]))for r in records),'records':records,'all_geometry_exact':True,'outside_objects_exact':len(outside),'scope':'Appearance of exactly eight hidden channel-interior faces only. No new user texture approval or canonical publication.'},indent=2)+'\n');assert sha(source)==authority['approved_model_sha256'];print(O)
