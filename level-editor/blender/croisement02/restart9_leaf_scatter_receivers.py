"""Private terminal leaf artwork on exact terrain triangles and first-hit ownership."""
from pathlib import Path
import sys,json,hashlib,collections,math
import bpy,numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.bvhtree import BVHTree
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from restart2_export_receiver_surfaces import clip_polygon,screen
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def material(path):
 mat=bpy.data.materials.new(path.stem);mat.use_nodes=True;n=mat.node_tree.nodes;n.clear();t=n.new('ShaderNodeTexImage');t.image=bpy.data.images.load(str(path));t.image.pack();t.interpolation='Closest';em=n.new('ShaderNodeEmission');tr=n.new('ShaderNodeBsdfTransparent');mix=n.new('ShaderNodeMixShader');o=n.new('ShaderNodeOutputMaterial');l=mat.node_tree.links;l.new(t.outputs['Color'],em.inputs[0]);l.new(t.outputs['Alpha'],mix.inputs[0]);l.new(tr.outputs[0],mix.inputs[1]);l.new(em.outputs[0],mix.inputs[2]);l.new(mix.outputs[0],o.inputs['Surface']);return mat
def main():
 out=OUT/'restart9-hiding-scatter/scatter-surfaces-v1';out.mkdir(parents=True,exist_ok=False);audit=json.loads((OUT/'restart9-hiding-scatter/terrain-receivers-v2/report.json').read_text());base=OUT/'restart7-source-patch-delivery/contracts-v1';manifest=json.loads((base/'manifest.json').read_text());resources={r['path']:Path(r['source'])for r in manifest['resources']};owners=[];vertices=[];triangles=[];modelpins=[]
 for pin in audit['models']:
  model=Path(pin['path']);assert sha(model)==pin['sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update()
  for record in pin['objects']:
   obj=bpy.data.objects[record['name']];assert np.max(np.abs(np.array(obj.matrix_world)-np.array(record['matrix_world'])))<1e-8;obj.data.calc_loop_triangles();points=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);start=len(vertices);vertices.extend(points)
   for tri in obj.data.loop_triangles:triangles.append(tuple(start+int(i)for i in tri.vertices));owners.append(obj.name)
  modelpins.append(dict(path=str(model),sha256=sha(model)))
 points=np.array(vertices);bvh=BVHTree.FromPolygons([Vector(p)for p in vertices],triangles,all_triangles=True);unique={}
 for r in audit['records']:
  if r['applied']is None:continue
  key=tuple(r['display_position'])+(r['applied']['sha256'],);unique.setdefault(key,dict(record=r,instances=[]))['instances'].append(r['id'])
 bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.name='Leaf Scatter Endpoints';records=[]
 for number,value in enumerate(unique.values()):
  record=value['record'];f=record['applied'];source=resources[f['path']];assert sha(source)==f['sha256'];rgba=np.array(Image.open(source).convert('RGBA'));h,w=rgba.shape[:2];x,y=np.array(record['display_position'])+f['offset'];masks={};support=[]
  for py,px in np.argwhere(rgba[:,:,3]>0):
   hit,normal,index,d=bvh.ray_cast(Vector((float(x+px)+.5,-(float(y+py)+.5)/SIN,0))+RAY*6000,-RAY);assert hit is not None;owner=owners[index];masks.setdefault(owner,np.zeros((h,w),bool))[py,px]=True;support.append(list(hit))
  source_count=int((rgba[:,:,3]>0).sum());assert sum(int(mask.sum())for mask in masks.values())==source_count;objects=[];receiverrows=[]
  for oi,(owner,mask)in enumerate(masks.items()):
   image=rgba.copy();image[~mask,3]=0;texture=out/f'endpoint-{number:02}-receiver-{oi:02}.png';Image.fromarray(image).save(texture);verts=[];uv=[]
   for indices,triangle_owner in zip(triangles,owners):
    if triangle_owner!=owner:continue
    poly=[points[i]for i in indices];normal=np.cross(poly[1]-poly[0],poly[2]-poly[0]);
    if np.dot(normal,np.array(RAY))<=1e-9:continue
    projected=np.array([screen(p)for p in poly]);
    if projected[:,0].max()<x or projected[:,0].min()>x+w or projected[:,1].max()<y or projected[:,1].min()>y+h:continue
    for axis,bound,keep in [(0,x,1),(0,x+w,-1),(1,y,1),(1,y+h,-1)]:
     poly=clip_polygon(poly,axis,bound,keep)
     if not poly:break
    for i in range(1,len(poly)-1):
     tri=[poly[0],poly[i],poly[i+1]]
     if np.linalg.norm(np.cross(tri[1]-tri[0],tri[2]-tri[0]))<1e-8:continue
     for p in tri:
      sx,sy=screen(p);verts.append(p+np.array(RAY)*.002);uv.append(((sx-x)/w,1-(sy-y)/h))
   assert verts,owner;mesh=bpy.data.meshes.new(f'Leaf receiver {number}/{oi}');mesh.from_pydata(verts,[],[(i,i+1,i+2)for i in range(0,len(verts),3)]);mesh.update();layer=mesh.uv_layers.new(name='Native source projection')
   for loop in mesh.loops:layer.data[loop.index].uv=uv[loop.vertex_index]
   mesh.materials.append(material(texture));obj=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(obj);obj['endpoint_index']=number;obj['physical_receiver']=owner;obj['source_sha256']=sha(source);objects.append(obj.name);receiverrows.append(dict(owner=owner,source_pixels=int(mask.sum()),texture=texture.name,texture_sha256=sha(texture),triangles=len(verts)//3))
  records.append(dict(index=number,instances=value['instances'],profile=record['profile'],display_position=record['display_position'],bbox=[int(x),int(y),w,h],source=str(source),source_sha256=sha(source),source_pixels=source_count,objects=objects,receivers=receiverrows,minimum_support_z=min(p[2]for p in support),maximum_support_z=max(p[2]for p in support)))
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';world=bpy.data.worlds.new('Neutral');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.8;scene.world=world;model=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();reviews=[]
 review_ids=['mission-Emb05_FoB_MP-patch-015','mission-Tac21_FoB_EC-patch-009','mission-Tac21_FoB_EC-patch-010','mission-Tac19_FoB_EC-patch-000']
 for id in review_ids:
  row=next(r for r in records if id in r['instances']);own=[bpy.data.objects[n]for n in row['objects']]
  for o in scene.objects:
   if o.type=='MESH':o.hide_render=o not in own
  paths=[]
  for i in range(8):
   a=i*math.pi/4;cam=frame(scene,own,Vector((math.sin(a)*COS,-math.cos(a)*COS,SIN)),384,1.3);file=out/f'endpoint-{row["index"]:02}-{i:02}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);paths.append(file)
  sheet(paths,out/f'endpoint-{row["index"]:02}-eight.png');reviews.append(dict(id=id,index=row['index'],sheet=f'endpoint-{row["index"]:02}-eight.png',views=[dict(path=p.name,sha256=sha(p))for p in paths]))
 (out/'manifest.json').write_text(json.dumps(dict(status='PRIVATE_SOURCE_APPEARANCE_RECEIVER_CANDIDATE',model_sha256=sha(model),models=modelpins,records=records,reviews=reviews,instance_count=sum(len(r['instances'])for r in records),source_preserving_ray_offset=.002,missing_applied_instance='mission-Tac19_FoB_EC-patch-015',scope='Terminal leaf artwork partitioned by exact native pixel terrain first hits. Background geometry/materials unchanged. Plane carriers are copied/clipped support triangles, not leaf-volume claims. Other scenery contact/occlusion remains pending.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
