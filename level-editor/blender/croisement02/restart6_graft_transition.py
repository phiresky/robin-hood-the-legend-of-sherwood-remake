"""Round the Boolean's thin annular ledge without moving protected upper wood."""
import bmesh,numpy as np
from mathutils import Vector

def blend(obj):
 mesh=obj.data;bm=bmesh.new();bm.from_mesh(mesh);faces=[f for f in bm.faces if min(v.co.z for v in f.verts)>88 and max(v.co.z for v in f.verts)<110];bmesh.ops.triangulate(bm,faces=faces)
 for _ in range(4):
  edges=[e for e in bm.edges if min(v.co.z for v in e.verts)>88 and max(v.co.z for v in e.verts)<110 and e.calc_length()>1]
  if not edges:break
  bmesh.ops.subdivide_edges(bm,edges=edges,cuts=1,use_grid_fill=True)
 movable=[v for v in bm.verts if 90<v.co.z<110];weights={v:min(1,(v.co.z-90)/8,(110-v.co.z)/4)for v in movable};old={v:v.co.copy()for v in movable}
 for _ in range(70):
  shifts={v:sum((e.other_vert(v).co for e in v.link_edges),Vector())/len(v.link_edges)-v.co for v in movable}
  for v,d in shifts.items():v.co+=d*(.45*weights[v])
 bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces));result=dict(nonmanifold=sum(not e.is_manifold for e in bm.edges),degenerate=sum(f.calc_area()<1e-9 for f in bm.faces),moved=len(movable),max_movement=max((v.co-old[v]).length for v in movable));bm.to_mesh(mesh);bm.free();mesh.update();mesh.normals_split_custom_set([(0,0,0)]*len(mesh.loops))
 return result
