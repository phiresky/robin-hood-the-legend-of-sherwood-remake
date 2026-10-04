"""Check native mission sign elevations against the exact selected bank receiver."""
import json,sys,math
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,bank_workspace
from tree_geometry import SIN,COS,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 root=OUT/'state-sign-candidate';fit=json.loads((root/'fit-v6/fit.json').read_text());level_path=OUT/'baseline/Croisement02.rhp.json';level=json.loads(level_path.read_text());worker=bank_workspace('croisement02-north-woodland-bank');model=worker/'model.blend';digest=sha(model);bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();banks=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-north-woodland-bank'];assert len(banks)==5
 bvh=[]
 for o in banks:
  vertices=[o.matrix_world@v.co for v in o.data.vertices];bvh.append((o.name,BVHTree.FromPolygons(vertices,[tuple(p.vertices)for p in o.data.polygons])))
 records=[]
 for instance in fit['instances']:
  t=instance['target'];obstacle=t['obstacle_index'];native_z=0.
  if obstacle!=65535:
   points=level['sight_obstacles'][obstacle]['points'];heights=[p['z_top']for p in points];assert max(heights)-min(heights)<.002;native_z=sum(heights)/len(heights)
  z=native_z/COS;x=t['position_x'];y=t['position_y'];world=Vector((x,-(y+native_z)/SIN,z));contacts=[]
  for name,tree in bvh:
   hit,normal,index,distance=tree.ray_cast(world+Vector((0,0,2000)),Vector((0,0,-1)),4000)
   if hit is not None:contacts.append(dict(receiver=name,z=hit.z,clearance=world.z-hit.z,normal=list(normal)))
  terrain=max([0.,*[c['z']for c in contacts]]);records.append(dict(target_index=instance['target_index'],mission=instance['mission'],native_target=t,native_implicit_z=native_z,world_anchor=list(world),native_projected_anchor=[world.x,-SIN*world.y-COS*world.z],bank_contacts=contacts,ground_fallback_z=0,physical_clearance=world.z-terrain,status='Contact matches native placement' if abs(world.z-terrain)<.01 else 'HOLD native display elevation differs from physical terrain'))
 assert sha(model)==digest
 write_json(root/'placement-audit.json',dict(status='Read-only native pose versus selected terrain contact audit',native_level_sha256=sha(level_path),bank_model_sha256=digest,bank_model=str(model),records=records,semantics='Negative serialized position_z computes height from the associated projection top plane, otherwise0. Later action coordinates do not recompute display position. Native gameplay height and physical display support are audited separately.',model_changes=False));print([(r['target_index'],r['native_implicit_z'],r['physical_clearance'])for r in records])

if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
