"""Compare native initial cut-plane intersections with the approved baseline."""
import sys,json
from pathlib import Path
import bpy
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from render_slots import acquire,release
from evidence_io import sha,write_json

def section(obj,cut):
 world=[obj.matrix_world@v.co for v in obj.data.vertices];result=[]
 for edge in obj.data.edges:
  a,b=(world[i]for i in edge.vertices)
  if min(a.x,b.x)<cut<max(a.x,b.x):
   p=a.lerp(b,(cut-a.x)/(b.x-a.x));result.append(tuple(round(v,6)for v in p))
 return sorted(result)
def main():
 variant=sys.argv[sys.argv.index('--')+1] if '--' in sys.argv else 'geometry-v5';d=OUT/'restart3-initial-fence'/variant;models=[OUT/'texture-fill-round-1/croisement02-south-field-wattle-fence/experiment/bake-v1/worker.blend',d/'model.blend'];rows=[]
 for model in models:
  bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();rows.append({o.name:{str(c):section(o,c)for c in [1018,1170]}for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')=='croisement02-south-field-wattle-fence'})
 proof=[]
 for name,sections in rows[0].items():
  for cut,points in sections.items():
   new=rows[1][name][cut];proof.append(dict(object=name,cut=float(cut),baseline_crossings=len(points),candidate_crossings=len(new),exact_equal=points==new,changed_pairs=sum(a!=b for a,b in zip(points,new))))
 write_json(d/'cut-plane-guard.json',dict(status='PASS' if all(r['exact_equal']for r in proof)else 'HOLD incompatible cut section',model_sha256=sha(models[1]),base_sha256=sha(models[0]),sections=proof,applied_model_sha256=sha(OUT/'fence-state-candidate-v2/worker.blend')));print(proof,flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
