"""Private joint stem/crown construction with explicit provisional foliage domain."""
import sys,math,json,random,shutil,hashlib
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'tree05-crown-prototype-v2';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN));UP=Vector((0,SIN,COS))
def leafmaterial(im,name):
 m=bpy.data.materials.new(name);m.use_nodes=True;n=m.node_tree.nodes;n.clear();l=m.node_tree.links;t=n.new('ShaderNodeTexImage');t.image=im;t.interpolation='Closest';t.extension='CLIP';e=n.new('ShaderNodeEmission');e.inputs[1].default_value=1;tr=n.new('ShaderNodeBsdfTransparent');mix=n.new('ShaderNodeMixShader');out=n.new('ShaderNodeOutputMaterial');l.new(t.outputs['Color'],e.inputs[0]);l.new(t.outputs['Alpha'],mix.inputs[0]);l.new(tr.outputs[0],mix.inputs[1]);l.new(e.outputs[0],mix.inputs[2]);l.new(mix.outputs[0],out.inputs[0]);return m

def main():
 assert shutil.disk_usage(ROOT).free>25*1024**3;OUT.mkdir(exist_ok=False);acquire()
 try:
  src=B/'tree05-wood-prototype-v1/worker.blend';bpy.ops.wm.open_mainfile(filepath=str(src));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene;stems=[o for o in scene.objects if o.type=='MESH'];assert len(stems)==4;beforeimages={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};records=[]
  # Keep source-facing portion exactly; shorten only entirely off-raster continuation.
  for o in stems:
   z0=max(v.co.z for v in o.data.vertices if -v.co.y*SIN-v.co.z*COS>=-.01);changed=0
   for v in o.data.vertices:
    sy=-v.co.y*SIN-v.co.z*COS
    if sy<-.01:v.co.z=z0+(v.co.z-z0)*.35;changed+=1
   o.data.update();records.append(dict(node=o['source_node'],observed_vertices_unchanged=True,offmap_vertices_shortened=changed))
  # Short inferred forks connect preserved stems into the crown wholly beyond the raster.
  branch_records=[]
  for stem in stems:
   if stem['source_node']=='building-021':continue
   rings={}
   for v in stem.data.vertices:rings.setdefault(round(v.co.z,4),[]).append(v.co.copy())
   z=max(rings);start=sum(rings[z],Vector())/len(rings[z]);node=int(stem['stem_component'])
   for sign in (-1,1):
    end=start+Vector((sign*18,sign*12,30));axis=(end-start).normalized();u=axis.cross(Vector((0,0,1))).normalized();v=axis.cross(u);vs=[];fs=[]
    for j,(t,r) in enumerate(((0,1.7),(.45,1.1),(1,.2))):
     c=start.lerp(end,t)
     for k in range(8):vs.append(tuple(c+(u*math.cos(k*math.tau/8)+v*math.sin(k*math.tau/8))*r))
    fs.append(tuple(reversed(range(8))))
    for j in range(2):
     for k in range(8):fs.append((j*8+k,j*8+(k+1)%8,(j+1)*8+(k+1)%8,(j+1)*8+k))
    fs.append(tuple(range(16,24)));m=bpy.data.meshes.new('Inferred crown fork');m.from_pydata(vs,[],fs);m.update();m.materials.append(stem.data.materials[-1]);uv=m.uv_layers.new(name='UVMap')
    for face in m.polygons:
     face.use_smooth=len(face.vertices)==4
     for li in face.loop_indices:uv.data[li].uv=(-1,-1)
    obj=bpy.data.objects.new(f'Inferred crown fork {node}-{sign}',m);scene.collection.objects.link(obj);obj['asset_group']='croisement03-tree-05';obj['source_node']=stem['source_node'];obj['inferred_branch']=True;maxsy=max(-p[1]*SIN-p[2]*COS for p in vs);assert maxsy<0;branch_records.append(dict(node=node,max_native_source_y=maxsy))
  imagepath=B/'tree05-canopy-fragment-source-v1/000.png';im=bpy.data.images.load(str(imagepath),check_existing=False);im.pack();mat=leafmaterial(im,'Arbre07 fragment provisional native source foliage');pixels=np.array(Image.open(imagepath));native_union=np.logical_or.reduce([np.array(Image.open(q).convert('RGBA'))[:,:,3]>0 for q in sorted(imagepath.parent.glob('???.png'))]);vertices=[];faces=[];uvs=[]
  # One native-facing cell per local source block; spacing and RGB remain native.
  for yy in range(0,154,3):
   for xx in range(0,79,3):
    x1,y1=min(xx+3,79),min(yy+3,154)
    if not np.any(native_union[yy:y1,xx:x1]):continue
    corners=[(xx,yy),(x1,yy),(x1,y1),(xx,y1)];ids=[];depth=-435-42*math.sin(xx*.127+yy*.079)-17*math.cos(xx*.263-yy*.117)
    for x,y in corners:
     ids.append(len(vertices));vertices.append((492+x,depth,(-depth*SIN-y)/COS));uvs.append((x/79,1-y/154))
    faces.append(ids)
  native_faces=len(faces);rng=random.Random(13075);py,px=np.nonzero(native_union);protected_centers=np.column_stack((492+px+.5,py+.5));crossing_raster_ambiguities_omitted=0
  # Small crossing leaflets add local volume while remaining edge-on to native rays.
  for f in list(faces):
   center=sum((Vector(vertices[i]) for i in f),Vector())/4;tex=[uvs[i] for i in f]
   for step in range(1,4):
    a=rng.uniform(0,math.tau);c=center-RAY*rng.uniform(8,110)+Vector((rng.uniform(-3,3),0,0))+UP*rng.uniform(-3,3);u=(Vector((math.cos(a),0,0))+UP*math.sin(a))*rng.uniform(3,7);v=RAY*rng.uniform(3,7);ids=[]
    q0=c-u/2;q1=c+u/2;a2=np.array([q0.x,-q0.y*SIN-q0.z*COS]);b2=np.array([q1.x,-q1.y*SIN-q1.z*COS]);d2=b2-a2;parameter=np.clip(((protected_centers-a2)@d2)/np.dot(d2,d2),0,1);dist=np.linalg.norm(protected_centers-(a2+parameter[:,None]*d2),axis=1)
    if np.any(dist<.005):crossing_raster_ambiguities_omitted+=1;continue
    for p,t in zip((c-u/2-v/2,c+u/2-v/2,c+u/2+v/2,c-u/2+v/2),tex):ids.append(len(vertices));vertices.append(tuple(p));uvs.append(t)
    faces.append(ids)
  # Irregular small crossed clusters form inferred off-map volume; no external leaf examples.
  patches=[]
  for yy in range(0,147,7):
   for xx in range(0,72,7):
    if np.count_nonzero(pixels[yy:yy+7,xx:xx+7,3])>=12:patches.append((xx,yy))
  for k in range(3200):
   lobe=k%4;cx,cy,cz=((505,-365,355),(528,-350,370),(548,-330,345),(562,-330,330))[lobe];dx,dy,dz=(rng.uniform(-1,1) for _ in range(3))
   if dx*dx+dy*dy+dz*dz>1:continue
   center=Vector((cx+dx*43,cy+dy*90,cz+dz*65));sy=-center.y*SIN-center.z*COS
   # In-map unseen RGB is not added; inferred cards stay above the original raster.
   if sy>-1:continue
   size=rng.uniform(6,12);a=rng.uniform(0,math.tau);u=Vector((math.cos(a),math.sin(a),0))*size;v=Vector((-math.sin(a)*.45,math.cos(a)*.45,.89))*size;px,py=rng.choice(patches);ids=[]
   for p,t in zip((center-u/2-v/2,center+u/2-v/2,center+u/2+v/2,center-u/2+v/2),((px/79,1-(py+7)/154),((px+7)/79,1-(py+7)/154),((px+7)/79,1-py/154),(px/79,1-py/154))):
    projected=-p.y*SIN-p.z*COS
    if projected>-.05:p.z+=(projected+.05)/COS
    ids.append(len(vertices));vertices.append(tuple(p));uvs.append(t)
   faces.append(ids)
  # Complete clipped fragment edges behind every observed native leaf cell.
  # Each small lobe is irregular in three dimensions and tied to a lower support.
  boundary_lobes=((493,25,12,25),(568,32,12,26),(493,70,13,26),(568,80,13,25),(507,127,13,23),(554,136,13,24),(511,-15,16,25),(548,-25,17,29))
  by,bx=np.nonzero((np.array(Image.open(B/'tree05-bark-proposal-v2/proposed-bark.png'))>0)|(np.array(Image.open(B/'tree06-bark-proposal-v3/proposed-bark.png'))>0));bark_centers=np.column_stack((bx+.5,by+.5));boundary_faces=0;boundary_bark_exclusions=0
  for li,(gx,gy,rw,rh) in enumerate(boundary_lobes):
   for k in range(170):
    dx,dy,dz=(rng.uniform(-1,1) for _ in range(3))
    if dx*dx+dy*dy+dz*dz>1:continue
    center=Vector((gx+rw*dx,-330+dy*42,0));source_y=gy+rh*dz
    center.z=(-center.y*SIN-source_y)/COS
    size=rng.uniform(4,8);a=rng.uniform(0,math.tau);b=rng.uniform(-.8,.8)
    u=Vector((math.cos(a),math.sin(a),b)).normalized()*size
    v=u.cross(Vector((rng.uniform(-1,1),rng.uniform(-1,1),1))).normalized()*size*rng.uniform(.7,1.2)
    tx,ty=rng.choice(patches);ids=[]
    quad=(center-u/2-v/2,center+u/2-v/2,center+u/2+v/2,center-u/2+v/2)
    projected=np.array([(p.x,-p.y*SIN-p.z*COS) for p in quad]);low=projected.min(axis=0)-.01;high=projected.max(axis=0)+.01
    if np.any(np.all((bark_centers>=low)&(bark_centers<=high),axis=1)):
     boundary_bark_exclusions+=1;continue
    for p,t in zip(quad,((tx/79,1-(ty+7)/154),((tx+7)/79,1-(ty+7)/154),((tx+7)/79,1-ty/154),(tx/79,1-ty/154))):
     ids.append(len(vertices));vertices.append(tuple(p));uvs.append(t)
    faces.append(ids);boundary_faces+=1
  mesh=bpy.data.meshes.new('Arbre07 fragment native cells and inferred crown clusters');mesh.from_pydata(vertices,[],faces);mesh.update();uv=mesh.uv_layers.new(name='UVMap')
  for f in mesh.polygons:
   for li in f.loop_indices:uv.data[li].uv=uvs[mesh.loops[li].vertex_index]
  mesh.materials.append(mat);foliage=bpy.data.objects.new('Arbre07 fragment provisional crown context',mesh);scene.collection.objects.link(foliage);foliage['asset_group']='croisement03-arbre07-fragment-tree05-provisional';foliage['source_ownership']='Exact frame0 fragment; shared runtime animation ownership unchanged'
  tree='05';x0=492;base=-435;support_records=[];pixels=np.array(Image.open(B/'tree05-canopy-fragment-source-v1/000.png'));py,px=np.nonzero(pixels[:,:,3]);
  for j,(xx,yy) in enumerate(((5,22),(73,27),(38,45),(4,66),(75,71),(20,130),(59,137))):
   near=np.argmin((px-xx)**2+(py-yy)**2)
   if (px[near]-xx)**2+(py[near]-yy)**2>100:continue
   xx,yy=float(px[near])+.5,float(py[near])+.5
   depth=base-42*math.sin(xx*.127+yy*.079)-17*math.cos(xx*.263-yy*.117);leaf=Vector((x0+xx,depth,(-depth*SIN-yy)/COS));end=leaf-RAY*105;stem=min([o for o in stems if o['source_node']!='building-021'],key=lambda o:abs(sum(v.co.x for v in o.data.vertices)/len(o.data.vertices)-end.x));levels={}
   for v in stem.data.vertices:levels.setdefault(round(v.co.z,3),[]).append(v.co.copy())
   z=min(levels,key=lambda z:abs(z-(end.z-20)));start=sum(levels[z],Vector())/len(levels[z]);
   if start.z<20:continue
   mid=start.lerp(end,.6)+Vector((0,0,5))-RAY*15;axis=(end-start).normalized();u=axis.cross(Vector((0,0,1))).normalized();v=axis.cross(u);vertices=[];faces=[]
   for c,r in ((start,1.65),(mid,.9),(end,.18)):
    for k in range(8):vertices.append(tuple(c+(u*math.cos(k*math.tau/8)+v*math.sin(k*math.tau/8))*r))
   faces.append(tuple(reversed(range(8))))
   for ring in range(2):
    for k in range(8):faces.append((ring*8+k,ring*8+(k+1)%8,(ring+1)*8+(k+1)%8,(ring+1)*8+k))
   faces.append(tuple(range(16,24)));mesh=bpy.data.meshes.new('Inferred lower crown support');mesh.from_pydata(vertices,[],faces);mesh.update();mesh.materials.append(stem.data.materials[-1]);uv=mesh.uv_layers.new(name='UVMap')
   for f in mesh.polygons:
    f.use_smooth=len(f.vertices)==4
    for li in f.loop_indices:uv.data[li].uv=(-1,-1)
   o=bpy.data.objects.new(f'Inferred cluster support {j}',mesh);scene.collection.objects.link(o);o['asset_group']=f'croisement03-tree-{tree}';o['inferred_branch']=True;o['source_node']=stem['source_node'];support_records.append(dict(start=list(start),end=list(end),source_projection=[x0+xx,yy],inferred=True))
  assert beforeimages=={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.name in beforeimages}
  target=Vector((535,-350,190))
  for i in range(8):
   cam=bpy.data.objects[f'Tree13 view{i}'];a=i*math.tau/8;direction=Vector((math.sin(a)*COS,-math.cos(a)*COS,SIN));cam.location=target+direction*1000;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=480
  assert all(o.get('asset_group') in {'croisement03-tree-05','croisement03-arbre07-fragment-tree05-provisional'} for o in scene.objects if o.type=='MESH')
  scene.cycles.transparent_max_bounces=128;bpy.data.libraries.write(str(OUT/'worker.blend'),{scene},fake_user=True,compress=True);render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},OUT/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    p=Image.open(OUT/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',p.size,'#333333');bg.alpha_composite(p);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(OUT/'actual'/f'{mode}.png')
  write_json(OUT/'receipt.json',dict(status='PRIVATE prototype only; full native/contact audit and neighbouring context pending',model_sha256=sha(OUT/'worker.blend'),source_model_sha256=sha(src),records=records,inferred_branches=branch_records,inferred_lower_supports=support_records,original_image_rgba_exact=True,provisional_native_leaf_pixels=int(np.count_nonzero(pixels[:,:,3])),native_faces=native_faces,native_14frame_union_pixels=int(native_union.sum()),crossing_raster_ambiguities_omitted=crossing_raster_ambiguities_omitted,inferred_leaf_faces=len(foliage.data.polygons)-native_faces,boundary_completion_faces=boundary_faces,boundary_bark_exclusions=boundary_bark_exclusions,boundary_lobes=boundary_lobes,native_view_index=0,limits=['Exact dynamic frame0 spatial fragment, not static exclusive ownership; complete runtime Arbre07 untouched.','Unknown bark remains gray.','Fourteen-frame union supports native-facing cells; wind and global dynamic ordering remain unproven. Adjacent trees4/6 and ivy remain separate context.','Short inferred forks support complete off-map crown. Native/context/convergence review required; no geometry approval implied.']))
 finally:release()
if __name__=='__main__':main()
