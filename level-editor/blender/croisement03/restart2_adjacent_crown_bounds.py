"""Measure frozen private tree proportions and record source versus inferred scope."""
import sys,json
from pathlib import Path
import bpy
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT/'level-editor/refinement'))
from render_slots import acquire,release

def main():
 p=ROOT/'level-editor/work/croisement03-refinement/restart2'/sys.argv[sys.argv.index('--')+1];assert not (p/'proportions.json').exists();acquire()
 try:
  bpy.ops.wm.open_mainfile(filepath=str(p/'worker.blend'));rows=[];allpoints=[]
  for o in bpy.data.scenes['Tree13 isolated wood'].objects:
   if o.type!='MESH':continue
   ps=[o.matrix_world@v.co for v in o.data.vertices];allpoints.extend(ps);rows.append(dict(name=o.name,group=o.get('asset_group'),branch=bool(o.get('inferred_branch')),bounds=[[min(v[i] for v in ps),max(v[i] for v in ps)] for i in range(3)]))
  bounds=[[min(v[i] for v in allpoints),max(v[i] for v in allpoints)] for i in range(3)];size=[hi-lo for lo,hi in bounds];(p/'proportions.json').write_text(json.dumps(dict(world_bounds=bounds,width_depth_height=size,depth_at_least_width=size[1]>=size[0],objects=rows,limits=['Original raster clips the upper canopy; upper crown volume and forks are inferred.','Exact Arbre06 frame interval is private physical context; original dynamic ownership remains unchanged.']),indent=2)+'\n');print(size)
 finally:release()
if __name__=='__main__':main()
