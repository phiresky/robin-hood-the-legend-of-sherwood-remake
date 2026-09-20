"""Give the northwest cottage coherent rounded thatch sections and a ridge cap."""
import math

import bpy
import bmesh
from mathutils import Vector

TAG = 'northwest-cottage-round2-rounded-thatch-v1'
NODES = ('building-065', 'building-066')


def refine_ridge_perimeter():
    """Lower the common ridge two units; retain fixed eaves and shared planes."""
    report=[]
    for obj in bpy.data.collections['Derby Working'].all_objects:
        if obj.type!='MESH' or obj.hide_render or obj.get('source_node') not in NODES:
            continue
        if obj.get('northwest_ridge_perimeter'):
            raise ValueError('Ridge perimeter correction already applied')
        obj.data=obj.data.copy();changed=0
        for vertex in obj.data.vertices:
            if vertex.co.z>70:
                vertex.co.z-=2.0*min(1,(vertex.co.z-70)/51.1);changed+=1
        obj.data.update();obj['northwest_ridge_perimeter']=True
        report.append({'source_node':obj['source_node'],'changed_roof_vertices':changed,
                       'ridge_reduction':2.0,'eave_height_unchanged':70})
    if len(report)!=2:raise ValueError('Expected both roof halves')
    return report


def refine_door_shell():
    """Replace solid half-house wall volumes with joined thin wall assemblies.

    Three regularly spaced, vertical door gaps use shared sill/header heights.
    They open through the front wall rather than terminating in a fake recess.
    The existing roof and applied framing remain separate and unchanged.
    """
    objects = [o for o in bpy.data.collections['Derby Working'].all_objects
               if o.type == 'MESH' and not o.hide_render and o.get('source_node') in NODES]
    if len(objects) != 2:
        raise ValueError('Expected exactly the two cottage source meshes')
    northwest = Vector((469.176,-3132.367,0))
    northeast = Vector((577.957,-3084.875,0))
    southwest = Vector((515.581,-3238.669,0))
    southeast = Vector((624.386,-3191.113,0))
    midwest = Vector((496.417,-3194.767,0))
    mideast = Vector((605.222,-3147.269,0))
    center = (northwest+northeast+southwest+southeast)/4
    report = []
    for obj in objects:
        if obj.get('northwest_open_door_shell'):
            raise ValueError('Open doorway shell already applied')
        mesh=obj.data.copy();obj.data=mesh
        bm=bmesh.new();bm.from_mesh(mesh)
        unseen=set(bm.verts); remove=[]
        while unseen:
            seed=unseen.pop();component={seed};pending=[seed]
            while pending:
                for edge in pending.pop().link_edges:
                    for vertex in edge.verts:
                        if vertex in unseen:
                            unseen.remove(vertex);component.add(vertex);pending.append(vertex)
            low=min(v.co.z for v in component);high=max(v.co.z for v in component)
            span=max(v.co.x for v in component)-min(v.co.x for v in component)
            if low<.1 and abs(high-66)<.02 and span>90:
                remove.extend(component)
        if len(remove)!=8:
            raise ValueError(f'Expected one inherited eight-vertex solid wall volume: {len(remove)}')
        bmesh.ops.delete(bm,geom=remove,context='VERTS')

        def panel(a,b,low=0,high=66):
            tangent=(b-a).normalized();inside=Vector((-tangent.y,tangent.x,0))
            if inside.dot(center-(a+b)/2)<0:inside=-inside
            rings=[]
            for z in (low,high):
                rings.append([bm.verts.new(Vector((p.x,p.y,z))) for p in
                              (a,b,b+inside*2.5,a+inside*2.5)])
            bm.faces.new(rings[0][::-1]);bm.faces.new(rings[1])
            for i in range(4):
                bm.faces.new((rings[0][i],rings[0][(i+1)%4],rings[1][(i+1)%4],rings[1][i]))

        if obj['source_node']=='building-065':
            panel(northwest,northeast)
            panel(midwest,northwest);panel(northeast,mideast)
        else:
            panel(southwest,midwest);panel(mideast,southeast)
            def at(x):return southwest.lerp(southeast,(x-southwest.x)/(southeast.x-southwest.x))
            # The door has parallel openings and common structural rails,
            # not one independently extruded opening per mask pixel.
            boundaries=[southwest.x,593.85,595.15,599.85,601.15,605.85,607.15,southeast.x]
            for index,(left,right) in enumerate(zip(boundaries,boundaries[1:])):
                if index in (1,3,5):
                    panel(at(left),at(right),0,16)
                    panel(at(left),at(right),45,66)
                else:
                    panel(at(left),at(right))
        bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
        bad_edges=sum(not e.is_manifold for e in bm.edges)
        bad_faces=sum(f.calc_area()<1e-7 for f in bm.faces)
        if bad_edges or bad_faces:raise ValueError(f'Wall shell topology: {bad_edges}, {bad_faces}')
        bmesh.ops.triangulate(bm,faces=list(bm.faces));bm.to_mesh(mesh);bm.free();mesh.update()
        obj['northwest_open_door_shell']=True
        report.append({'source_node':obj['source_node'],'removed_solid_wall_vertices':len(remove),
                       'nonmanifold_edges':bad_edges,'degenerate_faces':bad_faces,
                       'door_gap_world_height':[16,45], 'wall_thickness':2.5})
    return report


def _facade_relief(bm):
    """Shallow timbers on a single shared facade plane, from measured source lines.

    Screen points locate visible framing only; every point intersects the same
    vertical wall plane. Depth never comes from an independently fitted pixel.
    """
    a = Vector((515.581, -3238.669, 0))
    b = Vector((624.386, -3191.113, 0))
    tangent = (b-a).normalized()
    normal = Vector((tangent.y, -tangent.x, 0))

    def point(x, y):
        p = a.lerp(b, (x-a.x)/(b.x-a.x))
        p.z = (-p.y*.573576436351046-y)/.819152044288992
        if not 1 <= p.z <= 65:
            raise ValueError(f'Facade anchor outside supported wall: {x}, {y}: {p.z}')
        return p

    def beam(start, end, width, depth=1.0):
        p, q = point(*start), point(*end)
        axis = (q-p).normalized()
        side = normal.cross(axis).normalized()*(width/2)
        rings = [[bm.verts.new(r+s+normal*d) for r,s in
                  ((p,-side),(q,-side),(q,side),(p,side))]
                 for d in (-.15, depth)]
        bm.faces.new(rings[0][::-1]); bm.faces.new(rings[1])
        for i in range(4):
            bm.faces.new((rings[0][i],rings[0][(i+1)%4],
                          rings[1][(i+1)%4],rings[1][i]))

    # Uprights and braces follow the surviving front timber-frame artwork.
    for x, top, bottom, width in ((547,1802,1846,2.3),
                                 (580,1796,1839,2.0),
                                 (616,1788,1828,2.3)):
        beam((x,top),(x,bottom),width)
    beam((520,1828),(544,1845),1.5,.8)
    beam((549,1845),(576,1818),1.5,.8)
    beam((550,1819),(576,1812),1.6,.8)


def refine_facade():
    """Incremental facade pass; call on the validated rounded-roof candidate."""
    objects = [o for o in bpy.data.collections['Derby Working'].all_objects
               if o.type == 'MESH' and not o.hide_render
               and o.get('source_node') == 'building-066']
    if len(objects) != 1:
        raise ValueError('Expected one south facade source')
    obj = objects[0]
    if obj.get('northwest_facade_relief'):
        raise ValueError('Facade relief already applied')
    mesh = obj.data.copy(); obj.data = mesh
    bm = bmesh.new(); bm.from_mesh(mesh)
    _facade_relief(bm)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    bad_edges = sum(not e.is_manifold for e in bm.edges)
    bad_faces = sum(f.calc_area() < 1e-7 for f in bm.faces)
    if bad_edges or bad_faces:
        raise ValueError(f'Facade topology: {bad_edges}, {bad_faces}')
    bmesh.ops.triangulate(bm, faces=list(bm.faces))
    bm.to_mesh(mesh); bm.free(); mesh.update()
    obj['northwest_facade_relief'] = True
    return {'source_node': 'building-066', 'nonmanifold_edges': bad_edges,
            'degenerate_faces': bad_faces, 'faces': len(mesh.polygons)}


def refine():
    objects = [o for o in bpy.data.collections['Derby Working'].all_objects
               if o.type == 'MESH' and not o.hide_render
               and o.get('source_node') in NODES]
    if len(objects) != 2:
        raise ValueError('Expected exactly the two cottage half-shells')
    report = []
    for obj in objects:
        if obj.get('refinement_recipe') == TAG:
            raise ValueError('Recipe must run on the immutable baseline')
        mesh = obj.data.copy()
        obj.data = mesh
        bm = bmesh.new()
        bm.from_mesh(mesh)
        # The footprint, level ridge and straight eave rails are shared controls.
        # Subdivision adds a shallow transverse crown without independently
        # tracing either roof end or changing the wall planes underneath.
        top = [f for f in bm.faces if all(v.co.z > 69.9 for v in f.verts)]
        edges = list({e for f in top for e in f.edges})
        bmesh.ops.subdivide_edges(bm, edges=edges, cuts=7, use_grid_fill=True)
        for v in bm.verts:
            z = v.co.z
            if z >= 69.9:
                t = max(0.0, min(1.0, (z - 70.0) / 51.1))
                v.co.z += 3.0 * math.sin(math.pi * t)
        eaves = [e for e in bm.edges if all(abs(v.co.z-70.0) < .01 for v in e.verts)]
        bmesh.ops.bevel(bm, geom=eaves, offset=.8, segments=3,
                        affect='EDGES', clamp_overlap=True)
        # A small closed roll sits on the observed ridge, never above its ends
        # as independent spikes. It belongs to the north source part only.
        if obj['source_node'] == 'building-065':
            a = Vector((511.74, -3188.805, 120.75))
            b = Vector((596.237, -3151.918, 120.75))
            tangent = (b-a).normalized()
            across = Vector((-tangent.y, tangent.x, 0))
            rings = []
            for center, scale in ((a, .45), (a+tangent*2, 1),
                                  (b-tangent*2, 1), (b, .45)):
                rings.append([bm.verts.new(center + across*(2.4*scale*math.cos(i*math.tau/12))
                    + Vector((0, 0, 1.8*scale*math.sin(i*math.tau/12))))
                    for i in range(12)])
            bm.faces.new(rings[0][::-1])
            bm.faces.new(rings[-1])
            for low, high in zip(rings, rings[1:]):
                for i in range(12):
                    bm.faces.new((low[i], low[(i+1)%12], high[(i+1)%12], high[i]))
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        for f in bm.faces:
            f.smooth = f.normal.z > .1 and all(v.co.z >= 68.9 for v in f.verts)
        for e in bm.edges:
            if e.is_manifold:
                e.smooth = e.calc_face_angle() < .35
        bad_edges = sum(not e.is_manifold for e in bm.edges)
        bad_faces = sum(f.calc_area() < 1e-7 for f in bm.faces)
        if bad_edges or bad_faces:
            raise ValueError(f'{obj.name}: {bad_edges} open edges, {bad_faces} bad faces')
        bmesh.ops.triangulate(bm, faces=list(bm.faces))
        bm.to_mesh(mesh)
        bm.free()
        mesh.update()
        obj['refinement_recipe'] = TAG
        report.append({'source_node': obj['source_node'], 'vertices': len(mesh.vertices),
                       'faces': len(mesh.polygons), 'nonmanifold_edges': bad_edges,
                       'degenerate_faces': bad_faces})
    return report
