"""Reopen initial rigging and approved context for native-first contact evidence."""
import sys,json,math,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from PIL import Image
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import SIN,COS,RAY
from stage_review_scene import signature
from refinement_review import _tree
from render_slots import acquire,release
from restart4_stump_final_contact import frame,sheet
ROOT=OUT/'restart5-initial-nets'

def main(key):
 assert shutil.disk_usage(OUT).free>25*1024**3
 worker=ROOT/f'candidate-v2/profile-{key}';proof=json.loads((worker/'report.json').read_text());model=worker/'model.blend';assert sha(model)==proof['model_sha256'];out=worker/'contact-v1';assert not out.exists();out.mkdir();neighbors=json.loads((ROOT/'source/context-selection-final.json').read_text());wanted=[43,45,46]if key=='00'else[37,38,39,40];records=[dict(asset_id='croisement02-initial-net-'+('01'if key=='00'else'03'),model=str(model),model_sha256=proof['model_sha256'],role='candidate')]+[dict(r,role='context')for r in neighbors if int(r['asset_id'][-2:])in wanted];inputs=[]
 for r in records:
  assert sha(Path(r['model']))==r['model_sha256'];bpy.ops.wm.open_mainfile(filepath=r['model']);bpy.context.view_layer.update();obs=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==r['asset_id']];assert obs,r['asset_id'];inputs.append(dict(r,objects=[dict(name=o.name,matrix=[list(v)for v in o.matrix_world],signature=signature(o))for o in obs]))
 bpy.ops.wm.read_factory_settings(use_empty=True);scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=8;scene.cycles.transparent_max_bounces=512;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';scene.view_settings.look='None';world=bpy.data.worlds.new('Neutral review world');world.use_nodes=True;world.node_tree.nodes['Background'].inputs['Strength'].default_value=.8;scene.world=world;own=[];context=[]
 for rec in inputs:
  with bpy.data.libraries.load(rec['model'],link=False)as(src,dst):dst.objects=[r['name']for r in rec['objects']]
  for o in dst.objects:
   scene.collection.objects.link(o);p=o.parent
   while p:
    if not p.users_collection:scene.collection.objects.link(p)
    p=p.parent
  bpy.context.view_layer.update()
  for o,r in zip(dst.objects,rec['objects']):
   assert signature(o)==r['signature'];assert np.max(np.abs(np.array(o.matrix_world)-np.array(r['matrix'])))<1e-5;matrix=o.matrix_world.copy();o.parent=None;o.matrix_world=matrix;o.hide_render=False;(own if rec['role']=='candidate'else context).append(o)
 wood=[o for o in context if o.get('projection_component')!='crown'and 'crown'not in o.name.lower()];bt,_,_=_tree(wood);rig=[o for o in own if o.name!='Ground camouflage net'];crossings=[]
 for o in rig:
  for e in o.data.edges:
   a,b=[o.matrix_world@o.data.vertices[i].co for i in e.vertices];delta=b-a
   if delta.length<1e-7:continue
   p,n,idx,dist=bt.ray_cast(a,delta.normalized(),delta.length)
   if p is not None and 1e-5<dist<delta.length-1e-5:crossings.append(dict(object=o.name,edge=e.index,point=list(p)))
 center=sum((o.matrix_world@v.co for o in own for v in o.data.vertices),Vector())/sum(len(o.data.vertices)for o in own);bpy.ops.mesh.primitive_plane_add(size=650,location=(center.x,center.y,0));floor=bpy.context.object;floor.name='Ground contact guideZ0';m=bpy.data.materials.new('Neutral ground');m.diffuse_color=(.13,.15,.11,1);floor.data.materials.append(m);views=[]
 for i,d in enumerate([RAY,Vector((.65,-.65,.5)).normalized(),Vector((.65,.65,.5)).normalized(),Vector((-.85,0,.5)).normalized()]):
  cam=frame(scene,own,d,512,1.28);scene.render.filepath=str(out/f'contact-{i}.png');bpy.ops.render.render(write_still=True);views.append(dict(view=i,camera=[list(r)for r in cam.matrix_world],ortho_scale=cam.data.ortho_scale))
  if i==0:
   floor.hide_render=True;scene.render.filepath=str(out/'native-joint.png');bpy.ops.render.render(write_still=True);floor.hide_render=False;x,y,z=cam.location;half=cam.data.ortho_scale/2;sy=-y*SIN-z*COS;box=[x-half,sy-half,x+half,sy+half];source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGBA');rec=proof['source'];source.alpha_composite(Image.open(rec['source']).convert('RGBA'),tuple(rec['origin']));native=source.transform((512,512),Image.Transform.EXTENT,box,Image.Resampling.NEAREST);native.save(out/'native-source.png');comp=Image.new('RGBA',(1024,512),(32,32,32,255));comp.paste(native,(0,0));comp.paste(Image.alpha_composite(native,Image.open(out/'native-joint.png').convert('RGBA')),(512,0));comp.convert('RGB').save(out/'source-comparison.png')
 sheet([out/f'contact-{i}.png'for i in range(4)],out/'contact-sheet.png')
 # Wood-only support close-up separates real contact from leafy occlusion.
 for o in context:
  if o not in wood:o.hide_render=True
 for o in own:o.hide_render=o.name=='Ground camouflage net'
 frame(scene,rig,RAY,512,1.15);scene.render.filepath=str(out/'wood-support-native.png');bpy.ops.render.render(write_still=True)
 write_json(out/'evidence.json',dict(model_sha256=proof['model_sha256'],inputs=inputs,source_world_matrices_verified=True,views=views,ground_min_z=min((o.matrix_world@v.co).z for o in own for v in o.data.vertices),wood_crossings=crossings,wood_crossing_count=len(crossings),scope='Actual initial geometry against frozen approved tree appearances; neutralZ0 ground contact guide.',limitations=['Finite rig-edge intersection check complements views; it is not exhaustive volumetric containment proof.','Native-only ambiguous high fragments remain source presentation, not fabricated geometry.']))
 assert sha(model)==proof['model_sha256']
if __name__=='__main__':
 acquire()
 try:main(sys.argv[sys.argv.index('--')+1])
 finally:release()
