"""Inspect approved fern35 against isolated private three-stem wood geometry."""
import json,sys,math,shutil
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'tree13-fern35-joint-v6'
def main():
 assert shutil.disk_usage(ROOT).free>25*1024**3;OUT.mkdir(exist_ok=False);wood=B/'tree13-wood-v6/worker.blend';fern=B/'texture-round1/croisement03-fern-35/experiment/baked-preserved-v1/worker.blend';hashes={str(p):sha(p) for p in (wood,fern)};acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(fern));bpy.context.view_layer.update();source=next(o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement03-fern-35');matrix=source.matrix_world.copy();scene=bpy.data.scenes.new('Tree13 fern35 local contact');copy=source.copy();copy.parent=None;scene.collection.objects.link(copy);copy.matrix_world=matrix;copy.hide_render=False
  with bpy.data.libraries.load(str(wood),link=False) as (a,b):b.objects=list(a.objects)
  nodes=[]
  for o in b.objects:
   if o.type!='MESH' or o.get('asset_group')!='croisement03-tree-13':continue
   scene.collection.objects.link(o);o.hide_render=False;nodes.append(o['source_node'])
  assert sorted(nodes)==['building-031','building-032','building-048'];record=json.loads((B/'fern-ownership-v1/assets/croisement03-fern-35/inspection/construction.json').read_text());root=Vector(record['geometry']['root_world']);ground=record['geometry']['ground_z'];plane=bpy.data.meshes.new('Local diagnostic ground');s=65;plane.from_pydata([(root.x-s,root.y-s,ground),(root.x+s,root.y-s,ground),(root.x+s,root.y+s,ground),(root.x-s,root.y+s,ground)],[],[(0,1,2,3)]);o=bpy.data.objects.new('Local diagnostic ground',plane);scene.collection.objects.link(o);mat=bpy.data.materials.new('Local ground neutral');mat.diffuse_color=(.22,.25,.19,1);plane.materials.append(mat)
  scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=256;scene.render.resolution_x=scene.render.resolution_y=384;scene.view_settings.view_transform='Standard';scene.world=bpy.data.worlds.new('Tree13 local ambient');scene.world.color=(.18,.18,.18);sun=bpy.data.objects.new('Tree13 local sun',bpy.data.lights.new('Tree13 local sun','SUN'));scene.collection.objects.link(sun);sun.data.energy=2;sun.rotation_euler=(-Vector((-.45,-.55,.70))).to_track_quat('-Z','Y').to_euler();views={};center=root+Vector((0,0,22));elev=math.radians(35)
  for i in range(8):
   az=i*math.tau/8;direction=Vector((math.sin(az)*math.cos(elev),-math.cos(az)*math.cos(elev),math.sin(elev)));cam=bpy.data.objects.new(f'Local view{i}',bpy.data.cameras.new(f'Local view{i}'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=130;cam.data.clip_end=20000;cam.location=center+direction*3000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();views[f'view-{i}']=cam.name
  bpy.context.window.scene=scene;render_views(scene.name,views,OUT,modes=('textured',),width=384);sheet=Image.new('RGB',(1536,768))
  for i in range(8):sheet.paste(Image.open(OUT/f'view-{i}-textured.png').convert('RGB'),((i%4)*384,(i//4)*384))
  sheet.save(OUT/'sheet.png');assert all(sha(Path(p))==h for p,h in hashes.items());write_json(OUT/'receipt.json',dict(status='PRIVATE local contact diagnostic; visual review pending',source_models=hashes,sheet_sha256=sha(OUT/'sheet.png'),native_view_index=0,ground_z=ground,fern_root=list(root),wood_nodes=nodes,limits=['Tree13 wood material remains unresolved gray; this does not establish final source appearance.','Ground is a diagnostic plane, not final terrain.','All three stems present; local cameras intentionally crop upper continuation.','Native mask75 and canopy06 are unfinished context, not silently replaced.']))
 finally:release()
if __name__=='__main__':main()
