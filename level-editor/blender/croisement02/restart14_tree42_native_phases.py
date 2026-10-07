"""Render all saved physical phases against their original-direction source frames."""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from render_slots import acquire,release
from tree_geometry import SIN,RAY
OUT=ROOT/'level-editor/work/croisement02-refinement/restart14-canopy-animation';BASE=OUT/('tree42-motion-v5'if '--bounded-trial'in sys.argv else 'tree42-motion-v4'if '--coherent'in sys.argv else 'tree42-motion-v3'if '--smooth'in sys.argv else 'tree42-motion-v2'if '--dense'in sys.argv else 'tree42-motion-v1');DEST=BASE/'native-phases-v1';sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 if '--bounded-trial'in sys.argv:
  from restart14_tree42_bounded_trial import guard,install_write_guards,finish
  guard();install_write_guards()
 DEST.mkdir(exist_ok=False);r=json.loads((BASE/'report.json').read_text());assert sha(BASE/'prototype.blend')==r['prototype_sha256'];source=json.loads((OUT/'source-reconciliation-v1/report.json').read_text())['groups'][1];bpy.ops.wm.open_mainfile(filepath=str(BASE/'prototype.blend'));scene=bpy.context.scene;crown=next(o for o in scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-42'and o.get('projection_component')=='crown')
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o!=crown
 x,y,w,h=616,688,342,288;target=Vector((x+w/2,-(y+h/2)/SIN,0));data=bpy.data.cameras.new('Native phase camera');data.type='ORTHO';data.sensor_fit='HORIZONTAL';data.ortho_scale=w;data.clip_end=20000;cam=bpy.data.objects.new('Native phase camera',data);scene.collection.objects.link(cam);cam.location=target+RAY*5000;cam.rotation_euler=(-RAY).to_track_quat('-Z','Y').to_euler();scene.camera=cam;scene.render.threads_mode='FIXED'if '--bounded-trial'in sys.argv else scene.render.threads_mode;scene.render.threads=2 if '--bounded-trial'in sys.argv else scene.render.threads;scene.render.engine='CYCLES';scene.cycles.samples=32;scene.cycles.transparent_max_bounces=512;scene.render.resolution_x=w;scene.render.resolution_y=h;scene.render.resolution_percentage=100;scene.render.film_transparent=True;scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.view_settings.view_transform='Standard';rows=[];sheet=Image.new('RGB',(w*2*4,(h+22)*4),'#303030');draw=ImageDraw.Draw(sheet);gif=[];evaluated=[]
 for phase,f in enumerate(source['frames']):
  scene.frame_set(1+phase*4);bpy.context.view_layer.update();ev=crown.evaluated_get(bpy.context.evaluated_depsgraph_get());me=ev.to_mesh();evaluated.append(np.array([v.co[:]for v in me.vertices]));ev.to_mesh_clear();p=DEST/f'phase-{phase:02}.png';scene.render.filepath=str(p);guard()if '--bounded-trial'in sys.argv else None;bpy.ops.render.render(write_still=True);a=np.array(Image.open(p));native=Image.new('RGBA',(w,h));sx,sy,sw,sh=f['bbox'];native.paste(Image.open(f['path']),(sx-x,sy-y));n=np.array(native);am=a[:,:,3]>=128;nm=n[:,:,3]>=128;missing=nm&~am;extra=am&~nm;row={'phase':phase,'tick':phase*4,'source_sha256':sha(Path(f['path'])),'render_sha256':sha(p),'source_pixels':int(nm.sum()),'actual_alpha_pixels':int(am.sum()),'missing_source_centers':int(missing.sum()),'extra_alpha_centers':int(extra.sum()),'alpha_iou':float((am&nm).sum()/max(1,(am|nm).sum()))};rows.append(row)
  pair=Image.new('RGB',(w*2,h+22),'#303030');pair.paste(native,(0,22),native);im=Image.fromarray(a);pair.paste(im,(w,22),im);ImageDraw.Draw(pair).text((5,5),f'Phase{phase}: native source | physical crown (fixed material)',fill='white');sheet.paste(pair,((phase%4)*w*2,(phase//4)*(h+22)));gif.append(pair)
 sheet.save(DEST/'all-fourteen-source-physical.png');gif[0].save(DEST/'source-physical-cycle.gif',save_all=True,append_images=gif[1:],duration=160,loop=0);pair=Image.new('RGB',(w*2,(h+22)*2),'#303030');pair.paste(gif[0],(0,0));pair.paste(gif[7],(0,h+22));pair.save(DEST/'source-phase0-phase7.png');continuity=[{'from_phase':i,'to_phase':(i+1)%14,'max_vertex_step':float(np.linalg.norm(evaluated[(i+1)%14]-evaluated[i],axis=1).max())}for i in range(14)];assert np.array_equal(evaluated[1],evaluated[13]);(DEST/'report.json').write_text(json.dumps({'status':'MEASURED_SOURCE_VS_PHYSICAL_NOT_PARITY_PASS','prototype_sha256':r['prototype_sha256'],'camera':{'native_bbox':[x,y,w,h],'location':list(cam.location),'rotation':list(cam.rotation_euler)},'rows':rows,'continuity':continuity,'clock':{'ticks_per_phase':4,'hz':25,'cycle_ticks':56,'interpolation':'CONSTANT native phase holds'},'limitations':['Actual alpha includes inferred rear/interior foliage; excess is not automatically source ownership.','Native art changes include visibility/color details that static texture geometry motion may not explain.','GIF shows all14 actual saved poses at160ms each; no source-texture swaps used.']},indent=2)+'\n')
 if '--bounded-trial'in sys.argv:finish('native')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
