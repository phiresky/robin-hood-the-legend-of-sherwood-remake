"""Assign each observed leaf pixel once across overlapping clump volumes."""
from pathlib import Path
import sys,json,hashlib,math
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from refinement_review import _tree
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 parent=OUT/'restart11-hiding-mound/clump-volumes-v2';out=OUT/'restart11-hiding-mound/clump-volumes-v3';out.mkdir(exist_ok=False);r=json.loads((parent/'validation.json').read_text());assert sha(parent/'model.blend')==r['model_sha256'];source=json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());rgba=np.array(Image.open(source['source']).convert('RGBA'));h,w=rgba.shape[:2];bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));scene=bpy.context.scene;bpy.context.view_layer.update();objects=[o for o in scene.objects if o.type=='MESH'];tree,owners,_=_tree(objects);assigned={o:np.zeros((h,w),bool)for o in objects}
 for y,x in np.argwhere(rgba[:,:,3]>0):
  p,n,ti,d=tree.ray_cast(Vector((x+.5-w/2,-(y+.5-h/2)/SIN,0))+RAY*500,-RAY);assert p is not None;assigned[owners[ti]][y,x]=True
 masks=[]
 for obj in objects:
  pixel=rgba.copy();pixel[:,:,3]=np.where(assigned[obj],rgba[:,:,3],0);file=out/(obj.name.replace(' ','-')+'-observed.png');Image.fromarray(pixel).save(file);im=bpy.data.images.load(str(file));im.pack()
  for i,m in enumerate(list(obj.data.materials)):
   mat=m.copy();mat.name=obj.name+' '+m.name
   for node in mat.node_tree.nodes:
    if node.type=='TEX_IMAGE':node.image=im
   obj.data.materials[i]=mat
  masks.append(dict(object=obj.name,path=file.name,sha256=sha(file),native_pixels=int(assigned[obj].sum())))
 total=sum(assigned.values());assert np.array_equal(total,rgba[:,:,3]>0);tree,owners,_=_tree(objects);tested=0
 for y,x in np.argwhere(rgba[:,:,3]>0):
  p,n,ti,d=tree.ray_cast(Vector((x+.5-w/2,-(y+.5-h/2)/SIN,0))+RAY*500,-RAY);assert p is not None and assigned[owners[ti]][y,x];tested+=1
 model=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));solid=bpy.data.materials.new('Lit solid');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.65,.65,.65,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.85;light=bpy.data.objects.new('Review light',bpy.data.lights.new('Review light','AREA'));scene.collection.objects.link(light);light.location=(30,-55,85);light.rotation_euler=(-light.location).to_track_quat('-Z','Y').to_euler();light.data.energy=6000;light.data.size=35;original={o:list(o.data.materials)for o in objects}
 for mode in ['actual','solid','opacity-gray']:
  scene.view_layers[0].material_override=solid if mode=='solid'else None
  if mode=='opacity-gray':
   for obj in objects:
    for i,mat in enumerate(original[obj]):
     neutral=mat.copy();shader=next(n for n in neutral.node_tree.nodes if n.type=='BSDF_PRINCIPLED')
     for socket in ['Base Color','Emission Color']:
      for link in list(shader.inputs[socket].links):neutral.node_tree.links.remove(link)
     shader.inputs['Base Color'].default_value=(.65,.65,.65,1);shader.inputs['Emission Strength'].default_value=0;obj.data.materials[i]=neutral
  paths=[]
  for i in range(8):
   angle=i*math.pi/4;frame(scene,objects,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)),384,1.15);file=out/f'{mode}-{i:02}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);paths.append(file)
  sheet(paths,out/f'{mode}-eight.png')
 (out/'validation.json').write_text(json.dumps(dict(status='PRIVATE_OWNERSHIP_PROTOTYPE',model_sha256=sha(model),parent_model_sha256=sha(parent/'model.blend'),source_sha256=source['source_sha256'],geometry_unchanged=True,source_pixels=tested,disjoint_source_ownership=True,masks=masks,scope='Each observed source pixel belongs to one clump only; removes duplicated projected artwork in overlapping volumes. Eight closed shapes unchanged. Alpha-aware gray views preserve opacity. Flat support only.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
