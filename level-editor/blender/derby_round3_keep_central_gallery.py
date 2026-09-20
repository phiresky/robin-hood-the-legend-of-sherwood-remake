"""Close the keep connector's inherited shell seams without changing its layout.

Run refine() on the reviewed full scene; only central gallery canonical parts
are edited. Hidden obsolete proxies and other architectural assemblies remain.
"""
import bpy
import bmesh

OWNED = {*range(152, 163), 170, 181}


def _write(obj, bm):
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    if bm.calc_volume(signed=True) < 0:
        bmesh.ops.reverse_faces(bm, faces=list(bm.faces))
    mesh = bpy.data.meshes.new(obj.data.name + ' closed assembly')
    bm.to_mesh(mesh)
    for mat in obj.data.materials:
        mesh.materials.append(mat)
    obj.data = mesh
    bm.free()


def _top_prism(obj, bottom):
    """Retain the authored top triangulation, add matching sides and underside."""
    matrix = obj.matrix_world
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    top = [f for f in bm.faces if (matrix.to_3x3() @ f.normal).normalized().z > .9]
    bmesh.ops.delete(bm, geom=[f for f in bm.faces if f not in top], context='FACES')
    bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.8)
    bmesh.ops.dissolve_degenerate(bm, edges=list(bm.edges), dist=.001)
    bm.verts.ensure_lookup_table()
    upper = list(bm.verts)
    faces = list(bm.faces)
    boundary = [tuple(e.verts) for e in bm.edges if e.is_boundary]
    inverse = matrix.inverted()
    lower = {}
    for v in upper:
        p = matrix @ v.co
        p.z = bottom(p) if callable(bottom) else bottom
        lower[v] = bm.verts.new(inverse @ p)
    for face in faces:
        bm.faces.new([lower[v] for v in reversed(face.verts)])
    for a, b in boundary:
        bm.faces.new([a, b, lower[b], lower[a]])
    _write(obj, bm)


def refine():
    rows = []
    for obj in list(bpy.data.objects):
        if obj.type != 'MESH' or obj.hide_render:
            continue
        node = obj.get('source_node', '')
        if node not in {f'building-{n:03}' for n in OWNED}:
            continue
        number = int(node[-3:])
        if number == 156 and 'modeled treads' in obj.name:
            continue
        if obj.get('central_shell_revision') == 1:
            continue
        before = len(obj.data.polygons)
        if number in (153, 158):
            bm = bmesh.new()
            bm.from_mesh(obj.data)
            bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.1)
            low = min((obj.matrix_world @ v.co).z for v in bm.verts)
            edges = [e for e in bm.edges if e.is_boundary and
                     all(abs((obj.matrix_world @ v.co).z-low)<.01 for v in e.verts)]
            adjacency = {}
            for edge in edges:
                a, b = edge.verts
                adjacency.setdefault(a, []).append(b)
                adjacency.setdefault(b, []).append(a)
            start = next((v for v, peers in adjacency.items() if len(peers)==1), next(iter(adjacency)))
            ordered, previous, current = [start], None, start
            while True:
                following = next((v for v in adjacency[current] if v != previous and v != start), None)
                if following is None:
                    break
                ordered.append(following)
                previous, current = current, following
                if len(ordered)>len(adjacency):
                    raise ValueError('Unexpected battlement underside boundary')
            bottom_face = bm.faces.new(ordered)
            caps = bmesh.ops.holes_fill(bm, edges=[e for e in bm.edges if e.is_boundary], sides=0)['faces']
            # Separate the horizontal underside from the vertical attachment;
            # one nonplanar fill across both would cut diagonally into the wall.
            bmesh.ops.triangulate(bm, faces=[bottom_face, *caps])
            _write(obj, bm)
        elif number == 156:
            treads = next(o for o in bpy.data.objects if o.type == 'MESH'
                          and o.get('source_node') == node and 'modeled treads' in o.name)
            bm = bmesh.new()
            bm.from_mesh(treads.data)
            inverse = obj.matrix_world.inverted()
            for vertex in bm.verts:
                p = treads.matrix_world @ vertex.co
                if p.z < 567.65:
                    p.z = 518.83075
                vertex.co = inverse @ p
            _write(obj, bm)
            treads.hide_render = True
            treads.hide_viewport = True
        elif number in (157, 159, 160):
            bottom = {157: 518.83075, 159: 354.0261,
                      160: lambda p: 533.7267 + (p.z - 543.30) * .584}[number]
            _top_prism(obj, bottom)
        else:
            bm = bmesh.new()
            bm.from_mesh(obj.data)
            # Face-island discrepancies are below one source pixel. Welding
            # joins their edges; caps complete previously absent undersides.
            bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=.8)
            if number == 161:
                inverse = obj.matrix_world.inverted()
                for vertex in bm.verts:
                    p = obj.matrix_world @ vertex.co
                    if p.z < 563:
                        p.z -= 9
                        vertex.co = inverse @ p
            caps = bmesh.ops.holes_fill(bm, edges=[e for e in bm.edges if e.is_boundary], sides=0)['faces']
            bmesh.ops.triangulate(bm, faces=caps)
            _write(obj, bm)
        obj['central_shell_revision'] = 1
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        rows.append(dict(node=node, name=obj.name, before_faces=before,
                         faces=len(bm.faces), boundary=sum(e.is_boundary for e in bm.edges),
                         nonmanifold=sum(not e.is_manifold for e in bm.edges),
                         volume=bm.calc_volume(signed=True)))
        bm.free()
    return rows
