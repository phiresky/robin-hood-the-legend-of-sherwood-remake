"""Private source-projected leaf-cover volume with explicit inferred shallow depth."""
from pathlib import Path
import sys,json,hashlib,math
import bpy,bmesh,numpy as np
from scipy.spatial import ConvexHull,Delaunay
from scipy.ndimage import distance_transform_edt
from mathutils import Vector
from PIL import Image
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart6_source_gap_audit import OUT,RAY,SIN,COS
from restart4_stump_final_contact import frame,sheet
from render_slots import acquire,release
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 out=OUT/'restart9-hiding-scatter/mound-flat-v2';out.mkdir(parents=True,exist_ok=False);b=OUT/'restart7-source-patch-delivery/contracts-v1';m=json.loads((b/'manifest.json').read_text());r=next(r for r in m['records']if r['profile'].endswith('hiding Pc'));c=json.loads((b/r['contract']).read_text());p=next(p for p in c['native']['patch_states']if p['id']==r['id']);f=p['initial'][0];source=Path(next(r['source']for r in m['resources']if r['path']==f['path']));assert sha(source)==f['sha256'];rgba=np.asarray(Image.open(source).convert('RGBA'));h,w=rgba.shape[:2];mask=rgba[:,:,3]>0;distance=distance_transform_edt(mask);distance/=distance.max();vertices=[];faces=[];materials=[];keys={}
 def occupied(x,y):return 0<=x<w and 0<=y<h and bool(mask[y,x])
 def vertex(x,y,cx,cy,top):
  adjacent=[(a,z)for a,z in [(x-1,y-1),(x,y-1),(x-1,y),(x,y)]if occupied(a,z)];diagonal=len(adjacent)==2 and adjacent[0][0]!=adjacent[1][0]and adjacent[0][1]!=adjacent[1][1];key=(x,y,top,cx if diagonal else -1,cy if diagonal else -1)
  if key not in keys:
   d=min([distance[b,a]if occupied(a,b)else 0. for a,b in [(x-1,y-1),(x,y-1),(x-1,y),(x,y)]]);z=.15+4.85*d**.85 if top else 0.;keys[key]=len(vertices);vertices.append((float(x-w/2),float(-(y-h/2+z*COS)/SIN),float(z)))
  return keys[key]
 for y,x in np.argwhere(mask):
  x=int(x);y=int(y);corners=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)];top=[vertex(a,b,x,y,True)for a,b in corners];bottom=[vertex(a,b,x,y,False)for a,b in corners];faces.append(tuple(top));materials.append(0);faces.append(tuple(reversed(bottom)));materials.append(1)
  for i,(nx,ny)in enumerate([(x,y-1),(x+1,y),(x,y+1),(x-1,y)]):
   if not occupied(nx,ny):faces.append((top[i],bottom[i],bottom[(i+1)%4],top[(i+1)%4]));materials.append(1)
 bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.name='Leaf Cover Prototype';mesh=bpy.data.meshes.new('Closed shallow leaf mound');mesh.from_pydata(vertices,[],faces);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();obj=bpy.data.objects.new('Hiding cover initial leaf mound',mesh);scene.collection.objects.link(obj);obj['inferred_maximum_height']=5.;obj['source_sha256']=sha(source)
 mat=bpy.data.materials.new('Exact initial leaf artwork');mat.use_nodes=True;nodes=mat.node_tree.nodes;nodes.clear();tex=nodes.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(source));tex.image.pack();tex.interpolation='Closest';emit=nodes.new('ShaderNodeEmission');transparent=nodes.new('ShaderNodeBsdfTransparent');mix=nodes.new('ShaderNodeMixShader');output=nodes.new('ShaderNodeOutputMaterial');links=mat.node_tree.links;links.new(tex.outputs['Color'],emit.inputs['Color']);links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(transparent.outputs[0],mix.inputs[1]);links.new(emit.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],output.inputs['Surface']);mesh.materials.append(mat);unknown=bpy.data.materials.new('Unobserved lower surface');unknown.diffuse_color=(.3,.3,.3,1);mesh.materials.append(unknown);uv=mesh.uv_layers.new(name='Native source projection')
 for poly in mesh.polygons:
  poly.material_index=materials[poly.index]
  for li in poly.loop_indices:
   v=mesh.vertices[mesh.loops[li].vertex_index].co;sx=v.x+w/2;sy=-v.y*SIN-v.z*COS+h/2;uv.data[li].uv=(sx/w,1-sy/h)
 bm=bmesh.new();bm.from_mesh(mesh);validation=dict(closed=all(e.is_manifold for e in bm.edges)and all(v.is_manifold for v in bm.verts),volume=float(bm.calc_volume(signed=True)),minimum_z=min(v.co.z for v in bm.verts),maximum_z=max(v.co.z for v in bm.verts),vertices=len(mesh.vertices),polygons=len(mesh.polygons));bm.free();assert validation['closed']and validation['volume']>0
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';world=bpy.data.worlds.new('Neutral');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.8;scene.world=world
 model=out/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;obj=bpy.data.objects['Hiding cover initial leaf mound'];bpy.context.view_layer.update();solid=bpy.data.materials.new('Review solid');solid.diffuse_color=(.58,.58,.58,1);views=[]
 for mode in ['actual','solid']:
  scene.view_layers[0].material_override=solid if mode=='solid'else None;paths=[]
  for i in range(8):
   a=i*math.pi/4;direction=Vector((math.sin(a)*COS,-math.cos(a)*COS,SIN));cam=frame(scene,[obj],direction,384,1.3);file=out/f'{mode}-{i:02}.png';scene.render.filepath=str(file);bpy.ops.render.render(write_still=True);paths.append(file);views.append(dict(mode=mode,index=i,path=file.name,sha256=sha(file),matrix=[list(row)for row in cam.matrix_world]))
  sheet(paths,out/f'{mode}-eight.png')
 sourcecopy=out/'native-source.png';Image.open(source).save(sourcecopy);(out/'validation.json').write_text(json.dumps(dict(status='PRIVATE_PROTOTYPE_REVIEW_PENDING',model_sha256=sha(model),source=str(source),source_sha256=sha(source),geometry=validation,source_pixel_count=int((rgba[:,:,3]>0).sum()),source_uv_projection=True,views=views,inference='Closed source-alpha-bounded shallow volumes up to5scene units; gray sides extrude along source rays so native silhouette is preserved. FlatZ0 support only; translated plateau and two edge variants need separate contact proof. Gray underside is unknown, not generated.',scope='Initial leaf cover only; no scatter/motion/gameplay claim.'),indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
