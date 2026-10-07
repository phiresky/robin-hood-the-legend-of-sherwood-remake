"""Bounded complete bank/ramp prototype with per-face native first-hit seed ownership."""
import sys, math, json, shutil
from pathlib import Path
import bpy, numpy as np
from PIL import Image
from mathutils import Vector
from mathutils.geometry import tessellate_polygon
from mathutils.bvhtree import BVHTree
R=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(R/'level-editor/refinement'),str(R/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
from render_views import render_views
B=R/'level-editor/work/croisement03-refinement';O=B/'restart2/bank-full-prototype-v2'
S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=Vector((0,-C,S))

def world(o):return world(o.parent)@o.matrix_parent_inverse@o.matrix_basis if o.parent else o.matrix_basis.copy()

def mesh_object(name,points,scene):
 top=[Vector((p['x'],-p['y']/S,p['z_top']/C)) for p in points];bottom=[Vector((p.x,p.y,0)) for p in top];n=len(top)
 indices={tuple(v):i for i,v in enumerate(top)};faces=[]
 for tri in tessellate_polygon([top]):
  face=tuple(v if isinstance(v,int) else indices[tuple(v)] for v in tri)
  if (top[face[1]]-top[face[0]]).cross(top[face[2]]-top[face[0]]).z<0:face=face[::-1]
  faces.extend([face,tuple(i+n for i in face[::-1])])
 # Outward side winding follows the signed XY area.
 area=sum(top[i].x*top[(i+1)%n].y-top[(i+1)%n].x*top[i].y for i in range(n))
 for i in range(n):
  j=(i+1)%n;face=(i,j,j+n,i+n)
  faces.append(face if area<0 else face[::-1])
 mesh=bpy.data.meshes.new(name);mesh.from_pydata(top+bottom,[],faces);mesh.update();o=bpy.data.objects.new(name,mesh);scene.collection.objects.link(o)
 return o

def main():
 assert shutil.disk_usage(R).free>10*1024**3+32*1024**2
 O.mkdir(exist_ok=False);acquire()
 try:
  lp=B/'baseline/Croisement03.rhp.json';sp=B/'baseline/covered.png';base=B/'baseline/croisement03-baseline.blend';profilep=B/'restart2/tree02-ridge-ray-guard-v4/receipt.json';seedp=B/'restart2/bank-exposed-rock-proposal-v1/candidate-rock-seeds.png'
  pins={str(p):sha(p) for p in (lp,sp,base,profilep,seedp)};level=json.loads(lp.read_text());profile=json.loads(profilep.read_text())['ridge_profile'];source=np.array(Image.open(sp).convert('RGBA'));seed=np.array(Image.open(seedp))>0
  bpy.ops.wm.read_factory_settings(use_empty=True);bpy.context.preferences.filepaths.save_version=0;s=bpy.context.scene;s.name='C3 full bank prototype';s.render.engine='CYCLES';s.cycles.samples=8;s.cycles.transparent_max_bounces=256;s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_x=384;s.render.resolution_y=384;s.render.resolution_percentage=100;s.render.film_transparent=True;s.view_settings.view_transform='Standard';s.view_settings.look='None'
  s.world=bpy.data.worlds.new('Neutral world');s.world.color=(.18,.18,.18)
  sun=bpy.data.lights.new('Review sunlight','SUN');sun.energy=2;light=bpy.data.objects.new(sun.name,sun);s.collection.objects.link(light);light.rotation_euler=Vector((-.45,-.55,.7)).to_track_quat('Z','Y').to_euler()
  unknown=bpy.data.materials.new('Unclassified inferred bank surface');unknown.diffuse_color=(.25,.25,.25,1);unknown.use_nodes=True;unknown.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.25,.25,.25,1)
  own=[]
  for index in (52,53,54):
   points=json.loads(json.dumps(level['sight_obstacles'][index]['points']))
   if index==52:points=[points[0]]+[dict(x=x,y=my,z_top=85) for x,sy,my in profile]+points[3:]
   o=mesh_object(f'Candidate bank {index}',points,s);o['native_obstacle']=index;o['gameplay_metadata_json']=json.dumps(level['sight_obstacles'][index],sort_keys=True);o.data.materials.append(unknown);o['appearance_status']='Only proposed per-face first-hit rock seeds; all other surfaces unclassified';own.append(o)
  # Append only unchanged west-path context; preserve its hierarchy-composed transform.
  names=[f'building-{i:03}.001' for i in (94,95,96,97)]
  with bpy.data.libraries.load(str(base),link=False) as (a,b):
   assert all(n in a.objects for n in names);b.objects=names
  context=[o for o in b.objects if o];transforms={o:world(o) for o in context}
  for o in context:o.parent=None;o.matrix_world=transforms[o];o.hide_render=False;s.collection.objects.link(o)
  bpy.context.view_layer.update();rows=[]
  for o in own:
   o.data.calc_loop_triangles();tris=list(o.data.loop_triangles);vs=[v.co.copy() for v in o.data.vertices];rows.append((o,BVHTree.FromPolygons(vs,[list(t.vertices) for t in tris],all_triangles=True),tris))
  assignments={};misses=[]
  for y,x in zip(*np.nonzero(seed)):
   origin=Vector((x+.5,-(y+.5)/S,0))+RAY*5000;hits=[]
   for o,bvh,tris in rows:
    p,n,f,d=bvh.ray_cast(origin,-RAY)
    if p is not None:hits.append((d,o.name,tris[f].polygon_index))
   if not hits:misses.append([int(x),int(y)]);continue
   _,name,face=min(hits);assignments.setdefault((name,face),[]).append((int(x),int(y)))
  for (name,face),pixels in assignments.items():
   o=bpy.data.objects[name];xs,ys=zip(*pixels);left,top,right,bottom=min(xs),min(ys),max(xs)+1,max(ys)+1;rgba=source[top:bottom,left:right].copy();rgba[:,:,3]=0
   for x,y in pixels:rgba[y-top,x-left,3]=255
   path=O/f'{name.replace(" ","-")}-face-{face}.png';Image.fromarray(rgba).save(path);image=bpy.data.images.load(str(path));image.pack();mat=bpy.data.materials.new(f'Proposed first-hit source {name}/{face}');mat.use_nodes=True;n=mat.node_tree.nodes;n.clear();tex=n.new('ShaderNodeTexImage');tex.image=image;tex.interpolation='Closest';tex.extension='CLIP';em=n.new('ShaderNodeEmission');mat.node_tree.links.new(tex.outputs['Color'],em.inputs['Color']);bs=n.new('ShaderNodeBsdfPrincipled');bs.inputs['Base Color'].default_value=(.25,.25,.25,1);mix=n.new('ShaderNodeMixShader');out=n.new('ShaderNodeOutputMaterial');links=mat.node_tree.links;links.new(tex.outputs['Alpha'],mix.inputs[0]);links.new(bs.outputs[0],mix.inputs[1]);links.new(em.outputs[0],mix.inputs[2]);links.new(mix.outputs[0],out.inputs['Surface']);o.data.materials.append(mat);o.data.polygons[face].material_index=len(o.data.materials)-1
   uv=o.data.uv_layers.active or o.data.uv_layers.new(name='Native seed coordinates')
   for li in o.data.polygons[face].loop_indices:
    v=o.data.vertices[o.data.loops[li].vertex_index].co;uv.data[li].uv=((v.x-left)/(right-left),1-(-v.y*S-v.z*C-top)/(bottom-top))
  points=[o.matrix_world@v.co for o in own+context for v in o.data.vertices];lo=Vector(tuple(min(p[i] for p in points) for i in range(3)));hi=Vector(tuple(max(p[i] for p in points) for i in range(3)));target=(lo+hi)/2;views={};scales=[]
  for i in range(8):
   a=i*math.tau/8;direction=Vector((math.sin(a)*C,-math.cos(a)*C,S));camdata=bpy.data.cameras.new(f'Bank view{i}');camdata.type='ORTHO';cam=bpy.data.objects.new(camdata.name,camdata);s.collection.objects.link(cam);cam.location=target+direction*2500;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();bpy.context.view_layer.update();inv=cam.matrix_world.inverted();local=[inv@p for p in points];ds=[-p.z for p in local];camdata.clip_start=min(ds)-25;camdata.clip_end=max(ds)+25;scales.append(2*max(max(abs(p.x),abs(p.y)) for p in local)+40);views[f'view-{i}']=cam.name
  for name in views.values():s.objects[name].data.ortho_scale=max(scales)
  s.camera=s.objects[views['view-0']];bpy.ops.wm.save_as_mainfile(filepath=str(O/'worker.blend'));assert (O/'worker.blend').stat().st_size<8*1024**2;modelhash=sha(O/'worker.blend');bpy.ops.wm.open_mainfile(filepath=str(O/'worker.blend'));s=bpy.data.scenes['C3 full bank prototype'];render_views(s.name,views,O/'views',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(O/'views'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(O/f'{mode}8.png')
  assert sha(O/'worker.blend')==modelhash;assert all(sha(Path(p))==h for p,h in pins.items());size=sum(p.stat().st_size for p in O.rglob('*') if p.is_file());assert size<32*1024**2
  write_json(O/'receipt.json',dict(status='Private full-bank support prototype; visual and source-neighbor guards pending',model_sha256=modelhash,input_hashes=pins,owned_nodes=[52,53,54],unchanged_context_nodes=[94,95,96,97],seed_pixels=int(seed.sum()),assigned_pixels=sum(map(len,assignments.values())),seed_misses=misses,assigned_face_count=len(assignments),bytes=size,limits=['Native bank source seeds are proposals, not final source ownership.','Coarse complete side walls retain shelf planes but rock strata and ivy separation still require refinement.','Source tree first-hit constraints and native exact-camera comparison are pending.','No gameplay or canonical data written.']))
 finally:release()
if __name__=='__main__':main()
