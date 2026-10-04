"""Test native-projection-preserving hanging-cord attachment hypotheses against tree wood."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(Path(__file__).parent)]
from catalog import OUT,tree_workspace
from log_trap_state_candidate import sha
from tree_geometry import RAY,SIN,COS


def main():
 base=OUT/'net-endpoint-candidate-v3';manifest=json.loads((base/'manifest.json').read_text());assert sha(base/'worker.blend')==manifest['model_sha256'];bpy.ops.wm.open_mainfile(filepath=str(base/'worker.blend'));scene=bpy.context.scene;wood=[];bindings=[]
 for index in [43,45,46]:
  worker=tree_workspace(index);frames=json.loads((worker/'modified/views.json').read_text());names=[n for n in frames['object_names']if 'wood 'in n];assert len(names)==2
  with bpy.data.libraries.load(str(worker/'model.blend'),link=False)as(src,dst):dst.objects=list(names)
  for obj in dst.objects:
   cursor=obj
   while cursor:
    if cursor.name not in scene.objects:scene.collection.objects.link(cursor)
    cursor=cursor.parent
   wood.append((index,obj))
  bindings.append(dict(tree=index,worker=str(worker),model_sha256=sha(worker/'model.blend'),objects=names))
 bpy.context.view_layer.update();trees=[(index,obj,BVHTree.FromPolygons([obj.matrix_world@v.co for v in obj.data.vertices],[list(p.vertices)for p in obj.data.polygons]))for index,obj in wood];records=[]
 for name in ['Bag hanging cord','Wood hanging cord']:
  obj=bpy.data.objects[name];points=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);maximum=points[:,2].max();top=Vector(tuple(points[points[:,2]>maximum-.8].mean(axis=0)));proposals=[]
  for shift in range(-90,91):
   origin=top+RAY*shift+Vector((0,0,.001))
   for index,receiver,tree in trees:
    location,normal,face,distance=tree.ray_cast(origin,Vector((0,0,1)),350)
    if location is not None:proposals.append(dict(camera_ray_shift=shift,tree=index,wood_object=receiver.name,vertical_extension=float(distance),attachment_world=list(location),attachment_source=[location.x,-location.y*SIN-location.z*COS],normal=list(normal)))
  proposals.sort(key=lambda p:(abs(p['camera_ray_shift']),p['vertical_extension']));records.append(dict(cord=name,current_upper_world=list(top),proposals=proposals,selected_for_review=proposals[0]if proposals else None))
 result=dict(status='Attachment hypotheses only; no geometry or reviewed tree changed',model_sha256=manifest['model_sha256'],tree_bindings=bindings,cords=records,limitations=['A wood ray hit does not establish native visual draw order or source attachment identity.','Any longer cord needs native canopy/wood occlusion and full-scene review.','Camera-depth shifts are inferred and must preserve bag/wood separation and ground clearance.']);(base/'tree-attachment-audit.json').write_text(json.dumps(result,indent=2)+'\n');print([(r['cord'],len(r['proposals']),r['selected_for_review'])for r in records])
if __name__=='__main__':main()
