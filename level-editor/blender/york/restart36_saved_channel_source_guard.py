"""Build a private gatehouse guide recess while freezing approved gate and jamb."""
import hashlib,json,math,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'gatehouse-channel-candidate-v3'
assert OUT.exists() and not (OUT/'saved-native-guard.json').exists()
assert shutil.disk_usage(ROOT).free>10*1024**3
assert int(next(x.split()[1]for x in Path('/proc/meminfo').read_text().splitlines()if x.startswith('MemAvailable:')))*1024>6*1024**3
sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
acquire()
import bpy
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from mathutils.geometry import barycentric_transform
from PIL import Image
sys.path.insert(0,str(ROOT/'level-editor/refinement/blender'))
from refinement_workspace import _geometry
from render_views import render_views
sha=lambda p:hashlib.sha256(Path(p).read_bytes()).hexdigest()
P=W/'approved-shed-jamb-motion-integration-v1';plan=json.loads((P/'gatehouse-channel-proposal.json').read_text());approval=json.loads((P/'integration.json').read_text());ownership=json.loads((P/'jamb-union-ownership.json').read_text());assert ownership['jamb_owns_first']==ownership['explicit_known_jamb_domain']==127
source=ROOT/'level-editor/library/3d-assets/york/york-castle-west-gatehouse/model.glb';assert sha(source)==plan['source_glb_sha256']
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=str(source),import_pack_images=False)
external_images={}
for image in bpy.data.images:
 if image.source=='FILE' and image.filepath and not image.packed_file:
  image.filepath=str(Path(bpy.path.abspath(image.filepath)).resolve());external_images[image.filepath]=sha(image.filepath)
scene=bpy.context.scene;scene.name='York gatehouse channel review';scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.render.film_transparent=True
scene.world=bpy.data.worlds.new('Private review light');scene.world.use_nodes=True;scene.world.node_tree.nodes.get('Background').inputs['Color'].default_value=(.8,.8,.8,1);scene.view_settings.view_transform='Standard'
pivot=Vector(json.loads((source.parent/'asset.json').read_text())['source_origin_scene'])
roots=[o for o in scene.objects if o.parent is None]
assert len(roots)==1
roots[0].location+=pivot;bpy.context.view_layer.update()
meshes=[o for o in scene.objects if o.type=='MESH'];assert len(meshes)==8
by_node={o['source_node']:o for o in meshes};assert set(by_node)=={f'building-{i}'for i in [763,764,765,774,776,777,778,779]}
# CPU raw GLB bounds establish the import coordinate convention before any change.
base=Vector(plan['channel_basis']['origin']);axis=Vector(plan['channel_basis']['horizontal_axis']);normal=Vector(plan['channel_basis']['normal'])
for r in plan['components']:
 o=by_node[r['source_node']];points=[o.matrix_world@v.co for v in o.data.vertices];lo=[min(((p-base).dot(axis),(p-base).dot(normal),p.z)[k]for p in points)for k in range(3)];hi=[max(((p-base).dot(axis),(p-base).dot(normal),p.z)[k]for p in points)for k in range(3)];assert max(abs(a-b)for x,y in zip([lo,hi],r['bounds_channel_basis'])for a,b in zip(x,y))<.002
for source_model,name,digest in [
 (W/'gate-textures-v1/covered/model.blend','scenery-york-castle-portcullis','6ed0064f493f79cb38f249fdbc41177eac3cee5659fb86c73f59ae746ad19b70'),
 (W/'jamb-clearance-candidate-v1/model.blend','building-778-portcullis-jamb-return','5a4f548e83e1ce7e66765fdbcd68a59e94e460528485d5f610a603f17160f504')]:
 assert sha(source_model)==digest
 with bpy.data.libraries.load(str(source_model),link=False)as(a,b):b.objects=[name]
 scene.collection.objects.link(b.objects[0])
bpy.context.view_layer.update();gate=bpy.data.objects['scenery-york-castle-portcullis'];jamb=bpy.data.objects['building-778-portcullis-jamb-return'];protected={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o not in [by_node['building-778'],by_node['building-779']]}
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));back=Vector((0,-c,s));ray=-back
# Compare complete source-facing union, rather than attributing duplicated proxy
# border pixels to masonry after the approved jamb already owns them.
def snapshot_native():
 objects=[]
 for o in scene.objects:
  if o.type!='MESH' or o.hide_render:continue
  m=o.data;m.calc_loop_triangles();vs=[o.matrix_world@v.co for v in m.vertices];tris=list(m.loop_triangles);tree=BVHTree.FromPolygons(vs,[tuple(t.vertices)for t in tris],all_triangles=True);objects.append((o,m,vs,tris,tree))
 result={}
 for y in range(780,1030):
  for x in range(2250,2470):
   origin=Vector((x+.5,-(y+.5)/s,0))+back*1000;nearest=None
   for o,m,vs,tris,tree in objects:
    loc,n,ti,d=tree.ray_cast(origin,ray)
    if loc is None or(nearest and d>=nearest[0]):continue
    t=tris[ti];mat=o.data.materials[t.material_index].name if o.data.materials else None;uv=None
    if m.uv_layers.active:
     p=barycentric_transform(loc,*[vs[i]for i in t.vertices],*[Vector((*m.uv_layers.active.data[i].uv,0))for i in t.loops]);uv=[p.x,p.y]
    nearest=(d,o.name,mat,uv)
   if nearest:result[(x,y)]=nearest
 return result
before=snapshot_native()
model=OUT/'model.blend';digest=sha(model);validation=json.loads((OUT/'validation.json').read_text());assert digest==validation['model_sha256']
bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;bpy.context.view_layer.update();after=snapshot_native();assert set(before)==set(after)
changes=[];max_depth=0.;max_uv=0.
for pixel,a in before.items():
 b=after[pixel];max_depth=max(max_depth,abs(a[0]-b[0]));uv=max(abs(x-y)for x,y in zip(a[3],b[3]))if a[3]and b[3]else 0.;max_uv=max(max_uv,uv)
 if a[1:3]!=b[1:3]or abs(a[0]-b[0])>.002 or((a[3]is None)!=(b[3]is None))or uv>2e-5:changes.append({'pixel':pixel,'before':a,'after':b})
assert not changes,changes
report={'status':'PASS_SAVED_FULL_ASSEMBLY_NATIVE_SOURCE_GUARD','model_sha256':digest,'native_samples':len(before),'owner_material_uv_failures':0,'maximum_hit_depth_difference':max_depth,'maximum_uv_difference':max_uv,'reference':'Pinned original canonical gatehouse plus exact approved gate and jamb; compare saved corrected assembly at every crop pixel.','scope':'Original native camera source coverage/ownership/material/UV; no generated texture or runtime proof.'}
(OUT/'saved-native-guard.json').write_text(json.dumps(report,indent=2)+'\n');assert sha(model)==digest;print(json.dumps(report))
