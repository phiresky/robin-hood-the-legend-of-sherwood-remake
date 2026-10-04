"""Reopen the private wattle correction and review source coverage and actual materials."""
import json,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
from PIL import Image
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release
from render_multiview_asset import render

def main():
 base=OUT/'wattle99-source-candidate/v6';dst=base/'reopened-review';dst.mkdir(exist_ok=False);model=base/'model.blend';digest=sha(model);evidence=json.loads((base/'evidence.json').read_text());assert evidence['model_sha256']==digest;bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;asset='croisement02-southwest-path-wattle-fence';objects=[o for o in scene.objects if o.type=='MESH' and o.get('asset_group')==asset];assert len(objects)==1;obj=objects[0];bm=bmesh.new();bm.from_mesh(obj.data);topology=dict(vertices=len(bm.verts),faces=len(bm.faces),nonmanifold_edges=sum(not e.is_manifold for e in bm.edges),degenerate_faces=sum(f.calc_area()<1e-8 for f in bm.faces));bm.free();assert not topology['nonmanifold_edges'] and not topology['degenerate_faces'];scene.render.engine='CYCLES';scene.cycles.samples=16;render(base/'views.json',dst/'views',modes=('textured',),width=384);sheet=Image.new('RGB',(1536,768))
 for i in range(8):sheet.paste(Image.open(dst/f'views/view-{i}-textured.png'),((i%4)*384,(i//4)*384))
 sheet.save(dst/'actual-textured.png')
 inventory=json.loads((base/'mask-inventory.json').read_text());expected=np.asarray(Image.open(base/'private-wattle-domain.png').convert('L'))>0
 for index in [6001,42,129]:
  row=next(r for r in inventory['masks']if r['index']==index);a=np.asarray(Image.open(row['png']).convert('L'))>0;x,y=row['box_top_left'];expected[y:y+a.shape[0],x:x+a.shape[1]]&=~a
 crop=(526,741,792,1026);l,t,r,b=crop;expected=expected[t:b,l:r];target=Vector(((l+r)/2,-(t+b)/2/SIN,0));data=bpy.data.cameras.new('Exact native fence camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.clip_end=20000;data.ortho_scale=r-l;camera=bpy.data.objects.new(data.name,data);scene.collection.objects.link(camera);camera.location=target+RAY*5000;camera.rotation_euler=(target-camera.location).to_track_quat('-Z','Y').to_euler();scene.camera=camera
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o!=obj
 scene.render.resolution_x=r-l;scene.render.resolution_y=b-t;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.color_mode='RGBA';scene.render.filepath=str(dst/'native-actual.png');bpy.ops.render.render(write_still=True);actual=Image.open(scene.render.filepath).convert('RGBA');alpha=np.asarray(actual)[:,:,3]>127;source=Image.open(OUT/'animation-references/composite-frame-0.png').convert('RGB').crop(crop);overlay=np.asarray(source).copy();overlay[expected&~alpha]=[255,30,30];overlay[expected&alpha]=[0,220,100];comparison=Image.new('RGB',((r-l)*3*3,(b-t)*3));native_composite=source.copy();native_composite.paste(actual,mask=actual.getchannel('A'))
 for i,image in enumerate([source,native_composite,Image.fromarray(overlay)]):comparison.paste(image.resize(((r-l)*3,(b-t)*3),Image.Resampling.NEAREST),(i*(r-l)*3,0))
 comparison.save(dst/'native-comparison.png');assert sha(model)==digest;write_json(dst/'validation.json',dict(status='Private coverage evidence; visual review pending',model_sha256=digest,topology=topology,expected_pixels=int(expected.sum()),covered_pixels=int((expected&alpha).sum()),missing_pixels=int((expected&~alpha).sum()),recall=float((expected&alpha).sum()/expected.sum()),source_crop=crop,excluded_foreground_masks=[6001,42,129],limitation='Conservative exclusions protect foreground source RGB but are not a new native ownership claim; unobserved rear and excluded pixels remain unknown.'))
 print(dst)

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
