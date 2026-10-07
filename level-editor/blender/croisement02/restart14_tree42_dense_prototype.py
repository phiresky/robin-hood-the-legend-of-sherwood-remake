"""Prepare a private subpixel crown-motion candidate without changing materials."""
from pathlib import Path
import sys,json,hashlib
import bpy,numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender'),str(Path(__file__).parent)]
from render_slots import acquire,release
from refinement_workspace import _geometry
from tree_geometry import SIN,COS,RAY
OUT=ROOT/'level-editor/work/croisement02-refinement';BASE=OUT/'restart2-textures/approved6-canopy-fill-v1/croisement02-tree-42/native-front-preparation-v2/experiment/bake-v1/worker.blend';DEST=OUT/('restart14-canopy-animation/tree42-motion-v4'if '--coherent'in sys.argv else 'restart14-canopy-animation/tree42-motion-v3'if '--smooth'in sys.argv else 'restart14-canopy-animation/tree42-motion-v2');sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
def sample_field(field,points,bbox):
 x=points[:,0]-bbox[0]-.5;y=points[:,1]-bbox[1]-.5;h,w=field.shape[:2];ok=(x>=0)&(y>=0)&(x<w-1)&(y<h-1);ix=np.clip(np.floor(x).astype(int),0,w-2);iy=np.clip(np.floor(y).astype(int),0,h-2);fx=np.clip(x-ix,0,1)[:,None];fy=np.clip(y-iy,0,1)[:,None];v=field[iy,ix]*(1-fx)*(1-fy)+field[iy,ix+1]*fx*(1-fy)+field[iy+1,ix]*(1-fx)*fy+field[iy+1,ix+1]*fx*fy;v[~ok]=0;return v

def main():
 DEST.mkdir(exist_ok=False);assert sha(BASE)=='8c89650bc54509823ece450fa41109bfbf19f891743bb556175fd02928b849e5';flow_path=OUT/('restart14-canopy-animation/tree42-coherent-correspondence-v2/flows.npz'if '--coherent'in sys.argv else 'restart14-canopy-animation/tree42-smooth-correspondence-v1/flows.npz'if '--smooth'in sys.argv else 'restart14-canopy-animation/tree42-dense-correspondence-v1/flows.npz');packet=np.load(flow_path);bpy.ops.wm.open_mainfile(filepath=str(BASE));scene=bpy.data.scenes['Croisement02 Refinement'];bpy.context.window.scene=scene;objects=[o for o in scene.objects if o.type=='MESH'and o.get('asset_group')=='croisement02-tree-42'];guards={o.name:_geometry(o,protect_appearance=True)for o in objects};crown=next(o for o in objects if o.get('projection_component')=='crown');world=np.array([crown.matrix_world@v.co for v in crown.data.vertices]);native=np.c_[world[:,0],-world[:,1]*SIN-world[:,2]*COS];initial=np.array([v.co[:]for v in crown.data.vertices]);inverse=crown.matrix_world.inverted().to_3x3();crown.shape_key_add(name='Approved static basis');stats=[]
 for phase,field in enumerate(packet['flow'][1:],1):
  delta=sample_field(field,native,packet['bbox']);disp=np.c_[delta[:,0],-delta[:,1]*SIN,-delta[:,1]*COS];local=np.array([inverse@Vector(v)for v in disp]);key=crown.shape_key_add(name=f'Native subpixel motion {phase:02}');key.data.foreach_set('co',(initial+local).reshape(-1));stats.append({'phase':phase,'moving_vertices':int(np.count_nonzero(np.linalg.norm(disp,axis=1)>1e-5)),'maximum_displacement':float(np.linalg.norm(disp,axis=1).max()),'ray_depth_change_max':float(np.max(np.abs(disp@np.array(RAY))))})
 for phase in range(15):
  for i,key in enumerate(crown.data.shape_keys.key_blocks[1:],1):key.value=float(i==phase%14);key.keyframe_insert('value',frame=1+phase*4)
 action=crown.data.shape_keys.animation_data.action
 for layer in action.layers:
  for strip in layer.strips:
   for slot in action.slots:
    bag=strip.channelbag(slot)
    if bag:
     for curve in bag.fcurves:
      for key in curve.keyframe_points:key.interpolation='CONSTANT'
 scene.render.fps=25;scene.frame_start=1;scene.frame_end=56;scene.frame_set(1);bpy.context.view_layer.update();assert guards=={o.name:_geometry(o,protect_appearance=True)for o in objects}
 for o in scene.objects:
  if o.type=='MESH':o.hide_render=o not in objects
 for s in list(bpy.data.scenes):
  if s!=scene:bpy.data.scenes.remove(s)
 bpy.ops.wm.save_as_mainfile(filepath=str(DEST/'prototype.blend'));bpy.ops.wm.open_mainfile(filepath=str(DEST/'prototype.blend'));scene=bpy.context.scene;scene.frame_set(1);assert guards=={name:_geometry(scene.objects[name],protect_appearance=True)for name in guards};(DEST/'report.json').write_text(json.dumps({'status':'PRIVATE_SUBPIXEL_PROTOTYPE_PENDING_COVERAGE','source_worker_sha256':sha(BASE),'prototype_sha256':sha(DEST/'prototype.blend'),'flow_sha256':sha(flow_path),'static_basis_appearance_uv_geometry_exact':True,'wood_fixed':True,'phase_motion':stats,'limits':['Image-plane fit plus smooth local inference; rear/depth not independently observed.','No texture changes; coverage and temporal continuity must be measured before acceptance.']},indent=2)+'\n')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
