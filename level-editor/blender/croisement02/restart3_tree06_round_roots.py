"""Small research mesh of smoothly tapered roots; source rays are diagnostic constraints."""
import sys,json,math
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3]
sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT
from tree_geometry import SIN,COS,RAY
from render_slots import acquire,release
from evidence_io import sha,write_json

PATHS=[
 [[651,507,67,16],[657,521,58,14],[656,535,50,8.5],[653,542,47,5.5],[652,546,45.5,2],[655,548,45,1],[655,551,41,.3]],
 [[651,514,61,9],[641,521,53,4],[641,525,49,2.8],[638,527,45.5,.8],[636,529,41,.3]],
 [[660,514,62,10],[672,519,55,6],[679,522,48,2],[681,524,45,.7],[683,526,41,.3]],
 [[659,521,58,8],[665,526,52,8],[670,531,48,4],[674,535,42,.3]],
 [[650,526,56,4],[647,534,50,1],[648,538,48,1],[651,541,46.5,2],[653,544,42,.2]],
 [[659,524,56,9],[666,533,50,5.2],[667,538,48,3.5],[665,542,46,2],[662,547,45,1],[662,550,41,.2]],
]


def main():
 dest=OUT/'restart3-tree06-root/research-roots-v4';dest.mkdir(exist_ok=False)
 bpy.ops.wm.read_factory_settings(use_empty=True);objects=[]
 for branch,path in enumerate(PATHS):
  control=np.array(path,float);samples=[]
  for i in range(len(control)-1):
   a,b,c,d=control[max(0,i-1)],control[i],control[i+1],control[min(len(control)-1,i+2)]
   for t in np.linspace(0,1,8,endpoint=False):samples.append(.5*((2*b)+(-a+c)*t+(2*a-5*b+4*c-d)*t*t+(-a+3*b-3*c+d)*t*t*t))
  samples.append(control[-1]);centers=[Vector((x,(-y-COS*z)/SIN,z))for x,y,z,r in samples];verts=[];faces=[];n=24
  for i,(center,sample)in enumerate(zip(centers,samples)):
   axis=(centers[min(i+1,len(centers)-1)]-centers[max(i-1,0)]).normalized();side=axis.cross(RAY).normalized();up=axis.cross(side).normalized()
   for j in range(n):
    angle=j*math.tau/n;radius=float(max(.15,sample[3])*(1+.025*math.cos(3*angle+i*.12)));verts.append(tuple(center+radius*(side*math.cos(angle)+up*math.sin(angle))))
  faces.append(tuple(reversed(range(n))))
  for i in range(len(centers)-1):
   for j in range(n):faces.append((i*n+j,i*n+(j+1)%n,(i+1)*n+(j+1)%n,(i+1)*n+j))
  faces.append(tuple((len(centers)-1)*n+j for j in range(n)))
  mesh=bpy.data.meshes.new('Round root branch');mesh.from_pydata(verts,[],faces);mesh.update()
  obj=bpy.data.objects.new(f'Root branch {branch}',mesh);bpy.context.scene.collection.objects.link(obj);objects.append(obj)
 bpy.ops.object.select_all(action='DESELECT')
 for o in objects:o.select_set(True)
 bpy.context.view_layer.objects.active=objects[0];bpy.ops.object.join();obj=bpy.context.object;obj.name='Northwest Tree 06 / Rounded root research'
 obj.data.remesh_voxel_size=.35;obj.data.use_remesh_preserve_volume=True;bpy.ops.object.voxel_remesh()
 bm=bmesh.new();bm.from_mesh(obj.data)
 for _ in range(3):bmesh.ops.smooth_vert(bm,verts=list(bm.verts),factor=.15,use_axis_x=True,use_axis_y=True,use_axis_z=True)
 bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));nonmanifold=sum(not e.is_manifold for e in bm.edges);bm.to_mesh(obj.data);bm.free()
 for face in obj.data.polygons:face.use_smooth=True
 obj['asset_group']='croisement02-tree-06';obj['source_node']='building-057';obj['part_name']='Inferred rounded root continuation';obj['research_only']=True
 mat=bpy.data.materials.new('Research roots / neutral blue');mat.use_nodes=True;mat.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.34,.53,.7,1);mat.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value=.9;obj.data.materials.append(mat)
 obj.data.calc_loop_triangles();tree=BVHTree.FromPolygons([tuple(v.co)for v in obj.data.vertices],[tuple(t.vertices)for t in obj.data.loop_triangles],all_triangles=True)
 region=next(r for r in json.loads((OUT/'restart3-northern-source-audit/report.json').read_text())['regions']if r['region']==8);samples=[]
 for row in region['pixels']:
  x,y=row['pixel'];origin=Vector((x+.5,-(y+.5)/SIN,0))+RAY*5000;hit,normal,face,distance=tree.ray_cast(origin,-RAY,10000)
  clearance=(hit-Vector(row['current_world_hit'])).dot(RAY)if hit is not None else None
  samples.append(dict(pixel=row['pixel'],wood=row['classification']=='wood_domain_residual',clearance=clearance,visible=clearance is not None and clearance>0))
 bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(dest/'root.blend'),compress=True)
 write_json(dest/'report.json',dict(status='Research-only smooth geometry; no source appearance or approval claim',model_sha256=sha(dest/'root.blend'),native_centerline_controls=PATHS,vertices=len(obj.data.vertices),faces=len(obj.data.polygons),nonmanifold_edges=nonmanifold,target313_covered=sum(r['wood']and r['visible']for r in samples),bank131_overreach=sum(not r['wood']and r['visible']for r in samples),samples=samples,limitations=['Six inferred tapered round roots; source samples constrain the fit but do not define per-pixel extrusions.','Current projected residual is reported explicitly; source texture projection has not been applied.','Existing approved tree geometry is not changed or copied.']))
 print(dest,flush=True)


if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
