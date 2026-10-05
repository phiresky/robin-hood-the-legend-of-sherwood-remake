"""Relax the graft transition above preserved roots and share external normals across owner cuts."""
import argparse,json,sys
from pathlib import Path
import bpy,numpy as np
from mathutils import Vector
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from evidence_io import sha,write_json
from render_slots import acquire,release
from refinement_workspace import _geometry
from rebuild_tree32_roots import check

def main():
 p=argparse.ArgumentParser();p.add_argument('index',type=int,choices=[43,45,46]);index=p.parse_args(sys.argv[sys.argv.index('--')+1:]).index;source=OUT/f'restart2-wood/tree{index}-branch-fitted-v1';out=OUT/f'restart2-wood/tree{index}-branch-blend-v1';out.mkdir(exist_ok=False);e= json.loads((source/'evidence.json').read_text());bpy.ops.wm.open_mainfile(filepath=str(source/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0;objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')==f'croisement02-tree-{index}' and o.get('projection_component')!='crown'];outside={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood};parts=[]
 for obj in wood:
  positions=np.array([obj.matrix_world@v.co for v in obj.data.vertices]);initial=positions.copy();edges=np.array([list(edge.vertices) for edge in obj.data.edges]);a=np.concatenate((edges[:,0],edges[:,1]));b=np.concatenate((edges[:,1],edges[:,0]));counts=np.bincount(a,minlength=len(positions));z=initial[:,2];weight=np.sin(np.pi*np.clip((z-35)/45,0,1))**2;weight[(z<=35)|(z>=80)]=0
  for step in range(80):
   mean=np.column_stack([np.bincount(a,weights=positions[b,d],minlength=len(positions)) for d in range(3)])/np.maximum(1,counts)[:,None];positions+=(mean-positions)*weight[:,None]*(.5 if step%2==0 else -.52);delta=positions-initial;length=np.linalg.norm(delta,axis=1);positions=initial+delta*np.minimum(1,3/np.maximum(length,1e-10))[:,None]
  inv=obj.matrix_world.inverted()
  for v,world in zip(obj.data.vertices,positions):v.co=inv@Vector(world)
  obj.data.update();parts.append(dict(object=obj.name,max_displacement=float(np.max(np.linalg.norm(positions-initial,axis=1))),root_vertices_unchanged=bool(np.array_equal(positions[z<=35],initial[z<=35])),topology=check(obj.data)))
 cut={43:86.,45:20.,46:25.}[index];acc={};keys={}
 for obj in wood:
  keys[obj]=[tuple(round(float(c),4) for c in obj.matrix_world@v.co) for v in obj.data.vertices]
  for face in obj.data.polygons:
   points=[obj.matrix_world@obj.data.vertices[i].co for i in face.vertices]
   if all(abs(p.z-cut)<.0005 for p in points):continue
   normal=np.array(obj.matrix_world.to_3x3()@face.normal)*face.area
   for i in face.vertices:
    key=keys[obj][i];acc[key]=acc.get(key,np.zeros(3))+normal
 for obj in wood:
  values=[]
  for v,key in zip(obj.data.vertices,keys[obj]):
   normal=acc.get(key,np.array(v.normal));length=np.linalg.norm(normal);values.append(tuple(normal/length) if length else tuple(v.normal))
  for face in obj.data.polygons:face.use_smooth=True
  obj.data.normals_split_custom_set_from_vertices(values)
 if outside!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood}:raise ValueError('Outside appearance changed')
 bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));write_json(out/'evidence.json',dict(model_sha256=sha(out/'model.blend'),previous_worker=e['previous_worker'],previous_model_sha256=e['previous_model_sha256'],geometry_input=str(source),geometry_input_sha256=sha(source/'model.blend'),parts=parts,band_z=[35,80],owner_cap_z=cut,protected_appearance=outside,status='Private bounded graft relaxation; source ray checks and projection pending'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
