"""Build private source-constrained leaf cover and shallow recess endpoints."""
import sys,json,math,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image
P=Path(__file__).resolve().parent;sys.path.insert(0,str(P))
from restart5_initial_net_candidate_v2 import point,material
from tree_geometry import RAY,SIN,COS
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
ROOT=OUT/'restart9-hole-endpoints'
def main(phase):
 assert phase in ['initial','applied'];assert shutil.disk_usage(OUT).free>25*1024**3
 plan=json.load(open(ROOT/'source-plan-v1/plan.json'));rec=next(x for x in plan['phases']if x['phase']==phase);source=Path(rec['source']);assert sha(source)==rec['sha256'];a=np.array(Image.open(source).convert('RGBA'));mask=a[:,:,3]>0;hh,ww=mask.shape;ox,oy=rec['offset'];dest=ROOT/f'candidate-v1/{phase}';dest.mkdir(parents=True,exist_ok=False)
 # Vertices follow source rays. Height/depth is a bounded hidden-shape hypothesis.
 def height(x,y):
  if phase=='applied':
   r=math.sqrt(((x-38)/17)**2+((y-24)/12)**2)
   if r<1:return .3-8*(1-r*r)**2
   return .6+.4*math.sin(x*.65+y*.4)**2
  return 1.2+.55*math.sin(x*.4+y*.28)+.25*math.cos(x*.8-y*.3)
 vs=[];fs=[];indices={}
 def vertex(x,y,cell,top):
  adjacent=[(x-1,y-1),(x,y-1),(x,y),(x-1,y)];occupied=[0<=cx<ww and 0<=cy<hh and mask[cy,cx]for cx,cy in adjacent];ci=adjacent.index(cell);component=ci if sum(occupied)==2 and occupied[(ci+2)%4]else-1;key=(x,y,component,top)
  if key not in indices:
   z=height(x,y);z=z if top else(z-.45 if phase=='applied'else .1);indices[key]=len(vs);vs.append(tuple(point(ox+x,oy+y,z)))
  return indices[key]
 for y,x in np.argwhere(mask):
  x=int(x);y=int(y);corners=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)];bottom=[vertex(a,b,(x,y),False)for a,b in corners];top=[vertex(a,b,(x,y),True)for a,b in corners];fs.extend([tuple(top),tuple(reversed(bottom))])
  for i,(dx,dy)in enumerate([(0,-1),(1,0),(0,1),(-1,0)]):
   nx,ny=x+dx,y+dy
   if not(0<=nx<ww and 0<=ny<hh and mask[ny,nx]):fs.append((bottom[i],bottom[(i+1)%4],top[(i+1)%4],top[i]))
 bpy.ops.wm.read_factory_settings(use_empty=True);bpy.context.preferences.filepaths.save_version=0;scene=bpy.context.scene;scene.name='Hole '+phase;mesh=bpy.data.meshes.new('Leaf-covered trap '+phase);mesh.from_pydata(vs,[],fs);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));nonmanifold=sum(not e.is_manifold for e in bm.edges);assert nonmanifold==0,nonmanifold;volume=bm.calc_volume();assert volume>0;bm.to_mesh(mesh);bm.free();ob=bpy.data.objects.new(mesh.name,mesh);scene.collection.objects.link(ob);ob['asset_group']='croisement02-hole-'+phase;ob['source_node']='mission-hole-'+phase;ob['native_profile']='Croisement01 - hole';ob['state_role']=phase;ob['source_anchor']='native display_position plus frame offset';ob['hidden_depth_inferred']=True;mat=material(source);gray=material(None,True);mesh.materials.append(mat);mesh.materials.append(gray);uv=mesh.uv_layers.new(name='Initial native source projection')
 for p in mesh.polygons:
  p.material_index=0 if p.normal.dot(RAY)>.05 else 1
  for li in p.loop_indices:
   v=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((v.x-ox)/ww,1-(-v.y*SIN-v.z*COS-oy)/hh)
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=64;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';world=bpy.data.worlds.new('Neutral world');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.7;scene.world=world;light=bpy.data.lights.new('Review sun','SUN');light.energy=2;lo=bpy.data.objects.new(light.name,light);scene.collection.objects.link(lo);lo.rotation_euler=(.6,-.4,-.4);cam=bpy.data.objects.new('Review camera',bpy.data.cameras.new('Review camera'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.clip_end=10000;scene.camera=cam;points=[v.co for v in mesh.vertices];center=(Vector(tuple(min(p[i]for p in points)for i in range(3)))+Vector(tuple(max(p[i]for p in points)for i in range(3))))/2;cam.data.ortho_scale=max((p-center).length for p in points)*2.25;cam.location=center+RAY*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'));digest=sha(dest/'model.blend');solid=bpy.data.materials.new('Solid review');solid.diffuse_color=(.5,.5,.5,1);solid.use_nodes=True;cameras=[]
 for i in range(8):
  angle=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(angle)*COS,math.sin(angle)*COS,SIN));cam.location=center+direction*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();cameras.append(dict(view=i,matrix=[list(r)for r in cam.matrix_world],ortho_scale=cam.data.ortho_scale))
  for mode in ['actual','solid']:
   scene.view_layers[0].material_override=solid if mode=='solid'else None;scene.render.filepath=str(dest/f'view-{i}-{mode}.png');bpy.ops.render.render(write_still=True)
 for mode in ['actual','solid']:
  sheet=Image.new('RGB',(1536,768),(30,30,30))
  for i in range(8):
   im=Image.open(dest/f'view-{i}-{mode}.png').convert('RGBA');sheet.paste(im,((i%4)*384,(i//4)*384),im)
  sheet.save(dest/f'{mode}-sheet.png')
 scene.view_layers[0].material_override=None;scene.render.resolution_x=ww*6;scene.render.resolution_y=hh*6;target=point(ox+ww/2,oy+hh/2,0);cam.location=target+RAY*3000;cam.rotation_euler=(target-cam.location).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=hh;scene.render.filepath=str(dest/'native.png');bpy.ops.render.render(write_still=True);src=Image.open(source).convert('RGBA').resize((ww*6,hh*6),Image.Resampling.NEAREST);rendered=Image.open(dest/'native.png').convert('RGBA');sheet=Image.new('RGB',(ww*12,hh*6),(35,35,35));sheet.paste(src,(0,0),src);sheet.paste(rendered,(ww*6,0),rendered);sheet.save(dest/'source-comparison.png');write_json(dest/'report.json',dict(status='Private prototype; standalone and contact review pending',model_sha256=digest,source=rec,source_plan_sha256=sha(ROOT/'source-plan-v1/plan.json'),vertices=len(vs),faces=len(fs),closed_manifold=True,volume=volume,min_z=min(v[2]for v in vs),max_z=max(v[2]for v in vs),cameras=cameras,limitations=['Source-exact ray-constrained low leaf relief; individual leaf folds/twig support hidden shape inferred.','Applied8-unit depression is a shallow visible recess hypothesis; no invented collapse animation or claimed full underground depth.','Gray reverse and wall material unknown, pending approved geometry before fill.','State-local terrain aperture required and not yet constructed. Existing terrain and source contracts untouched.']));print(digest,flush=True)
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
