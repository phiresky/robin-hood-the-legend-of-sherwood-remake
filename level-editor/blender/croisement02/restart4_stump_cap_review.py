"""Review the saved upper-stump candidate without rewriting geometry."""
import json,sys
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
from refinement_workspace import _geometry
from render_multiview_asset import render
from audit_scene_first_hit import full_mask
from refinement_review import _tree

def main():
 asset='croisement02-logging-clearing-stumps';base=OUT/'scenery-round-1/assets'/asset;old=OUT/'texture-fill-round-1'/asset/'experiment/bake-v1/worker.blend';digest='b73e09d157db4104ec3a9a2c847e86cfde5fd689d1fdc55b91b24ded25448f0c';assert sha(old)==digest
 out=OUT/'restart4-source-gaps/stump-cap-v3';bpy.ops.wm.open_mainfile(filepath=str(out/'model.blend'));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;bpy.context.view_layer.update();objects=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH'];modelsha=sha(out/'model.blend')
 packet=json.loads((base/'modified/views.json').read_text())
 for view in packet['views']:view['crop']={'width':packet['tile_size'][0],'height':packet['tile_size'][1]}
 write_json(out/'cameras.json',packet);scene.cycles.transparent_max_bounces=512;scene.cycles.samples=8;render(out/'cameras.json',out/'actual',width=384)
 images=[Image.open(out/f'actual/view-{i}-textured.png').convert('RGB')for i in range(8)];w,h=images[0].size;sheet=Image.new('RGB',(w*4,h*2))
 for i,im in enumerate(images):sheet.paste(im,((i%4)*w,(i//4)*h))
 sheet.save(out/'actual/sheet.png')
 crop=[1280,275,1400,395];l,t,r,b=crop;data=bpy.data.cameras.new('Native source correction review');data.type='ORTHO';data.ortho_scale=r-l;data.clip_end=10000;cam=bpy.data.objects.new(data.name,data);scene.collection.objects.link(cam);scene.camera=cam;center=Vector(((l+r)/2,-(t+b)/2/SIN,0));cam.location=center+RAY*6000;cam.rotation_euler=(center-cam.location).to_track_quat('-Z','Y').to_euler();scene.render.resolution_x=(r-l)*4;scene.render.resolution_y=(b-t)*4;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA'
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o.get('asset_group')!=asset
 scene.render.filepath=str(out/'native.png');bpy.ops.render.render(write_still=True);native=Image.open(base/'reference/source.png').convert('RGBA').crop(crop).resize(((r-l)*4,(b-t)*4),Image.Resampling.NEAREST);comparison=Image.new('RGBA',((r-l)*8,(b-t)*4),(32,32,32,255));comparison.paste(native,(0,0));comparison.paste(Image.alpha_composite(native,Image.open(out/'native.png').convert('RGBA')),((r-l)*4,0));comparison.convert('RGB').save(out/'source-comparison.png')
 tree,owners,_=_tree([o for o in objects if o.get('asset_group')==asset]);audit=json.loads((OUT/'restart3-scene-audit/coherent-batch-v3-v1/first-hit/audit.json').read_text());hits=[]
 for sample in audit['components'][12]['samples']:
  x,y=sample['pixel'];hit,normal,index,dist=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);hits.append(dict(pixel=[x,y],object=owners[index].name if hit is not None else None))
 write_json(out/'proposal.json',dict(status='Private corrective geometry; self/root/contact review pending',model_sha256=modelsha,parent_model_sha256=digest,changed_source='building-026',unchanged_source='building-027',other_objects_exact=True,bottom_ring_exact=True,middle_radius_scale=1.22,cap_radius_scale=1.8,height_unchanged=True,existing_atlas_payloads_retained=True,source_reprojection='Native108 on corrected source-facing surfaces; retained old fill for unknown/reverse.',coverage=dict(target=221,hits=sum(h['object'] is not None for h in hits),samples=hits),limitations=['Native upper cut surface supports widening; hidden circular depth inferred.','Source-visible UV is recomputed for changed front surfaces; old reverse atlas and UV remain.','No fallen-log028 root or shed geometry touched.']))
 assert sha(old)==digest
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
