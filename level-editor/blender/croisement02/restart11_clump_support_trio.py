"""Test only flat, bank-foot and wall-toe rigid leaf-clump support variants."""
from pathlib import Path
import sys,json,hashlib,math
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from leaf_state_scene_context import load_scene
from refinement_review import _tree
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def rawtree(objects):
 points=[];faces=[]
 for obj in objects:
  obj.data.calc_loop_triangles();start=len(points);points +=[obj.matrix_world@v.co for v in obj.data.vertices];faces +=[tuple(start+i for i in t.vertices)for t in obj.data.loop_triangles]
 return BVHTree.FromPolygons(points,faces,all_triangles=True)

def main():
 out=OUT/'restart11-hiding-mound/support-trio-v1';out.mkdir(exist_ok=False);parent=OUT/'restart11-hiding-mound/clump-volumes-v4';pr=json.loads((parent/'validation.json').read_text());assert sha(parent/'model.blend')==pr['model_sha256'];audit=json.loads((OUT/'restart9-hiding-scatter/terrain-receivers-v2/report.json').read_text());scene,static,pins,base=load_scene();ground=[o for o in static if o.name.startswith('Croisement02 Terrain')or o.get('asset_group')=='croisement02-north-woodland-bank'];wall=[o for o in static if o.get('asset_group')=='croisement02-southeast-stone-wall-and-gate'];assert ground and wall;terrain=rawtree(ground);withwall=rawtree(ground+wall)
 bpy.ops.wm.open_mainfile(filepath=str(parent/'model.blend'));scene=bpy.context.scene;bpy.context.view_layer.update();templates=[o for o in scene.objects if o.type=='MESH'];records=[];allobjects=[]
 for tag,instance in [('flat','mission-Emb05_FoB_MP-patch-015'),('sloped','mission-Tac21_FoB_EC-patch-010'),('wall','mission-Tac02_FoB_EC-patch-022')]:
  row=next(r for r in audit['records']if r['id']==instance);x0,y0=np.array(row['display_position'])+row['initial']['offset'];center=Vector((float(x0+26),float(-(y0+16.5)/SIN),0));support=withwall if tag=='wall'else terrain;objects=[];clumps=[]
  for index,template in enumerate(templates):
   obj=template.copy();obj.data=template.data.copy();obj.name=f'{tag} {template.name}';scene.collection.objects.link(obj)
   for vertex in obj.data.vertices:vertex.co+=center
   obj.data.update();bpy.context.view_layer.update();tree=rawtree([obj]);maskrow=next(r for r in pr['masks']if r['object']==template.name);mask=np.array(Image.open(parent/maskrow['path']))[:,:,3]>0;required=0.;samples=[];corners_missing=0
   for y,x in np.argwhere(mask):
    for dx,dy in [(a,b)for a in [.005,.5,.995]for b in [.005,.5,.995]]:
     raybase=Vector((float(x0+x+dx),float(-(y0+y+dy)/SIN),0));receiver,_,_,_=support.ray_cast(raybase+RAY*2000,-RAY);back,_,_,_=tree.ray_cast(raybase-RAY*2000,RAY)
     if back is None:corners_missing+=1;continue
     assert receiver is not None;need=(receiver-back).dot(RAY)+.01*(index+1);required=max(required,need);samples.append(need)
   assert required*RAY.z<8, (tag,index,required)
   for vertex in obj.data.vertices:vertex.co+=RAY*required
   obj.data.update();objects.append(obj);allobjects.append(obj);clumps.append(dict(object=obj.name,template=template.name,rigid_ray_shift=required,height_gain=required*RAY.z,visible_support_samples=len(samples),pixel_corner_coverage_misses=corners_missing,minimum_sampled_clearance=min((required-v+.01*(index+1)for v in samples),default=None),maximum_sampled_clearance=max((required-v+.01*(index+1)for v in samples),default=None)))
  bpy.context.view_layer.update();tree,owners,_=_tree(objects);total=0;missing=[];source=np.array(Image.open(json.loads((OUT/'restart9-hiding-scatter/mound-flat-v2/validation.json').read_text())['source']))
  for y,x in np.argwhere(source[:,:,3]>0):
   p,n,ti,d=tree.ray_cast(Vector((float(x0+x+.5),float(-(y0+y+.5)/SIN),0))+RAY*2000,-RAY)
   if p is None:missing.append([int(x),int(y)])
   else:total+=1
  assert total==1011 and not missing;records.append(dict(tag=tag,instance=instance,objects=[o.name for o in objects],native_opaque_centers=total,clumps=clumps))
 for obj in templates:bpy.data.objects.remove(obj,do_unlink=True)
 model=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));solid=bpy.data.materials.new('Lit solid');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.65,.65,.65,1);solid.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.85;light=bpy.data.objects.new('Review light',bpy.data.lights.new('Review light','AREA'));scene.collection.objects.link(light);light.data.energy=6000;light.data.size=35;original={o:list(o.data.materials)for o in allobjects}
 for row in records:
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
 (out/'validation.json').write_text(json.dumps(dict(status='PRIVATE_TRIO_REVIEW',model_sha256=sha(model),parent_sha256=sha(parent/'model.blend'),static_base_sha256=sha(base),substitutions=pins,records=records,scope='Only flat/sloped/wall trio. Clump shapes remain rigid; shifts preserve source rays and avoid spikes. Support sampled over owned opaque texel areas. Unknown rear appearance separate; no mass propagation.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
