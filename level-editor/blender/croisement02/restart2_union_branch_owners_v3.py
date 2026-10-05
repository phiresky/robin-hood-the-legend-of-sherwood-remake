"""Private exact unions of existing capped wood components without inventing new branches."""
import argparse,json,sys
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Matrix
from mathutils.kdtree import KDTree
ROOT=Path(__file__).resolve().parents[3];sys.path[:0]=[str(Path(__file__).parent),str(ROOT/'level-editor/refinement'),str(ROOT/'level-editor/refinement/blender')]
from catalog import OUT,tree_workspace
from evidence_io import sha,write_json
from refinement_workspace import _geometry
from render_slots import acquire,release
from rebuild_tree32_roots import check

def main():
 parser=argparse.ArgumentParser();parser.add_argument('index',type=int,choices=[43,45,46]);index=parser.parse_args(sys.argv[sys.argv.index('--')+1:]).index;worker=tree_workspace(index);digest=sha(worker/'model.blend');out=OUT/f'restart2-wood/tree{index}-branch-union-v3';out.mkdir(exist_ok=False)
 bpy.ops.wm.open_mainfile(filepath=str(worker/'model.blend'));bpy.context.view_layer.update();bpy.context.preferences.filepaths.save_version=0;objects=list(bpy.data.collections['Croisement02 Working'].all_objects);wood=[o for o in objects if o.type=='MESH' and o.get('asset_group')==worker.name and o.get('projection_component')!='crown'];protected={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood};components=[]
 for original in wood:
  bm=bmesh.new();bm.from_mesh(original.data);pending=set(bm.verts)
  while pending:
   seed=pending.pop();seen={seed};stack=[seed]
   while stack:
    v=stack.pop()
    for edge in v.link_edges:
     other=edge.other_vert(v)
     if other in pending:pending.remove(other);seen.add(other);stack.append(other)
   verts=list(seen);lookup={v:i for i,v in enumerate(verts)};faces=[f for f in bm.faces if f.verts[0] in seen];mesh=bpy.data.meshes.new('Closed existing tube');mesh.from_pydata([original.matrix_world@v.co for v in verts],[],[[lookup[v] for v in f.verts] for f in faces]);mesh.update();obj=bpy.data.objects.new(mesh.name,mesh);bpy.context.scene.collection.objects.link(obj);components.append(obj)
  bm.free()
 components.sort(key=lambda o:len(o.data.polygons),reverse=True);combined=components[0];bpy.ops.object.select_all(action='DESELECT');combined.select_set(True);bpy.context.view_layer.objects.active=combined
 for piece in components[1:]:
  modifier=combined.modifiers.new('Join existing overlapping capped tubes','BOOLEAN');modifier.operation='UNION';modifier.solver='EXACT';modifier.use_self=True;modifier.object=piece;bpy.ops.object.modifier_apply(modifier=modifier.name);bpy.data.objects.remove(piece,do_unlink=True)
 modifier=combined.modifiers.new('Close Boolean slivers','REMESH');modifier.mode='VOXEL';modifier.voxel_size=.35;modifier.use_smooth_shade=True;bpy.ops.object.modifier_apply(modifier=modifier.name)
 modifier=combined.modifiers.new('Relax only narrow union ridges','SMOOTH');modifier.factor=.8;modifier.iterations=35;bpy.ops.object.modifier_apply(modifier=modifier.name)
 mesh=combined.data.copy();bpy.data.objects.remove(combined,do_unlink=True);bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.00001);bmesh.ops.dissolve_degenerate(bm,edges=list(bm.edges),dist=.00001);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();mesh.update();full=check(mesh);normals=KDTree(len(mesh.vertices));values=[]
 for v in mesh.vertices:normals.insert(v.co,v.index);values.append(tuple(v.normal))
 normals.balance();neutral=bpy.data.materials.new('Private joined wood preview');neutral.diffuse_color=(.4,.4,.4,1);cut={43:86.,45:20.,46:25.}[index];lower={43:108,45:111,46:113}[index];parts={}
 for obj in wood:
  part=int(obj['source_node'].split('-')[-1]);bm=bmesh.new();bm.from_mesh(mesh);bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(0,0,cut),plane_no=(0,0,1),clear_inner=part!=lower,clear_outer=part==lower);bmesh.ops.holes_fill(bm,edges=[e for e in bm.edges if e.is_boundary],sides=0);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));result=bpy.data.meshes.new('Continuous branch owner');bm.to_mesh(result);bm.free();obj.data=result;obj.parent=None;obj.matrix_world=Matrix.Identity(4);result.materials.append(neutral)
  for f in result.polygons:f.use_smooth=True
  result.normals_split_custom_set_from_vertices([values[normals.find(v.co)[1]] for v in result.vertices]);parts[part]=check(result)
 if protected!={o.name:_geometry(o,protect_appearance=True) for o in objects if o.type=='MESH' and o not in wood}:raise ValueError('Outside appearance changed')
 bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'));write_json(out/'evidence.json',dict(model_sha256=sha(out/'model.blend'),previous_worker=str(worker),previous_model_sha256=digest,component_count=len(components),full_geometry=full,parts=parts,owner_split_z=cut,protected_appearance=protected,status='Private union with .35 voxel closure and bounded junction smoothing; surface/source/ground checks pending',limitations=['No additional branches or root depth inferred.','Exact Boolean union removes internal intersecting caps; semantic owner partition is internal only.','Capped components with physical gaps may remain disconnected; visual review required.']))
 if sha(worker/'model.blend')!=digest:raise ValueError('Selected input changed')
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
