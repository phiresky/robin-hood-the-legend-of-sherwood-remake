"""Read-only crown support diagnostic with physical cutout alpha at nearest points."""
import argparse,json,sys
from pathlib import Path
import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from evidence_io import sha,write_json
from physical_opacity import OpacityRegistry,_alpha
from render_slots import acquire,release

def main():
 p=argparse.ArgumentParser();p.add_argument('worker',type=Path);p.add_argument('output',type=Path);a=p.parse_args(sys.argv[sys.argv.index('--')+1:]);w=a.worker.resolve();digest=sha(w/'model.blend');a.output.parent.mkdir(parents=True,exist_ok=True)
 if a.output.exists():raise FileExistsError(a.output)
 bpy.ops.wm.open_mainfile(filepath=str(w/'model.blend'));bpy.context.view_layer.update();cfg=json.loads((w/'workspace.json').read_text());objs=[o for o in bpy.data.collections['Croisement02 Working'].all_objects if o.type=='MESH' and o.get('asset_group')==cfg['asset_id']];crown,=[o for o in objs if o.get('projection_component')=='crown'];wood=[o for o in objs if o!=crown]
 tip=max((o.matrix_world@v.co for o in wood for v in o.data.vertices),key=lambda p:p.z)
 dep=bpy.context.evaluated_depsgraph_get();evaluated=crown.evaluated_get(dep);mesh=evaluated.to_mesh();mesh.calc_loop_triangles();verts=[evaluated.matrix_world@v.co for v in mesh.vertices];triangles=[tuple(t.vertices) for t in mesh.loop_triangles];opacity=OpacityRegistry()
 for t in mesh.loop_triangles:opacity.add(evaluated,mesh,t)
 tree=BVHTree.FromPolygons(verts,triangles,all_triangles=True);hits=tree.find_nearest_range(tip,30);accepted=[]
 for point,normal,index,distance in hits:
  record=opacity.records[index];alpha=1 if record is None else _alpha(record,point)
  if alpha>=.5:accepted.append(dict(point=list(point),distance=distance,alpha=alpha,triangle=index))
 accepted.sort(key=lambda row:row['distance']);write_json(a.output,dict(model_sha256=digest,wood_tip=list(tip),search_radius=30,triangles_near_tip=len(hits),physically_opaque_closest_points=len(accepted),nearest_occupied_sample=accepted[0] if accepted else None,interpretation='Distance to a physically opaque nearest point on a triangle; upper bound, not exhaustive alpha-boundary distance. Physical opacity only; source ownership not used.'))
 evaluated.to_mesh_clear()
 if sha(w/'model.blend')!=digest:raise ValueError('Read-only model changed')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
