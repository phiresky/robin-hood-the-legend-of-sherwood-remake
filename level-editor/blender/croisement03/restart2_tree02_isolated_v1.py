"""Bounded isolated Tree02 construction with source guards before review rendering."""
import sys,math,json,hashlib,shutil,random
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from evidence_io import sha,write_json
from restart2_tree03_wood_v2 import mesh_for
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'tree02-isolated-prototype-v1';SIN=math.sin(math.radians(35));COS=math.cos(math.radians(35));RAY=Vector((0,-COS,SIN));UP=Vector((0,SIN,COS));CAP=128*1024**2
# The second small lower track has an inferred buried connection to the one main stem.
SPECS={6:dict(ground=238,points=[(210,238,5),(211,175,3.7),(211,163,3.2),(210,150,3),(209,138,3),(209,128,3),(213,118,3),(216,102,2.6),(215,78,2.4),(205,45,2),(195,5,1.3),(189,-25,.2)]),61:dict(ground=238,points=[(209,131,3),(218,126,3),(230,129,3.5),(241,122,3.5),(250,112,3),(257,97,2.4),(256,70,1.8),(245,35,1.2),(231,-5,.2)]),62:dict(ground=238,points=[(209,136,2.7),(202,141,2.5),(196,144,2),(187,136,1.5),(178,115,.9),(175,88,.2)]),63:dict(ground=238,points=[(210,174,2.4),(202,162,2.2),(200,155,2.1),(201,145,2),(202,141,1.5)])}
def space_guard():
 assert shutil.disk_usage(ROOT).free>10*1024**3+CAP,'10GiB floor plus remaining write estimate'
def material(im,name,fol=False):
 m=bpy.data.materials.new(name);m.use_nodes=True;n=m.node_tree.nodes;n.clear();l=m.node_tree.links;t=n.new('ShaderNodeTexImage');t.image=im;t.interpolation='Closest';t.extension='CLIP';e=n.new('ShaderNodeEmission');e.inputs[1].default_value=1;l.new(t.outputs['Color'],e.inputs[0]);mix=n.new('ShaderNodeMixShader');out=n.new('ShaderNodeOutputMaterial')
 if fol:
  back=n.new('ShaderNodeBsdfTransparent');l.new(t.outputs['Alpha'],mix.inputs[0])
 else:
  back=n.new('ShaderNodeBsdfPrincipled');back.inputs['Base Color'].default_value=(.18,.18,.18,1);g=n.new('ShaderNodeNewGeometry');dot=n.new('ShaderNodeVectorMath');dot.operation='DOT_PRODUCT';dot.inputs[1].default_value=RAY;front=n.new('ShaderNodeMath');front.operation='GREATER_THAN';front.inputs[1].default_value=0;a=n.new('ShaderNodeMath');a.operation='MULTIPLY';l.new(g.outputs['Normal'],dot.inputs[0]);l.new(dot.outputs['Value'],front.inputs[0]);l.new(front.outputs[0],a.inputs[0]);l.new(t.outputs['Alpha'],a.inputs[1]);l.new(a.outputs[0],mix.inputs[0])
 l.new(back.outputs[0],mix.inputs[1]);l.new(e.outputs[0],mix.inputs[2]);l.new(mix.outputs[0],out.inputs[0]);return m

def main():
 space_guard();assert not OUT.exists();OUT.mkdir();acquire()
 try:
  space_guard();bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.name='Tree02 isolated';scene.render.engine='CYCLES';scene.cycles.samples=24;scene.cycles.transparent_max_bounces=128;scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.resolution_x=384;scene.render.resolution_y=384;scene.render.film_transparent=True;scene.view_settings.view_transform='Standard';scene.world=bpy.data.worlds.new('Neutral review world');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.6,.6,.6,1);scene.world.node_tree.nodes['Background'].inputs[1].default_value=.8
  review=json.loads((B/'tree02-bark-proposal-v1/root-source-classification.json').read_text())
  for p,h in review['evidence'].items():assert sha(Path(p))==h
  native=Image.open(B.parent/'baseline/covered.png').convert('RGBA');mask=Image.open(B/'tree02-bark-proposal-v1/proposed-bark.png').convert('L');box=mask.getbbox();crop=native.crop(box);crop.putalpha(mask.crop(box));crop.save(OUT/'bark.png');im=bpy.data.images.load(str(OUT/'bark.png'));im.pack();bark=material(im,'Tree02 accepted bark and unknown wood');left,top,right,bottom=box;wood=[]
  for key,spec in SPECS.items():
   mesh=mesh_for(f'Tree02 branch {key}',spec);mesh.materials.append(bark);uv=mesh.uv_layers.active
   for f in mesh.polygons:
    f.use_smooth=len(f.vertices)==4
    for li in f.loop_indices:
     p=mesh.vertices[mesh.loops[li].vertex_index].co;uv.data[li].uv=((p.x-left)/(right-left),1-((-p.y*SIN-p.z*COS)-top)/(bottom-top))
   o=bpy.data.objects.new(f'Tree02 branch {key}',mesh);scene.collection.objects.link(o);o['asset_group']='croisement03-tree-02';o['source_node']='building-006';o['component']=key;wood.append(o)
  # Check attachment centerlines, not a visual coincidence of disconnected tubes.
  attachments=[]
  for child,parent,childidx in [(61,6,0),(62,6,0),(63,6,0)]:
   c=np.array(SPECS[child]['points'][childidx][:2]);near=min((float(np.linalg.norm(c-np.array(p[:2]))),p[2]) for p in SPECS[parent]['points']);assert near[0] <= near[1]+SPECS[child]['points'][childidx][2];attachments.append(dict(child=child,parent=parent,distance=near[0],overlapping_radii=True))
  scope=json.loads((B/'tree03-canopy-fragment-source-v1/scope.json').read_text());frames=[]
  for row in scope['frames']:
   p=Path(row['source_path']);assert sha(p)==row['source_sha256'];frames.append(np.array(Image.open(p).convert('RGBA').crop((175,0,225,175))))
  pixels=frames[0];union=np.logical_or.reduce([a[:,:,3]>0 for a in frames]);Image.fromarray(pixels).save(OUT/'native-leaves.png');li=bpy.data.images.load(str(OUT/'native-leaves.png'));li.pack();leaves=material(li,'Tree02 provisional Arbre08 fragment',True);verts=[];faces=[];uvs=[]
  for yy in range(0,175,3):
   for xx in range(0,50,3):
    x1,y1=min(xx+3,50),min(yy+3,175)
    if not np.any(union[yy:y1,xx:x1]):continue
    depth=-470-20*math.sin(xx*.127+yy*.079);ids=[]
    for x,y in ((xx,yy),(x1,yy),(x1,y1),(xx,y1)):ids.append(len(verts));verts.append((175+x,depth,(-depth*SIN-y)/COS));uvs.append((x/50,1-y/175))
    faces.append(ids)
  native_faces=len(faces);rng=random.Random(206);patches=[(x,y) for y in range(0,168,7) for x in range(0,43,7) if np.count_nonzero(pixels[y:y+7,x:x+7,3])>=12];assert patches
  protected=np.array(mask)>0;neighbor_mask=np.array(Image.open(B/'tree03-bark-proposal-v1/proposed-bark.png'))>0;by,bx=np.nonzero(protected|neighbor_mask);protected_xy=np.column_stack((bx+.5,by+.5));inferred=0;skipped=0
  # Narrow native fragment is supplemented by irregular depth and a complete off-map crown.
  for k in range(2400):
   if k<1400:
    dx,dy,dz=[rng.uniform(-1,1) for _ in range(3)]
    if dx*dx+dy*dy+dz*dz>1:continue
    c=Vector((207+dx*61,-347+dy*77,353+dz*52))
    if -c.y*SIN-c.z*COS>-3:continue
   else:
    x,y=rng.choice([(177,25),(222,25),(181,80),(222,85),(197,141)]);c=Vector((x+rng.uniform(-13,13),rng.uniform(-365,-305),0));sy=y+rng.uniform(-19,19);c.z=(-c.y*SIN-sy)/COS
   size=rng.uniform(4,8);a=rng.uniform(0,math.tau);u=Vector((math.cos(a),math.sin(a),rng.uniform(-.6,.6))).normalized()*size;v=u.cross(Vector((rng.uniform(-1,1),rng.uniform(-1,1),1))).normalized()*size;quad=(c-u/2-v/2,c+u/2-v/2,c+u/2+v/2,c-u/2+v/2);projected=np.array([(p.x,-p.y*SIN-p.z*COS) for p in quad]);low=projected.min(0)-.01;high=projected.max(0)+.01
   if np.any(np.all((protected_xy>=low)&(protected_xy<=high),axis=1)):skipped+=1;continue
   tx,ty=rng.choice(patches);ids=[]
   for p,t in zip(quad,((tx/50,1-(ty+7)/175),((tx+7)/50,1-(ty+7)/175),((tx+7)/50,1-ty/175),(tx/50,1-ty/175))):ids.append(len(verts));verts.append(tuple(p));uvs.append(t)
   faces.append(ids);inferred+=1
  mesh=bpy.data.meshes.new('Tree02 irregular provisional crown');mesh.from_pydata(verts,[],faces);mesh.update();uv=mesh.uv_layers.new(name='UVMap');mesh.materials.append(leaves)
  for f in mesh.polygons:
   for i in f.loop_indices:uv.data[i].uv=uvs[mesh.loops[i].vertex_index]
  foliage=bpy.data.objects.new('Tree02 provisional native and inferred foliage',mesh);scene.collection.objects.link(foliage);foliage['asset_group']='croisement03-arbre08-fragment-tree02-provisional';foliage['source_ownership']='Spatial frame0 fragment only; shared dynamic ownership unresolved'
  # Review cameras: native direction first, no neighbor model saved into this scene.
  target=Vector((208,-360,230))
  for i in range(8):
   data=bpy.data.cameras.new(f'Tree02 view{i}');data.type='ORTHO';data.ortho_scale=500;cam=bpy.data.objects.new(data.name,data);scene.collection.objects.link(cam);angle=i*math.tau/8;d=Vector((math.sin(angle)*COS,-math.cos(angle)*COS,SIN));cam.location=target+d*1000;cam.rotation_euler=(-d).to_track_quat('-Z','Y').to_euler()
  space_guard();bpy.data.libraries.write(str(OUT/'worker.blend'),{scene},fake_user=True,compress=True);assert (OUT/'worker.blend').stat().st_size<32*1024**2
  write_json(OUT/'construction.json',dict(status='PRIVATE unreviewed; source/neighbor guards required before renders',model_sha256=sha(OUT/'worker.blend'),native_faces=native_faces,native_leaf_pixels=int((pixels[:,:,3]>0).sum()),union_pixels=int(union.sum()),source_spec=SPECS,attachments=attachments,inferred_leaf_faces=inferred,excluded_bark_crossing_cards=skipped,source_review_sha256=sha(B/'tree02-bark-proposal-v1/root-source-classification.json'),limits=['Lower gray closure is inferred beneath native ledge; final terrain receiver excluded.','Own canopy interval175..225 is provisional; Tree03 source leaves must remain separate.','Unknown bark remains gray and no texture API authorized.']))
  print('SAVED',sha(OUT/'worker.blend'),(OUT/'worker.blend').stat().st_size)
 finally:release()
if __name__=='__main__':main()
