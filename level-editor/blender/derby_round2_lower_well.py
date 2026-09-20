"""Source-supported lower well ironwork and coping contacts.

The stone shaft is retained. Curves follow the continuous iron construction in
the artwork; the authored silhouette is a check, not a pixel-staircase mesh.
"""
import math

import bpy
import bmesh
from mathutils import Matrix, Vector

TAG = "lower-well-round2-curved-iron-v1"


def add_ground_pail():
    """Add the separately visible shallow yard vessel, without inferred handles."""
    from mathutils.bvhtree import BVHTree
    working = bpy.data.collections["Derby Working"]
    owned = [o for o in working.all_objects if o.type == "MESH"
             and o.get("source_node") == "building-046" and not o.hide_render]
    if any(o.get("projection_component") == "ground-pail" for o in owned):
        return {"reused": True}
    template = next(o for o in owned if o.get("projection_component") == "bucket")
    def unchanged_geometry():
        return {o.name: ([tuple(o.matrix_world @ v.co) for v in o.data.vertices],
                         [tuple(f.vertices) for f in o.data.polygons]) for o in owned}
    prior_geometry = unchanged_geometry()
    terrain = next(o for o in working.all_objects if o.type == "MESH"
                   and not o.hide_render and o.name.startswith("Derby Terrain /"))
    terrain_points = [terrain.matrix_world @ v.co for v in terrain.data.vertices]
    tree = BVHTree.FromPolygons(terrain_points, [list(f.vertices) for f in terrain.data.polygons])
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    x, y = 829.0, -1761 / sine
    hit, _, _, _ = tree.ray_cast(Vector((x, y, 200)), Vector((0,0,-1)), 2000)
    if hit is None:
        raise ValueError("Ground pail has no terrain support")
    z = hit.z + .02
    y = (-1761-z*cosine)/sine
    vertices, faces = [], []
    # Modest flared open vessel: source resolves a shallow dark hollow and rim.
    profile = [(3.4,0),(5,4.5),(4.15,4.5),(2.7,.65),(0,.65),(0,0)]
    segments = 32
    # Disk centers use a single vertex so there are no degenerate axis quads.
    rings=[]
    for radius, height in profile:
        ring=[]
        for j in range(segments if radius else 1):
            angle=j*math.tau/segments
            ring.append(len(vertices))
            vertices.append((x+radius*math.cos(angle),y+radius*math.sin(angle),z+height))
        rings.append(ring)
    for a,b in zip(rings,rings[1:]+rings[:1]):
        if len(a)==len(b)==1:
            continue
        for j in range(segments):
            n=(j+1)%segments
            faces.append((a[0],b[n],b[j]) if len(a)==1 else
                         (a[j],a[n],b[0]) if len(b)==1 else (a[j],a[n],b[n],b[j]))
    # Separate top and underside center fans close the solid floor volume.
    mesh=bpy.data.meshes.new("Lower well / shallow ground pail")
    mesh.from_pydata(vertices,[],faces)
    bm=bmesh.new();bm.from_mesh(mesh)
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    validation={"nonmanifold_edges":sum(not e.is_manifold for e in bm.edges),
                "degenerate_faces":sum(f.calc_area()<1e-8 for f in bm.faces)}
    bm.to_mesh(mesh);bm.free()
    if any(validation.values()):raise ValueError(validation)
    mesh.materials.append(template.data.materials[0])
    uv=mesh.uv_layers.new(name="UVMap")
    for loop in mesh.loops:
        p=mesh.vertices[loop.vertex_index].co
        uv.data[loop.index].uv=(p.x/1920,1+(p.y*sine+p.z*cosine)/2752)
    obj=bpy.data.objects.new("Lower Bailey Well / Shallow ground pail",mesh)
    working.objects.link(obj)
    for key in template.keys():
        if not key.startswith("reprojection_"):
            obj[key]=template[key]
    obj.parent=template.parent;obj.matrix_world=Matrix.Identity(4)
    obj["projection_component"]="ground-pail"
    obj["round2_component_role"]="ground-pail"
    contacts=[]
    for j in range(segments):
        p=Vector(vertices[j]);ground,_,_,_=tree.ray_cast(p+Vector((0,0,1)),Vector((0,0,-1)),10)
        contacts.append(p.z-ground.z if ground is not None else None)
    if any(gap is None or abs(gap)>.1 for gap in contacts):raise ValueError(contacts)
    if prior_geometry != unchanged_geometry():
        raise ValueError("Ground pail addition changed existing well components")
    return {"reused":False,"validation":validation,"ground_contact_gaps":contacts,
            "existing_components_unchanged":True,
            "source_center":[829,1761],"height":4.5,"rim_radius":5,
            "native_mask66_covers_pail":False}


def split_projection_components():
    """Separate evidence receivers without moving or rebuilding any surface."""
    from collections import Counter
    working = bpy.data.collections["Derby Working"]
    objects = [o for o in working.all_objects if o.type == "MESH"
               and o.get("source_node") == "building-046" and not o.hide_render]
    roles = {o.get("projection_component") for o in objects}
    if len(objects) == len(roles) and roles in ({"shaft", "frame", "bucket"},
                                              {"shaft", "frame", "bucket", "ground-pail"}):
        return {"reused": True}
    if len(objects) != 1 or len(objects[0].data.polygons) != 1196:
        raise ValueError("Component split requires the reviewed 1196-face well")
    original = objects[0]

    def surfaces(items):
        return Counter(tuple(sorted(tuple(o.matrix_world @ o.data.vertices[i].co)
                                    for i in p.vertices))
                       for o in items for p in o.data.polygons)

    before = surfaces(objects)
    roles = [("shaft", 0, 180), ("frame", 180, 1132), ("bucket", 1132, 1196)]
    copies = [original]
    for _ in range(2):
        obj = original.copy()
        obj.data = original.data.copy()
        working.objects.link(obj)
        copies.append(obj)
    original.data = original.data.copy()
    validation = {}
    for obj, (role, start, end) in zip(copies, roles):
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        bm.faces.ensure_lookup_table()
        bmesh.ops.delete(bm, geom=[f for f in bm.faces if not start <= f.index < end], context="FACES")
        validation[role] = {"nonmanifold_edges": sum(not e.is_manifold for e in bm.edges),
                            "degenerate_faces": sum(f.calc_area() < 1e-8 for f in bm.faces)}
        if any(validation[role].values()):
            raise ValueError(validation)
        bm.to_mesh(obj.data)
        bm.free()
        obj["projection_component"] = role
        obj["round2_component_role"] = role
        obj.name = "Lower Bailey Well / " + role.capitalize()
    if surfaces(copies) != before:
        raise ValueError("Projection split changed world-space surfaces")
    return {"reused": False, "exact_world_surfaces": True, "validation": validation}


def refine():
    objects = [o for o in bpy.data.collections["Derby Working"].all_objects
               if o.type == "MESH" and o.get("source_node") == "building-046"
               and not o.hide_render]
    if objects and all(o.get("round2_lower_well") == TAG for o in objects):
        return {"reused": True}
    if len(objects) != 1:
        raise ValueError(f"Expected one visible well, found {len(objects)}")
    obj = objects[0]
    if obj.get("round2_lower_well") == TAG:
        return {"reused": True}
    mesh = obj.data
    points = [obj.matrix_world @ v.co for v in mesh.vertices]
    adjacency = [set() for _ in points]
    for e in mesh.edges:
        a, b = e.vertices
        adjacency[a].add(b)
        adjacency[b].add(a)
    unseen = set(range(len(points)))
    shaft_vertices = set()
    while unseen:
        todo = [unseen.pop()]
        component = set(todo)
        while todo:
            for j in adjacency[todo.pop()]:
                if j in unseen:
                    unseen.remove(j)
                    component.add(j)
                    todo.append(j)
        if max(points[i].x for i in component) - min(points[i].x for i in component) > 16:
            shaft_vertices.update(component)
    if not shaft_vertices:
        raise ValueError("Could not identify connected shaft rings")
    shaft = [points[i] for i in shaft_vertices]
    center = Vector(((min(p.x for p in shaft) + max(p.x for p in shaft)) / 2,
                     (min(p.y for p in shaft) + max(p.y for p in shaft)) / 2,
                     max(p.z for p in shaft)))
    vertices, faces = [], []
    remap = {}
    for i in sorted(shaft_vertices):
        remap[i] = len(vertices)
        vertices.append(points[i].copy())
    for face in mesh.polygons:
        if all(i in shaft_vertices for i in face.vertices):
            faces.append(tuple(remap[i] for i in face.vertices))
    retained_faces = len(faces)
    sine, cosine = math.sin(math.radians(35)), math.cos(math.radians(35))
    # Both frame feet meet the coping at their observed screen height.
    frame_y = (-1727 - center.z * cosine) / sine
    feet_radii = [math.hypot(x-center.x, frame_y-center.y) for x in (800,823)]
    if not all(10.4 < r < 14.7 for r in feet_radii):
        raise ValueError(f"Frame feet do not fit coping: {feet_radii}")

    def pixel(x, y, depth=frame_y):
        return Vector((x, depth, (-depth * sine - y) / cosine))

    def tube(path, radius, sides=8):
        start = len(vertices)
        for i, p in enumerate(path):
            tangent = (path[min(i + 1, len(path) - 1)] - path[max(i - 1, 0)]).normalized()
            u = tangent.cross(Vector((0, 1, 0))).normalized()
            v = tangent.cross(u).normalized()
            for j in range(sides):
                a = j * math.tau / sides
                vertices.append(p + radius * (u * math.cos(a) + v * math.sin(a)))
        faces.append(tuple(start + i for i in reversed(range(sides))))
        for k in range(len(path) - 1):
            for j in range(sides):
                n = (j + 1) % sides
                faces.append((start + k*sides+j, start+k*sides+n,
                              start+(k+1)*sides+n, start+(k+1)*sides+j))
        end = start + (len(path)-1)*sides
        faces.append(tuple(end+i for i in range(sides)))

    def smooth_trace(anchors, radius):
        # Centripetal-like short segments with limited Hermite tangents keep
        # the continuous bends within the source-supported iron silhouette.
        controls = [pixel(*p) for p in anchors]
        path = []
        for i in range(len(controls)-1):
            a, b = controls[i:i+2]
            ma = (b-controls[max(0, i-1)]) * .35
            mb = (controls[min(len(controls)-1, i+2)]-a) * .35
            for step in range(6):
                t = step / 6
                path.append((2*t**3-3*t*t+1)*a + (t**3-2*t*t+t)*ma
                            + (-2*t**3+3*t*t)*b + (t**3-t*t)*mb)
        tube(path+[controls[-1]], radius)

    smooth_trace([(800,1727),(799.5,1718),(798.5,1711),(798,1707),
                  (792.5,1701)], .48)
    smooth_trace([(798,1707),(801,1705),(802.5,1699),(804,1694),
                  (807,1689),(811.5,1686),(815.5,1687.5),(819,1691),
                  (821,1696),(822.5,1699),(825,1698.5),
                  (827.5,1695),(830,1690)], .48)
    smooth_trace([(823,1727),(823,1716),(823.5,1707),(825,1699)], .48)
    tube([pixel(811.5,1686.5), pixel(811.5,1691)], .32)
    tube([pixel(811.5,1691), pixel(811.5,1696.5)], 1.05, 12)
    tube([pixel(811.5,1696.5), pixel(812,1731)], .17)

    # The rear bucket bottom is on the coping; its observed eight-pixel height
    # determines the body height rather than an arbitrary tiny cylinder.
    bucket_y = (-1725-center.z*cosine)/sine
    bucket_z = center.z + .06
    start = len(vertices)
    rings = [(2.2,bucket_z),(3.1,bucket_z+8.5),(2.65,bucket_z+8.5),
             (1.8,bucket_z+.5)]
    for radius, z in rings:
        for j in range(16):
            a = j*math.tau/16
            vertices.append(Vector((808+radius*math.cos(a),bucket_y+radius*math.sin(a),z)))
    for k in range(4):
        for j in range(16):
            n=(j+1)%16
            faces.append((start+k*16+j,start+k*16+n,
                          start+((k+1)%4)*16+n,start+((k+1)%4)*16+j))

    output = bpy.data.meshes.new("Lower well / curved iron and supported bucket")
    output.from_pydata(vertices, [], faces)
    output.update()
    fallback = bpy.data.materials.new("Lower well / neutral source-unknown fallback")
    fallback.diffuse_color = (.22,.22,.22,1)
    fallback.use_nodes = True
    fallback.node_tree.nodes.get("Principled BSDF").inputs["Base Color"].default_value = (.22,.22,.22,1)
    output.materials.append(fallback)
    bm=bmesh.new()
    bm.from_mesh(output)
    bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
    validation={"nonmanifold_edges":sum(not e.is_manifold for e in bm.edges),
                "degenerate_faces":sum(f.calc_area()<1e-8 for f in bm.faces)}
    bm.to_mesh(output)
    bm.free()
    if any(validation.values()):
        raise ValueError(validation)
    uv=output.uv_layers.new(name="UVMap")
    for loop in output.loops:
        p=output.vertices[loop.vertex_index].co
        uv.data[loop.index].uv=(p.x/1920,1+(p.y*sine+p.z*cosine)/2752)
    obj.data=output
    obj.matrix_world=Matrix.Identity(4)
    obj["round2_lower_well"]=TAG
    return {"reused":False,"retained_shaft_faces":retained_faces,
            "faces":len(faces),"validation":validation,
            "coping_z":center.z,"frame_y":frame_y,"feet_radii":feet_radii,
            "bucket_base": [808,bucket_y,bucket_z],
            "limitations":["Rear depth and bucket support require rendered review; no ground pail added"]}
