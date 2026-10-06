"""Render native and oblique paint-transfer evidence with unchanged native017 geometry."""
import json,math,sys
from pathlib import Path
import bpy
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire
from review_evidence import sha
R=ROOT/'level-editor/work/croisement01-refinement/restart2';LIB=ROOT/'level-editor/library';base=LIB/'3d-assets/croisement01/croisement01-group-005';new=R/'tree03-visual-transfer-v1/3d-assets/croisement01/croisement01-group-005';tree=R/'tree03-integration-v2/assets/croisement01-tree-03';out=R/'tree03-visual-transfer-v1/rendered-proof-v1';assert not out.exists();out.mkdir();acquire();bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=16;scene.cycles.transparent_max_bounces=256;scene.render.resolution_x=256;scene.render.resolution_y=512;scene.render.resolution_percentage=100;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.film_transparent=True;scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.world=bpy.data.worlds.new('Neutral');scene.world.color=(.15,.15,.15)
def load(folder):
 before=set(bpy.data.objects);bpy.ops.import_scene.gltf(filepath=str(folder/'model.glb'));added=set(bpy.data.objects)-before;pivot=Vector(json.loads((folder/'asset.json').read_text())['source_origin_scene'])
 for o in added:
  if o.parent not in added:o.location+=pivot
 bpy.context.view_layer.update();return added
old=load(base);changed=load(new);added=load(tree)
oldmesh=[o for o in old if o.type=='MESH' and o.name.split('.')[0]=='building-017'];newmesh=[o for o in changed if o.type=='MESH' and o.name.split('.')[0]=='building-017'];treemesh=[o for o in added if o.type=='MESH'];assert len(oldmesh)==len(newmesh)==1
for o in old|changed|added:o.hide_render=True
s,c=math.sin(math.radians(35)),math.cos(math.radians(35));target=Vector((170,-930,(930*s-390)/c));data=bpy.data.cameras.new('Native transfer camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=95;data.clip_end=20000;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);scene.camera=camera;images=[];records=[]
for angle in [0,math.radians(45)]:
 direction=Vector((math.sin(angle)*c,-math.cos(angle)*c,s));camera.location=target+direction*5000;camera.rotation_euler=(-direction).to_track_quat('-Z','Y').to_euler()
 for label,objects in [('Before native017',oldmesh),('After native017',newmesh),('Approved Tree03',treemesh),('Combined after transfer',newmesh+treemesh)]:
  for o in old|changed|added:o.hide_render=True
  for o in objects:o.hide_render=False
  filename=f'{len(records):02}.png';scene.render.filepath=str(out/filename);bpy.ops.render.render(write_still=True);im=Image.open(out/filename).convert('RGBA');panel=Image.new('RGB',im.size,'#444444');panel.paste(im,mask=im.getchannel('A'));ImageDraw.Draw(panel).text((4,4),('Native camera ' if angle==0 else 'Oblique ')+label,fill='white');images.append(panel);records.append(dict(label=label,native=angle==0,file=filename,sha256=sha(out/filename)))
board=Image.new('RGB',(1024,1024),'#444444')
for i,im in enumerate(images):board.paste(im,((i%4)*256,(i//4)*512))
board.save(out/'sheet.png');(out/'evidence.json').write_text(json.dumps(dict(source_group_sha256=sha(base/'model.glb'),transferred_group_sha256=sha(new/'model.glb'),tree_glb_sha256=sha(tree/'model.glb'),transfer_proof_sha256=sha(R/'tree03-visual-transfer-v1/proof.json'),sheet_sha256=sha(out/'sheet.png'),views=records,scope='Only native017 and approvedTree03 shown. Geometry and gameplay unchanged; exact46sourcecells have transferred visual ownership. No full-map appearance claim.'),indent=2)+'\n')
