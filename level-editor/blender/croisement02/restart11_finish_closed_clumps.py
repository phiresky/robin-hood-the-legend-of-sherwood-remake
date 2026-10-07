"""Test closed native-silhouette cut boundaries on the irregular flat prototype."""
from pathlib import Path
import sys,json,hashlib,math
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from refinement_review import _tree
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 parent=OUT/'restart11-hiding-mound/closed-small-clumps-v3';out=OUT/'restart11-hiding-mound/closed-small-clumps-v4';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(parent/'geometry-intermediate.blend'));scene=bpy.context.scene;objects=[o for o in scene.objects if o.type=='MESH'];records=[]
 source=json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text());rgba=np.array(Image.open(source['source']).convert('RGBA'));h,w=rgba.shape[:2];mask=rgba[:,:,3]>0
 for mat in bpy.data.materials:
  mat['foliage_physical_opacity']=False;mat['opacity_semantics']='closed-solid'
  if not mat.use_nodes:continue
  for shader in mat.node_tree.nodes:
   if shader.type!='BSDF_PRINCIPLED':continue
   for link in list(shader.inputs['Alpha'].links):mat.node_tree.links.remove(link)
   shader.inputs['Alpha'].default_value=1
  mat.update_tag()
 for obj in objects:
  bm=bmesh.new();bm.from_mesh(obj.data);records.append(dict(object=obj.name,vertices=len(obj.data.vertices),faces=len(obj.data.polygons),volume=bm.calc_volume(signed=True),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges)));bm.free();obj.data.update();obj.update_tag()
 bpy.context.view_layer.update()
 from restart11_clump_support_trio import rawtree
 tree=rawtree(objects);missing=[];foreign=[]
 for y in range(h):
  for x in range(w):
   p,_,_,_=tree.ray_cast(Vector((x+.5-w/2,-(y+.5-h/2)/SIN,0))+RAY*500,-RAY)
   if mask[y,x]and p is None:missing.append([x,y])
   if not mask[y,x]and p is not None:foreign.append([x,y])
 model=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));solid=bpy.data.materials.new('Lit closed-solid diagnostic');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.65,.65,.65,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.85;light=bpy.data.objects.new('Review light',bpy.data.lights.new('Review light','AREA'));scene.collection.objects.link(light);light.location=(30,-55,85);light.rotation_euler=(-light.location).to_track_quat('-Z','Y').to_euler();light.data.energy=6000;light.data.size=35
 for mode in ['actual','solid']:
  scene.view_layers[0].material_override=solid if mode=='solid'else None;paths=[]
  for i in range(8):
   angle=i*math.pi/4;frame(scene,objects,Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN)),384,1.15);file=out/f'{mode}-{i:02}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);paths.append(file)
  sheet(paths,out/f'{mode}-eight.png')
 (out/'validation.json').write_text(json.dumps(dict(status='PRIVATE_CLOSED_BOUNDARY_TEST',model_sha256=sha(model),parent_sha256=sha(parent/'geometry-intermediate.blend'),records=records,missing_native_centers=missing,foreign_native_centers=foreign,scope='Explicit silhouette-cut surfaces close formerly alpha-clipped envelope ends. Gray solid equals actual physical coverage, with no alpha discard. Exact source first-hit and topology must be reviewed before any support variant.'),indent=2)+'\n')
 print(json.dumps(dict(nonmanifold=sum(x['nonmanifold_edges']for x in records),missing=missing,foreign=foreign)))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
