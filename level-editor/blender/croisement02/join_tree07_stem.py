"""Cut one native branch junction and share its exterior loop with the stem."""
import math
import bpy,bmesh
from mathutils import Matrix,Vector
from tree_geometry import COS

def cut_upper(obj):
    original=[obj.matrix_world@v.co for v in obj.data.vertices];attempts=[]
    for cut in (145.,150.,140.,135.):
        bm=bmesh.new();bm.from_mesh(obj.data);bmesh.ops.transform(bm,matrix=obj.matrix_world,verts=list(bm.verts))
        bmesh.ops.bisect_plane(bm,geom=list(bm.verts)+list(bm.edges)+list(bm.faces),dist=.0001,plane_co=(0,0,cut),plane_no=(0,0,1),clear_inner=True)
        edges=[e for e in bm.edges if e.is_boundary];vertices={v for e in edges for v in e.verts}
        connected=set();pending=[next(iter(vertices))] if vertices else []
        while pending:
            v=pending.pop()
            if v in connected:continue
            connected.add(v);pending.extend(e.other_vert(v) for e in v.link_edges if e in edges)
        valid=(len(vertices)>=8 and connected==vertices and all(sum(e in edges for e in v.link_edges)==2 for v in vertices) and all(abs(v.co.z-cut)<.002 for v in vertices))
        attempts.append(dict(cut=cut,boundary_vertices=len(vertices),single_closed_loop=valid))
        if not valid:bm.free();continue
        bmesh.ops.subdivide_edges(bm,edges=edges,cuts=3,use_grid_fill=False)
        edges=[e for e in bm.edges if e.is_boundary];vertices={v for e in edges for v in e.verts}
        cx=sum(v.co.x for v in vertices)/len(vertices);cy=sum(v.co.y for v in vertices)/len(vertices)
        boundary=sorted(vertices,key=lambda v:math.atan2(-(v.co.y-cy)/COS,v.co.x-cx))
        points=[v.co.copy() for v in boundary];angles=[math.atan2(-(p.y-cy)/COS,p.x-cx) for p in points]
        bmesh.ops.holes_fill(bm,edges=edges,sides=0);bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        mesh=bpy.data.meshes.new('Tree07 retained upper junction');bm.to_mesh(mesh);bm.free()
        for mat in obj.data.materials:mesh.materials.append(mat)
        obj.data=mesh;obj.parent=None;obj.matrix_world=Matrix.Identity(4);bpy.context.view_layer.update()
        before={tuple(round(c,5) for c in p) for p in original if p.z>cut+.002};after={tuple(round(c,5) for c in v.co) for v in mesh.vertices if v.co.z>cut+.002}
        if before!=after:raise ValueError('Retained upper junction vertices moved')
        return points,angles,dict(cut=cut,attempts=attempts,retained_upper_vertices=len(before),upper_vertex_positions_identical=True)
    raise ValueError('No single closed upper-junction cut: '+str(attempts))

def share_normals(objects,cut):
    key=lambda p:tuple(round(c,4) for c in p)
    normals={}
    for obj in objects:
        for face in obj.data.polygons:
            points=[obj.matrix_world@obj.data.vertices[i].co for i in face.vertices]
            if all(abs(p.z-cut)<.002 for p in points):continue
            normal=(obj.matrix_world.to_3x3()@face.normal).normalized()*face.area
            for p in points:
                if abs(p.z-cut)<.002:normals[key(p)]=normals.get(key(p),Vector((0,0,0)))+normal
    for obj in objects:
        inverse=obj.matrix_world.to_3x3().inverted();values=[]
        for vertex in obj.data.vertices:
            p=obj.matrix_world@vertex.co;k=key(p)
            values.append(tuple((inverse@normals[k]).normalized()) if abs(p.z-cut)<.002 and k in normals else tuple(vertex.normal))
        obj.data.normals_split_custom_set_from_vertices(values)
    return len(normals)
