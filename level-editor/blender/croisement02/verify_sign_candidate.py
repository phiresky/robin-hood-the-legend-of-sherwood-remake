"""Reopen the saved mission sign and verify solid topology and all native poses."""
import json,math,sys
from pathlib import Path
import bpy,bmesh
import numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 base=OUT/'state-sign-candidate';directory=base/'candidate-v2';model=directory/'model.blend';digest=sha(model);fit=json.loads((base/'fit-v6/fit.json').read_text());p=fit['parameters'];bpy.ops.wm.open_mainfile(filepath=str(model));scene=bpy.context.scene;root=scene.objects['Rotating sign pivot'];parts=[scene.objects['Panneau '+s]for s in ['board','post']];topology=[]
 for o in parts:
  bm=bmesh.new();bm.from_mesh(o.data);nonmanifold=sum(not e.is_manifold for e in bm.edges);degenerate=sum(f.calc_area()<1e-9 for f in bm.faces);volume=bm.calc_volume(signed=True);bm.free();assert not nonmanifold and not degenerate and volume>0
  topology.append(dict(object=o.name,vertices=len(o.data.vertices),faces=len(o.data.polygons),nonmanifold_edges=nonmanifold,degenerate_faces=degenerate,signed_volume=volume))
 poses=[]
 for i in range(32):
  expected=math.radians(p['initial_angle_degrees']-i*11.25)
  for tick in (0,1):
   scene.frame_set(1+2*i+tick);assert abs(root.rotation_euler.z-expected)<1e-5
  poses.append(dict(native_frame=i,start_tick=i*2,duration_ticks=2,angle_degrees=math.degrees(root.rotation_euler.z)))
 scene.frame_set(65);assert abs(root.rotation_euler.z-math.radians(p['initial_angle_degrees']-360))<1e-5
 contact=(p['board_y']+p['board_thickness']/2)-(-p['post_width']/2);assert 0<=contact<=.2
 row0=fit['profile']['rows'][0];alias=[]
 from PIL import Image
 for row in fit['profile']['rows']:
  mismatches=0
  for a,b in zip(row0['frames'],row['frames']):
   assert a['offset']==b['offset'] and a['ticks']==b['ticks'];mismatches+=int((np.asarray(Image.open(a['image']))!=np.asarray(Image.open(b['image']))).sum())
  assert mismatches==0;alias.append(dict(action_id=row['action_id'],frames=32,rgba_difference_pixels=mismatches))
 assert len(fit['instances'])==5 and {i['mission']for i in fit['instances']}=={'S03_FoB_MP'}
 assert sha(model)==digest
 write_json(directory/'validation.json',dict(status='PASS solid topology, native pose timing and action alias preservation; visual/source fit and placed integration remain separate',model_sha256=digest,topology=topology,board_post_overlap_world=contact,poses=poses,action_aliases=alias,mission_instances=fit['instances'],native_ticks_per_second=25,cycle_ticks=64,cycle_seconds=2.56,unchanged_model=True,source_fit_limitations=fit['limitations'],remaining=['Independent eight-view geometry review','Neutral hidden texture completion and native source color check','Five placements on exact terrain receivers','Mission visibility and live animation integration']))
 print(directory/'validation.json')

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
