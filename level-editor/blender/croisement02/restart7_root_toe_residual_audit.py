"""Read-only residual source-center classification after private toe hypotheses."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree38_contour import ROOT,OUT,RAY,SIN,covered,projected_tree
from refinement_review import _tree
from evidence_io import sha,write_json
from render_slots import acquire,release
acquire()
try:
 result=[];inventory=json.load(open(ROOT/'remaining-inventory-v1/report.json'))['rows']
 for number in [19,25]:
  model=ROOT/f'tree{number}-toe-finished-v12/model.blend';bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();asset=f'croisement02-tree-{number}';objects=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==asset and o.get('projection_component')!='crown'];coords=next(r['coordinates']for r in inventory if r['mask']==number);tree,_,_=_tree(objects);projected,verts,world,faces,owners=projected_tree(objects);rows=[]
  for x,y in coords:
   hit,n,i,d=tree.ray_cast(Vector((x+.5,-(y+.5)/SIN,0))+RAY*6000,-RAY);center=hit is not None and hit.z>=0;count=0
   for dy in [.1,.3,.5,.7,.9]:
    for dx in [.1,.3,.5,.7,.9]:
     p,_,_,_=tree.ray_cast(Vector((x+dx,-(y+dy)/SIN,0))+RAY*6000,-RAY);count+=int(p is not None and p.z>=0)
   q,_,j,d=projected.find_nearest(Vector((x+.5,y+.5,0)));ids=faces[j];p=barycentric_transform(q,*[verts[k]for k in ids],*[world[k]for k in ids]);rows.append(dict(pixel=[x,y],center_covered=center,positive_subsamples_of25=count,projected_distance=d,nearest_world=list(p)))
  result.append(dict(mask=number,model_sha256=sha(model),targets=len(rows),centers_covered=sum(r['center_covered']for r in rows),rows=rows))
 write_json(ROOT/'source-role132-v1/private-toe-residual-v12.json',dict(records=result,scope='Private geometry only, not integrated/approved.5x5 coverage is sampled, not exhaustive. Residual shadow/ivy/AA role remains separately evaluated; no force-all contour fitting.'))
finally:release()
