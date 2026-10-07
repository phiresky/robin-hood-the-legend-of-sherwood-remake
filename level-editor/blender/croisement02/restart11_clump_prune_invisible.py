"""Remove zero-opacity clumps without altering visible geometry or source ownership."""
from pathlib import Path
import sys,json,hashlib,io,math
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,SIN,COS
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def signature(obj):
 return dict(vertices=[list(v.co)for v in obj.data.vertices],polygons=[list(p.vertices)for p in obj.data.polygons],uv=[list(u.uv)for u in obj.data.uv_layers.active.data],materials=[m.name for m in obj.data.materials],matrix=[list(r)for r in obj.matrix_world])
def main():
 parent=OUT/'restart11-hiding-mound/support-trio-v1';out=OUT/'restart11-hiding-mound/support-trio-v2';out.mkdir(exist_ok=False);r=json.loads((parent/'validation.json').read_text());assert sha(parent/'model.blend')==r['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));scene=bpy.context.scene;bpy.context.view_layer.update();removed=[];surviving={}
 for obj in list(scene.objects):
  if obj.type!='MESH':continue
  image=next(n.image for n in obj.data.materials[0].node_tree.nodes if n.type=='TEX_IMAGE');rgba=np.array(Image.open(io.BytesIO(bytes(image.packed_file.data))).convert('RGBA'))
  if not rgba[:,:,3].any():removed.append(obj.name);bpy.data.objects.remove(obj,do_unlink=True)
  else:surviving[obj.name]=signature(obj)
 assert len(removed)==3 and len(surviving)==21
 for row in r['records']:
  row['objects']=[n for n in row['objects']if n not in removed];row['clumps']=[c for c in row['clumps']if c['object']not in removed];assert len(row['objects'])==7
 model=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();assert {o.name:signature(o)for o in scene.objects if o.type=='MESH'}==surviving
 allobjects=[o for o in scene.objects if o.type=='MESH'];original={o:list(o.data.materials)for o in allobjects};solid=bpy.data.materials.new('Solid diagnostic');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.65,.65,.65,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.85;light=bpy.data.objects.new('Review light',bpy.data.lights.new('Review light','AREA'));scene.collection.objects.link(light);light.data.energy=6000;light.data.size=35
 for row in r['records']:
  own=[bpy.data.objects[n]for n in row['objects']];center=sum((o.matrix_world@Vector(v)for o in own for v in o.bound_box),Vector())/(8*len(own));light.location=center+Vector((30,-55,85));light.rotation_euler=(center-light.location).to_track_quat('-Z','Y').to_euler()
  for obj in allobjects:obj.hide_render=obj not in own
  for mode in ['actual','solid','opacity-gray']:
   scene.view_layers[0].material_override=solid if mode=='solid'else None
   for obj in own:
    for i,mat in enumerate(original[obj]):
     if mode!='opacity-gray':obj.data.materials[i]=mat;continue
     neutral=mat.copy();shader=next(n for n in neutral.node_tree.nodes if n.type=='BSDF_PRINCIPLED')
     for socket in ['Base Color','Emission Color']:
      for link in list(shader.inputs[socket].links):neutral.node_tree.links.remove(link)
     shader.inputs['Base Color'].default_value=(.65,.65,.65,1);shader.inputs['Emission Strength'].default_value=0;obj.data.materials[i]=neutral
   paths=[]
   for i in range(8):
    angle=i*math.pi/4;frame(scene,own,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)),384,1.15);file=out/f'{row["tag"]}-{mode}-{i:02}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);paths.append(file)
   sheet(paths,out/f'{row["tag"]}-{mode}-eight.png')
 r.update(status='PRIVATE_TRIO_REVIEW',model_sha256=sha(model),parent_sha256=sha(parent/'model.blend'),removed_zero_opacity_objects=removed,survivor_geometry_uv_material_transform_exact=True,scope='Seven visible clumps per limited support variant. Only zero-alpha objects removed. Native, actual alpha and support limitations remain separate from solid diagnostic.')
 (out/'validation.json').write_text(json.dumps(r,indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
