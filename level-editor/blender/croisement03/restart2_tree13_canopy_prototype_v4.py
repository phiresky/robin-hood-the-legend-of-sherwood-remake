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
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'tree13-canopy-prototype-v4';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN));UP=Vector((0,SIN,COS))
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
  native_faces=len(faces);rng=random.Random(13075)
  # Small crossing leaflets add local volume while remaining edge-on to native rays.
  for f in list(faces):
   center=sum((Vector(vertices[i]) for i in f),Vector())/4;tex=[uvs[i] for i in f]
   for step in range(1,7):
    c=center-RAY*(step*7);u=Vector((6.7,0,0));v=RAY*6.7;ids=[]
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
  write_json(OUT/'receipt.json',dict(status='PRIVATE prototype only; full native/contact audit and neighbouring context pending',model_sha256=sha(OUT/'worker.blend'),source_model_sha256=sha(src),records=records,original_image_rgba_exact=True,provisional_native_leaf_pixels=3244,native_faces=native_faces,inferred_leaf_faces=len(faces)-native_faces,native_view_index=0,limits=['No new source ownership approval.','Unknown bark remains gray.','Animated Arbre06 and neighbouring12/14 retained as separate unfinished context.','Branch crown connection and source raster audit still required; no gallery readiness.']))
 finally:release()
if __name__=='__main__':main()
