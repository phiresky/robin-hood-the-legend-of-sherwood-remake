"""Inventory individual initial-fence members without changing the worker."""
import sys,json
from pathlib import Path
import bpy,numpy as np
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import write_json,sha
from tree_geometry import SIN,COS

def main():
 p=OUT/'texture-fill-round-1/croisement02-south-field-wattle-fence/experiment/bake-v1/worker.blend';bpy.ops.wm.open_mainfile(filepath=str(p));bpy.context.view_layer.update();rows=[]
 for obj in bpy.context.scene.objects:
  if obj.type!='MESH' or obj.get('asset_group')!='croisement02-south-field-wattle-fence':continue
  adj=[[]for v in obj.data.vertices]
  for e in obj.data.edges:a,b=e.vertices;adj[a].append(b);adj[b].append(a)
  seen=set();components=[]
  for i in range(len(adj)):
   if i in seen:continue
   q=[i];seen.add(i);ids=[]
   while q:
    n=q.pop();ids.append(n)
    for k in adj[n]:
     if k not in seen:seen.add(k);q.append(k)
   p=np.array([obj.matrix_world@obj.data.vertices[n].co for n in ids]);screen=np.column_stack([p[:,0],-p[:,1]*SIN-p[:,2]*COS]);components.append(dict(vertices=ids,bounds=np.array([p.min(0),p.max(0)]).tolist(),screen_bounds=np.array([screen.min(0),screen.max(0)]).tolist(),center=p.mean(0).tolist()))
  rows.append(dict(object=obj.name,source_node=obj.get('source_node'),components=components,materials=[m.name for m in obj.data.materials],matrix=[list(r)for r in obj.matrix_world]))
 d=OUT/'restart3-initial-fence/member-survey-v1';d.mkdir(exist_ok=False);write_json(d/'survey.json',dict(worker_sha256=sha(OUT/'texture-fill-round-1/croisement02-south-field-wattle-fence/experiment/bake-v1/worker.blend'),objects=rows));print([(r['object'],len(r['components']))for r in rows],flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
