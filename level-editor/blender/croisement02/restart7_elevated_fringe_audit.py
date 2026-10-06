"""Read-only subpixel and projected-footprint audit for two elevated source edges."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.geometry import barycentric_transform
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree38_contour import ROOT,OUT,RAY,SIN,projected_tree
from refinement_review import _tree
from evidence_io import sha,write_json
from render_slots import acquire,release
D=OUT/'restart7-elevated-fringe-audit-v1'
def clip(poly,axis,bound,greater):
 result=[]
 for a,b in zip(poly,poly[1:]+poly[:1]):
  ia=(a[axis]>=bound)if greater else(a[axis]<=bound);ib=(b[axis]>=bound)if greater else(b[axis]<=bound)
  if ia:result.append(a)
  if ia!=ib:
   t=(bound-a[axis])/(b[axis]-a[axis]);result.append(a+t*(b-a))
 return result

def main():
 D.mkdir(exist_ok=False);rows=[]
 for number,pixel,digest in [(19,[1594,147],'8908000d0712c8383074bc464b17cb6c39346d4bb39bb80791063141c11187dd'),(25,[117,918],'2c7f634653453cbe2514b59db2390d4f07b0b232d252e376d3b03cb739f4a88c')]:
  model=ROOT/f'tree{number}-toe-finished-v12/model.blend';assert sha(model)==digest;bpy.ops.wm.open_mainfile(filepath=str(model));bpy.context.view_layer.update();own=[o for o in bpy.context.scene.objects if o.type=='MESH'and o.get('asset_group')==f'croisement02-tree-{number}'];wood=[o for o in own if o.get('projection_component')!='crown'and 'crown'not in o.name.lower()];tree,owners,_=_tree(own);pt,verts,world,faces,names=projected_tree(wood);x,y=pixel;q,_,idx,dist=pt.find_nearest(Vector((x+.5,y+.5,0)));ids=faces[idx];nearest=barycentric_transform(q,*[verts[k]for k in ids],*[world[k]for k in ids]);hits=[]
  for dy in (np.arange(64)+.5)/64:
   for dx in (np.arange(64)+.5)/64:
    p,_,j,_=tree.ray_cast(Vector((x+dx,-(y+dy)/SIN,0))+RAY*6000,-RAY)
    if p is not None and p.z>=0:hits.append(dict(offset=[float(dx),float(dy)],owner=owners[j].name,z=p.z))
  overlaps=[]
  for ids,name in zip(faces,names):
   poly=[np.array(verts[k][:2])for k in ids]
   if max(v[0]for v in poly)<x or min(v[0]for v in poly)>x+1 or max(v[1]for v in poly)<y or min(v[1]for v in poly)>y+1:continue
   for axis,bound,greater in[(0,x,True),(0,x+1,False),(1,y,True),(1,y+1,False)]:
    if poly:poly=clip(poly,axis,bound,greater)
   if len(poly)<3:continue
   area=abs(sum(a[0]*b[1]-b[0]*a[1]for a,b in zip(poly,poly[1:]+poly[:1])))/2
   if area>1e-10:overlaps.append(dict(owner=name,area=float(area),polygon=[p.tolist()for p in poly]))
  rows.append(dict(tree=number,pixel=pixel,model=str(model),model_sha256=digest,nearest_object=names[idx],nearest_source=list(q)[:2],nearest_world=list(nearest),center_to_surface_pixels=dist,positive_samples_of4096=len(hits),sample_hits=hits,wood_triangle_intersections=overlaps,scope='Projection clipping includes all wood triangle surfaces, with no source alpha assumption. Subsamples use full own tree physical opacity; no ground or model mutation.'));assert sha(model)==digest
 write_json(D/'report.json',dict(status='Read-only exact projected footprint and64x64 subpixel ray audit',records=rows,publication=False));print([(r['tree'],r['positive_samples_of4096'],len(r['wood_triangle_intersections']))for r in rows],flush=True)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
