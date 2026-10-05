"""Measure exact residual foliage pixels against the sign and evaluated bank."""
import sys,json
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from tree_geometry import SIN,COS,RAY
from sign_source_raster import raster
from restart2_sign_fragment_bounds import Surface
from sign_rigid_depth import groups_for

def main():
 dest=OUT/'restart2-fence/sign7-residual-depth-v2';dest.mkdir(exist_ok=False)
 base=OUT/'restart2-fence/shrub57-sign-bend-v7/model.blend';bpy.ops.wm.open_mainfile(filepath=str(base));obj=bpy.data.objects['West Rock Foliage 57'];box=[59,274,78,288]
 rgba,roles,depth,faces=raster(obj,box,True);world=np.array([tuple(obj.matrix_world@v.co) for v in obj.data.vertices]);project=np.column_stack((world[:,0],-SIN*world[:,1]-COS*world[:,2]));groups,roots=groups_for(obj.data,world,project)
 pixels=[(76,285),(73,286),(75,286),(76,286),(60,275)];leaf={}
 for x,y in pixels:
  f=int(faces[y-box[1],x-box[0]]);g=roots[obj.data.polygons[f].vertices[0]];leaf[(x,y)]=dict(face=f,component=g,depth=float(depth[y-box[1],x-box[0]]),component_min=world[groups[g]].min(0).tolist(),component_max=world[groups[g]].max(0).tolist())
 manifest=json.loads((OUT/'restart2-fence/sign-neighbors-v4/manifest.json').read_text());banks=[]
 for key in ['north-woodland-bank','west-rock-outcrop']:
  info=manifest['inputs'][key];p=Path(info['worker'])/'model.blend';bpy.ops.wm.open_mainfile(filepath=str(p));bpy.context.view_layer.update();vertices=[];triangles=[]
  for n in info['objects']:
   o=bpy.data.objects[n];o.data.calc_loop_triangles();offset=len(vertices);vertices.extend(tuple(o.matrix_world@v.co) for v in o.data.vertices);triangles.extend(tuple(offset+i for i in t.vertices) for t in o.data.loop_triangles)
  banks.append((key,BVHTree.FromPolygons(vertices,triangles,all_triangles=True)))
 assembly=OUT/'state-sign-candidate/five-instances-v3';row=next(r for r in json.loads((assembly/'assembly.json').read_text())['instances'] if r['target_index']==7);bpy.ops.wm.open_mainfile(filepath=str(assembly/'model.blend'));scene=bpy.context.scene;records=[]
 for phase in range(32):
  scene.frame_set(1+phase*2);bpy.context.view_layer.update();s=Surface([('sign',scene.objects[n]) for n in row['parts'] if 'native_body_frame' in scene.objects[n] and scene.objects[n].scale.x>.5])
  for x,y in pixels:
   origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000;hits=list(s.intersections(origin))
   if not hits:continue
   front=float(hits[0][0].dot(RAY));back=float(hits[-1][0].dot(RAY));b=[]
   for key,bank in banks:
    h=bank.ray_cast(origin,-RAY,10000)
    if h[0] is not None:b.append(dict(asset=key,depth=float(h[0].dot(RAY)),world=list(h[0])))
   records.append(dict(phase=phase,pixel=[x,y],leaf=leaf[(x,y)],sign_front=front,sign_back=back,retreat_to_front=leaf[(x,y)]['depth']-front,retreat_to_back=leaf[(x,y)]['depth']-back,banks=b,free_interval_behind_sign=min([back-r['depth'] for r in b],default=None)))
 write_json(dest/'report.json',dict(model_sha256=sha(base),sign_model_sha256=sha(assembly/'model.blend'),records=records));print(dest)
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
