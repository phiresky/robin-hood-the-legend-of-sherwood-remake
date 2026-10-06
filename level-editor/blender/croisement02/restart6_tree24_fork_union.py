"""Union the three overlapping tree24 fork shells and round their local join."""
import sys,json
from pathlib import Path
import bpy,bmesh,numpy as np
from mathutils import Vector
HERE=Path(__file__).resolve().parent;sys.path.insert(0,str(HERE))
from restart6_tree24_contour import ROOT
from evidence_io import sha,write_json
from render_slots import acquire,release

def main():
 source=ROOT/'tree24-contour-v2/model.blend';out=ROOT/'tree24-fork-union-v4';out.mkdir(exist_ok=False);bpy.ops.wm.open_mainfile(filepath=str(source));bpy.context.view_layer.update();obj=next(o for o in bpy.context.scene.objects if o.type=='MESH'and'Crown'not in o.name);bm=bmesh.new();bm.from_mesh(obj.data);bm.verts.ensure_lookup_table();remaining=set(bm.verts);groups=[]
 while remaining:
  todo=[remaining.pop()];component=set(todo)
  while todo:
   for e in todo.pop().link_edges:
    for v in e.verts:
     if v in remaining:remaining.remove(v);component.add(v);todo.append(v)
  groups.append({v.index for v in component})
 chosen=[g for g in groups if len(g)in[264,348,72,96]];assert len(chosen)==4;bm.free();parts=[]
 for index,g in enumerate(chosen):
  part=obj.copy();part.data=obj.data.copy();bpy.context.scene.collection.objects.link(part);part.name=f'Fork union temporary {index}';mesh=bmesh.new();mesh.from_mesh(part.data);mesh.verts.ensure_lookup_table();bmesh.ops.delete(mesh,geom=[v for v in mesh.verts if v.index not in g],context='VERTS');mesh.to_mesh(part.data);mesh.free();parts.append(part)
 # Keep the other small branch/root components untouched.
 bm=bmesh.new();bm.from_mesh(obj.data);bm.verts.ensure_lookup_table();remove=set.union(*chosen);bmesh.ops.delete(bm,geom=[v for v in bm.verts if v.index in remove],context='VERTS');bm.to_mesh(obj.data);bm.free()
 union=parts[0]
 for other in parts[1:]:
  bpy.ops.object.select_all(action='DESELECT');union.select_set(True);bpy.context.view_layer.objects.active=union;mod=union.modifiers.new('Exact fork volume union','BOOLEAN');mod.operation='UNION';mod.solver='EXACT';mod.object=other;bpy.ops.object.modifier_apply(modifier=mod.name);bpy.data.objects.remove(other,do_unlink=True)
 bm=bmesh.new();bm.from_mesh(union.data);matrix=union.matrix_world.copy();inverse=matrix.inverted()
 original_local={v:v.co.copy()for v in bm.verts}
 for v in bm.verts:v.co=matrix@v.co
 faces=[f for f in bm.faces if min(v.co.z for v in f.verts)<155 and max(v.co.z for v in f.verts)>95];bmesh.ops.triangulate(bm,faces=faces)
 for _ in range(5):
  edges=[e for e in bm.edges if min(v.co.z for v in e.verts)<155 and max(v.co.z for v in e.verts)>95 and e.calc_length()>.75]
  if not edges:break
  bmesh.ops.subdivide_edges(bm,edges=edges,cuts=1,use_grid_fill=True)
 movable=[v for v in bm.verts if 100<v.co.z<150];weights={v:min(1,(v.co.z-100)/7,(150-v.co.z)/7)for v in movable};old={v:v.co.copy()for v in movable}
 for _ in range(150):
  shifts={v:sum((e.other_vert(v).co for e in v.link_edges),Vector())/len(v.link_edges)-v.co for v in movable}
  for v,d in shifts.items():v.co+=d*.3*weights[v]
 stats=dict(nonmanifold=sum(not e.is_manifold for e in bm.edges),degenerate=sum(f.calc_area()<1e-9 for f in bm.faces),moved=len(movable),max_movement=max((v.co-old[v]).length for v in movable));assert stats['nonmanifold']==0 and stats['degenerate']==0,stats
 for v in bm.verts:v.co=original_local[v]if v in original_local and v not in weights else inverse@v.co
 bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));bm.to_mesh(union.data);bm.free();union.data.normals_split_custom_set([(0,0,0)]*len(union.data.loops));bpy.ops.object.select_all(action='DESELECT');obj.select_set(True);union.select_set(True);bpy.context.view_layer.objects.active=obj;bpy.ops.object.join();bpy.context.preferences.filepaths.save_version=0;bpy.ops.wm.save_as_mainfile(filepath=str(out/'model.blend'),compress=True);write_json(out/'construction.json',dict(parent_sha256=sha(source),model_sha256=sha(out/'model.blend'),joint=stats,scope='Private local fork union; exact source coverage/material and outside joint guards pending.'))
if __name__=='__main__':
 acquire()
 try:main()
 finally:release()
