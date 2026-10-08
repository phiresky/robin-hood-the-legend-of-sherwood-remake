"""Build a private gatehouse guide recess while freezing approved gate and jamb."""
import hashlib,json,math,shutil,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];W=ROOT/'level-editor/work/york-refinement/restart2';OUT=W/'gatehouse-channel-candidate-v1'
assert not OUT.exists()
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
bpy.ops.import_scene.gltf(filepath=str(source),import_pack_images=True)
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
   origin=Vector((x+.5,-(y+.5)/s,0))+back*10000;nearest=None
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
lo,hi=map(Vector,plan['continuous_swept_channel_bounds']);mid=(lo+hi)/2
bpy.ops.mesh.primitive_cube_add(size=2,location=base+axis*mid.x+normal*mid.y+Vector((0,0,mid.z-base.z)))
cutter=bpy.context.object;cutter.name='Private portcullis swept guide cutter';basis=Matrix((axis,-normal,Vector((0,0,1)))).transposed();cutter.rotation_euler=basis.to_euler();cutter.dimensions=hi-lo;bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
unknown=bpy.data.materials.new('Unknown new guide-channel interior');unknown.diffuse_color=(.35,.35,.35,1);unknown.use_nodes=True;unknown.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.35,.35,.35,1);cutter.data.materials.append(unknown)
counts=[]
for node in ['building-778','building-779']:
 o=by_node[node];old_count=len(o.data.polygons);bpy.context.view_layer.objects.active=o;o.select_set(True);mod=o.modifiers.new('Scoped hidden portcullis guide channel','BOOLEAN');mod.operation='DIFFERENCE';mod.solver='EXACT';mod.object=cutter;mod.material_mode='TRANSFER';bpy.ops.object.modifier_apply(modifier=mod.name);o.select_set(False);counts.append({'source_node':node,'before_faces':old_count,'after_faces':len(o.data.polygons),'new_unknown_faces':sum(o.data.materials[p.material_index]==unknown for p in o.data.polygons)})
bpy.data.objects.remove(cutter,do_unlink=True);bpy.context.view_layer.update()
assert protected=={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.name in protected}
after=snapshot_native();assert set(before)==set(after),'Visible union coverage changed'
changes=[]
for pixel,a in before.items():
 b=after[pixel]
 if a[1:3]!=b[1:3]or abs(a[0]-b[0])>.002 or((a[3]is None)!=(b[3]is None))or(a[3]and max(abs(x-y)for x,y in zip(a[3],b[3]))>2e-5):changes.append({'pixel':pixel,'before':a,'after':b})
assert not changes, f'Native source owner/material/UV changed at{len(changes)}pixels'
OUT.mkdir();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(OUT/'model.blend'),compress=True);assert (OUT/'model.blend').stat().st_size<8*1024**2
bpy.ops.wm.open_mainfile(filepath=str(OUT/'model.blend'));scene=bpy.context.scene;bpy.context.view_layer.update();assert protected=={o.name:_geometry(o,protect_appearance=True)for o in scene.objects if o.name in protected}
gate=bpy.data.objects['scenery-york-castle-portcullis'];jamb=bpy.data.objects['building-778-portcullis-jamb-return'];gate_matrix=gate.matrix_world.copy();gvs=[gate.matrix_world@v.co for v in gate.data.vertices];gfaces=[list(p.vertices)for p in gate.data.polygons];context=[o for o in scene.objects if o.type=='MESH' and o!=gate];trees=[(o.name,BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(p.vertices)for p in o.data.polygons]))for o in context];motion=json.loads((W/'jamb-clearance-candidate-v1/motion-proposal.json').read_text());contact=[]
for row in motion['rows']:
 vs=[v+Vector((0,0,row['nominal_lift_world_z']))for v in gvs];gt=BVHTree.FromPolygons(vs,gfaces);crossings={name:len(gt.overlap(tree))for name,tree in trees if gt.overlap(tree)};contact.append({'frame':row['frame'],'crossings':crossings})
assert all(not r['crossings']for r in contact),contact
# Native first camera; complete context and isolated corrected pair both reviewed.
center=Vector((2352,-1810,200));cameras={}
for i,(yaw,elev)in enumerate([(0,35),(45,35),(90,35),(135,35),(180,35),(225,35),(270,35),(315,35)]):
 a,b=math.radians(yaw),math.radians(elev);v=Vector((math.sin(a)*math.cos(b),-math.cos(a)*math.cos(b),math.sin(b)));data=bpy.data.cameras.new('Review '+str(i));cam=bpy.data.objects.new(data.name,data);scene.collection.objects.link(cam);data.type='ORTHO';data.ortho_scale=330;data.clip_end=20000;cam.location=center+v*10000;cam.rotation_euler=(-v).to_track_quat('-Z','Y').to_euler();cameras[f'view-{i}']=cam.name
scene.render.resolution_x=320;scene.render.resolution_y=384
for state,frame in [('initial',0),('applied',44)]:
 gate.matrix_world=gate_matrix.copy();gate.matrix_world.translation.z+=motion['rows'][frame]['nominal_lift_world_z'];bpy.context.view_layer.update();dest=OUT/state;render_views(scene.name,cameras,dest,modes=('solid','textured'),width=320)
 for mode in ['solid','textured']:
  views=[Image.open(dest/f'view-{i}-{mode}.png').convert('RGBA')for i in range(8)];sheet=Image.new('RGBA',(1280,views[0].height*2))
  for i,im in enumerate(views):sheet.paste(im,((i%4)*320,(i//4)*im.height))
  sheet.save(dest/f'{mode}-eight.png')
report={'status':'PRIVATE_GUIDE_CHANNEL_GEOMETRY_REQUIRES_REVIEW','model_sha256':sha(OUT/'model.blend'),'source_glb_sha256':sha(source),'changed_components':counts,'native_union_rays_preserved':len(before),'native_owner_material_uv_changes':0,'approved_gate_jamb_and_other_objects_exact':len(protected),'contacts':contact,'scope':'New inferred guide channel in broad778/779proxy volumes only. Gate, approved jamb, proposed45poses and other gatehouse parts remain unchanged. New channel interior is explicitly unknown gray; no texture synthesis or approval inherited.'};(OUT/'validation.json').write_text(json.dumps(report,indent=2)+'\n');assert sum(p.stat().st_size for p in OUT.rglob('*')if p.is_file())<32*1024**2;print(json.dumps({'out':str(OUT),'native_union_rays':len(before),'changed_components':counts}))
