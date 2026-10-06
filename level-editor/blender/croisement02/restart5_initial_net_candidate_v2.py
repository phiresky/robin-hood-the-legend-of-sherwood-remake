"""Construct bounded initial net camouflage and source-constrained lifting rigging."""
import sys,json,math,shutil
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import RAY,SIN,COS
from refinement_review import _tree
from render_slots import acquire,release
ROOT=OUT/'restart5-initial-nets'
def point(x,y,z):return Vector((x,-(y+z*COS)/SIN,z))
def tube_path(points,radius=.65,sides=8,closed=False):
 vs=[];fs=[];count=len(points)
 for i,p in enumerate(points):
  tangent=(points[(i+1)%count]-points[(i-1)%count]) if closed else points[min(i+1,count-1)]-points[max(0,i-1)];tangent.normalize();helper=Vector((1,0,0)) if abs(tangent.x)<.85 else Vector((0,1,0));u=tangent.cross(helper).normalized();v=tangent.cross(u)
  vs.extend(tuple(p+radius*(u*math.cos(j*math.tau/sides)+v*math.sin(j*math.tau/sides)))for j in range(sides))
 for i in range(count if closed else count-1):
  for j in range(sides):fs.append((i*sides+j,i*sides+(j+1)%sides,((i+1)%count)*sides+(j+1)%sides,((i+1)%count)*sides+j))
 if not closed:fs.extend([tuple(range(sides-1,-1,-1)),tuple((count-1)*sides+j for j in range(sides))])
 return vs,fs

def ground_mesh(mask,origin):
 h,w=mask.shape;vs=[];fs=[];indices={}
 def vertex(x,y,cell,top):
  adjacent=[(x-1,y-1),(x,y-1),(x,y),(x-1,y)];occupied=[0<=cx<w and 0<=cy<h and mask[cy,cx] for cx,cy in adjacent];ci=adjacent.index(cell)
  # Diagonally touching leaf fringes retain separate manifold vertices.
  component=ci if sum(occupied)==2 and occupied[(ci+2)%4] else -1;key=(x,y,component,top)
  if key not in indices:
   z=.12 if not top else 1.2+.45*math.sin(x*.35+y*.25)+.25*math.cos(x*.65-y*.3)
   indices[key]=len(vs);vs.append(tuple(point(origin[0]+x,origin[1]+y,z)))
  return indices[key]
 for y,x in np.argwhere(mask):
  x=int(x);y=int(y);corners=[(x,y),(x+1,y),(x+1,y+1),(x,y+1)];bottom=[vertex(a,b,(x,y),False)for a,b in corners];top=[vertex(a,b,(x,y),True)for a,b in corners];fs.extend([tuple(top),tuple(reversed(bottom))])
  for i,(dx,dy)in enumerate([(0,-1),(1,0),(0,1),(-1,0)]):
   xx=x+dx;yy=y+dy
   if not(0<=xx<w and 0<=yy<h and mask[yy,xx]):fs.append((bottom[i],bottom[(i+1)%4],top[(i+1)%4],top[i]))
 return vs,fs

def collar(obj,z,near):
 obj.data.calc_loop_triangles();segments=[]
 for tri in obj.data.loop_triangles:
  v=[obj.matrix_world@obj.data.vertices[i].co for i in tri.vertices];hits=[]
  for a,b in zip(v,v[1:]+v[:1]):
   if (a.z-z)*(b.z-z)<0:hits.append(a+(b-a)*((z-a.z)/(b.z-a.z)))
  if len(hits)==2:segments.append(hits)
 vertices=[];edges=[]
 def index(p):
  for i,q in enumerate(vertices):
   if (p-q).length<1e-3:return i
  vertices.append(p);return len(vertices)-1
 for a,b in segments:edges.append((index(a),index(b)))
 graph={i:[]for i in range(len(vertices))}
 for a,b in edges:graph[a].append(b);graph[b].append(a)
 start=min(graph,key=lambda i:(vertices[i]-near).length);order=[start];prev=None;current=start
 for _ in range(len(vertices)+1):
  choices=[j for j in graph[current]if j!=prev];assert choices
  nxt=choices[0]
  if nxt==start:break
  order.append(nxt);prev,current=current,nxt
 else:raise ValueError('Unclosed support cross section')
 pts=[vertices[i]for i in order];center=sum(pts,Vector())/len(pts);out=[]
 for p in pts:
  d=p-center;d.z=0;out.append(p+d.normalized()*.85)
 return out

def material(path,gray=False):
 m=bpy.data.materials.new('Unknown initial rigging reverse' if gray else 'Exact initial rigging source');m.use_nodes=True;n=m.node_tree.nodes;n.clear();l=m.node_tree.links;out=n.new('ShaderNodeOutputMaterial');em=n.new('ShaderNodeEmission');em.inputs['Color'].default_value=(.18,.18,.18,1);l.new(em.outputs[0],out.inputs['Surface'])
 if not gray:
  tex=n.new('ShaderNodeTexImage');tex.image=bpy.data.images.load(str(path));tex.image.pack();tex.interpolation='Closest';tex.extension='CLIP';uv=n.new('ShaderNodeUVMap');uv.uv_map='Initial native source projection';l.new(uv.outputs[0],tex.inputs['Vector']);mix=n.new('ShaderNodeMixRGB');mix.inputs[1].default_value=(.18,.18,.18,1);l.new(tex.outputs['Alpha'],mix.inputs[0]);l.new(tex.outputs['Color'],mix.inputs[2]);l.new(mix.outputs[0],em.inputs['Color'])
 return m

def main(key):
 assert shutil.disk_usage(OUT).free>25*1024**3
 plan=json.loads((ROOT/'source/plan.json').read_text());rec=plan['records'][int(key)];treeid=43 if key=='00' else 39;context=json.loads((ROOT/'source/context-selection-initial.json').read_text());r=next(r for r in context if r['asset_id']==f'croisement02-tree-{treeid}');assert sha(Path(r['model']))==r['model_sha256'];bpy.ops.wm.open_mainfile(filepath=r['model']);bpy.context.view_layer.update();woods=[o for o in bpy.data.objects if o.type=='MESH'and o.get('asset_group')==r['asset_id']and o.get('projection_component')!='crown'and 'crown'not in o.name.lower()];tree,owners,_=_tree(woods)
 ox,oy=rec['origin'];mask=np.array(Image.open(ROOT/f'source/profile-{key}-ground-mask.png'))>0;source=Path(rec['source']);assert sha(source)==rec['source_sha256'];image=np.array(Image.open(source).convert('RGBA'));roles=mask|(np.array(Image.open(ROOT/f'source/profile-{key}-rope-mask.png'))>0);image[:,:,3]=np.where(roles,image[:,:,3],0)
 # Actual wood dictates line depth while its native projection follows the drawn line.
 controls=([(84.5,20),(84.5,120),(88.5,147),(85,165),(67,175)] if key=='00'else[(126.5,20),(126.5,130),(130.5,179),(126.5,198),(111,211)])
 rope_mask=np.array(Image.open(ROOT/f'source/profile-{key}-rope-mask.png'))>0
 known_y=np.flatnonzero(rope_mask.any(axis=1));known_x=np.array([np.where(rope_mask[y])[0].mean()+.5 for y in known_y]);path=[];support=None;contact=[]
 for a,b in zip(controls,controls[1:]):
  for t in np.linspace(0,1,max(2,int(abs(b[1]-a[1]))),endpoint=False):
   x,y=np.array(a)*(1-t)+np.array(b)*t
   if y<rec['ground_row_start']:x=float(np.interp(y,known_y+.5,known_x))
   hit,n,idx,dist=tree.ray_cast(point(ox+x,oy+y,0)+RAY*6000,-RAY)
   if hit is not None and y<rec['ground_row_start']+4:
    p=hit+RAY*1.0;contact.append(dict(source=[ox+x,oy+y],wood=owners[idx].name,point=list(hit)))
    if support is None:support=(owners[idx],hit.copy())
   else:
    # The downward continuation returns onto the ground rather than hovering.
    z=max(1.0,(rec['ground_row_start']+5-y)*.9);p=point(ox+x,oy+y,z)
   path.append(p)
 path.append(point(ox+controls[-1][0],oy+controls[-1][1],1.2));assert support is not None
 tie=collar(support[0],support[1].z,path[0]);nearest=min(tie,key=lambda p:(p-path[0]).length);path.insert(0,nearest)
 support_info=dict(asset_id=r['asset_id'],model=r['model'],model_sha256=r['model_sha256'],object=support[0].name,point=list(support[1]),tie_points=[list(p)for p in tie],line_contacts=contact)
 dest=ROOT/f'candidate-v2/profile-{key}';assert not dest.exists();dest.mkdir(parents=True);Image.fromarray(image).save(dest/'observed-source.png')
 bpy.ops.wm.read_factory_settings(use_empty=True);bpy.context.preferences.filepaths.save_version=0;scene=bpy.context.scene;scene.name='Initial net '+key;mat=material(dest/'observed-source.png');gray=material(None,True);records=[];objects=[]
 for name,geometry in [('Ground camouflage net',ground_mesh(mask,(ox,oy))),('Initial lifting line',tube_path(path)),('Inferred upper fastening loop',tube_path(tie,closed=True))]:
  vs,fs=geometry;mesh=bpy.data.meshes.new(name);mesh.from_pydata(vs,[],fs);mesh.update();bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));nonmanifold=sum(not e.is_manifold for e in bm.edges);assert not nonmanifold,(name,nonmanifold);volume=bm.calc_volume();assert volume>0;bm.to_mesh(mesh);bm.free();mesh.update();o=bpy.data.objects.new(name,mesh);scene.collection.objects.link(o);o['asset_group']='croisement02-initial-net-'+('01'if key=='00'else'03');o['source_node']='mission-initial-net-'+key;o['native_profile']=rec['profile'];o['state_role']='initial action0';o['projection_component']=name.lower().replace(' ','_');mesh.materials.append(mat);mesh.materials.append(gray);uv=mesh.uv_layers.new(name='Initial native source projection')
  for p in mesh.polygons:
   p.material_index=0 if p.normal.dot(RAY)>.05 and name!='Inferred upper fastening loop'else 1
   for li in p.loop_indices:
    v=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((v.x-ox)/rec['size'][0],1-(-v.y*SIN-v.z*COS-oy)/rec['size'][1])
  objects.append(o);records.append(dict(name=name,vertices=len(mesh.vertices),faces=len(mesh.polygons),closed_manifold=True,volume=volume,min_z=min(v.co.z for v in mesh.vertices)))
 scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';world=bpy.data.worlds.new('Neutral world');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.7;scene.world=world;light=bpy.data.lights.new('Review sun','SUN');light.energy=2;o=bpy.data.objects.new('Review sun',light);scene.collection.objects.link(o);o.rotation_euler=(.6,-.4,-.4)
 c=bpy.data.cameras.new('Review camera');c.type='ORTHO';c.clip_end=10000;cam=bpy.data.objects.new(c.name,c);scene.collection.objects.link(cam);scene.camera=cam;pts=[o.matrix_world@v.co for o in objects for v in o.data.vertices];center=(Vector(tuple(min(p[i]for p in pts)for i in range(3)))+Vector(tuple(max(p[i]for p in pts)for i in range(3))))/2;c.ortho_scale=max((p-center).length for p in pts)*2.25;cam.location=center+RAY*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();bpy.ops.wm.save_as_mainfile(filepath=str(dest/'model.blend'));h=sha(dest/'model.blend');solid=bpy.data.materials.new('Solid physical review');solid.diffuse_color=(.5,.5,.5,1);solid.use_nodes=True
 cameras=[]
 for i in range(8):
  a=-math.pi/2+i*math.pi/4;direction=Vector((math.cos(a)*COS,math.sin(a)*COS,SIN));cam.location=center+direction*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();cameras.append(dict(view=i,direction=list(direction),location=list(cam.location),ortho_scale=c.ortho_scale))
  for mode in ['actual','solid']:
   scene.view_layers[0].material_override=solid if mode=='solid'else None;scene.render.filepath=str(dest/f'view-{i}-{mode}.png');bpy.ops.render.render(write_still=True)
 for mode in ['actual','solid']:
  sheet=Image.new('RGB',(1536,768),(30,30,30))
  for i in range(8):
   im=Image.open(dest/f'view-{i}-{mode}.png').convert('RGBA');sheet.paste(im,((i%4)*384,(i//4)*384),im)
  sheet.save(dest/f'{mode}-sheet.png')
 scene.view_layers[0].material_override=None;scene.render.resolution_x=rec['size'][0]*3;scene.render.resolution_y=rec['size'][1]*3;target=point(ox+rec['size'][0]/2,oy+rec['size'][1]/2,0);cam.location=target+RAY*3000;cam.rotation_euler=(target-cam.location).to_track_quat('-Z','Y').to_euler();c.ortho_scale=rec['size'][1];scene.render.filepath=str(dest/'native.png');bpy.ops.render.render(write_still=True);src=Image.open(source).convert('RGBA').resize((rec['size'][0]*3,rec['size'][1]*3),Image.Resampling.NEAREST);after=Image.open(dest/'native.png').convert('RGBA');comp=Image.new('RGB',(src.width*2,src.height),(70,70,70));comp.paste(src,(0,0),src);comp.paste(after,(src.width,0),after);comp.save(dest/'source-comparison.png')
 write_json(dest/'report.json',dict(status='Private initial geometry; source/contact/root review pending',model_sha256=h,source=rec,support=support_info,objects=records,cameras=cameras,source_plan_sha256=sha(ROOT/'source/plan.json'),limitations=['Ground net thickness/camouflage relief and lifting cord depth inferred from source; closed meshes retain native source projection.','Sparse high fragments remain native-only ambiguous; no floating geometry or tree ownership is fabricated.','Upper fastening loop is an explicit physical attachment inference to unchanged actual tree wood.','Unknown reverse surfaces are gray; no API or texture approval.','Posttrigger e/i models and canonical state registry are unchanged.']))
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
