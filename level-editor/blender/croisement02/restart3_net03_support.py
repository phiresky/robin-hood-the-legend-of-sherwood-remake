"""Survey native cord-preserving attachment rays against approved nearby wood."""
import json,sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from tree_geometry import SIN,COS,RAY
from evidence_io import sha,write_json
from render_slots import acquire,release


def main():
 dest=OUT/'restart3-net03/support-survey-v1'
 if dest.exists():raise FileExistsError(dest)
 acquire()
 try:
  dest.mkdir(parents=True);records=[]
  for index in [37,38,39,40]:
   worker=tree_workspace(index)/'model.blend';digest=sha(worker)
   bpy.ops.wm.open_mainfile(filepath=str(worker));bpy.context.view_layer.update()
   objects=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.get('asset_group')==f'croisement02-tree-{index:02}' and o.get('projection_component')!='crown']
   vs=[];fs=[];owners=[];details=[]
   for obj in objects:
    start=len(vs);vs.extend(obj.matrix_world@v.co for v in obj.data.vertices);obj.data.calc_loop_triangles();fs.extend(tuple(start+i for i in t.vertices)for t in obj.data.loop_triangles);owners.extend([obj.name]*len(obj.data.loop_triangles));details.append(dict(name=obj.name,component=obj.get('projection_component'),vertices=len(obj.data.vertices)))
   if not vs:raise ValueError('Missing wood '+str(index))
   bvh=BVHTree.FromPolygons(vs,fs,all_triangles=True);hits=[]
   for cord,x in [('bag',1647),('wood',1629)]:
    for shift in range(-10,151,2):
     base=Vector((x,-629/SIN,(629-530)/COS))+Vector(RAY)*shift
     hit,normal,face,distance=bvh.ray_cast(base,Vector((0,0,1)),400)
     if hit is not None:
      hits.append(dict(cord=cord,camera_ray_shift=shift,from_world=list(base),attachment_world=list(hit),normal=list(normal),extension=float(distance),object=owners[face],native_attachment=[float(hit.x),float(-hit.y*SIN-hit.z*COS)]))
   write_json(dest/f'tree-{index}-wood.json',dict(worker=str(worker),model_sha256=digest,vertices=[list(v)for v in vs],triangles=fs,owners=owners,objects=details))
   records.append(dict(tree=index,worker=str(worker),model_sha256=digest,objects=details,wood_bounds=[[min(v[i]for v in vs)for i in range(3)],[max(v[i]for v in vs)for i in range(3)]],hits=hits))
   if sha(worker)!=digest:raise ValueError('Concurrent source change')
  write_json(dest/'manifest.json',dict(status='diagnostic attachment proposals only',ground_reference_source_y=629,records=records,limitations=['Projected cord columns alone do not prove a particular branch or depth.','Only existing wood is considered; canopy overlap is not a support.']))
 finally:release()
if __name__=='__main__':main()
