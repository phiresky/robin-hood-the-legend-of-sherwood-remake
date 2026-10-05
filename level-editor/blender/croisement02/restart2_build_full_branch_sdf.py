"""Private continuous upper branches grafted to retained native lower stem/root volume."""
import argparse,json,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Matrix
from mathutils.kdtree import KDTree
from mathutils.bvhtree import BVHTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from refinement_workspace import _geometry
from render_slots import acquire,release
from rebuild_tree32_roots import check

def tiny_faces(bm):
 for _ in range(100):
  faces=[f for f in bm.faces if f.calc_area()<1e-9]
  if not faces:return
  edge=min(faces[0].edges,key=lambda e:e.calc_length())
  if edge.calc_length()>.01:raise ValueError('Degenerate cleanup exceeds .01 world units')
  bmesh.ops.collapse(bm,edges=[edge],uvs=False)
 raise ValueError('Degenerate cleanup did not converge')

def main():
 parser=argparse.ArgumentParser();parser.add_argument('index',type=int,choices=[43,45,46]);index=parser.parse_args(sys.argv[sys.argv.index('--')+1:]).index;worker=tree_workspace(index);digest=sha(worker/'model.blend');out=OUT/f'restart2-wood/tree{index}-branch-sdf-v2'
 if (out/'model.blend').exists():raise FileExistsError(out/'model.blend')
 bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0;objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown'];protected={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood};original=bmesh.new()
 for obj in wood:
  mesh=obj.data.copy();mesh.transform(obj.matrix_world);original.from_mesh(mesh);bpy.data.meshes.remove(mesh)
 ref=bpy.data.meshes.new('Reference source wood');original.to_mesh(ref);oldsurface=BVHTree.FromPolygons([v.co for v in ref.vertices],[list(f.vertices) for f in ref.polygons]);original.free()
 data=np.load(out/'full-volume.npz');mesh=bpy.data.meshes.new('Continuous upper native wood');mesh.from_pydata(data['vertices'].tolist(),[],data['faces'].tolist());mesh.update();upper=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(upper);bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=1e-4);bmesh.ops.dissolve_degenerate(bm,edges=list(bm.edges),dist=1e-4);tiny_faces(bm);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();check(mesh)
 mesh=upper.data.copy();bpy.data.objects.remove(upper,do_unlink=True);mesh.update();full=check(mesh);distances=[oldsurface.find_nearest(v.co)[3] for v in mesh.vertices if v.co.z<20]
 normals=KDTree(len(mesh.vertices));values=[]
 for v in mesh.vertices:normals.insert(v.co,v.index);values.append(tuple(v.normal))
 normals.balance();neutral=bpy.data.materials.new('Private unprojected continuous branches');neutral.diffuse_color=(.4,.4,.4,1);cut={43:86.,45:20.,46:25.}[index];lowerpart={43:108,45:111,46:113}[index];parts={}
 for obj in wood:
  part=int(obj['source_node'].split('-')[-1]);bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(0,0,cut),plane_no=(0,0,1),clear_inner=part!=lowerpart,clear_outer=part==lowerpart);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0);tiny_faces(bm);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));result=bpy.data.meshes.new('Continuous native branch owner');bm.to_mesh(result);bm.free()
  for attr in list(result.attributes):
   if attr.name.startswith('reprojection_'):result.attributes.remove(attr)
  obj.data=result;obj.parent=None;obj.matrix_world=Matrix.Identity(4);result.materials.append(neutral)
  for f in result.polygons:f.use_smooth=True
  result.normals_split_custom_set_from_vertices([values[normals.find(v.co)[1]] for v in result.vertices]);parts[part]=check(result)
 if protected!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood}:raise ValueError('Protected appearance changed')
 bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));write_json(out/'evidence.json',dict(model_sha256=sha(out/'model.blend'),previous_worker=str(worker),previous_model_sha256=digest,full_geometry=full,parts=parts,lower_surface_to_old_distance_max=max(distances),protected_appearance=protected,status='Private complete continuous own-trace wood hypothesis; source/ground comparison required',limitations=['Upper transverse volume and blends inferred from own native trace.','Basal volume also reconstructed as inference; old footprint and native material require independent comparison.']))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
