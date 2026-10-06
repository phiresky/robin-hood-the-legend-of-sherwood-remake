"""Inspect source-ray support candidates in unchanged approved neighboring wood."""
import sys,json,shutil
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parents[1]/'refinement'),str(HERE.parents[1]/'refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from tree_geometry import RAY,SIN,COS
from refinement_review import _tree
from render_slots import acquire,release

def main():
 assert shutil.disk_usage(OUT).free>25*1024**3
 root=OUT/'restart5-initial-nets';plan=json.loads((root/'source/plan.json').read_text());context=json.loads((root/'source/context-selection-initial.json').read_text());rows=[]
 for r in context:
  model=Path(r['model']);assert sha(model)==r['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();obs=[o for o in bpy.data.objects if o.type=='MESH' and o.get('asset_group')==r['asset_id'] and o.get('projection_component')!='crown' and 'crown'not in o.name.lower()];assert obs
  tree,owners,_=_tree(obs);k=0 if int(r['asset_id'][-2:])>=43 else 1;source=plan['records'][k];ox,oy=source['origin'];px=ox+(84.5 if k==0 else 126.5);hits=[]
  for localy in range(20,source['ground_row_start'],2):
   p,n,i,dist=tree.ray_cast(Vector((px,-(oy+localy)/SIN,0))+RAY*6000,-RAY)
   if p is not None:hits.append(dict(source=[px,oy+localy],point=list(p),normal=list(n),object=owners[i].name))
  rows.append(dict(asset_id=r['asset_id'],model=str(model),model_sha256=r['model_sha256'],objects=[dict(name=o.name,matrix=[list(v)for v in o.matrix_world])for o in obs],source_ray_hits=hits))
 write_json(root/'source/support-survey.json',dict(records=rows,method='Native35degree center rays on exact unchanged approved wood; no crown approximation.'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
