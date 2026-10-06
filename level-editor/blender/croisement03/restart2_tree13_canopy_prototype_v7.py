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
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'tree13-canopy-prototype-v7';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN));UP=Vector((0,SIN,COS))
def leafmaterial(im,name):
 m=bpy.data.materials.new(name);m.use_nodes=True;n=m.node_tree.nodes;n.clear();l=m.node_tree.links;t=n.new('ShaderNodeTexImage');t.image=im;t.interpolation='Closest';t.extension='CLIP';e=n.new('ShaderNodeEmission');e.inputs[1].default_value=1;tr=n.new('ShaderNodeBsdfTransparent');mix=n.new('ShaderNodeMixShader');out=n.new('ShaderNodeOutputMaterial');l.new(t.outputs['Color'],e.inputs[0]);l.new(t.outputs['Alpha'],mix.inputs[0]);l.new(tr.outputs[0],mix.inputs[1]);l.new(e.outputs[0],mix.inputs[2]);l.new(mix.outputs[0],out.inputs[0]);return m

def main():
 assert shutil.disk_usage(ROOT).free>25*1024**3;OUT.mkdir(exist_ok=False);acquire()
 try:
  src=B/'tree13-wood-v6/worker.blend';bpy.ops.wm.open_mainfile(filepath=str(src));scene=bpy.data.scenes['Tree13 isolated wood'];bpy.context.window.scene=scene;stems=[o for o in scene.objects if o.type=='MESH'];assert len(stems)==3;beforeimages={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.has_data};records=[]
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
   rings={}
   for v in stem.data.vertices:rings.setdefault(round(v.co.z,4),[]).append(v.co.copy())
   z=min(rings,key=lambda z:abs(z-145));start=sum(rings[z],Vector())/len(rings[z]);node=int(stem['source_node'].rsplit('-',1)[1])
   for sign in (-1,1):
    end=start+Vector((sign*18,sign*12,30));axis=(end-start).normalized();u=axis.cross(Vector((0,0,1))).normalized();v=axis.cross(u);vs=[];fs=[]
    for j,(t,r) in enumerate(((0,1.7),(.45,1.1),(1,.2))):
     c=start.lerp(end,t)
     for k in range(8):vs.append(tuple(c+(u*math.cos(k*math.tau/8)+v*math.sin(k*math.tau/8))*r))
    fs.append(tuple(reversed(range(8))))
    for j in range(2):
     for k in range(8):fs.append((j*8+k,j*8+(k+1)%8,(j+1)*8+(k+1)%8,(j+1)*8+k))
    fs.append(tuple(range(16,24)));m=bpy.data.meshes.new('Inferred crown fork');m.from_pydata(vs,[],fs);m.update();m.materials.append(stem.data.materials[-1]);uv=m.uv_layers.new(name=f'Observed bark {node:03}')
    for face in m.polygons:
     face.use_smooth=len(face.vertices)==4
     for li in face.loop_indices:uv.data[li].uv=(-1,-1)
    obj=bpy.data.objects.new(f'Inferred crown fork {node}-{sign}',m);scene.collection.objects.link(obj);obj['asset_group']='croisement03-tree-13';obj['source_node']=stem['source_node'];obj['inferred_branch']=True;maxsy=max(-p[1]*SIN-p[2]*COS for p in vs);assert maxsy<0;branch_records.append(dict(node=node,max_native_source_y=maxsy))
  imagepath=B/'tree13-canopy-context-v1/provisional75-excluding-known-bark.png';im=bpy.data.images.load(str(imagepath),check_existing=False);im.pack();mat=leafmaterial(im,'Local75 provisional native source foliage');pixels=np.array(Image.open(imagepath));vertices=[];faces=[];uvs=[]
  # One native-facing cell per local source block; spacing and RGB remain native.
  for yy in range(0,57,7):
   for xx in range(0,126,7):
    x1,y1=min(xx+7,126),min(yy+7,57)
    if not np.any(pixels[yy:y1,xx:x1,3]):continue
    corners=[(xx,yy),(x1,yy),(x1,y1),(xx,y1)];ids=[];depth=-225-22*math.sin(xx*.081+yy*.043)
    for x,y in corners:
     ids.append(len(vertices));vertices.append((1006+x,depth,(-depth*SIN-y)/COS));uvs.append((x/126,1-y/57))
    faces.append(ids)
  native_faces=len(faces);rng=random.Random(13075);py,px=np.nonzero(pixels[:,:,3]);protected_centers=np.column_stack((1006+px+.5,py+.5));crossing_raster_ambiguities_omitted=0
  # Small crossing leaflets add local volume while remaining edge-on to native rays.
  for f in list(faces):
   center=sum((Vector(vertices[i]) for i in f),Vector())/4;tex=[uvs[i] for i in f]
   for step in range(1,7):
    a=rng.uniform(0,math.tau);c=center-RAY*rng.uniform(3,43);u=(Vector((math.cos(a),0,0))+UP*math.sin(a))*rng.uniform(4,8);v=RAY*rng.uniform(4,8);ids=[]
    q0=c-u/2;q1=c+u/2;a2=np.array([q0.x,-q0.y*SIN-q0.z*COS]);b2=np.array([q1.x,-q1.y*SIN-q1.z*COS]);d2=b2-a2;parameter=np.clip(((protected_centers-a2)@d2)/np.dot(d2,d2),0,1);dist=np.linalg.norm(protected_centers-(a2+parameter[:,None]*d2),axis=1)
    if np.any(dist<.005):crossing_raster_ambiguities_omitted+=1;continue
    for p,t in zip((c-u/2-v/2,c+u/2-v/2,c+u/2+v/2,c-u/2+v/2),tex):ids.append(len(vertices));vertices.append(tuple(p));uvs.append(t)
    faces.append(ids)
  # Irregular small crossed clusters form inferred off-map volume; no external leaf examples.
  patches=[]
  for yy in range(0,50,7):
   for xx in range(0,119,7):
    if np.count_nonzero(pixels[yy:yy+7,xx:xx+7,3])>=12:patches.append((xx,yy))
  for k in range(1000):
   lobe=k%3;cx,cy,cz=((1040,-170,158),(1067,-163,180),(1093,-172,159))[lobe];dx,dy,dz=(rng.uniform(-1,1) for _ in range(3))
   if dx*dx+dy*dy+dz*dz>1:continue
   center=Vector((cx+dx*39,cy+dy*72,cz+dz*53));sy=-center.y*SIN-center.z*COS
   # In-map unseen RGB is not added; inferred cards stay above the original raster.
   if sy>-1:continue
   size=rng.uniform(6,12);a=rng.uniform(0,math.tau);u=Vector((math.cos(a),math.sin(a),0))*size;v=Vector((-math.sin(a)*.45,math.cos(a)*.45,.89))*size;px,py=rng.choice(patches);ids=[]
   for p,t in zip((center-u/2-v/2,center+u/2-v/2,center+u/2+v/2,center-u/2+v/2),((px/126,1-(py+7)/57),((px+7)/126,1-(py+7)/57),((px+7)/126,1-py/57),(px/126,1-py/57))):
    projected=-p.y*SIN-p.z*COS
    if projected>-.05:p.z+=(projected+.05)/COS
    ids.append(len(vertices));vertices.append(tuple(p));uvs.append(t)
   faces.append(ids)
  mesh=bpy.data.meshes.new('Local75 native cells and inferred crown clusters');mesh.from_pydata(vertices,[],faces);mesh.update();uv=mesh.uv_layers.new(name='UVMap')
  for f in mesh.polygons:
   for li in f.loop_indices:uv.data[li].uv=uvs[mesh.loops[li].vertex_index]
  mesh.materials.append(mat);foliage=bpy.data.objects.new('Local75 provisional crown context',mesh);scene.collection.objects.link(foliage);foliage['asset_group']='croisement03-local75-provisional';foliage['source_ownership']='Provisional3250 minus six protected bark; neighbouring masks and animation unresolved'
  assert beforeimages=={im.name:hashlib.sha256(np.asarray(im.pixels[:],np.float32).tobytes()).hexdigest() for im in bpy.data.images if im.name in beforeimages}
  target=Vector((1068,-170,105))
  for i in range(8):
   cam=bpy.data.objects[f'Tree13 view{i}'];a=i*math.tau/8;direction=Vector((math.sin(a)*COS,-math.cos(a)*COS,SIN));cam.location=target+direction*1000;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();cam.data.ortho_scale=310
  scene.cycles.transparent_max_bounces=128;bpy.data.libraries.write(str(OUT/'worker.blend'),{scene},fake_user=True,compress=True);render_views(scene.name,{f'view-{i}':f'Tree13 view{i}' for i in range(8)},OUT/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    p=Image.open(OUT/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',p.size,'#333333');bg.alpha_composite(p);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(OUT/'actual'/f'{mode}.png')
  write_json(OUT/'receipt.json',dict(status='PRIVATE prototype only; full native/contact audit and neighbouring context pending',model_sha256=sha(OUT/'worker.blend'),source_model_sha256=sha(src),records=records,inferred_branches=branch_records,original_image_rgba_exact=True,provisional_native_leaf_pixels=3244,native_faces=native_faces,crossing_raster_ambiguities_omitted=crossing_raster_ambiguities_omitted,inferred_leaf_faces=len(faces)-native_faces,native_view_index=0,limits=['No new source ownership approval.','Unknown bark remains gray.','Animated Arbre06 and neighbouring12/14 retained as separate unfinished context.','Branch crown connection and source raster audit still required; no gallery readiness.']))
 finally:release()
if __name__=='__main__':main()
