"""Bounded private saved-model review of the audited Tree08 CPU candidate."""
import hashlib,json,math,shutil,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
R=ROOT/'level-editor/work/croisement01-refinement/restart2';source=R/'tree08-v11-curvature-cpu-v7';out=R/'tree08-wood-prototype-v11';cap=32*1024**2;round_cap=128*1024**2;floor=8*1024**3

def guard(reserve=1024**2):
 used=sum(p.stat().st_size for p in out.rglob('*') if p.is_file()) if out.exists() else 0
 assert used+reserve<=cap,('Lane32MiB cap',used,reserve)
 total=sum(p.stat().st_size for d in R.glob('tree08-wood-prototype-v*') for p in d.rglob('*') if p.is_file())
 assert total+reserve<=round_cap,('Round128MiB cap',total,reserve)
 assert shutil.disk_usage(R).free>=floor+(cap-used),'8GiB floor plus remaining lane output'

guard();audit=json.loads((source/'report.json').read_text());assert not audit['newly_lost_pixels'];assert all(s['nonoutward']==0 for s in audit['sections'])
assert json.loads((source/'serialized-topology.json').read_text())['status']=='PASS'
assert hashlib.sha256((source/'mesh.npz').read_bytes()).hexdigest()==audit['mesh_sha256']
acquire();guard();out.mkdir(exist_ok=False)
bpy.ops.wm.open_mainfile(filepath=str(R/'tree08-wood-prototype-v10/model.blend'));bpy.context.preferences.filepaths.save_version=0
obj=next(o for o in bpy.context.scene.objects if o.type=='MESH');oldmesh=obj.data;materials=list(oldmesh.materials);archive=np.load(source/'mesh.npz');vertices=[];faces=[];smooth=[]
for section in audit['mesh_sections']:
 i=section['index'];v=archive[f'vertices_{i}'];f=archive[f'faces_{i}'];offset=len(vertices);vertices.extend(v.tolist());faces.extend((f+offset).tolist());side_count=(len(v)//16-1)*32;smooth.extend(j<side_count for j in range(len(f)))
mesh=bpy.data.meshes.new('Audited outward Tree08 sweep triangles');mesh.from_pydata(vertices,[],faces);mesh.update();obj.data=mesh
for material in materials:mesh.materials.append(material)
for polygon,flag in zip(mesh.polygons,smooth):polygon.use_smooth=flag
bpy.data.meshes.remove(oldmesh);s,c=math.sin(math.radians(35)),math.cos(math.radians(35));ray=Vector((0,-c,s));down=Vector((0,-s,-c));right=Vector((1,0,0));uv=mesh.uv_layers.new(name='Native source projection')
for loop in mesh.loops:
 p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=((p.x-331)/446,1-(-p.y*s-p.z*c-11)/461)
obj['scope']='Private Tree08 transported sections; two oblique bends, intersecting junctions and soil contact pending review'
guard(8*1024**2);bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True)
model_hash=hashlib.sha256((out/'model.blend').read_bytes()).hexdigest()
# Reopen serialized float32 geometry for the independent native ray check.
bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'));scene=bpy.context.scene;obj=next(o for o in scene.objects if o.type=='MESH');mesh=obj.data
bvh=BVHTree.FromPolygons([v.co for v in mesh.vertices],[list(p.vertices) for p in mesh.polygons]);core=np.asarray(Image.open(R/'tree08-semantic-source-v1/bark-core-proposal.png'))>0;missing=[]
for yy,xx in np.argwhere(core):
 x,y=int(xx)+331+.5,int(yy)+11+.5
 if bvh.ray_cast(right*x+down*y+ray*2000,-ray,4000)[0] is None:missing.append([int(x-.5),int(y-.5)])
assert set(map(tuple,missing))==set(map(tuple,audit['miss_native_pixels'])),'Saved model coverage differs from CPU proof'
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
assert hashlib.sha256((out/'model.blend').read_bytes()).hexdigest()==model_hash
guard();(out/'receipt.json').write_text(json.dumps(dict(status='AWAITING SELF REVIEW; not user ready',model_sha256=model_hash,cpu_mesh_sha256=audit['mesh_sha256'],source_core_pixels=int(core.sum()),miss_native_pixels=missing,saved_model_matches_cpu_coverage=True,actual8_and_solid8=records,threads=2,lane_cap_bytes=cap,disk_floor_bytes=floor,limitations=audit['limitations']),indent=2)+'\n');print('TREE08 V11 SAVED MODEL REVIEW COMPLETE',model_hash,flush=True)
