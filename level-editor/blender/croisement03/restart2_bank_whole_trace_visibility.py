"""Check that saved source-traced bank features are actual native first hits."""
import json,math,sys,hashlib
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
R=Path(__file__).resolve().parents[3];sys.path.insert(0,str(R/'level-editor/refinement'))
from render_slots import acquire,release
B=R/'level-editor/work/croisement03-refinement';O=Path(sys.argv[sys.argv.index('--')+1]).resolve() if '--' in sys.argv else B/'restart2/bank-whole-prototype-v2';P=None;S=math.sin(math.radians(35));C=math.cos(math.radians(35));RAY=Vector((0,-C,S))
def main():
 acquire()
 try:
  model=O/'worker.blend';digest=hashlib.sha256(model.read_bytes()).hexdigest();bpy.ops.wm.open_mainfile(filepath=str(model));rows=[]
  for o in bpy.context.scene.objects:
   if not o.name.startswith('Candidate bank'):continue
   m=o.data;m.calc_loop_triangles();rows.append((o.name,BVHTree.FromPolygons([v.co for v in m.vertices],[list(t.vertices) for t in m.loop_triangles],all_triangles=True)))
  receipt=json.loads((O/'interface-construction.json').read_text());plan_path=Path(receipt.get('geometry_plan',B/'restart2/bank-whole-geometry-plan-v3/geometry.json'));geometry=json.loads(plan_path.read_text());source=json.loads((B/'restart2/bank-whole-source-plan-v1/plan.json').read_text());observed={tuple(p) for t in source['new_traces'] for key in ['points','lower'] for p in t[key]};observed|={tuple(p) for t in source['previous_main_breaks'][:2] for p in t['points']};records=[]
  for index in ('52','54'):
   for trace in geometry[index]['traces']:
    for i in trace['vertices']:
     p=Vector(geometry[index]['vertices'][i]);screen=(p.x,-p.y*S-p.z*C);matches=any(max(abs(screen[j]-q[j]) for j in (0,1))<1e-3 for q in observed)
     if not matches:continue
     hits=[]
     for name,tree in rows:
      hit,normal,face,distance=tree.ray_cast(p+RAY*5000,-RAY)
      if hit is not None:hits.append((distance,name,face))
     closest=min(hits) if hits else None;blocked=closest is not None and closest[0]<4999.99
     records.append(dict(owner=index,trace=trace['id'],source=list(screen),firsthit=closest,blocked=blocked,nearest_owner_surface_distance=next(tree for name,tree in rows if name=='Candidate bank '+index).find_nearest(p)[3],occlusion_world_distance=5000-closest[0] if closest else None))
  result=dict(model_sha256=digest,status='HOLD observed trace occlusion' if any(r['blocked'] for r in records) else 'PASS tested observed trace visibility',samples=len(records),blocked=sum(r['blocked'] for r in records),misses=sum(r['firsthit'] is None for r in records),records=records,limits=['Only source-traced crease vertices, not all possible rock pixels. Geometry point agreement alone does not guarantee visibility.'])
  (O/'saved-trace-visibility.json').write_text(json.dumps(result,indent=2)+'\n');assert hashlib.sha256(model.read_bytes()).hexdigest()==digest;print({k:v for k,v in result.items() if k not in ('records','limits')})
 finally:release()
if __name__=='__main__':main()
