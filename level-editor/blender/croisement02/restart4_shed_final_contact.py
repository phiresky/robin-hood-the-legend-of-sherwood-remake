"""Native-first wide/contact review with independently reopened placement guards."""
import json,sys,math
from pathlib import Path
import bpy,numpy as np
from PIL import Image
from mathutils import Vector,Matrix
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT,scenery_workspace,tree_workspace
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from stage_review_scene import signature
from render_multiview_asset import render
from refinement_review import _tree

def sheet(paths,target):
 ims=[Image.open(p).convert('RGB')for p in paths];w,h=ims[0].size;im=Image.new('RGB',(w*4,h*((len(ims)+3)//4)))
 for i,p in enumerate(ims):im.paste(p,((i%4)*w,(i//4)*h))
 im.save(target)

def frame(scene,objects,direction,size=512,padding=1.25):
 pts=[o.matrix_world@v.co for o in objects for v in o.data.vertices];center=Vector([(min(p[i]for p in pts)+max(p[i]for p in pts))/2 for i in range(3)]);data=bpy.data.cameras.new('Native-first contact camera');data.type='ORTHO';data.clip_end=20000;cam=bpy.data.objects.new(data.name,data);scene.collection.objects.link(cam);cam.location=center+direction*5000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();scene.camera=cam;bpy.context.view_layer.update();local=[cam.matrix_world.inverted()@p for p in pts];span=[max(p[i]for p in local)-min(p[i]for p in local)for i in range(2)];data.ortho_scale=max(span)*padding;offset=Vector([(max(p[i]for p in local)+min(p[i]for p in local))/2 for i in range(2)]+[0]);cam.location+=cam.matrix_world.to_quaternion()@offset;scene.render.resolution_x=size;scene.render.resolution_y=size;scene.render.resolution_percentage=100;bpy.context.view_layer.update();return cam

def main():
 root=OUT/'restart4-source-gaps';specs=[('croisement02-woodcutters-shed',root/'shed-east-v2',[scenery_workspace('croisement02-shrub-88'),tree_workspace(39),tree_workspace(40),tree_workspace(47)])]
 for asset,worker,neighbors in specs:
  out=worker/'wide-contact-v1';out.mkdir(exist_ok=False);model=worker/'model.blend';modelsha=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();own=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'and o.get('asset_group')==asset]
  scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.view_settings.view_transform='Standard';scene.view_settings.look='None';scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA'
  packet=json.loads((worker/'cameras.json').read_text());packet.pop('render_object_names',None);packet['tile_size']=[384,384]
  for i,v in enumerate(packet['views']):
   direction=RAY if i==0 else Matrix(v['camera_matrix_world']).to_quaternion()@Vector((0,0,1));cam=frame(scene,own,direction,384,1.22);v['camera_matrix_world']=[list(r)for r in cam.matrix_world];v['ortho_scale']=cam.data.ortho_scale;v['crop']={'width':384,'height':384}
  write_json(out/'cameras.json',packet);render(out/'cameras.json',out/'wide',modes=('textured','solid'),width=384)
  for mode in ['textured','solid']:sheet([out/f'wide/view-{i}-{mode}.png'for i in range(8)],out/f'{mode}-sheet.png')
  # Record every source transform only after each saved input is separately reopened.
  inputs=[]
  for path,role in [(model,'candidate')]+[(n/'model.blend','context')for n in neighbors]:
   digest=sha(path);bpy.ops.wm.open_mainfile(filepath=str(path));bpy.context.window.scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.view_layer.update();wanted=asset if role=='candidate'else path.parent.name;obs=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'and o.get('asset_group')==wanted];assert obs,(path,wanted)
   inputs.append(dict(model=str(path),model_sha256=digest,role=role,asset=wanted,objects=[dict(name=o.name,matrix=[list(r)for r in o.matrix_world],signature=signature(o))for o in obs]))
  bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';world=bpy.data.worlds.new('Neutral contact world');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.8;scene.world=world;own=[];context=[]
  for rec in inputs:
   with bpy.data.libraries.load(rec['model'],link=False)as(src,dst):dst.objects=[o['name']for o in rec['objects']]
   for obj in dst.objects:
    scene.collection.objects.link(obj);parent=obj.parent
    while parent:
     if not parent.users_collection:scene.collection.objects.link(parent)
     parent=parent.parent
   bpy.context.view_layer.update()
   for obj,expected in zip(dst.objects,rec['objects']):
    assert signature(obj)==expected['signature'];actual=np.array(obj.matrix_world);target=np.array(expected['matrix']);assert np.max(np.abs(actual-target))<1e-5,(obj.name,actual,target);matrix=obj.matrix_world.copy();obj.parent=None;obj.matrix_world=matrix;obj.hide_render=False
    (own if rec['role']=='candidate'else context).append(obj)
  center=sum((o.matrix_world@v.co for o in own for v in o.data.vertices),Vector())/sum(len(o.data.vertices)for o in own);bpy.ops.mesh.primitive_plane_add(size=550,location=(center.x,center.y,0));floor=bpy.context.object;floor.name='Neutral ground Z0';mat=bpy.data.materials.new('Contact floor');mat.diffuse_color=(.12,.13,.10,1);floor.data.materials.append(mat);views=[]
  directions=[RAY,Vector((.6124,-.6124,.5)),Vector((.6124,.6124,.5)),Vector((-.866,0,.5))]
  for i,direction in enumerate(directions):
   cam=frame(scene,own,direction.normalized(),512,1.35);scene.render.filepath=str(out/f'contact-{i}.png');bpy.ops.render.render(write_still=True);views.append(dict(image=f'contact-{i}.png',camera=[list(r)for r in cam.matrix_world],ortho_scale=cam.data.ortho_scale,sha256=sha(out/f'contact-{i}.png')))
   if i==0:
    floor.hide_render=True;scene.render.filepath=str(out/'native-joint.png');bpy.ops.render.render(write_still=True);floor.hide_render=False;x,y,z=cam.location;half=cam.data.ortho_scale/2;sy=-y*SIN-z*COS;box=[x-half,sy-half,x+half,sy+half];native=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA').transform((512,512),Image.Transform.EXTENT,box,Image.Resampling.NEAREST);native.save(out/'native-source.png');comp=Image.new('RGBA',(1024,512),(32,32,32,255));comp.paste(native,(0,0));comp.paste(Image.alpha_composite(native,Image.open(out/'native-joint.png').convert('RGBA')),(512,0));comp.convert('RGB').save(out/'source-comparison.png')
  sheet([out/f'contact-{i}.png'for i in range(4)],out/'contact-sheet.png')
  bounds={o.name:dict(min_z=min((o.matrix_world@v.co).z for v in o.data.vertices),max_z=max((o.matrix_world@v.co).z for v in o.data.vertices))for o in own}
  assert sha(model)==modelsha
  write_json(out/'evidence.json',dict(status='Read-only native-first wide and contact review; independent root review pending',model_sha256=modelsha,inputs=inputs,source_world_matrices_verified=True,own_ground_bounds=bounds,transparent_bounces=512,views=views,limitations=['NeutralZ0 ground guide, not source ground artwork.','Overview/source comparison is not exhaustive pixel ownership or intersection proof.','Candidate retains its original standalone texture disclosure.']))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
