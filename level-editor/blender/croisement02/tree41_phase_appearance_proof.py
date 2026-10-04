"""Native phase appearance on an unchanged 3D crown, with scoped rear alpha gates."""
import sys,json,hashlib,math
from pathlib import Path
import bpy
import numpy as np
from PIL import Image,ImageDraw
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from catalog import OUT,tree_workspace
from render_slots import acquire,release
from render_multiview_asset import render
from refinement_workspace import _geometry
from tree_geometry import SIN,COS,RAY

def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def savejson(p,v):p.write_text(json.dumps(v,indent=2)+'\n')
def texture(material):return next(n for n in material.node_tree.nodes if n.type=='TEX_IMAGE' and n.image)
def main():
 dest=OUT/'tree41-phase-appearance-proof-v2';inputs=json.loads((dest/'inputs.json').read_text());base=Path(inputs['model']);assert sha(base)==inputs['model_sha256'];x,y,w,h=inputs['atlas_bbox'];phases=[np.array(Image.open(r['image']).convert('RGBA')) for r in inputs['phases']];domain=np.array(Image.open(dest/'temporal-domain.png'))>0
 acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(base));worker=tree_workspace(41);frames=json.loads((worker/'modified/views.json').read_text());scene=bpy.data.scenes[frames['scene_name']];bpy.context.window.scene=scene;objects=[scene.objects[n] for n in frames['object_names']];crown=next(o for o in objects if o.get('projection_component')=='crown');wood=[o for o in objects if o!=crown];guard={o.name:_geometry(o,protect_appearance=True) for o in scene.objects};uv=crown.data.uv_layers['Foliage UV'];packed=np.array(Image.open(texture(crown.data.materials[4]).image.filepath).convert('RGBA'));ah,aw=packed.shape[:2];lookup=np.full((ah,aw,2),-1,np.int32)
  # Recover packed-atlas texel -> world -> native source coordinates from the
  # actual triangle UVs. Never reinterpret packed UVs as native atlas UVs.
  for face in crown.data.polygons:
   if face.material_index!=4:continue
   points=np.array([crown.matrix_world@crown.data.vertices[i].co for i in face.vertices]);coords=np.array([(uv.data[i].uv.x*aw,(1-uv.data[i].uv.y)*ah) for i in face.loop_indices]);assert len(coords)==3
   left,top=np.maximum(np.floor(coords.min(axis=0)).astype(int),0);right,bottom=np.minimum(np.ceil(coords.max(axis=0)).astype(int),[aw,ah]);yy,xx=np.mgrid[top:bottom,left:right];q=np.stack([xx+.5,yy+.5],axis=-1);matrix=np.stack([coords[1]-coords[0],coords[2]-coords[0]],axis=1)
   if abs(np.linalg.det(matrix))<1e-8:continue
   weights=(q-coords[0])@np.linalg.inv(matrix).T;inside=(weights[:,:,0]>=-1e-5)&(weights[:,:,1]>=-1e-5)&(weights.sum(axis=2)<=1+1e-5);world=points[0]+weights[:,:,0,None]*(points[1]-points[0])+weights[:,:,1,None]*(points[2]-points[0]);ix=np.floor(world[:,:,0]-x).astype(int);iy=np.floor(-world[:,:,1]*SIN-world[:,:,2]*COS-y).astype(int);valid=inside&(ix>=0)&(ix<w)&(iy>=0)&(iy<h);region=lookup[top:bottom,left:right];region[valid]=np.stack([ix,iy],axis=-1)[valid]
  valid=lookup[:,:,0]>=0;ix=np.clip(lookup[:,:,0],0,w-1);iy=np.clip(lookup[:,:,1],0,h-1);gated=valid&domain[iy,ix];variants=[crown];gates=[]
  for index in range(1,14):
   obj=crown.copy();obj.animation_data_clear();scene.collection.objects.link(obj);obj.name=crown.name+f' / native phase {index:02}';variants.append(obj)
   for slot in (0,2,4):
    material=crown.data.materials[slot].copy();material.name+=f' phase {index:02}';obj.material_slots[slot].link='OBJECT';obj.material_slots[slot].material=material
    if slot==0:path=Path(inputs['phases'][index]['image'])
    elif slot==2:
     rgba=np.array(Image.open(inputs['approved_atlas']).convert('RGBA'));rgba[domain,3]=np.minimum(rgba[domain,3],phases[index][domain,3]);path=dest/f'phase-{index:02}-paired-backs.png';Image.fromarray(rgba).save(path)
    else:
     rgba=packed.copy();rgba[gated,3]=np.minimum(rgba[gated,3],phases[index][iy[gated],ix[gated],3]);assert np.array_equal(rgba[:,:,:3],packed[:,:,:3]);assert np.array_equal(rgba[~gated],packed[~gated]);path=dest/f'phase-{index:02}-packed-backs.png';Image.fromarray(rgba).save(path);gates.append(dict(phase=index,changed_alpha_pixels=int((rgba[:,:,3]!=packed[:,:,3]).sum()),rgb_unchanged=True,outside_temporal_domain_unchanged=True))
    texture(material).image=bpy.data.images.load(str(path),check_existing=False)
   obj['animation_phase']=index;obj['phase_source_sha256']=inputs['phases'][index]['source_sha256']
  timeline=[1]
  for phase in inputs['phases']:timeline.append(timeline[-1]+phase['delay'])
  scales=[o.scale.copy() for o in variants]
  for phase,time in enumerate(timeline):
   active=phase%14
   for index,obj in enumerate(variants):obj.scale=scales[index] if active==index else (0,0,0);obj.keyframe_insert(data_path='scale',frame=time)
  for obj in variants:
   action=obj.animation_data.action
   for slot in action.slots:
    for layer in action.layers:
     for strip in layer.strips:
      bag=strip.channelbag(slot)
      if bag:
       for curve in bag.fcurves:
        for key in curve.keyframe_points:key.interpolation='CONSTANT'
  scene.render.fps=30;scene.frame_start=1;scene.frame_end=timeline[-1];scene.frame_set(1);bpy.context.view_layer.update()
  assert guard=={o.name:_geometry(o,protect_appearance=True) for o in scene.objects if o.name in guard},'Approved originals changed'
  for other in list(bpy.data.scenes):
   if other!=scene:bpy.data.scenes.remove(other)
  bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'))
  bpy.ops.object.select_all(action='DESELECT')
  for obj in variants+wood:obj.select_set(True)
  bpy.ops.export_scene.gltf(filepath=str(dest/'phase-appearance-tree41.glb'),export_format='GLB',use_selection=True,export_animations=True,export_force_sampling=False,export_animation_mode='SCENE',export_frame_range=True,export_cameras=False,export_lights=False)
  scene.render.engine='CYCLES';scene.cycles.samples=24;scene.cycles.transparent_max_bounces=64
  originalviews=frames['views'];frames['views']=[originalviews[i] for i in (0,2)];frames['tile_size']=[512,512]
  for view in frames['views']:view['crop']={'left':0,'top':0,'width':512,'height':512}
  audits=[]
  for phase in (0,7,11):
   scene.frame_set(timeline[phase]);frames['object_names']=[o.name for o in wood]+[variants[phase].name];savejson(dest/f'phase-{phase:02}-views.json',frames);render(dest/f'phase-{phase:02}-views.json',dest/f'phase-{phase:02}',width=512)
   # Source-native one-pixel-per-pixel alpha audit on actual saved crown geometry.
   audit=bpy.data.scenes.new('Native phase alpha audit');copy=variants[phase].copy();copy.animation_data_clear();copy.matrix_world=variants[phase].matrix_world.copy();copy.scale=scales[phase];copy.hide_render=False;audit.collection.objects.link(copy)
   yy,xx=np.where(domain);left,right=int(xx.min()+x)-5,int(xx.max()+x)+6;top,bottom=int(yy.min()+y)-5,int(yy.max()+y)+6;ww,hh=right-left,bottom-top
   target=Vector(((left+right)/2,-(top+bottom)/2/SIN,0));camdata=bpy.data.cameras.new('Native alpha');camdata.type='ORTHO';camdata.sensor_fit='HORIZONTAL';camdata.ortho_scale=ww;camdata.clip_end=20000;cam=bpy.data.objects.new('Native alpha',camdata);audit.collection.objects.link(cam);cam.location=target+RAY*5000;cam.rotation_euler=(target-cam.location).to_track_quat('-Z','Y').to_euler();audit.camera=cam
   audit.render.engine='CYCLES';audit.cycles.samples=8;audit.cycles.transparent_max_bounces=128;audit.render.resolution_x=ww;audit.render.resolution_y=hh;audit.render.resolution_percentage=100;audit.render.film_transparent=True;audit.render.image_settings.file_format='PNG';audit.render.image_settings.color_mode='RGBA';audit.view_settings.view_transform='Standard';audit.render.filepath=str(dest/f'phase-{phase:02}-native-alpha.png');bpy.ops.render.render(write_still=True,scene=audit.name)
   actual=np.array(Image.open(audit.render.filepath).convert('RGBA'))[:,:,3]>127;expected=phases[phase][top-y:bottom-y,left-x:right-x,3]>127;assert actual.shape==expected.shape;missing=expected&~actual;extra=actual&~expected
   visual=np.array(Image.open(inputs['phases'][phase]['image']).convert('RGB').crop((left-x,top-y,right-x,bottom-y)));visual[missing]=[255,30,30];visual[extra]=[0,220,255];Image.fromarray(visual).save(dest/f'phase-{phase:02}-alpha-difference.png');audits.append(dict(phase=phase,expected_pixels=int(expected.sum()),actual_pixels=int(actual.sum()),missing_pixels=int(missing.sum()),extra_pixels=int(extra.sum()),iou=float((expected&actual).sum()/max(1,(expected|actual).sum()))))
   bpy.data.objects.remove(copy,do_unlink=True);bpy.data.objects.remove(cam,do_unlink=True);bpy.data.cameras.remove(camdata);bpy.data.scenes.remove(audit);bpy.context.window.scene=scene
  sheet=Image.new('RGB',(1024,1596),'#ddd');draw=ImageDraw.Draw(sheet)
  for row,phase in enumerate((0,7,11)):
   for col,view in enumerate((0,2)):
    image=Image.open(dest/f'phase-{phase:02}'/f'view-{view}-textured.png').convert('RGB');sheet.paste(image,(col*512,row*532+20));draw.text((col*512+8,row*532+5),f'Own native RGBA phase{phase} '+('source' if view==0 else 'oblique'),fill='black')
  sheet.save(dest/'actual-phase-comparison.png');savejson(dest/'proof.json',dict(inputs_sha256=sha(dest/'inputs.json'),model_sha256=sha(base),candidate_sha256=sha(dest/'worker.blend'),glb_sha256=sha(dest/'phase-appearance-tree41.glb'),approved_originals_unchanged=True,phase0_geometry_and_appearance_unchanged=True,geometry_shared_across_phases=True,ray_depth_unchanged=True,packed_uv_mapping='Explicit triangle UV rasterization to world then native source coordinates',packed_back_alpha_gates=gates,source_alpha_audits=audits,temporal_new_pixels=inputs['temporal_new_pixels'],holds=['Temporal ownership5px dilation needs neighboring-crown review.','Appearance animation uses phase-specific material variants and standard node-scale STEP tracks, not final production packaging.','Rear alpha suppression is source-camera-driven inference; rear color remains approved original.','Runtime/editor integration and tick-rate parity not implemented.']))
 finally:release()
if __name__=='__main__':main()
