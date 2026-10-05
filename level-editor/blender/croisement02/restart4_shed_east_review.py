"""Review saved east shed continuation without rewriting geometry."""
import json,sys,math
from pathlib import Path
import bpy,bmesh,numpy as np
from PIL import Image
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from scenery_geometry import Mesh
from refinement_workspace import _geometry
from render_multiview_asset import render
from audit_scene_first_hit import full_mask
from refinement_review import _tree

def main():
 asset='croisement02-woodcutters-shed';base=OUT/'restart2-vegetation/shed-package-v1/assets'/asset;old=OUT/'restart2-textures/approved-prop-repairs-fill-v1'/asset/'stored-preparation-v3/experiment/native-front-retained-v1/worker.blend';digest='58a3cbc5183af47d652dff172ec015dc89bd64fc1004c56776367348076c5b24';assert sha(old)==digest
 out=OUT/'restart4-source-gaps/shed-east-v1';bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();obj=bpy.data.objects['Inferred complete east roof and side return'];original=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o!=obj];before={o.name:_geometry(o,True)for o in original};modelsha=sha(out/'model.blend');mesh=obj.data;bm=bmesh.new();bm.from_mesh(mesh);nonmanifold=sum(not e.is_manifold for e in bm.edges);degenerate=sum(f.calc_area()<1e-8 for f in bm.faces);bm.free();extension=Vector((95.4723,-(224.23895-185.53748)/SIN,0)).normalized()*70
 packet=json.loads((base/'modified/views.json').read_text());packet['object_names']=[o.name for o in original if o.get('asset_group')==asset]+[obj.name];packet.pop('render_object_names',None)
 for view in packet['views']:view['crop']={'width':packet['tile_size'][0],'height':packet['tile_size'][1]};view['ortho_scale']*=1.6
 write_json(out/'cameras.json',packet);scene.cycles.transparent_max_bounces=512;scene.cycles.samples=8;render(out/'cameras.json',out/'actual',width=384);images=[Image.open(out/f'actual/view-{i}-textured.png').convert('RGB')for i in range(8)];w,h=images[0].size;sheet=Image.new('RGB',(w*4,h*2))
 for i,im in enumerate(images):sheet.paste(im,((i%4)*w,(i//4)*h))
 sheet.save(out/'actual/sheet.png')
 crop=[1610,90,1850,305];l,t,r,b=crop;data=bpy.data.cameras.new('Native source east continuation');data.type='ORTHO';data.ortho_scale=r-l;data.clip_end=10000;cam=bpy.data.objects.new(data.name,data);scene.collection.objects.link(cam);scene.camera=cam;center=Vector(((l+r)/2,-(t+b)/2/SIN,0));cam.location=center+RAY*6000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=(r-l)*3;scene.render.resolution_y=(b-t)*3;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA'
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('asset_group')!=asset
 scene.render.filepath=str(out/'native.png');bpy.ops.render.render(write_still=True);native=Image.open(base/'reference/source.png').convert('RGBA').crop(crop).resize(((r-l)*3,(b-t)*3),Image.Resampling.NEAREST);comparison=Image.new('RGBA',((r-l)*6,(b-t)*3),(32,32,32,255));comparison.paste(native,(0,0));comparison.paste(Image.alpha_composite(native,Image.open(out/'native.png').convert('RGBA')),((r-l)*3,0));comparison.convert('RGB').save(out/'source-comparison.png')
 tree,owners,_=_tree([o for o in original+[obj]if o.get('asset_group')==asset]);audit=json.loads((OUT/'restart3-scene-audit/coherent-batch-v3-v1/first-hit/audit.json').read_text());hits=[]
 for n in [12,20]:
  for sample in audit['components'][n-1]['samples']:
   x,y=sample['pixel'];hit,normal,index,dist=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);hits.append(dict(region=n,pixel=[x,y],object=owners[index].name if hit is not None else None))
 write_json(out/'proposal.json',dict(status='Private source-supported east continuation; geometry/root/contact review pending',model_sha256=modelsha,parent_model_sha256=digest,original_geometry_materials_uv_exact=True,original_objects=list(before),added_object=obj.name,extension_length=70,extension_vector=list(extension),native_masks=dict(accepted=[127],excluded=[47,88,110]),nonmanifold_edges=nonmanifold,degenerate_faces=degenerate,min_z=min(v.co.z for v in mesh.vertices),coverage=dict(target=416,hits=sum(h['object'] is not None for h in hits),samples=hits),limitations=['Complete east extent and return are inferred beyond original map; not cutoff at1792.','Original filled roof/front/side/stump payloads retained exactly;58a3 texture remains pending its own user decision.','New unknown/reverse geometry gray until separate geometry approval and fill.','Native47 foreground tree excluded from roof/side projection.']))
 assert sha(old)==digest
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
