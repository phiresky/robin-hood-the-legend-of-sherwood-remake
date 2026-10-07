"""Bounded Tree08 chain and exact-junction model review with root context."""
import argparse,hashlib,json,math,shutil,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
R=ROOT/'level-editor/work/croisement01-refinement/restart2';source=R/'tree08-v12-chain-cpu-v3';out=R/'tree08-wood-prototype-v12-local-junctions';parser=argparse.ArgumentParser();parser.add_argument('--round-cap-mib',type=int,default=32);args=parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else []);cap=args.round_cap_mib*1024**2;round_cap=128*1024**2;floor=10*1024**3

def guard(reserve=1024**2):
 available=next(int(line.split()[1])*1024 for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'));assert available>=6*1024**3,('6GiB available-memory floor',available)
 folders=list(R.glob('tree08-wood-prototype-v12*'))+list(R.glob('tree08-v12-*'));used=sum(p.stat().st_size for folder in folders for p in folder.rglob('*') if p.is_file())
 assert used+reserve<=cap,('Complete v12 round32MiB cap',used,reserve)
 total=sum(p.stat().st_size for d in R.glob('tree08-wood-prototype-v*') for p in d.rglob('*') if p.is_file())
 assert total+reserve<=round_cap,('Round128MiB cap',total,reserve)
 assert shutil.disk_usage(R).free>=floor+(cap-used),'10GiB floor plus remaining lane output'

guard();world_origin=Vector((552.,-672.,235.));audit=json.loads((source/'report.json').read_text());assert not audit['lost_prior_core_pixels'] and audit['remaining_core_misses']==0
assert hashlib.sha256((source/'mesh.npz').read_bytes()).hexdigest()==audit['mesh_sha256']
packets=[R/'tree08-v12-remaining-group0-stitched-v3-conformed-stable-depth-corrected',R/'tree08-v12-remaining-group1-stitched-v2-stable']
union_receipts=[]
for packet in packets:
 checkpoint=json.loads((packet/'saved-precision-checkpoint.json').read_text())
 for file,digest in checkpoint['files'].items():assert hashlib.sha256(Path(file).read_bytes()).hexdigest()==digest
 proof=json.loads((packet/'source-and-intersections-planar.json').read_text());assert proof['status']=='PASS_LOCAL_DIAGNOSTICS' and not proof['intersections']
 if packet==packets[1]:
  saved_proof=json.loads((packet/'float32-proof/source-and-intersections-planar.json').read_text());assert saved_proof['status']=='PASS_LOCAL_DIAGNOSTICS' and not saved_proof['intersections']
 union_receipts.append(dict(packet=str(packet),mesh_sha256=hashlib.sha256((packet/'candidate.npz').read_bytes()).hexdigest(),proof_sha256=hashlib.sha256((packet/'source-and-intersections-planar.json').read_bytes()).hexdigest(),proof=proof))
core_proof=json.loads((packets[0]/'full-assembly-core-float32.json').read_text());assert core_proof['covered_core_pixels']==core_proof['core_pixels']==6276 and not core_proof['missing']
acquire();guard();out.mkdir(exist_ok=False)
bpy.ops.wm.open_mainfile(filepath=str(R/'tree08-wood-prototype-v10/model.blend'));bpy.context.preferences.filepaths.save_version=0
sys.path.insert(0,str(Path(__file__).parent))
from restart2_tree08_union import topology
oldobj=next(o for o in bpy.context.scene.objects if o.type=='MESH');oldmesh=oldobj.data;materials=list(oldmesh.materials)
bpy.data.objects.remove(oldobj,do_unlink=True);bpy.data.meshes.remove(oldmesh)
plan=json.loads((source/'fork-union-plan.json').read_text());archive=np.load(source/'mesh.npz');used={i for group in plan['groups'] for i in group};pieces=[]
for packet in packets:
 m=np.load(packet/'candidate.npz');pieces.append((m['vertices'],m['faces']))
for section in audit['mesh_sections']:
 i=section['index']
 if i not in used:pieces.append((archive[f'vertices_{i}'],archive[f'faces_{i}']))
vertices=[];faces=[]
for positions,triangles in pieces:
 offset=len(vertices);vertices.extend((positions-np.array(world_origin)).tolist());faces.extend((triangles+offset).tolist())
mesh=bpy.data.meshes.new('Original-surface local junctions and distinct source fragments');mesh.from_pydata(vertices,[],faces);mesh.update();obj=bpy.data.objects.new('Tree08 private local-junction candidate',mesh);bpy.context.scene.collection.objects.link(obj);obj.location=world_origin
for material in materials:mesh.materials.append(material)
for polygon in mesh.polygons:polygon.use_smooth=True
obj['source_node']='scenery-tree08-wood-prototype';bpy.context.view_layer.update();saved_topology=topology(obj)
assert not any(saved_topology[k] for k in ['zero_area_triangles','nonmanifold_edges','inconsistent_edge_winding'])
(out/'junction-union.json').write_text(json.dumps(dict(cpu_mesh_sha256=audit['mesh_sha256'],unions=union_receipts,assembled_topology=saved_topology,held_sections=plan['held_sections']),indent=2)+'\n')
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s));down=Vector((0,-s,-c));right=Vector((1,0,0));uv=mesh.uv_layers.new(name='Native source projection')
for loop in mesh.loops:
 p=obj.matrix_world@mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=((p.x-331)/446,1-(-p.y*s-p.z*c-11)/461)
obj['scope']='Private Tree08 continuous chains and locally reconstructed rooted fork junctions; held crossing and source fragments remain separate'
guard(8*1024**2);bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
assert (out/'model.blend').stat().st_size<=8*1024**2,'8MiB model cap'
model_hash=hashlib.sha256((out/'model.blend').read_bytes()).hexdigest()
# Reopen serialized float32 geometry for the independent native ray check.
bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'));scene=bpy.context.scene;obj=next(o for o in scene.objects if o.type=='MESH');mesh=obj.data
bvh=BVHTree.FromPolygons([obj.matrix_world@v.co for v in mesh.vertices],[list(p.vertices) for p in mesh.polygons]);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;missing=[]
for yy,xx in np.argwhere(core):
 x,y=int(xx)+331+.5,int(yy)+11+.5
 if bvh.ray_cast(right*x+down*y+ray*2000,-ray,4000)[0] is None:missing.append([int(x-.5),int(y-.5)])
assert not missing,'Saved model lost source core after local junction reconstruction'
reopened_topology=topology(obj);assert reopened_topology==saved_topology,'Saved topology changed'
scene.render.threads_mode='FIXED';scene.render.threads=2;scene.render.image_settings.file_format='PNG';scene.render.resolution_percentage=100;scene.render.film_transparent=False
scene.world=bpy.data.worlds.new('Neutral review background');scene.world.color=(.1,.1,.1)
camdata=bpy.data.cameras.new('Tree08 review camera');cam=bpy.data.objects.new('Tree08 review camera',camdata);scene.collection.objects.link(cam);camdata.type='ORTHO';camdata.clip_end=10000;scene.camera=cam
points=[obj.matrix_world@v.co for v in mesh.vertices];target=Vector(tuple((min(v[k] for v in points)+max(v[k] for v in points))/2 for k in range(3)));extent=0;directions=[]
for i in range(8):
 a=math.tau*i/8;direction=Vector((c*math.sin(a),-c*math.cos(a),s));side=direction.cross(Vector((0,0,1))).normalized();up=side.cross(direction).normalized();extent=max(extent,max(max(abs((v-target).dot(side)),abs((v-target).dot(up))) for v in points));directions.append(direction)
records=[]
for mode in ['actual','solid']:
 folder=out/mode;folder.mkdir();scene.render.engine='CYCLES' if mode=='actual' else 'BLENDER_WORKBENCH';scene.cycles.samples=4;scene.cycles.device='CPU';scene.cycles.use_denoising=False
 scene.display.shading.light='STUDIO';scene.display.shading.color_type='SINGLE';scene.display.shading.single_color=(.5,.5,.5);scene.display.shading.show_shadows=True;scene.display.shading.show_cavity=True
 scene.render.resolution_x=384;scene.render.resolution_y=384;camdata.ortho_scale=extent*2*1.12
 sheet=Image.new('RGB',(1536,816),'#222222');draw=ImageDraw.Draw(sheet)
 for i,direction in enumerate(directions):
  cam.location=target+direction*1800;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(folder/f'{i}.png');guard(2*1024**2);bpy.ops.render.render(write_still=True);sheet.paste(Image.open(folder/f'{i}.png').convert('RGB'),(i%4*384,i//4*408+24));draw.text((i%4*384+5,i//4*408+5),f'{mode} {i}'+(' native angle' if i==0 else ''),fill='white')
 guard(2*1024**2);sheet.save(folder/'sheet.png');records.append(dict(mode=mode,sheet_sha256=hashlib.sha256((folder/'sheet.png').read_bytes()).hexdigest()))
scene.render.engine='CYCLES';scene.render.resolution_x=446;scene.render.resolution_y=461;camdata.ortho_scale=461;native_target=right*554+down*241.5;cam.location=native_target+ray*1500;cam.rotation_euler=(-ray).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(out/'native.png');guard();bpy.ops.render.render(write_still=True)
# Append only the archived support objects intersecting the root neighborhood.
root_folder=out/'root-contact';root_folder.mkdir();inventory_path=R.parent/'grouped-inventory/inventory.json';inventory=json.loads(inventory_path.read_text());nodes={'ground'}|{f'building-{i:03d}' for i in list(range(10))+list(range(76,81))};root_points=[v for v in points if -v.y*s-v.z*c>340];lo=np.array([min(v[k] for v in root_points)-40 for k in range(3)]);hi=np.array([max(v[k] for v in root_points)+40 for k in range(3)])
selected=[entry for entry in inventory['objects'] if entry['source_node'] in nodes and all(entry['bounds_world'][1][k]>=lo[k] and entry['bounds_world'][0][k]<=hi[k] for k in [0,1])];wanted={entry['object'] for entry in selected}
with bpy.data.libraries.load(inventory['source_blend'],link=False) as (data_from,data_to):
 assert wanted<=set(data_from.objects);data_to.objects=sorted(wanted)
for receiver in data_to.objects:bpy.context.scene.collection.objects.link(receiver)
bpy.context.view_layer.update();receivers=[]
for receiver in data_to.objects:
 transform=receiver.matrix_world.copy();receiver.parent=None;receiver.matrix_world=transform;receiver.hide_render=False;receiver.color=(.22,.33,.38,1);receivers.append(receiver)
obj.color=(.58,.4,.22,1);scene.render.engine='BLENDER_WORKBENCH';scene.display.shading.color_type='OBJECT';scene.render.resolution_x=scene.render.resolution_y=384;camdata.ortho_scale=180;root_target=Vector(((lo[0]+hi[0])/2,(lo[1]+hi[1])/2,(lo[2]+hi[2])/2));sheet=Image.new('RGB',(768,816),'#222222');draw=ImageDraw.Draw(sheet)
for tile,i in enumerate([0,1,3,5]):
 direction=directions[i];cam.location=root_target+direction*1800;cam.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();scene.render.filepath=str(root_folder/f'{i}.png');guard();bpy.ops.render.render(write_still=True);sheet.paste(Image.open(root_folder/f'{i}.png').convert('RGB'),(tile%2*384,tile//2*408+24));draw.text((tile%2*384+5,tile//2*408+5),f'Root / archived support view {i}',fill='white')
sheet.save(root_folder/'sheet.png');terrain=[]
for receiver in receivers:
 vertices=[receiver.matrix_world@v.co for v in receiver.data.vertices];tree=BVHTree.FromPolygons(vertices,[list(p.vertices) for p in receiver.data.polygons]);terrain.append((receiver.get('source_node',receiver.name),tree))
contacts=[]
for anchor in json.loads((R/'tree08-topology-plan-v1/plan.json').read_text())['root_terrain_anchors']:
 x,y=anchor['native'];origin=right*x+down*y+ray*2000;hits=[]
 for node,tree in terrain:
  hit=tree.ray_cast(origin,-ray,4000)[0]
  if hit is not None:hits.append(dict(node=node,world=list(hit),ray_depth=float(hit.dot(ray))))
 wood=bvh.ray_cast(origin,-ray,4000)[0];contacts.append(dict(anchor=anchor,wood_first_hit=None if wood is None else list(wood),archived_support_hits=hits))
(root_folder/'receipt.json').write_text(json.dumps(dict(model_sha256=model_hash,archived_scene_sha256=hashlib.sha256(Path(inventory['source_blend']).read_bytes()).hexdigest(),receiver_nodes=[receiver.get('source_node',receiver.name) for receiver in receivers],source_anchor_rays=contacts,scope='Archived support diagnostic, not approved final terrain; model unchanged'),indent=2)+'\n')
assert hashlib.sha256((out/'model.blend').read_bytes()).hexdigest()==model_hash
guard();(out/'receipt.json').write_text(json.dumps(dict(status='AWAITING SELF REVIEW; not user ready',world_origin=list(world_origin),junction_proof_sha256=[receipt['proof_sha256'] for receipt in union_receipts],source_uv_uses_world_transform=True,model_sha256=model_hash,cpu_mesh_sha256=audit['mesh_sha256'],source_core_pixels=int(core.sum()),miss_native_pixels=missing,saved_model_matches_cpu_coverage=True,actual8_and_solid8=records,threads=2,lane_cap_bytes=cap,disk_floor_bytes=floor,cpu_stage_limitations=audit['limitations'],limitations=['Held crossing sections and three source fragments remain independent and require anatomical review.','Archived root supports are diagnostic, not final approved terrain.']),indent=2)+'\n');print('TREE08 V12 SAVED MODEL REVIEW COMPLETE',model_hash,flush=True)
