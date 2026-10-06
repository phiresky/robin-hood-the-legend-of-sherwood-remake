"""Isolate the three stem construction and correct one local observed bark edge."""
import json,sys,math,shutil
from pathlib import Path
import bpy
from mathutils import Vector,Matrix
from mathutils.bvhtree import BVHTree
from PIL import Image
import numpy as np
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from restart2_tree13_wood_v1 import mesh_for,SPECS,ASSET
from render_slots import acquire,release
from render_views import render_views
from evidence_io import sha,write_json
B=ROOT/'level-editor/work/croisement03-refinement/restart2';OUT=B/'tree13-wood-v2'
def main():
 assert shutil.disk_usage(ROOT).free>25*1024**3;OUT.mkdir(exist_ok=False);source=B/'tree13-wood-v1/assets'/ASSET/'model.blend';acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();original=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')==ASSET];assert len(original)==3;scene=bpy.data.scenes.new('Tree13 isolated wood');collection=bpy.data.collections.new('Tree13 isolated stems');scene.collection.children.link(collection);objects=[]
  for o in original:
   matrix=o.matrix_world.copy();copy=o.copy();copy.data=o.data.copy();copy.parent=None;collection.objects.link(copy);copy.matrix_world=matrix;node=int(o.get('source_node',o.name).split('.')[0].rsplit('-',1)[1]);copy['source_node']=f'building-{node:03}'
   if node==31:
    spec=dict(SPECS[31]);spec['points']=[(1065,87,7.2),(1066.5,60,6.6),(1064,25,5.9),(1059,0,5.4),(1057,-45,4.8),(1060,-95,3.8),(1056,-150,2.0)];copy.data=mesh_for('Tree13 native031 locally fitted stem',spec);copy.matrix_world.identity()
   objects.append(copy)
  bpy.context.window.scene=scene;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=64;scene.render.resolution_x=scene.render.resolution_y=384;scene.render.image_settings.color_mode='RGBA';scene.render.film_transparent=True;scene.view_settings.view_transform='Standard';scene.world=bpy.data.worlds.new('Tree13 diagnostic world');scene.world.use_nodes=True;scene.world.node_tree.nodes['Background'].inputs[0].default_value=(.22,.22,.22,1);sun=bpy.data.objects.new('Tree13 sun',bpy.data.lights.new('Tree13 sun','SUN'));scene.collection.objects.link(sun);sun.data.energy=2;sun.rotation_euler=(-Vector((-.45,-.55,.70))).to_track_quat('-Z','Y').to_euler();manifest=json.loads((source.parent/'modified/views.json').read_text());views={}
  for v in manifest['views']:
   cam=bpy.data.objects.new(f'Tree13 view{v["index"]}',bpy.data.cameras.new(f'Tree13 view{v["index"]}'));scene.collection.objects.link(cam);cam.data.type='ORTHO';cam.data.ortho_scale=v['ortho_scale'];cam.data.clip_end=20000;cam.matrix_world=Matrix(v['camera_matrix_world']);views[f'view-{v["index"]}']=cam.name
  bpy.data.libraries.write(str(OUT/'worker.blend'),{scene},fake_user=True,compress=True);render_views(scene.name,views,OUT/'actual',modes=('textured','solid'),width=384)
  for mode in ('textured','solid'):
   sheet=Image.new('RGB',(1536,768),'#333333')
   for i in range(8):
    im=Image.open(OUT/'actual'/f'view-{i}-{mode}.png').convert('RGBA');bg=Image.new('RGBA',im.size,'#333333');bg.alpha_composite(im);sheet.paste(bg.convert('RGB'),((i%4)*384,(i//4)*384))
   sheet.save(OUT/'actual'/f'{mode}.png')
  ray=Vector((0,-math.cos(math.radians(35)),math.sin(math.radians(35))));sin=math.sin(math.radians(35));trees=[(o,BVHTree.FromPolygons([o.matrix_world@v.co for v in o.data.vertices],[list(f.vertices) for f in o.data.polygons])) for o in objects];rows=[]
  for node in (48,31,32):
   domain=np.array(Image.open(B/f'tree13-bark-proposal-v1/node-{node:03}-proposed-bark.png'))>0;good=0;bad=[]
   for y,x in zip(*np.nonzero(domain)):
    origin=Vector((x+.5,-(y+.5)/sin,0))+ray*10000;hits=[]
    for o,t in trees:
     point,n,f,d=t.ray_cast(origin,-ray)
     if point is not None:hits.append((d,o['source_node']))
    hits.sort();owner=hits[0][1] if hits else None
    if owner==f'building-{node:03}':good+=1
    else:bad.append([int(x),int(y),owner])
   rows.append(dict(node=node,proposed_pixels=int(domain.sum()),correct_receiver=good,misses=bad))
  write_json(OUT/'receipt.json',dict(status='PRIVATE wood-only geometry; narrow bark ownership proposal and physical fern joint pending',model_sha256=sha(OUT/'worker.blend'),source_model_sha256=sha(source),actual8_sha256=sha(OUT/'actual/textured.png'),solid8_sha256=sha(OUT/'actual/solid.png'),native_view_index=0,source_edge_check=rows,only_three_stems_saved=True,limitations=['Local031centreline shift fits positively proposed bark without widening all stems.','Unknown bark remains gray; no static/animated leaf pixels reassigned.','Upper stem continuation inferred; canopy remains separate unresolved domain.']))
  print(rows)
 finally:release()
if __name__=='__main__':main()
