"""Reuse exact approved same-face cap materials while protecting newly observed native fronts."""
import sys,json,hashlib,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from restart3_tree06_root_correction import fingerprint
from restart3_tree06_root_review import configure
from restart2_sign_neighbors import camera_to,render
from restart7_wall101_cap_candidate import BASE,D,ASSET
E=OUT/'restart7-fence-residual/wall101-appearance-reuse-v1'
def geom(o):
 return hashlib.sha256(json.dumps(dict(matrix=[list(r)for r in o.matrix_world],verts=[list(v.co)for v in o.data.vertices],faces=[list(f.vertices)for f in o.data.polygons],uv={u.name:[list(x.uv)for x in u.data]for u in o.data.uv_layers}),sort_keys=True).encode()).hexdigest()
def main():
 E.mkdir(exist_ok=False);approval=OUT/'restart3-review-batches/batch-v14/user-approval.json';assert sha(approval)=='8417fd11d5bd7ee78f209f474111bb3619faa43e1b8e30f31a359cf7b6ea4981';approved=D/'model.blend';assert sha(approved)=='05151df87b5a40c98de2ed6b39a87d395b9bd7676cadaf15b99375bf46b7e71b'
 bpy.ops.wm.open_mainfile(filepath=str(BASE));bpy.context.view_layer.update();old=next(o for o in bpy.context.scene.objects if o.get('source_node')=='building-010'and o.get('asset_group')==ASSET);original_indices=[p.material_index for p in old.data.polygons];original_material_names=[m.name for m in old.data.materials]
 bpy.ops.wm.open_mainfile(filepath=str(approved));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==ASSET];obj=next(o for o in objects if o.get('source_node')=='building-010');before={o.name:geom(o)for o in objects};others={o.name:fingerprint(o)for o in objects if o!=obj};changed=json.loads((D/'changes.json').read_text())['faces'];oldassign=[p.material_index for p in obj.data.polygons];assert [m.name for m in obj.data.materials[:len(original_material_names)]]==original_material_names
 native_mat=next(m for m in obj.data.materials if m.name=='Cap observed native front');imgs=[n.image for n in native_mat.node_tree.nodes if n.type=='TEX_IMAGE'];source=next(i for i in imgs if i.size[:]==(1792,1152)and i.colorspace_settings.name!='Non-Color');mask=next(i for i in imgs if i.colorspace_settings.name=='Non-Color');cache={};rows=[]
 for idx in changed:
  face=obj.data.polygons[idx];original_index=original_indices[idx];wasfront=obj.data.materials[face.material_index].name=='Cap observed native front'
  if wasfront:
   if original_index not in cache:
    mat=obj.data.materials[original_index].copy();mat.name+=' / approved cap native over retained';nodes=mat.node_tree.nodes;links=mat.node_tree.links;shader=next((n for n in nodes if n.type in ['BSDF_PRINCIPLED','EMISSION','BSDF_DIFFUSE']),None);socket=shader.inputs['Base Color'if shader.type=='BSDF_PRINCIPLED'else 'Color']if shader else next(n for n in nodes if n.type=='OUTPUT_MATERIAL').inputs['Surface'];oldlink=socket.links[0].from_socket if socket.links else None
    if not shader and(oldlink is None or oldlink.type!='RGBA'):raise ValueError('Unsupported previous cap material')
    mix=nodes.new('ShaderNodeMixRGB');mix.inputs[1].default_value=socket.default_value if hasattr(socket,'default_value')else(.22,.22,.22,1)
    if oldlink:links.new(oldlink,mix.inputs[1])
    uv=nodes.new('ShaderNodeUVMap');uv.uv_map='Native cap projection'
    for image,slot in [(source,2),(mask,0)]:
     tex=nodes.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';links.new(uv.outputs[0],tex.inputs['Vector']);links.new(tex.outputs['Color'],mix.inputs[slot])
    links.new(mix.outputs[0],socket);cache[original_index]=len(obj.data.materials);obj.data.materials.append(mat)
   face.material_index=cache[original_index]
  else:face.material_index=original_index
  rows.append(dict(face=idx,original_approved_material=original_material_names[original_index],native_front_protected=wasfront,method='Native source over exact same-face approved donor'if wasfront else 'Restore exact same-face approved rear material'))
 assert before=={o.name:geom(o)for o in objects};assert others=={o.name:fingerprint(o)for o in objects if o!=obj};assert all(p.material_index==oldassign[p.index]for p in obj.data.polygons if p.index not in changed)
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(E/'model.blend'),compress=True);bpy.ops.wm.open_mainfile(filepath=str(E/'model.blend'));bpy.context.view_layer.update();objects=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==ASSET];assert before=={o.name:geom(o)for o in objects}
 scene=bpy.context.scene
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o not in objects
 camera=configure(scene);camera.data.ortho_scale=100;center=Vector((1543.5,(-845-COS*50)/SIN,50));sheet=Image.new('RGB',(1536,816),(45,45,45))
 for i in range(8):
  a=i*math.pi/4;camera_to(camera,center,Vector((math.sin(a)*COS,-math.cos(a)*COS,SIN)));pic=render(scene,E/f'actual-{i}.png');sheet.paste(pic,(i%4*384,i//4*408),pic.getchannel('A'));ImageDraw.Draw(sheet).text((i%4*384+5,i//4*408+388),f'Same-face retained cap appearance {i}',fill='white')
 sheet.save(E/'actual-eight.png');camera.data.ortho_scale=64;camera_to(camera,Vector((1542,-847/SIN,0)),RAY);render(scene,E/'native-after.png')
 previous=np.asarray(Image.open(D/'native-after.png').convert('RGBA'));current=np.asarray(Image.open(E/'native-after.png').convert('RGBA'));source_pixels=np.asarray(Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA'));known=[]
 for x,y in [(1541,844),(1542,844),(1543,844),(1544,844),(1544,845)]:
  rx=int((x-1510+.5)*6);ry=int((y-815+.5)*6);known.append(dict(pixel=[x,y],before=previous[ry,rx].tolist(),after=current[ry,rx].tolist(),exact_unchanged=bool(np.array_equal(previous[ry,rx],current[ry,rx]))))
 write_json(E/'receipt.json',dict(status='Private saved appearance reuse candidate; review pending',model_sha256=sha(E/'model.blend'),approved_geometry_sha256=sha(approved),approved_base_appearance_sha256=sha(BASE),geometry_approval_sha256=sha(approval),geometry_and_all_uv_exact=True,other_six_receivers_exact=True,unselected_face_assignments_exact=True,changed_faces=rows,native_front_samples=known,api_calls=0,scope='Exact same-face previously approved appearance reused on slightly repositioned cap. New source-facing colors override donor only inside native101; no new synthesis or ownership transfer.'))
 print(E,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
