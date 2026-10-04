"""Animate an isolated 3D crown using measured source-plane leaf displacement."""
import sys,json,hashlib,math
from pathlib import Path
import bpy
import numpy as np
from mathutils import Vector
from PIL import Image,ImageDraw
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from catalog import OUT,tree_workspace
from render_slots import acquire,release
from render_multiview_asset import render
from refinement_workspace import _geometry
S=math.sin(math.radians(35));C=math.cos(math.radians(35))
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 dest=OUT/'tree41-animation-proof';motion=json.loads((dest/'motion.json').read_text());base=Path(motion['model']);assert sha(base)==motion['model_sha256'];data=np.load(dest/'motion.npz');flows=data['flow'];x,y,w,h=data['bbox'];packet=json.loads((OUT/'forest-v4-sources/tree-41/partition.json').read_text());px,py,pw,ph=packet['native_bbox']
 acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(base));worker=tree_workspace(41);frames=json.loads((worker/'modified/views.json').read_text());scene=bpy.data.scenes[frames['scene_name']];bpy.context.window.scene=scene;objects=[scene.objects[n] for n in frames['object_names']];crown=next(o for o in objects if o.get('projection_component')=='crown');wood=[o for o in objects if o!=crown];guard={o.name:_geometry(o,protect_appearance=True) for o in wood};mesh=crown.data;rest=np.array([v.co[:] for v in mesh.vertices]);uv=mesh.uv_layers.active
  # Every existing leaf vertex samples its own source-atlas coordinate. This
  # keeps paired fronts/backs coherent and carries local material motion into depth.
  coords=np.zeros((len(rest),2));counts=np.zeros(len(rest))
  for loop in mesh.loops:
   u,v=uv.data[loop.index].uv;coords[loop.vertex_index]+=[px+u*pw,py+(1-v)*ph];counts[loop.vertex_index]+=1
  coords/=counts[:,None];ix=np.clip(np.rint(coords[:,0]-x).astype(int),0,w-1);iy=np.clip(np.rint(coords[:,1]-y).astype(int),0,h-1);inside=(coords[:,0]>=x)&(coords[:,0]<x+w)&(coords[:,1]>=y)&(coords[:,1]<y+h)
  basis=crown.shape_key_add(name='Approved static phase 0');keys=[];audits=[];inv=crown.matrix_world.inverted().to_3x3()
  for phase in range(1,len(flows)):
   field=flows[phase,iy,ix].copy();field[~inside]=0
   displacements=np.array([inv@Vector((float(dx),-S*float(dy),-C*float(dy))) for dx,dy in field]);key=crown.shape_key_add(name=f'Native measured phase {phase:02}');key.data.foreach_set('co',(rest+displacements).astype(np.float32).ravel());keys.append(key)
   worlddelta=np.array([crown.matrix_world.to_3x3()@Vector(d) for d in displacements]);depth=worlddelta@np.array([0,-C,S]);audits.append(dict(phase=phase,animated_vertices=int((np.linalg.norm(displacements,axis=1)>.01).sum()),max_displacement=float(np.linalg.norm(displacements,axis=1).max()),maximum_depth_delta=float(np.abs(depth).max())))
  timeline=[1];
  for phase in motion['phases']:timeline.append(timeline[-1]+phase['delay'])
  for phase,time in enumerate(timeline):
   for number,key in enumerate(keys,1):key.value=1 if phase==number else 0;key.keyframe_insert(data_path='value',frame=time)
  action=crown.data.shape_keys.animation_data.action
  # Blender's glTF exporter accepts the native shape-key action; no custom extension.
  for slot in action.slots:
   for layer in action.layers:
    for strip in layer.strips:
     channelbag=strip.channelbag(slot)
     if channelbag:
      for curve in channelbag.fcurves:
       for keyframe in curve.keyframe_points:keyframe.interpolation='CONSTANT'
  scene.render.fps=30;scene.frame_start=1;scene.frame_end=timeline[-1];scene.frame_set(1)
  assert np.array_equal(rest,np.array([v.co[:] for v in mesh.vertices]));assert guard=={o.name:_geometry(o,protect_appearance=True) for o in wood}
  bpy.ops.wm.save_as_mainfile(filepath=str(dest/'worker.blend'))
  bpy.ops.object.select_all(action='DESELECT')
  for o in objects:o.select_set(True)
  bpy.context.view_layer.objects.active=crown
  for other in list(bpy.data.scenes):
   if other!=scene:bpy.data.scenes.remove(other)
  bpy.ops.export_scene.gltf(filepath=str(dest/'animated-tree41.glb'),export_format='GLB',use_selection=True,export_animations=True,export_force_sampling=False,export_frame_range=True,export_morph=True,export_cameras=False,export_lights=False,export_yup=True)
  frames['views']=[frames['views'][i] for i in (0,2)];frames['tile_size']=[512,512]
  for view in frames['views']:view['crop']={'left':0,'top':0,'width':512,'height':512}
  (dest/'views.json').write_text(json.dumps(frames,indent=2)+'\n');scene.render.engine='CYCLES';scene.cycles.samples=24
  for phase in (0,7,11):scene.frame_set(timeline[phase]);render(dest/'views.json',dest/f'phase-{phase:02}',width=512)
  sheet=Image.new('RGB',(1024,1596),'#ddd');draw=ImageDraw.Draw(sheet)
  for row,phase in enumerate((0,7,11)):
   for col,view in enumerate((0,2)):
    image=Image.open(dest/f'phase-{phase:02}'/f'view-{view}-textured.png').convert('RGB');sheet.paste(image,(col*512,row*532+20));draw.text((col*512+8,row*532+5),f'Native phase{phase} '+('source' if view==0 else 'oblique'),fill='black')
  sheet.save(dest/'representative-phases.png')
  report=dict(model_sha256=sha(base),candidate_sha256=sha(dest/'worker.blend'),glb_sha256=sha(dest/'animated-tree41.glb'),wood_unchanged=True,approved_base_phase_unchanged=True,phases=audits,native_delays=[p['delay'] for p in motion['phases']],preview_ticks_per_second=30,coverage_supported_vertices=int(inside.sum()),total_crown_vertices=len(rest),holds=['Preview speed30 native ticks per second is presentation only; runtime tick-rate parity unverified.','Motion-only proof retains approved phase0 RGB/alpha; native per-phase RGB+alpha changes and temporal union are not fully reproduced.','GLB uses supported morph-target animation but editor runtime integration remains unimplemented.','Rear motion follows own source-atlas correspondence as inference.'])
  (dest/'proof.json').write_text(json.dumps(report,indent=2)+'\n')
 finally:release()
if __name__=='__main__':main()
