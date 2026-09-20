"""Restore the postern scaffold from authored timber silhouettes.

Screen anchors use the covered map's native coordinates. Mask 67 describes the
lower landing frame and mask 69 the taller side frame. Heights come from the
existing landing joins; silhouettes alone do not supply hidden depth.
"""
import math
import bpy
import bmesh
from mathutils import Vector, Matrix

ASSET = 'derby-southwest-postern'
TAG = 'postern-authored-scaffold-round2-v1'


def refine():
    collection = bpy.data.collections['Derby Working']
    owned = [o for o in collection.objects if o.type == 'MESH'
             and o.get('asset_group') == ASSET and not o.hide_render]
    if any(o.get('round2_postern') == TAG for o in owned):
        return {'reused': True}
    parts = {o['source_node']: o for o in owned}
    ladder = parts['building-039']
    if len(ladder.data.vertices) != 176 or len(ladder.data.polygons) != 132:
        raise ValueError('Lower ladder topology changed; re-audit before stripping old braces')
    # The first sixteen cuboids are the two rails and fourteen rungs. The last
    # six were inferred braces occupying the wrong screen-space silhouette.
    mesh = ladder.data.copy()
    ladder.data = mesh
    bm = bmesh.new(); bm.from_mesh(mesh); bm.verts.ensure_lookup_table()
    bmesh.ops.delete(bm, geom=list(bm.verts)[128:], context='VERTS')
    bm.to_mesh(mesh); bm.free(); mesh.update()
    report = {'reused': False, 'removed_misplaced_beams': 6, 'frames': []}
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    # Endpoints traced along the native mask's narrow timber strips. Keep both
    # frames open so original wall pixels cannot be copied across empty space.
    frames = [
        ('building-033', 'Lower landing scaffold', 67,
         (560.5, 2370, 600.5, 2359, 124.521), [
            ((560.5,2370),(560.5,2476),1.7),
            ((600.5,2359),(600.5,2470),1.7),
            ((562,2411),(600,2360),2.1),
            ((561.5,2418),(600.5,2406),1.8),
            ((560.5,2463),(600.5,2459),1.8),
         ]),
        ('building-034', 'Upper landing side scaffold', 69,
         (609.5,2311,628.5,2293,179.456), [
            ((609.5,2312),(609.5,2457),2.1),
            ((628.5,2293),(628.5,2441),1.5),
            ((611.5,2311),(628.5,2337),1.8),
            ((609.5,2360),(626.5,2336),1.8),
            ((609.5,2410),(626.5,2333),1.8),
            ((609.5,2405),(627.5,2382),1.8),
            ((610.5,2410),(628.5,2434),2.0),
            ((609.5,2450),(628.5,2431),2.0),
         ]),
    ]
    for node, label, mask, plane, lines in frames:
        source = parts[node]
        x0,y0,x1,y1,z0 = plane
        def lift(p):
            x,y = p
            join_y = y0+(x-x0)/(x1-x0)*(y1-y0)
            z = z0+(join_y-y)/cosine
            return Vector((x, -(join_y+z0*cosine)/sine, z))
        vertices, faces = [], []
        for a,b,width in lines:
            a,b = lift(a),lift(b)
            direction = (b-a).normalized()
            face_normal = Vector((-(y1-y0)/(x1-x0)/sine,-1,0)).normalized()
            side = direction.cross(face_normal).normalized()*width/2
            depth = direction.cross(side).normalized()*width/2
            start = len(vertices)
            vertices.extend(p+s*side+t*depth for p in (a,b)
                            for s,t in ((-1,-1),(1,-1),(1,1),(-1,1)))
            faces.extend(tuple(start+i for i in f) for f in
                         ((0,3,2,1),(4,5,6,7),(0,1,5,4),
                          (1,2,6,5),(2,3,7,6),(3,0,4,7)))
        mesh = bpy.data.meshes.new(label)
        mesh.from_pydata(vertices,[],faces); mesh.update()
        bm=bmesh.new(); bm.from_mesh(mesh)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bad={'nonmanifold_edges':sum(not e.is_manifold for e in bm.edges),
             'degenerate_faces':sum(f.calc_area()<1e-8 for f in bm.faces)}
        bm.to_mesh(mesh);bm.free()
        if any(bad.values()): raise ValueError(bad)
        uv=mesh.uv_layers.new(name='UVMap')
        for loop in mesh.loops:
            p=mesh.vertices[loop.vertex_index].co
            uv.data[loop.index].uv=(p.x/1920,1+(p.y*sine+p.z*cosine)/2752)
        if source.data.materials:
            mesh.materials.append(source.data.materials[0])
        obj=bpy.data.objects.new('Southwest Postern Tower / '+label,mesh)
        collection.objects.link(obj);obj.parent=source.parent
        obj.matrix_world=Matrix.Identity(4)
        for key in ('source_node','asset_group','asset_name','obstacle_index'):
            if key in source: obj[key]=source[key]
        obj['part_name']=label
        obj['round2_postern']=TAG
        obj['round2_component_role']=label
        obj['source_mask_index']=mask
        obj['source_mask_layer']=0
        report['frames'].append({'node':node,'mask':mask,'beams':len(lines),**bad})
    ladder['round2_postern']=TAG
    bpy.context.view_layer.update()
    return report
