"""Build private thick foliage screens with exact observed source faces."""
import os,sys,json,math,shutil,hashlib
from pathlib import Path
os.environ['OPENBLAS_NUM_THREADS']='2'
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
from mathutils.bvhtree import BVHTree
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[2]/'refinement')]
import qualify as q
from render_slots import acquire,release
S,C=math.sin(math.radians(35)),math.cos(math.radians(35));RAY=Vector((0,-C,S))
DEST=q.OUT/'geometry-v2-rounded02'
PROFILES={2}
CLEARANCE={}
def guard():
 assert shutil.disk_usage(q.ROOT).free>=10*1024**3+8*1024**2
 assert sum(p.stat().st_size for p in DEST.rglob('*')if p.is_file())<160*1024**2
 mem=dict(line.split(':',1)for line in Path('/proc/meminfo').read_text().splitlines());assert int(mem['MemAvailable'].split()[0])*1024>=10*1024**3

def make(row,n,phase,f,anchor):
 a=np.array(q.rgba(f));mask=a[:,:,3]>0;hh,ww=mask.shape;x0,y0=map(int,f['top_left']);pixels={(x0+int(x),y0+int(y)):False for y,x in zip(*np.where(mask))}
 if n==2:
  # Complete the clipped top with a small tapered, explicitly unknown extension.
  xx=[x for x,y in pixels if y==0]
  if xx:
   lo,hi=min(xx),max(xx)
   radius=(hi-lo+1)/2;center=(lo+hi+1)/2;rise=max(4,math.ceil(radius))
   for y in range(-rise,0):
    half=radius*math.sqrt(max(0,1-(abs(y)/rise)**2))
    start,end=math.floor(center-half),math.ceil(center+half)
    for x in range(start,max(start+1,end)):pixels[x,y]=True
 vertices=[];faces=[];mats=[];lookup={};xlo,xhi=row['source_union_box'][0],row['source_union_box'][2]
 def vertex(x,y,back,owner):
  around=[(x-1,y-1),(x,y-1),(x,y),(x-1,y)];occupied=[p in pixels for p in around];split=sum(occupied)==2 and occupied[0]==occupied[2]
  key=(x,y,back,owner if split else None)
  if key not in lookup:
   u=(x-xlo)/(xhi-xlo);wy=anchor['world_y_left']*(1-u)+anchor['world_y_right']*u+anchor['depth_bulge']*math.sin(math.pi*u);z=(-y-wy*S)/C;p=Vector((x,wy,z))-RAY*(anchor['thickness'] if back else 0)
   field=CLEARANCE.get(str(n))
   if field:
    bx,by,bxx,byy=field['box']
    if bx<=x<=bxx and by<=y<=byy:p+=RAY*field['camera_ray_advance'][y-by][x-bx]
   lookup[key]=len(vertices);vertices.append(tuple(p))
  return lookup[key]
 for (x,y),unknown in pixels.items():
  corners=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)];front=[vertex(*p,False,(x,y))for p in corners];back=[vertex(*p,True,(x,y))for p in corners];
  if str(n) in CLEARANCE:
   faces.extend([(front[0],front[2],front[1]),(front[0],front[3],front[2]),(back[0],back[1],back[2]),(back[0],back[2],back[3])]);mats.extend([2 if unknown else 0,2 if unknown else 0,1,1])
  else:faces.extend([tuple(front[::-1]),tuple(back)]);mats.extend([2 if unknown else 0,1])
  for k,(dx,dy) in enumerate([(0,-1),(1,0),(0,1),(-1,0)]):
   if (x+dx,y+dy)not in pixels:faces.append((front[k],front[(k+1)%4],back[(k+1)%4],back[k]));mats.append(1)
 mesh=bpy.data.meshes.new(f'Screen{n:02}-{phase}');mesh.from_pydata(vertices,[],faces);mesh.update();o=bpy.data.objects.new(mesh.name,mesh);bpy.context.collection.objects.link(o)
 texpath=DEST/f'profile-{n:02}'/f'{phase}-source.png';Image.fromarray(a).save(texpath);im=bpy.data.images.load(str(texpath));im.pack()
 observed=bpy.data.materials.new('Observed source pixels');observed.use_nodes=True;nt=observed.node_tree;nt.nodes.clear();out=nt.nodes.new('ShaderNodeOutputMaterial');em=nt.nodes.new('ShaderNodeEmission');tex=nt.nodes.new('ShaderNodeTexImage');tex.image=im;tex.interpolation='Closest';tex.extension='CLIP';uv=nt.nodes.new('ShaderNodeUVMap');uv.uv_map='Source';nt.links.new(uv.outputs['UV'],tex.inputs['Vector']);nt.links.new(tex.outputs['Color'],em.inputs['Color']);nt.links.new(em.outputs[0],out.inputs[0]);mesh.materials.append(observed)
 for name,col in [('Inferred thickness and back',(.24,.24,.24,1)),('Inferred off-map continuation',(.42,.32,.42,1))]:
  mat=bpy.data.materials.new(name);mat.diffuse_color=col;mat.use_nodes=True;bs=mat.node_tree.nodes.get('Principled BSDF');bs.inputs['Base Color'].default_value=col;bs.inputs['Roughness'].default_value=.9;mesh.materials.append(mat)
 layer=mesh.uv_layers.new(name='Source')
 for p,material in zip(mesh.polygons,mats):
  p.material_index=material
  for li in p.loop_indices:
   pos=Vector(vertices[mesh.loops[li].vertex_index]);sx=pos.x;sy=-pos.y*S-pos.z*C;layer.data[li].uv=((sx-x0)/ww,1-(sy-y0)/hh)
 o['source_profile']=row['profile'];o['source_phase']=phase;o['inferred_depth']=True;o['observed_source_sha256']=f['image']['sha256'];o['native_source_origin']=[x0,y0]
 mesh.calc_loop_triangles();tree=BVHTree.FromPolygons(vertices,[tuple(t.vertices)for t in mesh.loop_triangles],all_triangles=True);miss=[];extra=[];uvbad=[]
 for y in range(-2,hh+2):
  for x in range(-2,ww+2):
   sx,sy=x0+x+.5,y0+y+.5
   if sy<0:continue
   hit,normal,index,distance=tree.ray_cast(Vector((sx,-sy/S,0))+RAY*6000,-RAY);expected=0<=x<ww and 0<=y<hh and bool(mask[y,x])
   if expected and hit is None:miss.append([sx,sy])
   if not expected and hit is not None:extra.append([sx,sy])
   if expected and hit is not None:
    tri=mesh.loop_triangles[index]
    if tri.material_index!=0:uvbad.append([sx,sy,'unknown first'])
    px,py=hit.x,-hit.y*S-hit.z*C
    if abs(px-sx)>.02 or abs(py-sy)>.02:uvbad.append([sx,sy,'projection drift'])
    va,vb,vc=[mesh.vertices[k].co for k in tri.vertices];ab,ac,ap=vb-va,vc-va,hit-va;aa,bb,cc=ab.dot(ab),ab.dot(ac),ac.dot(ac);den=aa*cc-bb*bb;u=(cc*ap.dot(ab)-bb*ap.dot(ac))/den;v=(aa*ap.dot(ac)-bb*ap.dot(ab))/den;coords=[layer.data[k].uv for k in tri.loops];uv=coords[0]*(1-u-v)+coords[1]*u+coords[2]*v
    if (math.floor(uv.x*ww),hh-1-math.floor(uv.y*hh))!=(x,y):uvbad.append([sx,sy,'stored UV sample differs'])
 edgecounts={}
 for poly in mesh.polygons:
  for edge in poly.edge_keys:edgecounts[edge]=edgecounts.get(edge,0)+1
 record=dict(profile=n,phase=phase,source=f['image'],decoded_source=q.pin(texpath),vertices=len(vertices),faces=len(faces),boundary_or_nonmanifold_edges=sum(v!=2 for v in edgecounts.values()),missing_native_pixels=miss,extra_native_pixels=extra,first_hit_uv_failures=uvbad,inferred_pixels=sum(pixels.values()),inferred_parameters=anchor,world_bounds=[np.min(vertices,axis=0).tolist(),np.max(vertices,axis=0).tolist()])
 assert not miss and not extra and not uvbad and not record['boundary_or_nonmanifold_edges'],record
 return o,record

def render(obj,folder):
 scene=bpy.context.scene;points=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);lo,hi=points.min(0),points.max(0);center=Vector((lo+hi)/2);extent=float(np.linalg.norm(hi-lo));cam=bpy.data.objects.new('Original-camera-first',bpy.data.cameras.new('Orthographic review'));scene.collection.objects.link(cam);scene.camera=cam;cam.data.type='ORTHO';cam.data.ortho_scale=extent*1.12;cam.data.clip_end=10000
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.use_denoising=False;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=scene.render.resolution_y=256;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.world=bpy.data.worlds.new('Neutral');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.24,.24,.24,1);scene.world.node_tree.nodes['Background'].inputs[1].default_value=.8;scene.view_settings.view_transform='Standard'
 solid=bpy.data.materials.new('Solid inspection');solid.use_nodes=True;solid.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.46,.5,.43,1)
 light=bpy.data.objects.new('Review light',bpy.data.lights.new('Review light','AREA'));scene.collection.objects.link(light);light.location=center+Vector((100,-200,300));light.data.energy=120000;light.data.shape='DISK';light.data.size=150;light.rotation_euler=(center-light.location).to_track_quat('-Z','Y').to_euler()
 for mode in ['textured','solid']:
  scene.view_layers[0].material_override=solid if mode=='solid' else None;sheet=Image.new('RGB',(1024,552),(28,28,28));draw=ImageDraw.Draw(sheet)
  for i in range(8):
   guard();angle=i*math.pi/4;direction=Vector((C*math.sin(angle),-C*math.cos(angle),S));cam.location=center+direction*4000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();p=folder/f'{mode}-{i}.png';scene.render.filepath=str(p);bpy.ops.render.render(write_still=True);im=Image.open(p).convert('RGB');sheet.paste(im,((i%4)*256,(i//4)*276+20));draw.text(((i%4)*256+4,(i//4)*276+3),'Original camera'if i==0 else f'View {i+1}',fill='white')
  sheet.save(folder/f'{mode}-eight.png')
 scene.view_layers[0].material_override=None;bpy.data.objects.remove(cam,do_unlink=True);bpy.data.objects.remove(light,do_unlink=True)

def main():
 assert not DEST.exists();DEST.mkdir();guard();rows=json.loads((q.OUT/'qualification.json').read_text())['rows'];anchors=json.loads((q.OUT/'depth-proposal-v1.json').read_text());summary=[]
 for n,row in enumerate(rows,1):
  if n not in PROFILES:continue
  guard();bpy.ops.wm.read_factory_settings(use_empty=True);folder=DEST/f'profile-{n:02}';folder.mkdir();frames=row['art']['states']['initial']['frames']+row['art']['states']['transition']['frames'];records=[]
  for phase,f in zip(['initial','transition-0','transition-1','applied'],frames):
   obj,record=make(row,n,phase,f,anchors[str(n)]);obj.hide_render=phase!='initial';obj.hide_viewport=phase!='initial';records.append(record)
  bpy.context.preferences.filepaths.save_version=0;path=folder/'model.blend';bpy.ops.wm.save_as_mainfile(filepath=str(path),compress=True);modelpin=q.pin(path)
  # Reopen the saved model before checking every requested view.
  bpy.ops.wm.open_mainfile(filepath=str(path));objects=[o for o in bpy.context.scene.objects if o.type=='MESH']
  for phase in ['initial','applied']:
   obj=next(o for o in objects if o['source_phase']==phase)
   for other in objects:other.hide_render=other!=obj;other.hide_viewport=other!=obj
   review=folder/phase;review.mkdir();render(obj,review)
  assert q.pin(path)==modelpin;summary.append(dict(model=modelpin,states=records,review='Saved-model eight views generated; self-review and receiver contact acceptance pending.'));(folder/'construction.json').write_text(json.dumps(summary[-1],indent=2)+'\n')
 (DEST/'report.json').write_text(json.dumps(dict(status='PRIVATE_GEOMETRY_REVIEW_PENDING',profiles=summary,limits=['Depth/support/back surfaces are inferred and gray.','Front source samples are preserved; no hidden texture synthesis.','Neighbor alpha-aware contact must be reviewed before geometry acceptance.']),indent=2)+'\n')
if __name__=='__main__':
 acquire(slots=2)
 try:main()
 finally:release()
