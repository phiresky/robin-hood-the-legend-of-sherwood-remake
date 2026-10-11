"""Save and inspect provisional Tree01 wood in the retained bank context."""
import json,math,sys,shutil,hashlib
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
R=Path(__file__).resolve().parents[3];sys.path[:0]=[str(R/'level-editor/refinement'),str(Path(__file__).parent)]
from render_slots import acquire,release
from restart2_bank_boundary_ray import boundary_hits
B=R/'level-editor/work/croisement03-refinement/restart2';I=B/'tree01-cpu-wood-v1';O=B/'tree01-saved-wood-v1';S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=Vector((0,-C,S))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def resources():
 assert shutil.disk_usage(R).free>10*1024**3
 mem={line.split(':')[0]:int(line.split()[1])*1024 for line in Path('/proc/meminfo').read_text().splitlines()};assert mem['MemAvailable']>=6*1024**3,mem['MemAvailable']
def main():
 resources();acquire(slots=2)
 try:
  resources();O.mkdir(exist_ok=False);source=I/'geometry.json';data=json.loads(source.read_text());assert json.loads((I/'cpu-firsthits.json').read_text())['status']=='PASS_CPU_PROPOSED_BARK_SEEDS';base=B/'bank-whole-prototype-v3/worker.blend';basehash=sha(base);bpy.ops.wm.open_mainfile(filepath=str(base));s=bpy.context.scene;bpy.context.preferences.filepaths.save_version=0
  material=bpy.data.materials.new('Tree01 provisional wood without source ownership');material.diffuse_color=(.35,.24,.14,1);material.use_nodes=True;material.node_tree.nodes.get('Principled BSDF').inputs['Base Color'].default_value=(.35,.24,.14,1)
  for p in data['parts']:
   name=f'Provisional Tree01 wood {p["source_obstacle_context"]}';m=bpy.data.meshes.new(name);m.from_pydata(p['vertices'],[],p['faces']);m.update();o=bpy.data.objects.new(name,m);s.collection.objects.link(o);m.materials.append(material);o['source_obstacle_context']=p['source_obstacle_context'];o['status']='Inferred wood geometry; source texture ownership not assigned'
   bm=bmesh.new();bm.from_mesh(m);assert all(e.is_manifold for e in bm.edges) and bm.calc_volume()>0;bm.free()
  model=O/'worker.blend';bpy.ops.wm.save_as_mainfile(filepath=str(model));assert model.stat().st_size<8*1024**2;bpy.ops.wm.open_mainfile(filepath=str(model));s=bpy.context.scene;wood=[o for o in s.objects if o.name.startswith('Provisional Tree01 wood')];assert len(wood)==4
  saved={};errors=[]
  for p in data['parts']:
   o=next(o for o in wood if o['source_obstacle_context']==p['source_obstacle_context']);points=[list(o.matrix_world@v.co)for v in o.data.vertices];errors.append(float(np.max(np.abs(np.array(points)-p['vertices']))));saved[f'wood-{p["source_obstacle_context"]}']=(points,[list(f.vertices)for f in o.data.polygons])
  assert max(errors)<1e-4
  for key in ('52','53','54'):
   o=bpy.data.objects['Candidate bank '+key];o.data.calc_loop_triangles();saved['bank-'+key]=([list(o.matrix_world@v.co)for v in o.data.vertices],[list(t.vertices)for t in o.data.loop_triangles])
  bad=[]
  for p in data['parts']:
   node=p['source_obstacle_context'];mask=np.array(Image.open(B/f'tree01-bark-seed-proposal-v1/building-{node:03}-proposed-bark.png'))>0
   for y,x in np.argwhere(mask):
    hits=boundary_hits(np.array([x,-y/S,0.])+np.array(RAY)*5000,-np.array(RAY),saved)
    if not hits or hits[0]['owner']!=f'wood-{node}':bad.append([int(x),int(y),hits[0]['owner'] if hits else None])
  assert not bad,bad
  s.render.threads_mode='FIXED';s.render.threads=2;s.render.resolution_percentage=100;s.render.image_settings.file_format='PNG';s.render.image_settings.color_mode='RGBA';s.render.film_transparent=True
  camera=bpy.data.objects.new('Tree01 review camera',bpy.data.cameras.new('Tree01 review camera'));s.collection.objects.link(camera);camera.data.type='ORTHO';camera.data.sensor_fit='HORIZONTAL';camera.data.clip_end=10000;s.camera=camera
  box=(45,100,175,290);target=Vector(((box[0]+box[2])/2,-(box[1]+box[3])/2/S,0));camera.location=target+RAY*5000;camera.rotation_euler=(-RAY).to_track_quat('-Z','Y').to_euler();camera.data.ortho_scale=box[2]-box[0];s.render.resolution_x=box[2]-box[0];s.render.resolution_y=box[3]-box[1];s.render.engine='CYCLES';s.cycles.samples=8;s.render.filepath=str(O/'native-contact.png');bpy.ops.render.render(write_still=True)
  source_image=Image.open(B.parent/'baseline/covered.png').convert('RGBA').crop(box);actual=Image.open(O/'native-contact.png').convert('RGBA');sheet=Image.new('RGBA',(780,570),'#333333');sheet.alpha_composite(source_image.resize((390,570),Image.Resampling.NEAREST),(0,0));sheet.alpha_composite(actual.resize((390,570),Image.Resampling.NEAREST),(390,0));sheet.convert('RGB').save(O/'native-comparison.png')
  for o in s.objects:
   if o.type=='MESH':o.hide_render=o not in wood
  points=np.array([list(o.matrix_world@v.co)for o in wood for v in o.data.vertices]);center=Vector((points.min(0)+points.max(0))/2);scale=float(np.max(points.max(0)-points.min(0)))*1.25;s.render.engine='BLENDER_WORKBENCH';s.display.shading.light='STUDIO';s.display.shading.color_type='MATERIAL';s.render.resolution_x=320;s.render.resolution_y=320;camera.data.ortho_scale=scale;grid=Image.new('RGB',(1280,640),'#333333')
  for i in range(8):
   theta=math.radians(i*45);direction=Vector((math.sin(theta)*C,-math.cos(theta)*C,S));camera.location=center+direction*5000;camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler();s.render.filepath=str(O/f'solid-{i*45:03}.png');bpy.ops.render.render(write_still=True);im=Image.open(s.render.filepath).convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);grid.paste(bg.convert('RGB'),((i%4)*320,(i//4)*320))
  grid.save(O/'solid-eight.png');assert sha(base)==basehash;assert sum(p.stat().st_size for p in O.rglob('*')if p.is_file())<=32*1024**2
  report={'status':'HOLD_SAVED_PRIVATE_WOOD_VISUAL_REVIEW_REQUIRED','model_sha256':sha(model),'source_geometry_sha256':sha(source),'retained_bank_sha256':basehash,'saved_vertex_error':max(errors),'saved_bark_seed_firsthits':211,'source_ownership_assigned':False,'images':{p.name:sha(p)for p in O.glob('*.png')},'limits':['Untextured wood volume review only; no final tree appearance claim.','Foliage, hidden joins and ground roots remain unresolved.','Bank morphology remains a separate held candidate.']};(O/'receipt.json').write_text(json.dumps(report,indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
